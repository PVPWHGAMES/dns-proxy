use crate::config::AppConfig;
use crate::dns::cache::{CacheStats, DnsCache};
use crate::dns::handler::{DnsHandler, LogFilter, LogPage, TrafficStats};
use crate::dns::DnsQueryLog;
use socket2::{Domain, Protocol, Socket, Type};
use std::net::{IpAddr, Ipv6Addr, SocketAddr};
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream, UdpSocket};
use tokio::sync::{watch, Mutex};
use tracing::{debug, error, info, warn};
use trust_dns_proto::op::{Message, MessageType};
use trust_dns_proto::serialize::binary::{BinDecodable, BinEncodable};

/// Windows 上 UDP socket 收到 ICMP 端口不可达后返回的错误码
const WSAECONNRESET_CODE: i32 = 10054;
/// SIO_UDP_CONNRESET：为 TRUE 时 UDP socket 会把 ICMP 错误上报给 recv
const SIO_UDP_CONNRESET: u32 = 0x9800_000C;

/// 接收 DNS 查询的缓冲大小
///
/// 查询报文本身很小，但带 EDNS0 OPT 与 DNSSEC DO 位时会超过 512 字节，
/// 硬编码 512 会把这类查询截断成无法解析的垃圾。
const QUERY_BUFFER_SIZE: usize = 4096;
/// DNS over TCP 单条报文的长度上限（RFC 1035 用 2 字节长度前缀，理论上限 65535）
const TCP_MAX_MESSAGE_SIZE: usize = 65535;
/// 单条 TCP 连接空闲多久后回收
const TCP_CONNECTION_IDLE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

/// 监听传输协议
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Transport {
    Udp,
    Tcp,
    Both,
}

impl Transport {
    fn from_config(value: &str) -> Self {
        match value {
            "tcp" => Transport::Tcp,
            "both" => Transport::Both,
            _ => Transport::Udp,
        }
    }

    fn listens_udp(self) -> bool {
        matches!(self, Transport::Udp | Transport::Both)
    }

    fn listens_tcp(self) -> bool {
        matches!(self, Transport::Tcp | Transport::Both)
    }
}

/// 按客户端声明的 EDNS0 尺寸决定 UDP 响应是否必须截断
///
/// 客户端没有 OPT 记录时上限是 DNS 默认的 512 字节；返回的响应超过该值时，
/// 必须清空答案并置 TC 位，让客户端改用 TCP 重试（RFC 1035 §4.2.1）。
/// 直接回一个超尺寸报文会被客户端丢弃，表现为「域名解析不了」。
fn udp_response_exceeds_limit(query: &Message, response_len: usize) -> bool {
    response_len > query.max_payload() as usize
}

/// 构造只含查询段的截断响应（TC=1，无答案）
fn build_truncated_response(query: &Message) -> Vec<u8> {
    let mut response = Message::new();
    response.set_id(query.id());
    response.set_message_type(MessageType::Response);
    response.set_op_code(query.op_code());
    response.set_recursion_desired(query.recursion_desired());
    response.set_recursion_available(true);
    response.set_truncated(true);

    if let Some(question) = query.queries().first() {
        response.add_query(question.clone());
    }

    response.to_bytes().unwrap_or_default()
}

pub struct DnsServer {
    handler: Arc<DnsHandler>,
    sockets: Vec<Arc<UdpSocket>>,
    tcp_listeners: Vec<Arc<TcpListener>>,
    /// 通知所有监听循环退出；用 watch 而非 Notify 是为了避免「通知早于等待」丢信号
    shutdown: watch::Sender<bool>,
    running: Arc<Mutex<bool>>,
    listen_addr: String,
    transport: String,
    update_interval_minutes: u64,
}

