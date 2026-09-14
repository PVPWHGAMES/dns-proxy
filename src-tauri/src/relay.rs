//! 本地 TCP 中继
//!
//! 被改投过来的连接在这里落地，再由中继连回原目标。它只负责字节转发，
//! 不解析任何应用层协议——重定向负责把连接送过来，中继负责把数据接出去。
//!
//! 用标准库线程实现而不是 tokio：中继是"一个连接两条流"的模型，线程直写更直观，
//! 也便于在没有任何运行时依赖的情况下写单测。

use std::io::{ErrorKind, Read, Write};
use std::net::{IpAddr, Ipv4Addr, Shutdown, SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::{self, JoinHandle};
use std::time::Duration;

/// 接受连接后连接目标的超时
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// 非阻塞 accept 的轮询间隔
const ACCEPT_POLL: Duration = Duration::from_millis(50);
/// 转发缓冲区
const BUFFER_SIZE: usize = 16 * 1024;

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// 中继的启动参数
pub struct RelayConfig {
    /// 绑定哪张网卡。重定向注入包的目的地址是**客户端本机地址**，
    /// 所以这里要填面向客户端的网卡地址；绑 `0.0.0.0` 等于开放给整个网段。
    pub bind_addr: Ipv4Addr,
    /// 监听端口；传 0 表示由系统分配
    pub listen_port: u16,
    /// 收到连接后要连去的地方（重定向里是目标的哨兵端口）
    pub destination: SocketAddr,
    /// 只接受来自该地址的连接
    ///
    /// 重定向的分支 1 会交换源/目的地址，于是**原目标地址被写进源地址**，
    /// 中继看到的对端必然等于原目标。据此可以把任何别的来源（比如同网段的
    /// 其他机器把我们当中继用）直接拒掉，不需要额外配置。
    pub allowed_peer: Option<IpAddr>,
}

#[derive(Default)]
struct Counters {
    accepted: AtomicU64,
    rejected: AtomicU64,
    active: AtomicU64,
    bytes_up: AtomicU64,
    bytes_down: AtomicU64,
}

#[derive(Default)]
struct RelayState {
    listen_addr: Option<String>,
    destination: Option<String>,
    last_error: Option<String>,
}

/// 中继状态
#[derive(Clone, Debug, Default, serde::Serialize)]
pub struct RelayStatus {
    pub running: bool,
    /// 实际监听的地址（端口传 0 时由系统分配）
    pub listen_addr: Option<String>,
    pub destination: Option<String>,
    pub accepted: u64,
    /// 因来源地址不符而被拒的连接数
    pub rejected: u64,
    pub active: u64,
    pub bytes_up: u64,
    pub bytes_down: u64,
    pub last_error: Option<String>,
}

/// 本地中继
pub struct LocalRelay {
    state: Arc<Mutex<RelayState>>,
    counters: Arc<Counters>,
    stop_flag: Arc<AtomicBool>,
    thread: Mutex<Option<JoinHandle<()>>>,
}

impl LocalRelay {
    pub fn new() -> Self {
        Self {
            state: Arc::new(Mutex::new(RelayState::default())),
            counters: Arc::new(Counters::default()),
            stop_flag: Arc::new(AtomicBool::new(false)),
            thread: Mutex::new(None),
        }
    }

    pub fn start(&self, config: RelayConfig) -> Result<RelayStatus, String> {
        if self.is_running() {
            return Err("中继已在运行，请先停止".to_string());
        }

        let listener = TcpListener::bind((config.bind_addr, config.listen_port)).map_err(|error| {
            format!(
                "监听 {}:{} 失败：{}",
                config.bind_addr, config.listen_port, error
            )
        })?;
        listener
            .set_nonblocking(true)
            .map_err(|error| format!("设置非阻塞失败：{}", error))?;
        let listen_addr = listener
            .local_addr()
            .map_err(|error| format!("读取监听地址失败：{}", error))?;

        {
            let mut state = lock(&self.state);
            state.listen_addr = Some(listen_addr.to_string());
            state.destination = Some(config.destination.to_string());
            state.last_error = None;
        }
        for counter in [
            &self.counters.accepted,
            &self.counters.rejected,
            &self.counters.bytes_up,
            &self.counters.bytes_down,
        ] {
            counter.store(0, Ordering::Relaxed);
        }

        self.stop_flag.store(false, Ordering::SeqCst);
        let stop_flag = Arc::clone(&self.stop_flag);
        let state = Arc::clone(&self.state);
        let counters = Arc::clone(&self.counters);
        let destination = config.destination;
        let allowed_peer = config.allowed_peer;
        let thread = thread::Builder::new()
            .name("local-relay".to_string())
            .spawn(move || {
                accept_loop(
                    listener,
                    destination,
                    allowed_peer,
                    stop_flag,
                    state,
                    counters,
                )
            })
            .map_err(|error| format!("创建中继线程失败：{}", error))?;
        *lock(&self.thread) = Some(thread);

        Ok(self.status())
    }

    /// 停止监听；已建立的连接会自行结束（监听器关闭后不再有新连接）
    pub fn stop(&self) -> RelayStatus {
        self.stop_flag.store(true, Ordering::SeqCst);
        if let Some(thread) = lock(&self.thread).take() {
            let _ = thread.join();
        }
        lock(&self.state).listen_addr = None;
        self.status()
    }

    pub fn is_running(&self) -> bool {
        lock(&self.thread).is_some()
    }

    pub fn status(&self) -> RelayStatus {
        let state = lock(&self.state);
        RelayStatus {
            running: self.is_running(),
            listen_addr: state.listen_addr.clone(),
            destination: state.destination.clone(),
            accepted: self.counters.accepted.load(Ordering::Relaxed),
            rejected: self.counters.rejected.load(Ordering::Relaxed),
            active: self.counters.active.load(Ordering::Relaxed),
            bytes_up: self.counters.bytes_up.load(Ordering::Relaxed),
            bytes_down: self.counters.bytes_down.load(Ordering::Relaxed),
            last_error: state.last_error.clone(),
        }
    }
}

impl Default for LocalRelay {
    fn default() -> Self {
        Self::new()
    }
}

fn accept_loop(
    listener: TcpListener,
    destination: SocketAddr,
    allowed_peer: Option<IpAddr>,
    stop_flag: Arc<AtomicBool>,
    state: Arc<Mutex<RelayState>>,
    counters: Arc<Counters>,
) {
    while !stop_flag.load(Ordering::SeqCst) {
        match listener.accept() {
            Ok((client, peer)) => {
                if let Some(expected) = allowed_peer {
                    if peer.ip() != expected {
                        // 不是我们注入过来的连接：直接关掉，别把自己变成开放中继
                        counters.rejected.fetch_add(1, Ordering::Relaxed);
                        drop(client);
                        continue;
                    }
                }
                counters.accepted.fetch_add(1, Ordering::Relaxed);
                counters.active.fetch_add(1, Ordering::Relaxed);
                let counters = Arc::clone(&counters);
                let state = Arc::clone(&state);
                let _ = thread::Builder::new()
                    .name("local-relay-conn".to_string())
                    .spawn(move || {
                        serve(client, destination, Arc::clone(&counters), Arc::clone(&state));
                        counters.active.fetch_sub(1, Ordering::Relaxed);
                    });
            }
            Err(error) if error.kind() == ErrorKind::WouldBlock => thread::sleep(ACCEPT_POLL),
            Err(error) => {
                lock(&state).last_error = Some(format!("接受连接失败：{}", error));
                thread::sleep(ACCEPT_POLL);
            }
        }
    }
}

/// 处理一条连接：连上目标后在两个方向上转发字节
fn serve(
    client: TcpStream,
    destination: SocketAddr,
    counters: Arc<Counters>,
    state: Arc<Mutex<RelayState>>,
) {
    let upstream = match TcpStream::connect_timeout(&destination, CONNECT_TIMEOUT) {
        Ok(upstream) => upstream,
        Err(error) => {
            lock(&state).last_error = Some(format!("连接 {} 失败：{}", destination, error));
            return;
        }
    };
    let _ = client.set_nodelay(true);
    let _ = upstream.set_nodelay(true);

    let (Ok(client_reader), Ok(upstream_writer)) = (client.try_clone(), upstream.try_clone())
    else {
        lock(&state).last_error = Some("复制套接字句柄失败".to_string());
        return;
    };

    // 上行：调用方 → 目标
    let up_counters = Arc::clone(&counters);
    let up = thread::Builder::new()
        .name("local-relay-up".to_string())
        .spawn(move || {
            pump(client_reader, upstream_writer, &up_counters.bytes_up);
        });

    // 下行：目标 → 调用方（本线程处理）
    pump(upstream, client, &counters.bytes_down);
    if let Ok(handle) = up {
        let _ = handle.join();
    }
}

/// 单向搬运
///
/// 字节数在搬运过程中即时累加：只在连接结束时统计的话，界面上的活动连接
/// 会一直显示 0 字节，看不出它是否真的在传数据。
fn pump(mut from: TcpStream, mut to: TcpStream, counter: &AtomicU64) {
    let mut buffer = vec![0u8; BUFFER_SIZE];
    loop {
        match from.read(&mut buffer) {
            Ok(0) => break,
            Ok(length) => {
                if to.write_all(&buffer[..length]).is_err() {
                    break;
                }
                counter.fetch_add(length as u64, Ordering::Relaxed);
            }
            Err(error) if error.kind() == ErrorKind::Interrupted => continue,
            Err(_) => break,
        }
    }
    // 一个方向结束就收掉两个方向的写端，对端才会看到 EOF
    let _ = to.shutdown(Shutdown::Write);
    let _ = from.shutdown(Shutdown::Read);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(listen_port: u16, destination: SocketAddr) -> RelayConfig {
        RelayConfig {
            bind_addr: Ipv4Addr::LOCALHOST,
            listen_port,
            destination,
            allowed_peer: None,
        }
    }

    #[test]
    fn status_before_start_is_stopped_and_empty() {
        let relay = LocalRelay::new();
        let status = relay.status();
        assert!(!status.running);
        assert!(status.listen_addr.is_none());
        assert_eq!(status.accepted, 0);
        assert!(!relay.stop().running);
    }

    #[test]
    fn starting_twice_is_rejected() {
        // 用一个必然可绑定的端口；第二次 start 必须在绑定之前就被拒
        let relay = LocalRelay::new();
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).expect("探测空闲端口失败");
        let port = listener.local_addr().unwrap().port();
        drop(listener);

        let destination: SocketAddr = "127.0.0.1:9".parse().unwrap();
        relay
            .start(config(port, destination))
            .expect("首次启动应当成功");
        let error = relay
            .start(config(port, destination))
            .err()
            .expect("重复启动必须被拒");
        assert!(error.contains("已在运行"), "实际错误：{error}");
        relay.stop();
    }
}
