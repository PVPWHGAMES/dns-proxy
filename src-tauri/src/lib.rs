mod config;
mod dns;
mod tun;

use config::AppConfig;
use dns::server::DnsServer;
use dns::{DnsQueryLog, DnsStats, TrafficStats};
use std::sync::Arc;
use tauri::menu::{MenuBuilder, MenuItemBuilder};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{Manager, State};
use tokio::sync::Mutex;
use tun::device::TunDevice;
use tun::dns_intercept::DnsInterceptor;
use tun::{TunConfig, TunStatus};

pub struct AppState {
    server: Arc<Mutex<DnsServer>>,
    config: Arc<Mutex<AppConfig>>,
    tun_device: Arc<Mutex<TunDevice>>,
    tun_config: Arc<Mutex<TunConfig>>,
    tun_starting: Arc<Mutex<bool>>,
    latency_results: Arc<Mutex<Vec<DnsLatencyResult>>>,
    latency_last_test: Arc<Mutex<Option<String>>>,
}

#[tauri::command]
async fn get_config(state: State<'_, AppState>) -> Result<AppConfig, String> {
    let config = state.config.lock().await;
    Ok(config.clone())
}

#[tauri::command]
async fn save_config(state: State<'_, AppState>, new_config: AppConfig) -> Result<(), String> {
    let mut config = state.config.lock().await;
    *config = new_config.clone();
    config.save().map_err(|e| e.to_string())?;

    // 重启服务器以应用新配置
    let mut server = state.server.lock().await;
    if server.is_running().await {
        server.stop().await;
        // 等待端口释放
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        *server = DnsServer::new(config.clone());
        server.start().await.map_err(|e| e.to_string())?;
    } else {
        *server = DnsServer::new(config.clone());
    }

    Ok(())
}