impl DnsServer {
    /// 构造服务器
    ///
    /// 配置以 `Arc<Mutex<AppConfig>>` 传入并与 `DnsHandler`、界面状态共享同一份实例。
    /// 这里只短暂加锁读出构造期需要的几个字段，不再克隆整份配置 ——
    /// 订阅规则动辄十几万条，克隆一次就是十几 MB，而它只在初始化时需要。
    pub fn new(config: Arc<std::sync::Mutex<AppConfig>>) -> Self {
        let (cache_size, cache_ttl, listen_addr, transport, update_interval_minutes) = {
            let guard = config
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            (
                guard.proxy.cache_size,
                guard.proxy.cache_ttl,
                format!("{}:{}", guard.proxy.listen_address, guard.proxy.listen_port),
                guard.proxy.protocol.clone(),
                guard.subscription_update_interval,
            )
        };

        let cache = Arc::new(DnsCache::new(
            cache_size,
            std::time::Duration::from_secs(cache_ttl),
        ));
        let handler = Arc::new(DnsHandler::new(config, cache));
        let (shutdown, _) = watch::channel(false);

        Self {
            handler,
            sockets: Vec::new(),
            tcp_listeners: Vec::new(),
            shutdown,
            running: Arc::new(Mutex::new(false)),
            listen_addr,
            transport,
            update_interval_minutes,
        }
    }

    pub async fn start(&mut self) -> anyhow::Result<()> {
        {
            let running = self.running.lock().await;
            if *running {
                info!("DNS服务器已在运行");
                return Ok(());
            }
        } // 释放锁后再调用 &mut self 方法

        // 先停止可能存在的旧进程
        self.stop_internal().await;

        // 等待端口释放
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        let transport = Transport::from_config(&self.transport);
        let targets = self.listen_targets();

        let mut sockets: Vec<Arc<UdpSocket>> = Vec::new();
        let mut tcp_listeners: Vec<Arc<TcpListener>> = Vec::new();

        for addr in targets {
            if transport.listens_udp() {
                match self.bind_udp_socket(addr) {
                    Ok(socket) => {
                        info!("DNS服务器监听 UDP {}", addr);
                        sockets.push(Arc::new(socket));
                    }
                    Err(e) => {
                        // IPv4 回环绑定失败属于致命错误；IPv6 可能被系统禁用，仅告警
                        if addr.is_ipv4() {
                            error!("绑定UDP端口失败 {}: {}", addr, e);
                            self.force_release_port().await;
                            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                            let socket = self.bind_udp_socket(addr).map_err(|e| {
                                anyhow::anyhow!(
                                    "绑定UDP端口失败，请检查是否有其他程序占用53端口: {}",
                                    e
                                )
                            })?;
                            sockets.push(Arc::new(socket));
                        } else {
                            warn!("IPv6 UDP 监听地址绑定失败 {}: {}", addr, e);
                        }
                    }
                }
            }

            if transport.listens_tcp() {
                match Self::bind_tcp_listener(addr) {
                    Ok(listener) => {
                        // 配置了 TCP 却绑不上会让「both」静默退化成 UDP-only，
                        // 之后大响应截断重试全部失败，因此这里必须显式报错
                        info!("DNS服务器监听 TCP {}", addr);
                        tcp_listeners.push(Arc::new(listener));
                    }
                    Err(e) => {
                        return Err(anyhow::anyhow!(
                            "绑定TCP监听失败 {}: {}（协议设置为 {}）",
                            addr,
                            e,
                            self.transport
                        ));
                    }
                }
            }
        }

        if sockets.is_empty() && tcp_listeners.is_empty() {
            return Err(anyhow::anyhow!("DNS服务器未能绑定任何监听地址"));
        }

        let udp_count = sockets.len();
        let tcp_count = tcp_listeners.len();
        self.sockets = sockets;
        self.tcp_listeners = tcp_listeners;

        // 重置关闭信号，供本次运行的监听循环使用
        let (shutdown, shutdown_rx) = watch::channel(false);
        self.shutdown = shutdown;

        let mut running = self.running.lock().await;
        *running = true;

        // 启动时自动更新订阅
        let handler_init = self.handler.clone();
        tokio::spawn(async move {
            info!("启动时更新订阅...");
            handler_init.update_subscriptions().await;
        });

        // 每个监听地址一个 UDP 接收循环
        for socket in self.sockets.clone() {
            self.spawn_udp_loop(socket, shutdown_rx.clone());
        }

        // 每个监听地址一个 TCP accept 循环
        for listener in self.tcp_listeners.clone() {
            self.spawn_tcp_accept_loop(listener, shutdown_rx.clone());
        }

        info!(
            "DNS服务器已启动，UDP 监听地址数: {}，TCP 监听地址数: {}",
            udp_count, tcp_count
        );

        // 启动定时更新订阅
        if self.update_interval_minutes > 0 {
            let handler_timer = self.handler.clone();
            let running_timer = self.running.clone();
            let interval_minutes = self.update_interval_minutes;

            tokio::spawn(async move {
                let interval = std::time::Duration::from_secs(interval_minutes * 60);
                info!("定时更新订阅已启用，间隔: {} 分钟", interval_minutes);

                loop {
                    tokio::time::sleep(interval).await;

                    if !*running_timer.lock().await {
                        break;
                    }

                    info!("定时更新订阅...");
                    handler_timer.update_subscriptions().await;
                }
            });
        }

        // 启动定期连接池维护（每 60 秒清理过期连接和缓存）
        {
            let handler_maint = self.handler.clone();
            let running_maint = self.running.clone();

            tokio::spawn(async move {
                let interval = std::time::Duration::from_secs(60);
                info!("连接池定期维护已启用，间隔: 60 秒");

                loop {
                    tokio::time::sleep(interval).await;

                    if !*running_maint.lock().await {
                        break;
                    }

                    handler_maint.cleanup_expired_cache();
                    handler_maint.cleanup_idle_connections();
                }
            });
        }

        Ok(())
    }

