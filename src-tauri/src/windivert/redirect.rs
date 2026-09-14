//! 最小重定向：把指向某个目标的出站 TCP 连接改投到本地中继
//!
//! 采用官方 `streamdump` 演示的「反射」写法，并沿用它的四段映射：
//!
//! | 分支 | 匹配 | 改写 | 方向 |
//! |---|---|---|---|
//! | 1 | 出站 `dst == T:P` | 交换地址 + `dst 端口 = R` | 翻成入站 |
//! | 2 | 出站 `src 端口 == R` | 交换地址 + `src 端口 = P` | 翻成入站 |
//! | 3 | 出站 `dst == T:S` | `dst 端口 = P` | 不变 |
//! | 4 | `src == T:P` | `src 端口 = S` | 不变 |
//!
//! 其中 T:P 是被拦截的目标，R 是本地中继的监听端口，S 是**哨兵端口**。
//!
//! 哨兵端口解决的是自噬问题：中继接受连接后要连回原目标，而那个连接会被过滤器
//! 再抓一次。让中继改拨 `T:S`，由分支 3 把 S 映射回 P，中继的拨号就不会以 P 为
//! 目的端口，环路从设计上消失。分支 4 把服务回包的源端口映射成 S，中继的拨号
//! 套接字才自洽。
//!
//! 地址交换是给真实网卡用的：交换后注入包的源地址携带了"客户端以为的目标"，
//! 中继从 `accept()` 的对端地址就能得知，而且目的地址是客户端本机地址，
//! 不会被协议栈当作 martian 地址丢弃。在回环上交换是恒等操作，退化成纯端口改写。
//!
//! **这是本项目第一个非嗅探句柄**：它会真正把报文从协议栈取走。安全依据有两条，
//! 都已核实：驱动在句柄清理时会把未超时的包重新注入（`windivert_cleanup`），
//! 因此崩溃或退出不会留下黑洞；队列时限（默认 2 秒）之外的包会被丢弃，
//! 所以循环卡死时 TCP 靠重传可恢复、UDP 不可恢复。

use std::net::Ipv4Addr;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use super::worker::{lock, Worker};
use super::*;

/// 连续读失败多少次后判定重定向不可继续
const MAX_CONSECUTIVE_ERRORS: u32 = 8;

/// Windows 动态端口范围的下界
///
/// 中继监听端口落在这个范围里，就有可能与某个客户端的临时源端口撞上，
/// 从而被分支 2 误认为是中继自己发的包。规则校验据此给出提醒。
const EPHEMERAL_PORT_START: u16 = 49152;

/// 一条重定向规则（单目标）
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RedirectRule {
    /// 被拦截的目标地址
    pub target_addr: Ipv4Addr,
    /// 被拦截的目标端口
    pub target_port: u16,
    /// 本地中继的监听端口（被改投到这里）
    pub relay_port: u16,
    /// 哨兵端口：中继拨号时使用，由本规则映射回 `target_port`
    pub sentinel_port: u16,
}

impl RedirectRule {
    /// 校验端口组合是否自洽
    pub fn validate(&self) -> Result<(), String> {
        if self.target_port == self.relay_port {
            return Err("中继端口与目标端口相同，会造成环路".to_string());
        }
        if self.target_port == self.sentinel_port {
            return Err("哨兵端口与目标端口相同，映射会退化成环路".to_string());
        }
        if self.relay_port == self.sentinel_port {
            return Err("中继端口与哨兵端口相同，中继的拨号会被自己的回包分支抢走".to_string());
        }
        if self.relay_port >= EPHEMERAL_PORT_START {
            return Err(format!(
                "中继端口 {} 落在 Windows 动态端口范围内（{}+），可能与客户端临时源端口冲突后误判",
                self.relay_port, EPHEMERAL_PORT_START
            ));
        }
        Ok(())
    }