#[tauri::command]
async fn start_server(state: State<'_, AppState>) -> Result<(), String> {
    let mut server = state.server.lock().await;
    server.start().await.map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
async fn stop_server(state: State<'_, AppState>) -> Result<(), String> {
    let mut server = state.server.lock().await;
    server.stop().await;
    Ok(())
}

#[tauri::command]
async fn get_server_status(state: State<'_, AppState>) -> Result<bool, String> {
    let server = state.server.lock().await;
    Ok(server.is_running().await)
}

#[tauri::command]
async fn get_stats(state: State<'_, AppState>) -> Result<DnsStats, String> {
    let server = state.server.lock().await;
    let (total, blocked, cached, avg_latency) = server.get_stats();
    let is_running = server.is_running().await;

    Ok(DnsStats {
        total_queries: total,
        blocked_queries: blocked,
        cached_queries: cached,
        avg_latency,
        is_running,
    })
}

#[tauri::command]
async fn get_logs(state: State<'_, AppState>) -> Result<Vec<DnsQueryLog>, String> {
    let server = state.server.lock().await;
    Ok(server.get_logs())
}

#[tauri::command]
async fn clear_logs(state: State<'_, AppState>) -> Result<(), String> {
    let server = state.server.lock().await;
    server.clear_logs();
    Ok(())
}

#[tauri::command]
async fn clear_cache(state: State<'_, AppState>) -> Result<(), String> {
    let server = state.server.lock().await;
    server.clear_cache();
    Ok(())
}

#[tauri::command]
async fn get_traffic_stats(state: State<'_, AppState>) -> Result<TrafficStats, String> {
    let server = state.server.lock().await;
    Ok(server.get_traffic_stats())
}

#[tauri::command]
async fn update_subscriptions(state: State<'_, AppState>) -> Result<String, String> {
    let server = state.server.lock().await;
    server.update_subscriptions().await;

    // 同步更新 AppState 中的配置
    let new_config = server.get_config().await;
    let mut config = state.config.lock().await;
    *config = new_config;

    Ok("订阅已更新".to_string())
}

// TUN 相关命令

#[tauri::command]
async fn get_tun_config(state: State<'_, AppState>) -> Result<TunConfig, String> {
    let config = state.tun_config.lock().await;
    Ok(config.clone())
}

#[tauri::command]
async fn save_tun_config(state: State<'_, AppState>, new_config: TunConfig) -> Result<(), String> {
    let mut tun_config = state.tun_config.lock().await;
    *tun_config = new_config.clone();
    drop(tun_config);

    // 同步保存到主配置文件
    let mut app_config = state.config.lock().await;
    app_config.tun = new_config;
    app_config.save().map_err(|e| e.to_string())?;

    Ok(())
}

#[tauri::command]
async fn start_tun(state: State<'_, AppState>) -> Result<String, String> {
    let config = state.tun_config.lock().await;
    if !config.enabled {
        return Err("TUN模式未启用，请在配置中启用".to_string());
    }
    let tun_config = config.clone();
    drop(config);

    // 检查是否已在启动中
    {
        let starting = state.tun_starting.lock().await;
        if *starting {
            return Ok("TUN正在启动中...".to_string());
        }
    }

    // 设置启动中状态
    {
        let mut starting = state.tun_starting.lock().await;
        *starting = true;
    }

    // 克隆需要的 Arc 字段
    let tun_device = state.tun_device.clone();
    let tun_starting = state.tun_starting.clone();
    let server = state.server.clone();

    // 异步启动TUN
    tokio::spawn(async move {
        let result = start_tun_internal(tun_device.clone(), server, tun_config).await;
        let mut starting = tun_starting.lock().await;
        *starting = false;

        match result {
            Ok(_) => tracing::info!("TUN异步启动完成"),
            Err(e) => tracing::error!("TUN异步启动失败: {}", e),
        }
    });

    Ok("TUN正在启动...".to_string())
}

async fn start_tun_internal(
    tun_device: Arc<Mutex<TunDevice>>,
    server: Arc<Mutex<DnsServer>>,
    tun_config: TunConfig,
) -> Result<(), String> {
    let mut tun = tun_device.lock().await;

    // 更新TUN设备配置
    *tun = TunDevice::new(tun_config);

    // 启动TUN设备
    tun.start().await.map_err(|e| e.to_string())?;

    // 启动DNS拦截器
    let tun_clone = tun_device.clone();
    drop(tun);
    let server = server.lock().await;
    let handler = server.get_dns_handler();
    let interceptor = DnsInterceptor::new(tun_clone);
    interceptor.start(handler).await;

    Ok(())
}

#[tauri::command]
async fn stop_tun(state: State<'_, AppState>) -> Result<String, String> {
    let mut tun = state.tun_device.lock().await;
    tun.stop().await;
    Ok("TUN设备已停止".to_string())
}

#[tauri::command]
async fn get_tun_status(state: State<'_, AppState>) -> Result<TunStatus, String> {
    let tun = state.tun_device.lock().await;
    let config = state.tun_config.lock().await;
    let starting = state.tun_starting.lock().await;

    let active = tun.is_running().await;

    Ok(TunStatus {
        active,
        starting: *starting,
        interface_name: if active { config.interface_name.clone() } else { String::new() },
        ip_address: if active { config.gateway.clone() } else { String::new() },
        dns_redirected: active,
        packets_processed: 0,
    })
}

#[derive(serde::Serialize, Clone)]
struct DnsLatencyResult {
    name: String,
    ip: String,
    latency_ms: Option<u64>,
    error: Option<String>,
}

#[tauri::command]
async fn test_dns_latency(state: State<'_, AppState>) -> Result<Vec<DnsLatencyResult>, String> {
    let config = state.config.lock().await;
    let servers: Vec<_> = config.upstream.iter().filter(|s| s.enabled).cloned().collect();
    drop(config);

    let results = run_latency_test(&servers).await;

    // 保存结果
    {
        let mut saved_results = state.latency_results.lock().await;
        *saved_results = results.clone();
        let mut last_test = state.latency_last_test.lock().await;
        *last_test = Some(chrono::Local::now().format("%H:%M:%S").to_string());
    }

    Ok(results)
}

#[tauri::command]
async fn get_latency_results(state: State<'_, AppState>) -> Result<(Vec<DnsLatencyResult>, Option<String>), String> {
    let results = state.latency_results.lock().await.clone();
    let last_test = state.latency_last_test.lock().await.clone();
    Ok((results, last_test))
}

async fn run_latency_test(servers: &[crate::config::DnsServer]) -> Vec<DnsLatencyResult> {
    let mut results = Vec::new();

    // 创建优化的 HTTP 客户端（启用 HTTP/2、连接池）
    let http_client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(3))
        .pool_max_idle_per_host(8)
        .pool_idle_timeout(std::time::Duration::from_secs(90))
        .http2_prior_knowledge()
        .build()
        .unwrap_or_default();

    for server in servers {
        let name = server.name.clone();
        let ip = server.ip.clone();
        let addr = format!("{}:{}", server.ip, server.port);

        // 构造DNS查询请求 (查询 example.com A记录)
        let query = build_dns_query("example.com");

        let start = std::time::Instant::now();
        let result = match server.protocol {
            crate::config::DnsProtocol::Udp | crate::config::DnsProtocol::Tcp => {
                // UDP/TCP 测试
                match tokio::net::UdpSocket::bind("0.0.0.0:0").await {
                    Ok(socket) => {
                        if let Err(e) = socket.connect(&addr).await {
                            Err(format!("连接失败: {}", e))
                        } else {
                            match socket.send(&query).await {
                                Ok(_) => {
                                    let mut buf = vec![0u8; 512];
                                    match tokio::time::timeout(
                                        std::time::Duration::from_secs(2),
                                        socket.recv(&mut buf),
                                    )
                                    .await
                                    {
                                        Ok(Ok(len)) => Ok(buf[..len].to_vec()),
                                        Ok(Err(e)) => Err(format!("接收失败: {}", e)),
                                        Err(_) => Err("超时".to_string()),
                                    }
                                }
                                Err(e) => Err(format!("发送失败: {}", e)),
                            }
                        }
                    }
                    Err(e) => Err(format!("绑定失败: {}", e)),
                }
            }
            crate::config::DnsProtocol::Doh => {
                // DoH 测试 - 复用连接池
                let url = server
                    .doh_url
                    .as_deref()
                    .unwrap_or("https://cloudflare-dns.com/dns-query");
                match http_client
                    .post(url)
                    .header("Content-Type", "application/dns-message")
                    .header("Accept", "application/dns-message")
                    .body(query.clone())
                    .send()
                    .await
                {
                    Ok(resp) => match resp.bytes().await {
                        Ok(bytes) => Ok(bytes.to_vec()),
                        Err(e) => Err(format!("读取响应失败: {}", e)),
                    },
                    Err(e) => Err(format!("请求失败: {}", e)),
                }
            }
            crate::config::DnsProtocol::Dot => {
                // DoT 测试 - 使用 TLS 连接
                let dot_port = if server.port == 53 { 853 } else { server.port };
                test_dot_connection(&server.ip, dot_port, &query).await
            }
        };

        let latency = start.elapsed().as_millis() as u64;

        match result {
            Ok(_) => {
                results.push(DnsLatencyResult {
                    name,
                    ip,
                    latency_ms: Some(latency),
                    error: None,
                });
            }
            Err(e) => {
                results.push(DnsLatencyResult {
                    name,
                    ip,
                    latency_ms: None,
                    error: Some(e),
                });
            }
        }
    }

    // 按延迟排序
    results.sort_by(|a, b| {
        a.latency_ms
            .unwrap_or(u64::MAX)
            .cmp(&b.latency_ms.unwrap_or(u64::MAX))
    });

    results
}

