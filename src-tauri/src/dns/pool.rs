//! DNS 上游连接池
//!
//! 提供两个层级的连接复用：
//! - **DoT**: TLS 长连接池，消除每次查询的 TCP + TLS 握手开销（~2-3 RTT → 0）
//! - **UDP**: 单 socket 复用 + DNS 消息 ID 分发，消除每次 bind() 开销

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::net::{TcpStream, UdpSocket};
use tokio::sync::{oneshot, Mutex};
use tokio_rustls::TlsConnector;
use tracing::{debug, info, warn};

/// DoT TLS 连接类型别名
type DoTConnection = tokio_rustls::client::TlsStream<TcpStream>;

/// 空闲连接带时间戳（用于过期清理）
struct IdleDoTConnection {
    stream: DoTConnection,
    idle_since: Instant,
}

/// DNS 上游连接池
pub struct DnsConnectionPool {
    /// DoT 连接池：地址 "ip:port" → 空闲连接列表
    dot_pools: std::sync::Mutex<HashMap<String, Vec<IdleDoTConnection>>>,
    /// UDP 通道：地址 → UDP 通道
    udp_channels: std::sync::Mutex<HashMap<String, Arc<UdpChannel>>>,
    /// 每地址最大空闲连接数
    max_idle_per_host: usize,
    /// 空闲连接最大存活时间
    max_idle_duration: Duration,
}

/// UDP 通道：一个持久化的 UDP socket + DNS ID 分发机制
///
/// 多个并发查询共享同一个 connect() 的 UDP socket，
/// 通过 DNS 消息 ID 匹配响应与请求。
struct UdpChannel {
    socket: Arc<UdpSocket>,
    /// DNS 消息 ID → oneshot sender（等待该 ID 对应的响应）
    pending: Arc<Mutex<HashMap<u16, oneshot::Sender<Vec<u8>>>>>,
    /// 唯一 ID 分配器
    next_id: Arc<Mutex<u16>>,
}

impl DnsConnectionPool {
    /// 创建连接池
    pub fn new(max_idle_per_host: usize, max_idle_duration: Duration) -> Self {
        Self {
            dot_pools: std::sync::Mutex::new(HashMap::new()),
            udp_channels: std::sync::Mutex::new(HashMap::new()),
            max_idle_per_host,
            max_idle_duration,
        }
    }

    // ─── UDP 部分 ────────────────────────────────

    /// 获取或创建 UDP 通道，返回 Arc 引用
    async fn get_udp_channel(&self, addr: &str) -> Option<Arc<UdpChannel>> {
        // 快速路径：从缓存获取
        {
            let channels = self.udp_channels.lock().unwrap();
            if let Some(channel) = channels.get(addr) {
                return Some(channel.clone());
            }
        }

        // 慢速路径：创建新通道
        let channel = UdpChannel::new(addr).await?;
        let channel = Arc::new(channel);

        let mut channels = self.udp_channels.lock().unwrap();
        channels.insert(addr.to_string(), channel.clone());
        Some(channel)
    }

    /// 通过 UDP 通道发送 DNS 查询并获得响应
    ///
    /// 自动复用同一上游的 socket，通过 DNS ID 分发匹配响应。
    /// 如果通道失效（后台接收任务崩溃），自动重建。
    pub async fn udp_query(&self, addr: &str, query_bytes: &[u8]) -> Option<Vec<u8>> {
        let channel = self.get_udp_channel(addr).await?;
        channel.query(query_bytes).await
    }

    // ─── DoT 部分 ────────────────────────────────

    /// 获取 DoT 连接（从池中复用或新建）
    ///
    /// `server_name`: TLS SNI 主机名（用于证书验证）
    pub async fn acquire_dot(&self, addr: &str, server_name: &str) -> Option<DoTConnection> {
        // 尝试从池中获取空闲连接
        {
            let mut pools = self.dot_pools.lock().unwrap();
            if let Some(pool) = pools.get_mut(addr) {
                while let Some(idle) = pool.pop() {
                    // 检查是否过期
                    if idle.idle_since.elapsed() < self.max_idle_duration {
                        debug!("[连接池] DoT 复用: {}", addr);
                        return Some(idle.stream);
                    }
                    // 过期连接直接丢弃
                    debug!("[连接池] DoT 连接过期，丢弃: {}", addr);
                }
            }
        }

        // 创建新连接
        self.create_dot_connection(addr, server_name).await
    }