    /// 过滤器：四个分支，并要求不是我们自己注入的包
    pub fn filter(&self) -> String {
        format!(
            "!impostor and ip and tcp and (\
             (outbound and ip.DstAddr == {target} and tcp.DstPort == {port}) or \
             (outbound and tcp.SrcPort == {relay}) or \
             (outbound and ip.DstAddr == {target} and tcp.DstPort == {sentinel}) or \
             (ip.SrcAddr == {target} and tcp.SrcPort == {port}))",
            target = self.target_addr,
            port = self.target_port,
            relay = self.relay_port,
            sentinel = self.sentinel_port,
        )
    }
}

/// 一个包被归到哪一类
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Rewrite {
    /// 分支 1：客户端 → 中继
    ToRelay,
    /// 分支 2：中继 → 客户端
    ToClient,
    /// 分支 3：中继拨号 → 真实服务
    DialMapped,
    /// 分支 4：真实服务回包 → 中继的拨号套接字
    ReplyMapped,
    /// 命中过滤器但四个分支都不匹配：原样放行，绝不丢弃
    PassThrough,
    /// 结构上不支持（分片、非 IPv4/TCP 等）：原样放行
    Unsupported,
}

/// 按规则就地改写报文
///
/// 单独做成纯函数：改写逻辑是这段代码里最容易出错的部分，而它不需要驱动、
/// 不需要管理员权限就能被完整测到。
pub(crate) fn rewrite(rule: &RedirectRule, packet: &mut [u8]) -> Rewrite {
    if packet.len() < 20 || packet[0] >> 4 != 4 {
        return Rewrite::Unsupported;
    }

    let header_len = ((packet[0] & 0x0F) as usize) * 4;
    if header_len < 20 || packet.len() < header_len + 20 || packet[9] != 6 {
        return Rewrite::Unsupported;
    }

    // 分片包没有完整的 TCP 头：一律不动，也绝不丢弃（掩码含 MF 与分片偏移）
    let flags_and_offset = u16::from_be_bytes([packet[6], packet[7]]);
    if flags_and_offset & 0x3FFF != 0 {
        return Rewrite::Unsupported;
    }

    let src_addr = read_addr(packet, 12);
    let dst_addr = read_addr(packet, 16);
    let src_port = read_port(packet, header_len);
    let dst_port = read_port(packet, header_len + 2);

    // 分支 1：客户端发往被拦截目标
    if dst_addr == rule.target_addr && dst_port == rule.target_port {
        swap_addresses(packet);
        write_port(packet, header_len + 2, rule.relay_port);
        return Rewrite::ToRelay;
    }

    // 分支 2：中继发回客户端
    if src_port == rule.relay_port {
        swap_addresses(packet);
        write_port(packet, header_len, rule.target_port);
        return Rewrite::ToClient;
    }

    // 分支 3：中继拨号用的哨兵端口，映射回真实目标端口
    if dst_addr == rule.target_addr && dst_port == rule.sentinel_port {
        write_port(packet, header_len + 2, rule.target_port);
        return Rewrite::DialMapped;
    }

    // 分支 4：真实服务的回包，源端口映射成哨兵端口
    //
    // 这里刻意不要求入站方向：回环上它以出站形态出现，真实网卡上才是入站。
    // 真实网卡上不存在"源地址等于远端服务器"的出站包，所以放宽方向不会误伤。
    if src_addr == rule.target_addr && src_port == rule.target_port {
        write_port(packet, header_len, rule.sentinel_port);
        return Rewrite::ReplyMapped;
    }

    Rewrite::PassThrough
}

fn read_addr(packet: &[u8], offset: usize) -> Ipv4Addr {
    Ipv4Addr::new(
        packet[offset],
        packet[offset + 1],
        packet[offset + 2],
        packet[offset + 3],
    )
}

fn read_port(packet: &[u8], offset: usize) -> u16 {
    u16::from_be_bytes([packet[offset], packet[offset + 1]])
}

fn write_port(packet: &mut [u8], offset: usize, port: u16) {
    packet[offset..offset + 2].copy_from_slice(&port.to_be_bytes());
}