/// 测试 DoT (DNS-over-TLS) 连接
async fn test_dot_connection(ip: &str, port: u16, query: &[u8]) -> Result<Vec<u8>, String> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpStream;
    use tokio_rustls::TlsConnector;

    let addr = format!("{}:{}", ip, port);

    // 建立 TCP 连接
    let tcp = match tokio::time::timeout(std::time::Duration::from_secs(3), TcpStream::connect(&addr)).await {
        Ok(Ok(stream)) => stream,
        Ok(Err(e)) => return Err(format!("TCP连接失败: {}", e)),
        Err(_) => return Err("TCP连接超时".to_string()),
    };

    // 配置 TLS
    let mut root_store = rustls::RootCertStore::empty();
    root_store.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    let config = rustls::ClientConfig::builder()
        .with_root_certificates(root_store)
        .with_no_client_auth();

    let connector = TlsConnector::from(std::sync::Arc::new(config));

    // TLS 握手
    let domain = rustls::pki_types::ServerName::try_from(ip.to_string())
        .map_err(|e| format!("无效域名: {}", e))?;

    let mut tls = match tokio::time::timeout(std::time::Duration::from_secs(3), connector.connect(domain, tcp)).await {
        Ok(Ok(stream)) => stream,
        Ok(Err(e)) => return Err(format!("TLS握手失败: {}", e)),
        Err(_) => return Err("TLS握手超时".to_string()),
    };

    // 发送 DNS 查询（DoT 使用 TCP 格式：2字节长度前缀 + 查询数据）
    let len = (query.len() as u16).to_be_bytes();
    tls.write_all(&len).await.map_err(|e| format!("发送长度失败: {}", e))?;
    tls.write_all(query).await.map_err(|e| format!("发送查询失败: {}", e))?;

    // 读取响应长度
    let mut len_buf = [0u8; 2];
    tls.read_exact(&mut len_buf).await.map_err(|e| format!("读取响应长度失败: {}", e))?;
    let resp_len = u16::from_be_bytes(len_buf) as usize;

    if resp_len > 4096 {
        return Err(format!("响应长度异常: {}", resp_len));
    }

    // 读取响应数据
    let mut resp_buf = vec![0u8; resp_len];
    match tokio::time::timeout(std::time::Duration::from_secs(3), tls.read_exact(&mut resp_buf)).await {
        Ok(Ok(_)) => Ok(resp_buf),
        Ok(Err(e)) => Err(format!("读取响应失败: {}", e)),
        Err(_) => Err("读取响应超时".to_string()),
    }
}