    /// 启动单个 UDP 监听循环
    fn spawn_udp_loop(&self, socket: Arc<UdpSocket>, mut shutdown_rx: watch::Receiver<bool>) {
        let handler = self.handler.clone();
        let running_flag = self.running.clone();

        tokio::spawn(async move {
            // 按 EDNS0 协商值定缓冲；这里给足，避免大查询被内核截断
            let mut buf = vec![0u8; QUERY_BUFFER_SIZE];

            loop {
                if !*running_flag.lock().await {
                    break;
                }

                let (len, src_addr) = tokio::select! {
                    _ = shutdown_rx.changed() => break,
                    result = socket.recv_from(&mut buf) => {
                        match result {
                            Ok(value) => value,
                            Err(e) => {
                                if *running_flag.lock().await {
                                    // 10054 是 UDP socket 收到 ICMP 端口不可达后的正常反馈，
                                    // 不是故障，降级记录避免刷屏
                                    if e.raw_os_error() == Some(WSAECONNRESET_CODE) {
                                        debug!("接收DNS请求收到 ICMP 反馈: {}", e);
                                    } else {
                                        error!("接收DNS请求失败: {}", e);
                                    }
                                }
                                continue;
                            }
                        }
                    }
                };

                let query_bytes = buf[..len].to_vec();
                let handler = handler.clone();
                let socket = socket.clone();

                tokio::spawn(async move {
                    let Some(response) = handler.handle_query(&query_bytes).await else {
                        return;
                    };
                    let response = fit_udp_response(&query_bytes, response);
                    if let Err(e) = socket.send_to(&response, src_addr).await {
                        error!("发送DNS响应失败: {}", e);
                    }
                });
            }
        });
    }

    /// 启动单个 TCP accept 循环
    fn spawn_tcp_accept_loop(
        &self,
        listener: Arc<TcpListener>,
        mut shutdown_rx: watch::Receiver<bool>,
    ) {
        let handler = self.handler.clone();
        let running_flag = self.running.clone();

        tokio::spawn(async move {
            loop {
                if !*running_flag.lock().await {
                    break;
                }

                let (stream, peer) = tokio::select! {
                    _ = shutdown_rx.changed() => break,
                    result = listener.accept() => {
                        match result {
                            Ok(value) => value,
                            Err(e) => {
                                if *running_flag.lock().await {
                                    error!("接受DNS TCP连接失败: {}", e);
                                }
                                continue;
                            }
                        }
                    }
                };

                let handler = handler.clone();
                tokio::spawn(async move {
                    if let Err(e) = serve_dns_tcp_stream(stream, handler).await {
                        debug!(peer = %peer, "DNS over TCP 连接结束: {}", e);
                    }
                });
            }
        });
    }