/// 交换源/目的地址：让注入包携带「客户端以为的目标」，并避免目的地址是回送地址
fn swap_addresses(packet: &mut [u8]) {
    let src = [packet[12], packet[13], packet[14], packet[15]];
    let dst = [packet[16], packet[17], packet[18], packet[19]];
    packet[12..16].copy_from_slice(&dst);
    packet[16..20].copy_from_slice(&src);
}

#[derive(Default)]
struct Counters {
    to_relay: AtomicU64,
    to_client: AtomicU64,
    dial_mapped: AtomicU64,
    reply_mapped: AtomicU64,
    passed_through: AtomicU64,
    skipped: AtomicU64,
    send_failed: AtomicU64,
}

impl Counters {
    fn reset(&self) {
        for counter in [
            &self.to_relay,
            &self.to_client,
            &self.dial_mapped,
            &self.reply_mapped,
            &self.passed_through,
            &self.skipped,
            &self.send_failed,
        ] {
            counter.store(0, Ordering::Relaxed);
        }
    }
}

/// 规则的展示形态
#[derive(Clone, Debug, serde::Serialize)]
pub struct RedirectRuleView {
    pub target: String,
    pub relay_port: u16,
    pub sentinel_port: u16,
}

/// 重定向状态
#[derive(Clone, Debug, Default, serde::Serialize)]
pub struct RedirectStatus {
    pub running: bool,
    pub available: bool,
    pub message: Option<String>,
    pub rule: Option<RedirectRuleView>,
    /// 分支 1：客户端 → 中继
    pub to_relay: u64,
    /// 分支 2：中继 → 客户端
    pub to_client: u64,
    /// 分支 3：中继拨号被映射到真实服务
    pub dial_mapped: u64,
    /// 分支 4：服务回包被映射回哨兵端口
    pub reply_mapped: u64,
    pub passed_through: u64,
    pub skipped: u64,
    pub send_failed: u64,
}

/// 最小重定向器
pub struct Redirector {
    worker: Worker,
    rule: Mutex<Option<RedirectRule>>,
    counters: Arc<Counters>,
    last_error: Arc<Mutex<Option<String>>>,
}

impl Redirector {
    pub fn new(exe_dir: PathBuf) -> Self {
        Self {
            worker: Worker::new(exe_dir, "windivert-redirect"),
            rule: Mutex::new(None),
            counters: Arc::new(Counters::default()),
            last_error: Arc::new(Mutex::new(None)),
        }
    }

    /// 启动重定向
    pub fn start(&self, rule: RedirectRule) -> Result<RedirectStatus, String> {
        self.worker.reap_if_finished();
        if self.worker.is_running() {
            return Err("重定向已在运行，请先停止".to_string());
        }
        rule.validate()?;

        let filter = rule.filter();
        self.counters.reset();
        *lock(&self.last_error) = None;
        *lock(&self.rule) = Some(rule.clone());

        let counters = Arc::clone(&self.counters);
        let last_error = Arc::clone(&self.last_error);
        self.worker
            .start(&filter, LAYER_NETWORK, 0, move |library, handle, stop| {
                run_loop(library, handle, stop, rule, counters, last_error)
            })?;

        Ok(self.status())
    }

    /// 停止重定向并归还所有还在手上的报文
    ///
    /// 句柄关闭时驱动会把未超时的队列内容重新注入协议栈，所以这一步本身就是回滚。
    pub fn stop(&self) -> RedirectStatus {
        self.worker.stop();
        *lock(&self.rule) = None;
        self.status()
    }