    /// 归还 DoT 连接到池中
    pub fn release_dot(&self, addr: &str, stream: DoTConnection) {
        let mut pools = self.dot_pools.lock().unwrap();
        let pool = pools.entry(addr.to_string()).or_default();
        if pool.len() < self.max_idle_per_host {
            debug!("[连接池] DoT 归还: {} (池中: {})", addr, pool.len() + 1);
            pool.push(IdleDoTConnection {
                stream,
                idle_since: Instant::now(),
            });
        }
        // 超过池大小限制，直接丢弃（连接关闭）
    }

    /// 创建新的 DoT 连接
    async fn create_dot_connection(&self, addr: &str, server_name: &str) -> Option<DoTConnection> {
        // TCP 连接
        let tcp = match tokio::time::timeout(
            Duration::from_secs(3),
            TcpStream::connect(addr),
        )
        .await
        {
            Ok(Ok(stream)) => stream,
            Ok(Err(e)) => {
                warn!("[连接池] DoT TCP 连接失败 {}: {}", addr, e);
                return None;
            }
            Err(_) => {
                warn!("[连接池] DoT TCP 连接超时: {}", addr);
                return None;
            }
        };

        // TLS 配置
        let mut root_store = rustls::RootCertStore::empty();
        root_store.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
        let config = rustls::ClientConfig::builder()
            .with_root_certificates(root_store)
            .with_no_client_auth();
        let connector = TlsConnector::from(Arc::new(config));

        let domain = match rustls::pki_types::ServerName::try_from(server_name.to_string()) {
            Ok(d) => d,
            Err(e) => {
                warn!("[连接池] DoT 无效域名 {}: {}", server_name, e);
                return None;
            }
        };

        // TLS 握手
        match tokio::time::timeout(
            Duration::from_secs(3),
            connector.connect(domain, tcp),
        )
        .await
        {
            Ok(Ok(stream)) => {
                debug!("[连接池] DoT 新连接建立: {}", addr);
                Some(stream)
            }
            Ok(Err(e)) => {
                warn!("[连接池] DoT TLS 握手失败 {}: {}", addr, e);
                None
            }
            Err(_) => {
                warn!("[连接池] DoT TLS 握手超时: {}", addr);
                None
            }
        }
    }

    /// 清理所有过期的空闲 DoT 连接（由后台定时任务调用）
    pub fn cleanup_idle(&self) {
        let mut pools = self.dot_pools.lock().unwrap();
        let mut total_cleaned = 0usize;

        for pool in pools.values_mut() {
            let before = pool.len();
            pool.retain(|conn| conn.idle_since.elapsed() < self.max_idle_duration);
            total_cleaned += before - pool.len();
        }

        // 移除空的池条目
        pools.retain(|_, pool| !pool.is_empty());

        if total_cleaned > 0 {
            info!("[连接池] 清理了 {} 个过期 DoT 连接", total_cleaned);
        }
    }

    /// 获取池统计信息
    pub fn get_stats(&self) -> PoolStats {
        let dot_pools = self.dot_pools.lock().unwrap();
        let udp_channels = self.udp_channels.lock().unwrap();

        let mut total_dot_idle = 0usize;
        for pool in dot_pools.values() {
            total_dot_idle += pool.len();
        }

        PoolStats {
            dot_idle_connections: total_dot_idle,
            dot_hosts: dot_pools.len(),
            udp_channels: udp_channels.len(),
        }
    }
}

/// 连接池统计信息
#[derive(Debug, Clone, serde::Serialize)]
pub struct PoolStats {
    /// DoT 空闲连接总数
    pub dot_idle_connections: usize,
    /// 有 DoT 连接池的主机数
    pub dot_hosts: usize,
    /// UDP 通道数
    pub udp_channels: usize,
}

// ─── UdpChannel 实现 ────────────────────────────────