    pub async fn stop(&mut self) {
        let mut running = self.running.lock().await;
        if !*running {
            return;
        }
        *running = false;
        drop(running);

        self.release_listeners();
        info!("DNS服务器已停止");
    }

    /// 不检查运行状态，直接置位并释放监听资源
    async fn stop_internal(&mut self) {
        let mut running = self.running.lock().await;
        *running = false;
        drop(running);
        self.release_listeners();
    }

    /// 通知监听循环退出并释放端口
    ///
    /// 通知必须发生在清空句柄之前：循环持有 socket 的克隆，
    /// 只清空 self 里的引用并不会让它们停止接收或解绑端口。
    fn release_listeners(&mut self) {
        let _ = self.shutdown.send(true);
        self.sockets.clear();
        self.tcp_listeners.clear();
    }

    /// 需要监听的本地地址
    ///
    /// 除配置的监听地址外，额外监听 IPv6 回环 —— 系统网卡的 IPv6 DNS
    /// 常指向 ::1，只监听 IPv4 会让 IPv6 查询没有响应。
    fn listen_targets(&self) -> Vec<SocketAddr> {
        let mut targets = Vec::new();
        let mut port = 53u16;
        if let Ok(configured) = self.listen_addr.parse::<SocketAddr>() {
            port = configured.port();
            targets.push(configured);
        }

        let ipv6_loopback = SocketAddr::new(IpAddr::V6(Ipv6Addr::LOCALHOST), port);
        if !targets.contains(&ipv6_loopback) {
            targets.push(ipv6_loopback);
        }

        targets
    }

    fn bind_udp_socket(&self, addr: SocketAddr) -> anyhow::Result<UdpSocket> {
        let domain = if addr.is_ipv4() {
            Domain::IPV4
        } else {
            Domain::IPV6
        };

        let socket = Socket::new(domain, Type::DGRAM, Some(Protocol::UDP))?;

        // 设置 SO_REUSEADDR，允许快速重用端口
        socket.set_reuse_address(true)?;
        socket.bind(&addr.into())?;
        Self::disable_udp_conn_reset(&socket);

        // 转换为 tokio UdpSocket
        let std_socket: std::net::UdpSocket = socket.into();
        std_socket.set_nonblocking(true)?;
        let tokio_socket = UdpSocket::from_std(std_socket)?;

        Ok(tokio_socket)
    }

    /// 绑定 DNS over TCP 监听
    ///
    /// TCP 响应没有尺寸上限（只有 2 字节长度前缀），大响应与 DNSSEC 场景
    /// 依赖这条通道：UDP 响应被截断置 TC 后，客户端必然改走 TCP。
    fn bind_tcp_listener(addr: SocketAddr) -> anyhow::Result<TcpListener> {
        let domain = if addr.is_ipv4() {
            Domain::IPV4
        } else {
            Domain::IPV6
        };

        let socket = Socket::new(domain, Type::STREAM, Some(Protocol::TCP))?;
        socket.set_reuse_address(true)?;
        socket.bind(&addr.into())?;
        socket.listen(128)?;

        let std_listener: std::net::TcpListener = socket.into();
        std_listener.set_nonblocking(true)?;

        Ok(TcpListener::from_std(std_listener)?)
    }

    /// 关闭 UDP socket 的 ICMP 错误上报
    ///
    /// Windows 上 UDP socket 一旦收到 ICMP 端口不可达，后续 recv 会持续返回
    /// WSAECONNRESET(10054)。DNS 服务每收到一个失败的解析反馈就会刷一条错误日志，
    /// 这里从 socket 层面关闭该上报行为。
    fn disable_udp_conn_reset(socket: &Socket) {
        use std::os::windows::io::AsRawSocket;
        use windows::Win32::Networking::WinSock::{WSAIoctl, SOCKET};

        let disabled: u32 = 0;
        let mut bytes_returned: u32 = 0;
        unsafe {
            WSAIoctl(
                SOCKET(socket.as_raw_socket() as usize),
                SIO_UDP_CONNRESET,
                Some(&disabled as *const u32 as *const std::ffi::c_void),
                std::mem::size_of::<u32>() as u32,
                None,
                0,
                &mut bytes_returned,
                None,
                None,
            );
        }
    }