fn build_dns_query(domain: &str) -> Vec<u8> {
    // 简单构造DNS查询包
    let mut packet = Vec::new();

    // Transaction ID
    packet.extend_from_slice(&[0x12, 0x34]);

    // Flags: standard query
    packet.extend_from_slice(&[0x01, 0x00]);

    // Questions: 1
    packet.extend_from_slice(&[0x00, 0x01]);

    // Answer RRs: 0
    packet.extend_from_slice(&[0x00, 0x00]);

    // Authority RRs: 0
    packet.extend_from_slice(&[0x00, 0x00]);

    // Additional RRs: 0
    packet.extend_from_slice(&[0x00, 0x00]);

    // Query name
    for part in domain.split('.') {
        packet.push(part.len() as u8);
        packet.extend_from_slice(part.as_bytes());
    }
    packet.push(0); // root label

    // Query type: A (1)
    packet.extend_from_slice(&[0x00, 0x01]);

    // Query class: IN (1)
    packet.extend_from_slice(&[0x00, 0x01]);

    packet
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let config = AppConfig::load();
    let server = DnsServer::new(config.clone());
    let tun_config = config.tun.clone();
    let tun_device = TunDevice::new(tun_config.clone());

    let latency_results = Arc::new(Mutex::new(Vec::new()));
    let latency_last_test = Arc::new(Mutex::new(None));

    let state = AppState {
        server: Arc::new(Mutex::new(server)),
        config: Arc::new(Mutex::new(config)),
        tun_device: Arc::new(Mutex::new(tun_device)),
        tun_config: Arc::new(Mutex::new(tun_config)),
        tun_starting: Arc::new(Mutex::new(false)),
        latency_results: latency_results.clone(),
        latency_last_test: latency_last_test.clone(),
    };

    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .manage(state)
        .setup(|app| {
            // 创建系统托盘菜单
            let dns_status = MenuItemBuilder::with_id("dns_status", "DNS: 检查中...")
                .enabled(false)
                .build(app)?;
            let tun_status = MenuItemBuilder::with_id("tun_status", "TUN: 检查中...")
                .enabled(false)
                .build(app)?;
            let latency_info = MenuItemBuilder::with_id("latency_info", "延迟: 未测试")
                .enabled(false)
                .build(app)?;
            let restart_dns = MenuItemBuilder::with_id("restart_dns", "重启 DNS 服务")
                .build(app)?;
            let restart_tun = MenuItemBuilder::with_id("restart_tun", "重启 TUN")
                .build(app)?;
            let test_latency = MenuItemBuilder::with_id("test_latency", "测试延迟")
                .build(app)?;
            let show_window = MenuItemBuilder::with_id("show_window", "显示窗口")
                .build(app)?;
            let quit = MenuItemBuilder::with_id("quit", "退出")
                .build(app)?;

            let menu = MenuBuilder::new(app)
                .item(&dns_status)
                .item(&tun_status)
                .item(&latency_info)
                .separator()
                .item(&restart_dns)
                .item(&restart_tun)
                .item(&test_latency)
                .separator()
                .item(&show_window)
                .item(&quit)
                .build()?;

            // 创建系统托盘图标
            let _tray = TrayIconBuilder::new()
                .icon(app.default_window_icon().unwrap().clone())
                .menu(&menu)
                .on_menu_event(move |app, event| {
                    let id = event.id().as_ref();
                    match id {
                        "restart_dns" => {
                            let state = app.state::<AppState>();
                            let server = state.server.clone();
                            tauri::async_runtime::spawn(async move {
                                let mut server = server.lock().await;
                                server.stop().await;
                                tokio::time::sleep(std::time::Duration::from_millis(300)).await;
                                if let Err(e) = server.start().await {
                                    tracing::error!("重启 DNS 服务失败: {}", e);
                                }
                            });
                        }
                        "restart_tun" => {
                            let state = app.state::<AppState>();
                            let tun_device = state.tun_device.clone();
                            let tun_config = state.tun_config.clone();
                            tauri::async_runtime::spawn(async move {
                                let mut tun = tun_device.lock().await;
                                tun.stop().await;
                                drop(tun);
                                tokio::time::sleep(std::time::Duration::from_millis(300)).await;
                                let config = tun_config.lock().await.clone();
                                let mut tun = tun_device.lock().await;
                                *tun = TunDevice::new(config);
                                if let Err(e) = tun.start().await {
                                    tracing::error!("重启 TUN 失败: {}", e);
                                }
                            });
                        }
                        "test_latency" => {
                            let state = app.state::<AppState>();
                            let config = state.config.clone();
                            let latency_results = state.latency_results.clone();
                            let latency_last_test = state.latency_last_test.clone();
                            tauri::async_runtime::spawn(async move {
                                let servers = {
                                    let config = config.lock().await;
                                    config.upstream.iter().filter(|s| s.enabled).cloned().collect::<Vec<_>>()
                                };
                                let results = run_latency_test(&servers).await;
                                let mut saved = latency_results.lock().await;
                                *saved = results;
                                let mut last_test = latency_last_test.lock().await;
                                *last_test = Some(chrono::Local::now().format("%H:%M:%S").to_string());
                            });
                        }
                        "show_window" => {
                            if let Some(window) = app.get_webview_window("main") {
                                let _ = window.show();
                                let _ = window.set_focus();
                            }
                        }
                        "quit" => {
                            app.exit(0);
                        }
                        _ => {}
                    }
                })
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } = event
                    {
                        let app = tray.app_handle();
                        if let Some(window) = app.get_webview_window("main") {
                            let _ = window.show();
                            let _ = window.set_focus();
                        }
                    }
                })
                .build(app)?;

            // 自动启动 DNS 服务
            let app_handle_startup = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                let auto_start = {
                    let state = app_handle_startup.state::<AppState>();
                    let config = state.config.lock().await;
                    config.proxy.auto_start
                };
                if auto_start {
                    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                    let state = app_handle_startup.state::<AppState>();
                    let mut server = state.server.lock().await;
                    if let Err(e) = server.start().await {
                        tracing::error!("自动启动DNS服务失败: {}", e);
                    } else {
                        tracing::info!("DNS服务已自动启动");
                    }
                }
            });

            // 自动启动 TUN
            let app_handle_tun = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                let tun_enabled = {
                    let state = app_handle_tun.state::<AppState>();
                    let config = state.config.lock().await;
                    config.tun.enabled
                };
                if tun_enabled {
                    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                    let state = app_handle_tun.state::<AppState>();
                    let tun_config = state.tun_config.lock().await.clone();
                    drop(tun_config);

                    let mut tun = state.tun_device.lock().await;
                    let tun_config = state.tun_config.lock().await.clone();
                    *tun = TunDevice::new(tun_config);
                    match tun.start().await {
                        Ok(()) => {
                            tracing::info!("TUN已自动启动");
                            // 启动DNS拦截器
                            let tun_clone = state.tun_device.clone();
                            let server = state.server.lock().await;
                            let handler = server.get_dns_handler();
                            let interceptor = DnsInterceptor::new(tun_clone);
                            interceptor.start(handler).await;
                        }
                        Err(e) => {
                            tracing::error!("自动启动TUN失败: {}", e);
                        }
                    }
                }
            });

            // 定期更新托盘菜单状态
            let app_handle = app.handle().clone();
            let dns_status_item = dns_status.clone();
            let tun_status_item = tun_status.clone();
            let latency_info_item = latency_info.clone();
            tauri::async_runtime::spawn(async move {
                loop {
                    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                    let state = app_handle.state::<AppState>();

                    // 更新 DNS 状态
                    let dns_running = {
                        let server = state.server.lock().await;
                        server.is_running().await
                    };
                    let dns_text = if dns_running {
                        "DNS: 运行中"
                    } else {
                        "DNS: 已停止"
                    };
                    let _ = dns_status_item.set_text(dns_text);

                    // 更新 TUN 状态
                    let tun_running = {
                        let tun = state.tun_device.lock().await;
                        tun.is_running().await
                    };
                    let tun_text = if tun_running {
                        "TUN: 运行中"
                    } else {
                        "TUN: 已停止"
                    };
                    let _ = tun_status_item.set_text(tun_text);

                    // 更新延迟信息
                    let latency_text = {
                        let results = state.latency_results.lock().await;
                        let last_test = state.latency_last_test.lock().await;
                        if results.is_empty() {
                            "延迟: 未测试".to_string()
                        } else {
                            let fastest = results.iter().find(|r| r.latency_ms.is_some());
                            match fastest {
                                Some(r) => format!(
                                    "最快: {} ({}ms)",
                                    r.name,
                                    r.latency_ms.unwrap()
                                ),
                                None => "延迟: 全部失败".to_string(),
                            }
                        }
                    };
                    let _ = latency_info_item.set_text(&latency_text);
                }
            });

            // 自动测速定时任务
            let app_handle_auto = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                // 等待10秒后开始第一次测速
                tokio::time::sleep(std::time::Duration::from_secs(10)).await;

                loop {
                    let interval = {
                        let state = app_handle_auto.state::<AppState>();
                        let config = state.config.lock().await;
                        config.latency_test_interval
                    };

                    if interval == 0 {
                        // 禁用自动测速，等待较长时间后再检查
                        tokio::time::sleep(std::time::Duration::from_secs(60)).await;
                        continue;
                    }

                    // 执行测速
                    {
                        let state = app_handle_auto.state::<AppState>();
                        let servers = {
                            let config = state.config.lock().await;
                            config.upstream.iter().filter(|s| s.enabled).cloned().collect::<Vec<_>>()
                        };
                        let results = run_latency_test(&servers).await;
                        let mut saved = state.latency_results.lock().await;
                        *saved = results;
                        let mut last_test = state.latency_last_test.lock().await;
                        *last_test = Some(chrono::Local::now().format("%H:%M:%S").to_string());
                        tracing::info!("自动测速完成，间隔: {}秒", interval);
                    }

                    // 等待下次测速
                    tokio::time::sleep(std::time::Duration::from_secs(interval)).await;
                }
            });

            // 定期更新公网 IP（用于 ECS）
            let app_handle_ip = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                // 启动后等待 30 秒再执行第一次更新
                tokio::time::sleep(std::time::Duration::from_secs(30)).await;

                loop {
                    let state = app_handle_ip.state::<AppState>();
                    let ecs_enabled = {
                        let config = state.config.lock().await;
                        config.ecs.enabled
                    };

                    if ecs_enabled {
                        // 获取 server 的 handler 来更新公网 IP
                        let server = state.server.lock().await;
                        let handler = server.get_dns_handler();
                        handler.update_public_ip().await;
                        tracing::info!("自动更新公网 IP 完成");
                    }

                    // 每 5 分钟更新一次
                    tokio::time::sleep(std::time::Duration::from_secs(300)).await;
                }
            });

            Ok(())
        })
        .on_window_event(|window, event| {
            // 拦截关闭事件，最小化到托盘
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .invoke_handler(tauri::generate_handler![
            get_config,
            save_config,
            start_server,
            stop_server,
            get_server_status,
            get_stats,
            get_logs,
            clear_logs,
            clear_cache,
            update_subscriptions,
            get_traffic_stats,
            get_tun_config,
            save_tun_config,
            start_tun,
            stop_tun,
            get_tun_status,
            test_dns_latency,
            get_latency_results
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
