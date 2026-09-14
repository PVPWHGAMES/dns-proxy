//! FLOW 层只读观察
//!
//! 线程模型：`WinDivertRecv` 是阻塞调用，所以观察循环跑在独立的标准库线程上，
//! 不占用 tokio 的工作线程；停止时用 `WinDivertShutdown` 把它从阻塞里叫醒。
//!
//! 这个模块只观察，不参与转发：句柄以「嗅探 + 只收」模式打开，没有发送路径。

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use super::worker::{lock, Worker};
use super::*;

/// 单次快照返回给界面的上限
///
/// 观察目的下没人会逐条看完上千条流，而每帧都要跨 IPC 传输，超出部分用总数交代即可。
const SNAPSHOT_LIMIT: usize = 300;

/// 跟踪表的安全上限
///
/// 正常靠 `FLOW_DELETED` 摘除；万一某条流的删除事件丢了（进程被强杀等），
/// 这里兜底淘汰最旧的一条，避免观察表无限增长。
const MAX_TRACKED_FLOWS: usize = 2000;

/// 进程名缓存上限
const MAX_PROCESS_CACHE: usize = 512;

/// 连续读失败多少次后判定观察不可继续
const MAX_CONSECUTIVE_ERRORS: u32 = 8;

/// 一条活动流
#[derive(Clone, Debug, serde::Serialize)]
pub struct FlowEntry {
    pub endpoint_id: u64,
    pub process_id: u32,
    pub process_name: String,
    /// `TCP` / `UDP` / `ICMP` / `ICMPv6` / `协议 n`
    pub protocol: String,
    pub local_addr: String,
    pub local_port: u16,
    pub remote_addr: String,
    pub remote_port: u16,
    pub outbound: bool,
    pub loopback: bool,
    /// 本地时间 `HH:MM:SS`
    pub established_at: String,
    #[serde(skip)]
    established_ms: i64,
}

/// 观察状态
#[derive(Clone, Debug, Default, serde::Serialize)]
pub struct FlowMonitorStatus {
    pub running: bool,
    /// WinDivert.dll 是否就位
    pub available: bool,
    /// 失败原因（未提权、缺文件等），界面直接展示
    pub message: Option<String>,
    pub active_count: usize,
    pub total_established: u64,
    pub total_deleted: u64,
    /// 当前活动流涉及多少个不同进程
    pub process_count: usize,
}

#[derive(Default)]
struct MonitorState {
    flows: HashMap<u64, FlowEntry>,
    total_established: u64,
    total_deleted: u64,
    last_error: Option<String>,
}

/// FLOW 层观察器
pub struct FlowMonitor {
    worker: Worker,
    state: Arc<Mutex<MonitorState>>,
}

impl FlowMonitor {
    pub fn new(exe_dir: PathBuf) -> Self {
        Self {
            worker: Worker::new(exe_dir, "windivert-flow"),
            state: Arc::new(Mutex::new(MonitorState::default())),
        }
    }

    /// 开始观察。失败原因（缺文件 / 未提权）直接返回给界面，不做静默降级。
    pub fn start(&self) -> Result<FlowMonitorStatus, String> {
        self.worker.reap_if_finished();
        if self.worker.is_running() {
            return Ok(self.status());
        }

        {
            let mut state = lock(&self.state);
            state.flows.clear();
            state.total_established = 0;
            state.total_deleted = 0;
            state.last_error = None;
        }

        let state = Arc::clone(&self.state);
        self.worker
            .start(
                "true",
                LAYER_FLOW,
                FLAGS_OBSERVE_ONLY,
                move |library, handle, stop| run_loop(library, handle, stop, state),
            )?;

        Ok(self.status())
    }

    /// 停止观察。幂等：未运行时直接返回当前状态。
    pub fn stop(&self) -> FlowMonitorStatus {
        self.worker.stop();
        self.status()
    }

    pub fn status(&self) -> FlowMonitorStatus {
        let available = self.worker.library_available();
        let state = lock(&self.state);
        FlowMonitorStatus {
            running: self.worker.is_running(),
            available,
            message: match (&state.last_error, available) {
                (Some(error), _) => Some(error.clone()),
                (None, false) => Some("未找到 WinDivert.dll，应与程序放在同一目录".to_string()),
                (None, true) => None,
            },
            active_count: state.flows.len(),
            total_established: state.total_established,
            total_deleted: state.total_deleted,
            process_count: state
                .flows
                .values()
                .map(|flow| flow.process_id)
                .collect::<HashSet<_>>()
                .len(),
        }
    }

    /// 当前活动流快照，按建立时间由新到旧
    pub fn snapshot(&self) -> Vec<FlowEntry> {
        let state = lock(&self.state);
        let mut flows: Vec<FlowEntry> = state.flows.values().cloned().collect();
        flows.sort_by(|a, b| {
            b.established_ms
                .cmp(&a.established_ms)
                .then_with(|| a.process_name.cmp(&b.process_name))
                .then_with(|| a.endpoint_id.cmp(&b.endpoint_id))
        });
        flows.truncate(SNAPSHOT_LIMIT);
        flows
    }
}