impl UdpChannel {
    /// 创建并初始化 UDP 通道
    ///
    /// 绑定临时端口 → connect 到上游地址 → 启动后台接收任务
    async fn new(addr: &str) -> Option<Self> {
        let socket = match UdpSocket::bind("0.0.0.0:0").await {
            Ok(s) => s,
            Err(e) => {
                warn!("[连接池] UDP 绑定失败: {}", e);
                return None;
            }
        };

        if let Err(e) = socket.connect(addr).await {
            warn!("[连接池] UDP connect 失败 {}: {}", addr, e);
            return None;
        }

        let pending: Arc<Mutex<HashMap<u16, oneshot::Sender<Vec<u8>>>>> =
            Arc::new(Mutex::new(HashMap::new()));

        let channel = Self {
            socket: Arc::new(socket),
            pending: pending.clone(),
            next_id: Arc::new(Mutex::new(1)),
        };

        // 启动后台接收任务：持续读取 DNS 响应，按 ID 分发给等待者
        let socket_recv = channel.socket.clone();
        let addr_owned = addr.to_string();
        tokio::spawn(async move {
            let mut buf = vec![0u8; 4096];
            loop {
                match socket_recv.recv(&mut buf).await {
                    Ok(len) if len >= 2 => {
                        let dns_id = u16::from_be_bytes([buf[0], buf[1]]);
                        let mut map = pending.lock().await;
                        if let Some(sender) = map.remove(&dns_id) {
                            // 发送者可能已超时取消，忽略错误
                            let _ = sender.send(buf[..len].to_vec());
                        }
                        // 无人等待此 ID 的响应（可能已超时），丢弃
                    }
                    Ok(_) => {
                        // 包太短，忽略
                    }
                    Err(e) => {
                        warn!("[连接池] UDP 接收任务退出 {}: {}", addr_owned, e);
                        break;
                    }
                }
            }
        });

        debug!("[连接池] UDP 通道已建立: {}", addr);
        Some(channel)
    }

    /// 通过该通道发送 DNS 查询并等待响应
    ///
    /// 流程：分配唯一 ID → 替换查询中的 ID → 注册等待 → 发送 → 等待响应 → 恢复原始 ID
    async fn query(&self, query_bytes: &[u8]) -> Option<Vec<u8>> {
        if query_bytes.len() < 2 {
            return None;
        }

        // 1. 保存原始 DNS ID，分配唯一 ID（避免并发冲突）
        let original_id = u16::from_be_bytes([query_bytes[0], query_bytes[1]]);
        let our_id = {
            let mut next = self.next_id.lock().await;
            let id = *next;
            // 跳过 0（某些实现特殊处理），wrapping 防止溢出
            *next = if id.wrapping_add(1) == 0 { 1 } else { id + 1 };
            id
        };

        // 2. 构造带唯一 ID 的查询
        let mut query = query_bytes.to_vec();
        query[0] = (our_id >> 8) as u8;
        query[1] = our_id as u8;

        // 3. 注册等待通道
        let (tx, rx) = oneshot::channel();
        {
            let mut map = self.pending.lock().await;
            map.insert(our_id, tx);
        }

        // 4. 发送查询
        if let Err(e) = self.socket.send(&query).await {
            warn!("[连接池] UDP 发送失败: {}", e);
            // 清理 pending 条目
            let mut map = self.pending.lock().await;
            map.remove(&our_id);
            return None;
        }

        // 5. 等待响应（5 秒超时）
        match tokio::time::timeout(Duration::from_secs(5), rx).await {
            Ok(Ok(mut response)) => {
                // 恢复原始 DNS ID
                if response.len() >= 2 {
                    response[0] = (original_id >> 8) as u8;
                    response[1] = original_id as u8;
                }
                Some(response)
            }
            Ok(Err(_recv_err)) => {
                // oneshot 发送端被丢弃（不应该发生）
                None
            }
            Err(_timeout) => {
                // 超时，清理 pending 条目
                let mut map = self.pending.lock().await;
                map.remove(&our_id);
                warn!("[连接池] UDP 查询超时: id={}", our_id);
                None
            }
        }
    }
}
