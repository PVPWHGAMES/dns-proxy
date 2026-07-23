use crate::config::AppConfig;
use crate::dns::cache::DnsCache;
use crate::dns::handler::DnsHandler;
use crate::dns::DnsQueryLog;
use socket2::{Domain, Protocol, Socket, Type};
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::UdpSocket;
use tokio::sync::Mutex;
use tracing::{error, info, warn};

pub struct DnsServer {
    handler: Arc<DnsHandler>,
    socket: Option<Arc<UdpSocket>>,
    running: Arc<Mutex<bool>>,
    listen_addr: String,
    update_interval_minutes: u64,
}

impl DnsServer {
    pub fn new(config: AppConfig) -> Self {
        let cache = Arc::new(DnsCache::new(
            config.proxy.cache_size,
            std::time::Duration::from_secs(config.proxy.cache_ttl),
        ));
        let handler = Arc::new(DnsHandler::new(config.clone(), cache));
        let listen_addr = format!("{}:{}", config.proxy.listen_address, config.proxy.listen_port);
        let update_interval_minutes = config.subscription_update_interval;

        Self {
            handler,
            socket: None,
            running: Arc::new(Mutex::new(false)),
            listen_addr,
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

        // 尝试绑定端口（使用 socket2 设置 SO_REUSEADDR）
        let socket = match self.bind_socket_with_reuse() {
            Ok(s) => {
                info!("DNS服务器启动在 {}", self.listen_addr);
                Arc::new(s)
            }
            Err(e) => {
                error!("绑定端口失败 {}: {}", self.listen_addr, e);
                // 尝试强制释放端口并重试
                self.force_release_port().await;
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                self.bind_socket_with_reuse()
                    .map(Arc::new)
                    .map_err(|e| anyhow::anyhow!("绑定端口失败，请检查是否有其他程序占用53端口: {}", e))?
            }
        };

        self.socket = Some(socket.clone());
        let mut running = self.running.lock().await;
        *running = true;

        // 启动时自动更新订阅
        let handler_init = self.handler.clone();
        tokio::spawn(async move {
            info!("启动时更新订阅...");
            handler_init.update_subscriptions().await;
        });

        // 启动DNS查询处理
        let handler = self.handler.clone();
        let running_flag = self.running.clone();

        tokio::spawn(async move {
            let mut buf = vec![0u8; 512];
            loop {
                if !*running_flag.lock().await {
                    break;
                }

                match socket.recv_from(&mut buf).await {
                    Ok((len, src_addr)) => {
                        let handler = handler.clone();
                        let query_bytes = buf[..len].to_vec();
                        let socket = socket.clone();

                        tokio::spawn(async move {
                            if let Some(response) = handler.handle_query(&query_bytes, src_addr).await {
                                if let Err(e) = socket.send_to(&response, src_addr).await {
                                    error!("发送DNS响应失败: {}", e);
                                }
                            }
                        });
                    }
                    Err(e) => {
                        if *running_flag.lock().await {
                            error!("接收DNS请求失败: {}", e);
                        }
                    }
                }
            }
        });

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

        Ok(())
    }

    pub async fn stop(&mut self) {
        let mut running = self.running.lock().await;
        if !*running {
            return;
        }
        *running = false;
        self.socket = None;
        info!("DNS服务器已停止");
    }

    async fn stop_internal(&mut self) {
        let mut running = self.running.lock().await;
        *running = false;
        self.socket = None;
    }

    fn bind_socket_with_reuse(&self) -> anyhow::Result<UdpSocket> {
        let addr: SocketAddr = self.listen_addr.parse()?;
        let domain = if addr.is_ipv4() {
            Domain::IPV4
        } else {
            Domain::IPV6
        };

        let socket = Socket::new(domain, Type::DGRAM, Some(Protocol::UDP))?;

        // 设置 SO_REUSEADDR，允许快速重用端口
        socket.set_reuse_address(true)?;
        socket.bind(&addr.into())?;

        // 转换为 tokio UdpSocket
        let std_socket: std::net::UdpSocket = socket.into();
        std_socket.set_nonblocking(true)?;
        let tokio_socket = UdpSocket::from_std(std_socket)?;

        Ok(tokio_socket)
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

    pub fn get_logs(&self) -> Vec<DnsQueryLog> {
        self.handler.get_logs()
    }

    pub fn get_stats(&self) -> (u64, u64, u64, f64) {
        self.handler.get_stats()
    }

    pub fn clear_logs(&self) {
        self.handler.clear_logs();
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