/// 观察循环：收事件、维护活动流表
fn run_loop(
    library: &Windivert,
    handle: RawHandle,
    stop_flag: &AtomicBool,
    state: Arc<Mutex<MonitorState>>,
) {
    let mut process_names: HashMap<u32, String> = HashMap::new();
    let mut consecutive_errors = 0u32;

    loop {
        let mut address = WindivertAddress::default();
        match library.recv(handle, &mut address) {
            Ok(()) => consecutive_errors = 0,
            Err(code) => {
                // 停止时会以 ERROR_NO_DATA 返回，这是正常退出路径
                if stop_flag.load(Ordering::SeqCst)
                    || code == ERROR_NO_DATA
                    || code == ERROR_INVALID_HANDLE
                {
                    break;
                }

                consecutive_errors += 1;
                if consecutive_errors >= MAX_CONSECUTIVE_ERRORS {
                    lock(&state).last_error = Some(format!("读取流量事件失败（错误 {}）", code));
                    break;
                }
                thread::sleep(Duration::from_millis(20));
                continue;
            }
        }

        let flow = address.flow();
        match address.event() {
            EVENT_FLOW_ESTABLISHED => {
                if process_names.len() >= MAX_PROCESS_CACHE {
                    process_names.clear();
                }
                let process_name = process_names
                    .entry(flow.process_id)
                    .or_insert_with(|| process_name(flow.process_id))
                    .clone();

                let now = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map(|elapsed| elapsed.as_millis() as i64)
                    .unwrap_or_default();
                // 直接用当前本地时间取显示串，而不是把毫秒再转回去：
                // 毫秒只用来排序，显示走最稳定的 API，避免踩 chrono 的版本差异
                let established_at = chrono::Local::now().format("%H:%M:%S").to_string();

                let entry = FlowEntry {
                    endpoint_id: flow.endpoint_id,
                    process_id: flow.process_id,
                    process_name,
                    protocol: protocol_name(flow.protocol),
                    local_addr: library.format_address(&flow.local_addr),
                    local_port: flow.local_port,
                    remote_addr: library.format_address(&flow.remote_addr),
                    remote_port: flow.remote_port,
                    outbound: address.outbound(),
                    loopback: address.loopback(),
                    established_at,
                    established_ms: now,
                };

                let mut state = lock(&state);
                state.total_established += 1;
                if state.flows.len() >= MAX_TRACKED_FLOWS {
                    evict_oldest(&mut state.flows);
                }
                // 同一 endpoint 重复建立时以最新一次为准，避免留下两条同名记录
                state.flows.insert(flow.endpoint_id, entry);
            }
            EVENT_FLOW_DELETED => {
                let mut state = lock(&state);
                if state.flows.remove(&flow.endpoint_id).is_some() {
                    state.total_deleted += 1;
                }
            }
            _ => {}
        }
    }
}

fn evict_oldest(flows: &mut HashMap<u64, FlowEntry>) {
    if let Some(key) = flows
        .iter()
        .min_by_key(|(_, flow)| flow.established_ms)
        .map(|(key, _)| *key)
    {
        flows.remove(&key);
    }
}

fn protocol_name(protocol: u8) -> String {
    match protocol {
        6 => "TCP".to_string(),
        17 => "UDP".to_string(),
        1 => "ICMP".to_string(),
        58 => "ICMPv6".to_string(),
        other => format!("协议 {}", other),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn protocol_names_cover_the_observed_set() {
        assert_eq!(protocol_name(6), "TCP");
        assert_eq!(protocol_name(17), "UDP");
        assert_eq!(protocol_name(58), "ICMPv6");
        assert_eq!(protocol_name(200), "协议 200");
    }

    #[test]
    fn eviction_drops_the_oldest_entry() {
        let mut flows = HashMap::new();
        for (id, millis) in [(1u64, 300i64), (2, 100), (3, 200)] {
            flows.insert(
                id,
                FlowEntry {
                    endpoint_id: id,
                    process_id: 1,
                    process_name: "x".to_string(),
                    protocol: "TCP".to_string(),
                    local_addr: "127.0.0.1".to_string(),
                    local_port: 1,
                    remote_addr: "127.0.0.1".to_string(),
                    remote_port: 2,
                    outbound: true,
                    loopback: true,
                    established_at: "00:00:00".to_string(),
                    established_ms: millis,
                },
            );
        }

        evict_oldest(&mut flows);
        assert_eq!(flows.len(), 2);
        assert!(!flows.contains_key(&2));
    }

    #[test]
    fn status_without_library_is_not_running_and_explains_why() {
        let monitor = FlowMonitor::new(PathBuf::from("Z:\\definitely-not-here"));
        let status = monitor.status();
        assert!(!status.running);
        assert!(!status.available);
        assert!(
            status
                .message
                .as_deref()
                .is_some_and(|message| message.contains("WinDivert.dll")),
            "实际提示：{:?}",
            status.message
        );
        assert!(monitor.snapshot().is_empty());
    }

    #[test]
    fn stop_is_idempotent_when_never_started() {
        let monitor = FlowMonitor::new(PathBuf::from("Z:\\definitely-not-here"));
        assert!(!monitor.stop().running);
        assert!(!monitor.stop().running);
    }
}