    async fn force_release_port(&self) {
        info!("尝试强制释放端口...");
        // 使用 netsh 查找并结束占用端口的进程
        let port = self.listen_addr.split(':').last().unwrap_or("53");
        let output = std::process::Command::new("powershell")
            .args([
                "-Command",
                &format!(
                    "Get-NetUDPEndpoint -LocalPort {} -ErrorAction SilentlyContinue | Get-Process -ErrorAction SilentlyContinue",
                    port
                ),
            ])
            .output();

        if let Ok(output) = output {
            let stdout = String::from_utf8_lossy(&output.stdout);
            if !stdout.is_empty() {
                warn!("占用{}端口的进程: {}", port, stdout);
            }
        }
    }

    pub async fn is_running(&self) -> bool {
        *self.running.lock().await
    }

    /// 取最近 limit 条查询日志
    pub fn get_logs(&self, limit: usize) -> Vec<DnsQueryLog> {
        self.handler.get_logs(limit)
    }

    /// 取 id 大于 since_id 的新查询日志
    pub fn get_logs_since(&self, since_id: u64) -> Vec<DnsQueryLog> {
        self.handler.get_logs_since(since_id)
    }

    /// 按条件分页取查询日志
    pub fn get_logs_page(&self, offset: usize, limit: usize, filter: &LogFilter) -> LogPage {
        self.handler.get_logs_page(offset, limit, filter)
    }

    pub fn get_stats(&self) -> (u64, u64, u64, f64) {
        self.handler.get_stats()
    }

    pub fn get_traffic_stats(&self) -> TrafficStats {
        self.handler.get_traffic_stats()
    }

    pub fn clear_logs(&self) {
        self.handler.clear_logs();
    }

    pub fn clear_cache(&self) {
        self.handler.clear_cache();
    }

    pub fn get_cache_stats(&self) -> CacheStats {
        self.handler.get_cache_stats()
    }

    pub fn get_pool_stats(&self) -> crate::dns::pool::PoolStats {
        self.handler.get_pool_stats()
    }

    pub fn cleanup_expired_cache(&self) {
        self.handler.cleanup_expired_cache();
    }

    pub async fn update_subscriptions(&self) {
        self.handler.update_subscriptions().await;
    }

    pub async fn get_config(&self) -> AppConfig {
        self.handler.get_config().await
    }

    pub fn get_dns_handler(&self) -> Arc<DnsHandler> {
        self.handler.clone()
    }
}

/// 让 UDP 响应符合客户端声明的尺寸上限
fn fit_udp_response(query_bytes: &[u8], response: Vec<u8>) -> Vec<u8> {
    let Ok(query) = Message::from_bytes(query_bytes) else {
        // 查询都解析不了时无从判断上限，原样回包由客户端自行处理
        return response;
    };

    if !udp_response_exceeds_limit(&query, response.len()) {
        return response;
    }

    debug!(
        response_len = response.len(),
        max_payload = query.max_payload(),
        "UDP 响应超过客户端声明尺寸，置 TC 位要求走 TCP 重试"
    );
    build_truncated_response(&query)
}