    pub fn status(&self) -> RedirectStatus {
        let available = self.worker.library_available();
        let message = lock(&self.last_error).clone();
        RedirectStatus {
            running: self.worker.is_running(),
            available,
            message: match (message, available) {
                (Some(error), _) => Some(error),
                (None, false) => Some("未找到 WinDivert.dll，应与程序放在同一目录".to_string()),
                (None, true) => None,
            },
            rule: lock(&self.rule).as_ref().map(|rule| RedirectRuleView {
                target: format!("{}:{}", rule.target_addr, rule.target_port),
                relay_port: rule.relay_port,
                sentinel_port: rule.sentinel_port,
            }),
            to_relay: self.counters.to_relay.load(Ordering::Relaxed),
            to_client: self.counters.to_client.load(Ordering::Relaxed),
            dial_mapped: self.counters.dial_mapped.load(Ordering::Relaxed),
            reply_mapped: self.counters.reply_mapped.load(Ordering::Relaxed),
            passed_through: self.counters.passed_through.load(Ordering::Relaxed),
            skipped: self.counters.skipped.load(Ordering::Relaxed),
            send_failed: self.counters.send_failed.load(Ordering::Relaxed),
        }
    }
}

/// 重定向循环：收包、改写、注入
fn run_loop(
    library: &Windivert,
    handle: RawHandle,
    stop_flag: &AtomicBool,
    rule: RedirectRule,
    counters: Arc<Counters>,
    last_error: Arc<Mutex<Option<String>>>,
) {
    let mut buffer = vec![0u8; 0xFFFF];
    let mut consecutive_errors = 0u32;

    loop {
        let mut address = WindivertAddress::default();
        let length = match library.recv_packet(handle, &mut buffer, &mut address) {
            Ok(length) => {
                consecutive_errors = 0;
                length
            }
            Err(code) => {
                if stop_flag.load(Ordering::SeqCst)
                    || code == ERROR_NO_DATA
                    || code == ERROR_INVALID_HANDLE
                {
                    break;
                }

                consecutive_errors += 1;
                if consecutive_errors >= MAX_CONSECUTIVE_ERRORS {
                    *lock(&last_error) = Some(format!("读取报文失败（错误 {}）", code));
                    break;
                }
                thread::sleep(Duration::from_millis(20));
                continue;
            }
        };

        let packet = &mut buffer[..length];
        let outcome = rewrite(&rule, packet);

        let counter = match outcome {
            Rewrite::ToRelay => &counters.to_relay,
            Rewrite::ToClient => &counters.to_client,
            Rewrite::DialMapped => &counters.dial_mapped,
            Rewrite::ReplyMapped => &counters.reply_mapped,
            Rewrite::PassThrough => &counters.passed_through,
            Rewrite::Unsupported => &counters.skipped,
        };
        counter.fetch_add(1, Ordering::Relaxed);

        match outcome {
            Rewrite::ToRelay | Rewrite::ToClient => {
                // 非回环包要反射成入站，本机套接字才会接收它。回环包是例外：
                // 文档明确「WinDivert considers loopback packets to be outbound only」，
                // 翻成入站会把它送进不支持回环的路径。
                if !address.loopback() {
                    address.set_outbound(false);
                }
            }
            Rewrite::DialMapped | Rewrite::ReplyMapped => {
                // 这两个分支不改方向：拨号包继续往外走，回包继续往回走
            }
            Rewrite::PassThrough | Rewrite::Unsupported => {
                // 命中过滤器但没能归类：原样送回协议栈，绝不能吞掉
            }
        }

        // 所有注入的包都打上 impostor：过滤器里的 `!impostor` 因此能保证
        // 任何注入结果都不会被自己再抓一次（分支 3 的改写结果恰好会落回分支 1 的条件）
        address.set_impostor(true);
        library.calc_checksums(packet, &mut address);

        if library.send(handle, packet, &address).is_err() {
            counters.send_failed.fetch_add(1, Ordering::Relaxed);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 目标在公网、客户端在私网：这样地址交换才有可观察的效果
    fn rule() -> RedirectRule {
        RedirectRule {
            target_addr: Ipv4Addr::new(93, 184, 216, 34),
            target_port: 80,
            relay_port: 34567,
            sentinel_port: 34568,
        }
    }

    const CLIENT: Ipv4Addr = Ipv4Addr::new(192, 168, 1, 10);

    /// 造一个最小的 IPv4 + TCP 报文（20 字节 IP 头 + 20 字节 TCP 头）
    fn tcp_packet(src: Ipv4Addr, src_port: u16, dst: Ipv4Addr, dst_port: u16) -> Vec<u8> {
        let mut packet = vec![0u8; 40];
        packet[0] = 0x45;
        packet[2..4].copy_from_slice(&40u16.to_be_bytes());
        packet[8] = 64;
        packet[9] = 6;
        packet[12..16].copy_from_slice(&src.octets());
        packet[16..20].copy_from_slice(&dst.octets());
        packet[20..22].copy_from_slice(&src_port.to_be_bytes());
        packet[22..24].copy_from_slice(&dst_port.to_be_bytes());
        packet[32] = 0x50;
        packet[33] = 0x02;
        packet
    }

    fn ports(packet: &[u8]) -> (u16, u16) {
        (read_port(packet, 20), read_port(packet, 22))
    }

    fn addrs(packet: &[u8]) -> (Ipv4Addr, Ipv4Addr) {
        (read_addr(packet, 12), read_addr(packet, 16))
    }

    #[test]
    fn filter_covers_four_branches_and_excludes_our_own_packets() {
        let filter = rule().filter();
        assert!(filter.contains("!impostor"), "{filter}");
        assert!(filter.contains("outbound"), "{filter}");
        assert!(filter.contains("ip.DstAddr == 93.184.216.34 and tcp.DstPort == 80"), "{filter}");
        assert!(filter.contains("tcp.SrcPort == 34567"), "{filter}");
        assert!(filter.contains("tcp.DstPort == 34568"), "{filter}");
        assert!(filter.contains("ip.SrcAddr == 93.184.216.34 and tcp.SrcPort == 80"), "{filter}");
    }

    #[test]
    fn branch_one_sends_the_client_to_the_relay() {
        let mut packet = tcp_packet(CLIENT, 51000, rule().target_addr, 80);
        assert_eq!(rewrite(&rule(), &mut packet), Rewrite::ToRelay);
        // 交换后：源地址携带"客户端以为的目标"，目的地址是客户端本机
        assert_eq!(
            addrs(&packet),
            (rule().target_addr, CLIENT),
            "地址应被交换"
        );
        assert_eq!(
            ports(&packet),
            (51000, 34567),
            "目的端口应变成中继端口，源端口保持客户端临时端口"
        );
    }

    #[test]
    fn branch_two_sends_the_relay_reply_back_as_the_target() {
        // 中继在被接受的那条连接上回包：源 = 客户端本机地址（中继监听 0.0.0.0）:中继端口
        let mut packet = tcp_packet(CLIENT, 34567, rule().target_addr, 51000);
        assert_eq!(rewrite(&rule(), &mut packet), Rewrite::ToClient);
        assert_eq!(addrs(&packet), (rule().target_addr, CLIENT));
        assert_eq!(
            ports(&packet),
            (80, 51000),
            "源端口应还原成真实目标端口，客户端才会认为来自它连的目标"
        );
    }

    #[test]
    fn branch_three_maps_the_relay_dial_to_the_real_service() {
        let mut packet = tcp_packet(CLIENT, 52000, rule().target_addr, 34568);
        assert_eq!(rewrite(&rule(), &mut packet), Rewrite::DialMapped);
        // 地址不动：拨号本来就是要发往真实目标
        assert_eq!(addrs(&packet), (CLIENT, rule().target_addr));
        assert_eq!(ports(&packet), (52000, 80), "哨兵端口应被映射回真实端口");
    }

    #[test]
    fn branch_four_maps_the_service_reply_back_to_the_sentinel() {
        let mut packet = tcp_packet(rule().target_addr, 80, CLIENT, 52000);
        assert_eq!(rewrite(&rule(), &mut packet), Rewrite::ReplyMapped);
        assert_eq!(addrs(&packet), (rule().target_addr, CLIENT));
        assert_eq!(
            ports(&packet),
            (34568, 52000),
            "源端口应变成哨兵端口，中继的拨号套接字才认得来包"
        );
    }

    #[test]
    fn unrelated_packet_is_passed_through_untouched() {
        let mut packet = tcp_packet(CLIENT, 51000, rule().target_addr, 443);
        let original = packet.clone();
        assert_eq!(rewrite(&rule(), &mut packet), Rewrite::PassThrough);
        assert_eq!(packet, original, "不该改动的包必须一个字节都不变");
    }

    #[test]
    fn fragmented_packet_is_left_alone() {
        let mut packet = tcp_packet(CLIENT, 51000, rule().target_addr, 80);
        packet[6] = 0x20; // 置 MF 位：分片包没有完整 TCP 头
        let original = packet.clone();
        assert_eq!(rewrite(&rule(), &mut packet), Rewrite::Unsupported);
        assert_eq!(packet, original);
    }

    #[test]
    fn non_tcp_and_short_packets_are_unsupported() {
        let mut udp = tcp_packet(CLIENT, 51000, rule().target_addr, 80);
        udp[9] = 17;
        let original = udp.clone();
        assert_eq!(rewrite(&rule(), &mut udp), Rewrite::Unsupported);
        assert_eq!(udp, original);

        let mut short = vec![0x45u8; 12];
        assert_eq!(rewrite(&rule(), &mut short), Rewrite::Unsupported);
    }

    #[test]
    fn tcp_options_do_not_break_the_rewrite() {
        // 带选项的 SYN：IP 头仍是 20 字节，但 TCP 头是 32 字节
        let mut packet = tcp_packet(CLIENT, 51000, rule().target_addr, 80);
        packet[32] = 0x80;
        packet.extend_from_slice(&[0x02, 0x04, 0xFF, 0xD7]);
        assert_eq!(rewrite(&rule(), &mut packet), Rewrite::ToRelay);
        assert_eq!(ports(&packet), (51000, 34567));
    }

    #[test]
    fn rule_rejects_port_combinations_that_would_loop() {
        let mut broken = rule();
        broken.relay_port = broken.target_port;
        assert!(broken.validate().is_err(), "中继端口等于目标端口必须被拒");

        let mut broken = rule();
        broken.sentinel_port = broken.target_port;
        assert!(broken.validate().is_err(), "哨兵端口等于目标端口必须被拒");

        let mut broken = rule();
        broken.sentinel_port = broken.relay_port;
        assert!(broken.validate().is_err(), "哨兵端口等于中继端口必须被拒");

        let mut broken = rule();
        broken.relay_port = 50000; // 落在 Windows 动态端口范围内
        assert!(broken.validate().is_err(), "中继端口落在临时端口范围必须被拒");

        assert!(rule().validate().is_ok(), "正常规则应通过校验");
    }

    #[test]
    fn self_referential_rule_is_rejected_before_touching_the_driver() {
        // 运行库不在这个目录里，能拿到校验错误说明它发生在加载之前
        let redirector = Redirector::new(PathBuf::from("Z:\\definitely-not-here"));
        let error = redirector
            .start(RedirectRule {
                target_addr: Ipv4Addr::new(127, 0, 0, 1),
                target_port: 34566,
                relay_port: 34566,
                sentinel_port: 34568,
            })
            .err()
            .expect("自指规则必须被拒绝");
        assert!(error.contains("环路"), "实际错误：{error}");
    }

    #[test]
    fn status_without_library_is_stopped_and_explains_why() {
        let redirector = Redirector::new(PathBuf::from("Z:\\definitely-not-here"));
        let status = redirector.status();
        assert!(!status.running);
        assert!(!status.available);
        assert!(status.rule.is_none());
        assert!(
            status
                .message
                .as_deref()
                .is_some_and(|message| message.contains("WinDivert.dll")),
            "实际提示：{:?}",
            status.message
        );
        assert!(!redirector.stop().running);
    }
}