/// 处理一条 DNS over TCP 连接
///
/// RFC 7766 允许在同一连接上串行发送多条查询，因此这里循环处理，
/// 直到对端关闭或空闲超时。
async fn serve_dns_tcp_stream(
    mut stream: TcpStream,
    handler: Arc<DnsHandler>,
) -> anyhow::Result<()> {
    let mut length_buf = [0u8; 2];

    loop {
        let read_len = match tokio::time::timeout(
            TCP_CONNECTION_IDLE_TIMEOUT,
            stream.read_exact(&mut length_buf),
        )
        .await
        {
            Ok(result) => result?,
            Err(_) => return Ok(()),
        };
        if read_len == 0 {
            return Ok(());
        }

        let query_len = u16::from_be_bytes(length_buf) as usize;
        if query_len == 0 || query_len > TCP_MAX_MESSAGE_SIZE {
            return Err(anyhow::anyhow!("DNS TCP 报文长度非法: {}", query_len));
        }

        let mut query = vec![0u8; query_len];
        stream.read_exact(&mut query).await?;

        let Some(response) = handler.handle_query(&query).await else {
            warn!("DNS TCP 查询处理失败，关闭连接");
            return Ok(());
        };

        let mut framed = Vec::with_capacity(response.len() + 2);
        framed.extend_from_slice(&(response.len() as u16).to_be_bytes());
        framed.extend_from_slice(&response);
        stream.write_all(&framed).await?;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Subscription, SubscriptionType};
    use trust_dns_client::rr::{Name, RData, Record, RecordType};
    use trust_dns_proto::op::{Edns, Query};

    /// 生产环境里服务器与处理器共享同一份配置实例，测试同样走这条路径
    fn shared_config(config: AppConfig) -> Arc<std::sync::Mutex<AppConfig>> {
        Arc::new(std::sync::Mutex::new(config))
    }

    /// 被本地黑名单拦下的域名，查询不会触达任何上游
    const BLOCKED_DOMAIN: &str = "ads.example.com";

    /// 构造一份把 BLOCKED_DOMAIN 拦掉的配置
    ///
    /// 命中黑名单的查询在本地直接生成响应，测试因此完全不依赖网络。
    fn blocked_config() -> AppConfig {
        let mut config = AppConfig::default();
        config.subscriptions = vec![Subscription {
            name: "测试黑名单".to_string(),
            url: String::new(),
            enabled: true,
            rules: vec![BLOCKED_DOMAIN.to_string()],
            last_updated: None,
            sub_type: SubscriptionType::Blocklist,
            target_group: None,
        }];
        config
    }

    /// 构造一条查询，`udp_payload` 为 Some 时附带对应尺寸的 EDNS0 OPT
    fn build_query(domain: &str, udp_payload: Option<u16>) -> Vec<u8> {
        let mut message = Message::new();
        message.set_id(0x1234);
        message.set_message_type(MessageType::Query);
        message.set_recursion_desired(true);

        let mut query = Query::new();
        query.set_name(Name::from_ascii(domain).expect("合法域名"));
        query.set_query_type(RecordType::A);
        message.add_query(query);

        if let Some(payload) = udp_payload {
            let mut edns = Edns::new();
            edns.set_max_payload(payload);
            edns.set_version(0);
            message.set_edns(edns);
        }

        message.to_bytes().expect("查询序列化失败")
    }

    /// 构造一条带若干答案的响应
    fn build_response(domain: &str, answer_count: usize) -> Vec<u8> {
        let mut message = Message::new();
        message.set_id(0x1234);
        message.set_message_type(MessageType::Response);
        message.set_recursion_available(true);

        let mut query = Query::new();
        query.set_name(Name::from_ascii(domain).expect("合法域名"));
        query.set_query_type(RecordType::A);
        message.add_query(query);

        for index in 0..answer_count {
            let record = Record::from_rdata(
                Name::from_ascii(domain).expect("合法域名"),
                300,
                RData::A(trust_dns_client::rr::rdata::A(std::net::Ipv4Addr::new(
                    10,
                    0,
                    0,
                    index as u8,
                ))),
            );
            message.add_answer(record);
        }

        message.to_bytes().expect("响应序列化失败")
    }

    /// 服务器与外部持有者必须共用同一份配置实例
    ///
    /// 这是本次改动的核心不变式：过去界面状态与处理器各持一份克隆，
    /// 订阅规则会被复制多份（十几万条字符串，十几 MB 常驻内存）。
    /// 这里从外部改配置，服务器侧必须立刻看到。
    #[tokio::test]
    async fn server_shares_one_config_instance_with_external_holder() {
        let shared = shared_config(AppConfig::default());
        let server = DnsServer::new(shared.clone());

        {
            let mut guard = shared.lock().unwrap();
            guard.proxy.listen_port = 12345;
        }

        assert_eq!(
            server.get_config().await.proxy.listen_port,
            12345,
            "服务器侧看到的必须是同一个配置实例，而不是构造时克隆的副本"
        );
    }

    #[test]
    fn dns_server_listens_on_ipv4_and_ipv6_loopback() {
        // 网卡 IPv6 DNS 也可能指向 ::1，服务必须同时监听两个回环地址
        let server = DnsServer::new(shared_config(AppConfig::default()));
        let targets = server.listen_targets();

        assert!(
            targets.iter().any(|addr| addr.is_ipv4()),
            "应监听 IPv4 地址: {:?}",
            targets
        );
        assert!(
            targets
                .iter()
                .any(|addr| addr.is_ipv6() && addr.ip().is_loopback()),
            "应监听 IPv6 回环: {:?}",
            targets
        );
    }

    #[test]
    fn default_listen_address_is_loopback_only() {
        let server = DnsServer::new(shared_config(AppConfig::default()));
        let targets = server.listen_targets();

        assert!(
            targets.iter().all(|addr| addr.ip().is_loopback()),
            "默认必须只监听回环，实际: {:?}",
            targets
        );
    }

    #[test]
    fn transport_selects_listening_protocols() {
        assert!(Transport::from_config("udp").listens_udp());
        assert!(!Transport::from_config("udp").listens_tcp());

        assert!(!Transport::from_config("tcp").listens_udp());
        assert!(Transport::from_config("tcp").listens_tcp());

        assert!(Transport::from_config("both").listens_udp());
        assert!(Transport::from_config("both").listens_tcp());

        // 非法值退化为 UDP，保证至少有一个可用通道
        assert!(Transport::from_config("nonsense").listens_udp());
        assert!(!Transport::from_config("nonsense").listens_tcp());
    }

    #[test]
    fn response_within_advertised_size_is_sent_as_is() {
        let query = build_query("example.com", Some(4096));
        let response = build_response("example.com", 1);
        let fitted = fit_udp_response(&query, response.clone());

        assert_eq!(fitted, response, "未超限的响应不应被改写");
    }

    #[test]
    fn oversized_response_is_truncated_with_tc_bit() {
        let query = build_query("example.com", None);
        let mut response = build_response("example.com", 1);
        // 手工撑到 512 字节以上，模拟未协商 EDNS0 时的超尺寸响应
        response.resize(600, 0);

        assert!(
            udp_response_exceeds_limit(&Message::from_bytes(&query).unwrap(), response.len()),
            "600 字节响应必须判定为超限"
        );

        let fitted = fit_udp_response(&query, response);
        let message = Message::from_bytes(&fitted).expect("截断响应必须可解析");

        assert!(message.truncated(), "超尺寸响应必须置 TC 位");
        assert!(
            message.answers().is_empty(),
            "截断响应不应携带答案，避免客户端读到半截记录"
        );
        assert_eq!(message.id(), 0x1234, "截断响应必须保留原始事务 ID");
        assert_eq!(message.queries().len(), 1, "截断响应必须回显查询段");
    }

    #[test]
    fn edns_payload_raises_the_truncation_threshold() {
        let response_len = 1200;

        let without_edns = Message::from_bytes(&build_query("example.com", None)).unwrap();
        assert!(
            udp_response_exceeds_limit(&without_edns, response_len),
            "无 EDNS0 时上限是 512 字节"
        );

        let with_edns = Message::from_bytes(&build_query("example.com", Some(4096))).unwrap();
        assert!(
            !udp_response_exceeds_limit(&with_edns, response_len),
            "声明 4096 时 1200 字节响应不需要截断"
        );
    }

    #[test]
    fn response_below_dns_default_limit_is_never_truncated() {
        // max_payload() 下限是 512：声明小于 512 的客户端仍应收到完整小响应
        let query = build_query("example.com", Some(100));
        let response = build_response("example.com", 1);

        assert!(
            !udp_response_exceeds_limit(&Message::from_bytes(&query).unwrap(), response.len()),
            "小响应不应被视为超限"
        );
    }

    /// 在真实 TCP 连接上验证 DNS over TCP 的 2 字节长度前缀收发
    #[tokio::test]
    async fn tcp_query_is_answered_with_length_prefixed_response() {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("绑定测试监听失败");
        let addr = listener.local_addr().unwrap();

        let handler = Arc::new(DnsHandler::new(
            shared_config(blocked_config()),
            Arc::new(DnsCache::new(16, std::time::Duration::from_secs(300))),
        ));
        let server_handler = handler.clone();
        tokio::spawn(async move {
            let (stream, _) = listener.accept().await.expect("接受连接失败");
            let _ = serve_dns_tcp_stream(stream, server_handler).await;
        });

        let query = build_query(BLOCKED_DOMAIN, None);
        let mut client = TcpStream::connect(addr).await.expect("连接测试服务失败");

        let mut request = Vec::with_capacity(query.len() + 2);
        request.extend_from_slice(&(query.len() as u16).to_be_bytes());
        request.extend_from_slice(&query);
        client.write_all(&request).await.expect("发送查询失败");

        let mut length_buf = [0u8; 2];
        tokio::time::timeout(
            std::time::Duration::from_secs(5),
            client.read_exact(&mut length_buf),
        )
        .await
        .expect("等待响应长度超时")
        .expect("读取响应长度失败");

        let response_len = u16::from_be_bytes(length_buf) as usize;
        let mut response = vec![0u8; response_len];
        client
            .read_exact(&mut response)
            .await
            .expect("读取响应失败");

        let message = Message::from_bytes(&response).expect("TCP 响应必须可解析");
        assert_eq!(message.message_type(), MessageType::Response);
        assert_eq!(message.id(), 0x1234);
        assert_eq!(
            message.answers()[0].data().map(|d| d.to_string()),
            Some("0.0.0.0".to_string()),
            "TCP 通道应返回与 UDP 一致的拦截结果"
        );
    }

    /// 关闭后监听循环必须真正退出
    ///
    /// 循环各自持有 socket 的克隆，只清空 self 里的引用不会让它们停止接收。
    /// 这里用「关闭后再查询不应再收到响应」来观测循环是否已退出。
    #[tokio::test]
    async fn releasing_listeners_stops_the_udp_loop() {
        let mut server = DnsServer::new(shared_config(blocked_config()));
        // 监听循环以 running 标志为门控，测试里需显式进入运行态
        *server.running.lock().await = true;

        let socket = Arc::new(
            UdpSocket::bind("127.0.0.1:0")
                .await
                .expect("绑定测试 socket 失败"),
        );
        let addr = socket.local_addr().unwrap();

        let shutdown_rx = server.shutdown.subscribe();
        server.spawn_udp_loop(socket.clone(), shutdown_rx);

        let client = UdpSocket::bind("127.0.0.1:0")
            .await
            .expect("绑定客户端失败");
        let query = build_query(BLOCKED_DOMAIN, None);

        // 关闭前必须能拿到响应，证明循环确实在工作
        client.send_to(&query, addr).await.expect("发送查询失败");
        let mut buf = vec![0u8; QUERY_BUFFER_SIZE];
        let (len, _) = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            client.recv_from(&mut buf),
        )
        .await
        .expect("关闭前应收到响应")
        .expect("接收响应失败");
        assert!(len > 0);

        server.release_listeners();

        // 关闭后循环应已退出：同样的查询不应再被应答
        client.send_to(&query, addr).await.expect("发送查询失败");
        let after_shutdown = tokio::time::timeout(
            std::time::Duration::from_millis(500),
            client.recv_from(&mut buf),
        )
        .await;

        assert!(
            after_shutdown.is_err(),
            "关闭后仍有响应，说明 UDP 监听循环没有退出，端口与任务会被泄漏"
        );
    }
}
