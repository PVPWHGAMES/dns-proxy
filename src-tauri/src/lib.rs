// config 与 dns 对外开放只为端到端测试：真实互联网链路必须走真实的 DnsHandler
// 才能验证 DNS 应答链路，用假处理器测不出接线错误。
pub mod config;
pub mod dns;
mod system_dns;

use config::AppConfig;
use dns::server::DnsServer;
use dns::{CacheStats, DnsQueryLog, DnsStats, LogFilter, LogPage, PoolStats, TrafficStats};
use std::path::PathBuf;
use std::sync::Arc;
use system_dns::DnsSnapshot;
use tauri::menu::{MenuBuilder, MenuItemBuilder};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, State};
use tokio::sync::Mutex;
use tracing_subscriber::prelude::*;

pub struct AppState {
    server: Arc<Mutex<DnsServer>>,
    /// 与 `DnsHandler` 共享同一份配置实例
    ///
    /// 用标准库互斥锁而非 tokio 的：处理器侧原本就是标准库锁，改成 tokio 需要把
    /// 十几处 `lock().unwrap()` 全改成 `.lock().await`；反过来只改状态这一侧更小。
    /// 注意所有使用点都不得把锁守卫跨过 await。
    config: Arc<std::sync::Mutex<AppConfig>>,
    latency_results: Arc<Mutex<Vec<DnsLatencyResult>>>,
    latency_last_test: Arc<Mutex<Option<String>>>,
    /// 系统 DNS 接管前的原始配置，None 表示当前未接管
    ///
    /// 用标准库互斥锁而不是 tokio 的：退出事件处理器是同步上下文，
    /// 那里必须能不带 async 直接完成还原，否则进程退出后网卡仍指向 127.0.0.1。
    dns_snapshot: Arc<std::sync::Mutex<Option<DnsSnapshot>>>,
}

/// 取共享配置的锁，中毒时沿用内部数据
///
/// 配置里存的是订阅规则等可再生数据，因一次 panic 就把整份配置判死会让进程再也读不到它。
fn lock_config(config: &std::sync::Mutex<AppConfig>) -> std::sync::MutexGuard<'_, AppConfig> {
    config
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// 取锁并在中毒时沿用内部数据：这里保存的是还原所需的唯一凭据，
/// 因为一次 panic 就丢弃快照会让系统 DNS 永久回不去。
fn lock_snapshot(
    slot: &std::sync::Mutex<Option<DnsSnapshot>>,
) -> std::sync::MutexGuard<'_, Option<DnsSnapshot>> {
    slot.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// 按需接管或还原系统 DNS
///
/// 接管成功才记录快照；还原依据快照而不是「一律重置为 DHCP」，
/// 否则会抹掉用户手工配置的静态 DNS。
/// 还原失败时保留快照与落盘记录，留给下次启动重试 —— 凭据一丢就再也回不去了。
fn sync_dns_takeover(
    slot: &std::sync::Mutex<Option<DnsSnapshot>>,
    enabled: bool,
) -> Result<String, String> {
    let mut guard = lock_snapshot(slot);

    if enabled {
        if guard.is_some() {
            return Ok("系统 DNS 已处于接管状态".to_string());
        }

        // 采集前先还清上一次的欠账：落盘记录里存的是上一份原始配置，
        // 若直接采集覆盖，原始配置就永久丢失（网卡此刻未必在用，采集不到它）
        system_dns::recover_pending()
            .map_err(|error| format!("上次的系统 DNS 接管还没还回去，已中止本次接管：{}", error))?;

        let captured = system_dns::capture()?;
        if captured.adapters.is_empty() {
            return Err("没有找到在用的网卡，未接管系统 DNS".to_string());
        }

        // 先落盘再做改动：中途崩溃也能在下次启动时还原
        system_dns::save_pending(&captured)?;

        if let Err(error) = system_dns::takeover(&captured) {
            // 可能只改成功一部分，回滚一次再报错，避免留下半接管状态
            match system_dns::restore(&captured) {
                Ok(()) => system_dns::clear_pending(),
                // 回滚没成功说明网卡可能还停在 127.0.0.1，保留快照让下次启动自愈
                Err(rollback_error) => tracing::error!(
                    %rollback_error,
                    "接管失败且回滚未成功，已保留待还原快照以便下次启动自愈"
                ),
            }
            return Err(error);
        }

        let summary = captured.summary();
        *guard = Some(captured);
        Ok(summary)
    } else {
        // 内存里没有快照时回退到落盘记录：上次被强杀留下的接管只有文件还留着原始配置
        let captured = match guard.take() {
            Some(captured) => captured,
            None => match system_dns::load_pending() {
                Some(pending) => pending,
                None => return Ok("系统 DNS 未被接管".to_string()),
            },
        };

        match system_dns::restore(&captured) {
            Ok(()) => {
                system_dns::clear_pending();
                Ok("系统 DNS 已还原".to_string())
            }
            Err(error) => {
                // 还原失败就把快照放回去、并保留落盘记录：
                // 只删记录不留凭据，网卡会永久停在 127.0.0.1，谁都救不回来
                *guard = Some(captured);
                Err(error)
            }
        }
    }
}

/// 停止服务或退出时归还系统 DNS，失败只告警不阻断流程
fn release_dns_takeover(slot: &std::sync::Mutex<Option<DnsSnapshot>>) {
    if let Err(error) = sync_dns_takeover(slot, false) {
        tracing::error!(%error, "还原系统 DNS 失败，请手动检查网卡 DNS 设置");
    }
}

/// 完整重启 DNS 服务：先还原系统 DNS，再释放服务运行态，启动成功后重新接管。
async fn restart_dns_service(state: &AppState) -> Result<(), String> {
    release_dns_takeover(&state.dns_snapshot);

    {
        let mut server = state.server.lock().await;
        server.stop().await;
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        server.start().await.map_err(|error| error.to_string())?;
    }

    if lock_config(&state.config).proxy.takeover_system_dns {
        sync_dns_takeover(&state.dns_snapshot, true)?;
    }
    Ok(())
}

#[tauri::command]
async fn get_config(state: State<'_, AppState>) -> Result<AppConfig, String> {
    let config = lock_config(&state.config);
    Ok(config.clone())
}

#[tauri::command]
async fn save_config(state: State<'_, AppState>, new_config: AppConfig) -> Result<(), String> {
    let takeover_enabled = new_config.proxy.takeover_system_dns;

    {
        // 就地更新共享配置：服务器与处理器持有的是同一个实例，不需要再各同步一次
        let mut config = lock_config(&state.config);
        *config = new_config;
        config.save().map_err(|e| e.to_string())?;
    }

    // 重启服务器以应用新配置。配置已通过 Arc<Mutex<AppConfig>> 就地更新，
    // 服务器与 DnsHandler 会直接读到新值，因此只停再起，不重建实例 ——
    // 重建会构造全新的 DnsHandler，导致统计、日志等运行态数据全部归零。
    let was_running = {
        let mut server = state.server.lock().await;
        let was_running = server.is_running().await;
        if was_running {
            server.stop().await;
            // 等待端口释放
            tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        }
        if was_running {
            server.start().await.map_err(|e| e.to_string())?;
        }
        was_running
    };

    // 接管状态跟随配置：服务在跑才需要接管，停了就还回去
    if was_running {
        sync_dns_takeover(&state.dns_snapshot, takeover_enabled)?;
    } else {
        release_dns_takeover(&state.dns_snapshot);
    }

    Ok(())
}

/// 仅保存桌面应用自身行为，不重启 DNS 服务。
#[tauri::command]
async fn save_app_settings(
    state: State<'_, AppState>,
    new_config: AppConfig,
) -> Result<(), String> {
    let mut config = lock_config(&state.config);
    config.start_minimized = new_config.start_minimized;
    config.app_font = new_config.app_font;
    config.startup_delay_seconds = new_config.startup_delay_seconds;
    config.dns_restart_interval_hours = new_config.dns_restart_interval_hours;
    config.save().map_err(|error| error.to_string())
}

#[tauri::command]
async fn start_server(state: State<'_, AppState>) -> Result<(), String> {
    {
        let mut server = state.server.lock().await;
        server.start().await.map_err(|e| e.to_string())?;
    }

    let takeover = lock_config(&state.config).proxy.takeover_system_dns;
    if takeover {
        sync_dns_takeover(&state.dns_snapshot, true)?;
    }
    Ok(())
}

#[tauri::command]
async fn stop_server(state: State<'_, AppState>) -> Result<(), String> {
    {
        let mut server = state.server.lock().await;
        server.stop().await;
    }
    release_dns_takeover(&state.dns_snapshot);
    Ok(())
}

/// 当前系统 DNS 接管状态
#[derive(serde::Serialize)]
struct DnsTakeoverStatus {
    active: bool,
    /// 接管前的原始配置摘要
    detail: String,
    /// 配置项：启动服务时是否自动接管
    enabled: bool,
}

#[tauri::command]
async fn get_dns_takeover_status(state: State<'_, AppState>) -> Result<DnsTakeoverStatus, String> {
    let enabled = lock_config(&state.config).proxy.takeover_system_dns;
    let guard = lock_snapshot(&state.dns_snapshot);
    Ok(DnsTakeoverStatus {
        active: guard.is_some(),
        detail: guard.as_ref().map(DnsSnapshot::summary).unwrap_or_default(),
        enabled,
    })
}

/// 手动还原系统 DNS（界面上的兜底入口）
#[tauri::command]
async fn restore_system_dns(state: State<'_, AppState>) -> Result<String, String> {
    sync_dns_takeover(&state.dns_snapshot, false)
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

/// 取最近 limit 条查询日志（新到旧）
#[tauri::command]
async fn get_logs(
    state: State<'_, AppState>,
    limit: Option<usize>,
) -> Result<Vec<DnsQueryLog>, String> {
    let server = state.server.lock().await;
    Ok(server.get_logs(limit.unwrap_or(200)))
}

/// 取 id 大于 since_id 的新日志（旧到新），供前端增量刷新
#[tauri::command]
async fn get_logs_since(
    state: State<'_, AppState>,
    since_id: u64,
) -> Result<Vec<DnsQueryLog>, String> {
    let server = state.server.lock().await;
    Ok(server.get_logs_since(since_id))
}

/// 按条件分页取日志（新到旧）
///
/// 筛选在 Rust 侧完成，返回的 `total` 是整段缓冲区命中的条数，
/// 前端据此算页数；状态与类型用 "all" 表示不筛。
#[tauri::command]
async fn get_logs_page(
    state: State<'_, AppState>,
    offset: usize,
    limit: usize,
    keyword: Option<String>,
    action: Option<String>,
    query_type: Option<String>,
) -> Result<LogPage, String> {
    let server = state.server.lock().await;
    let filter = LogFilter {
        keyword: keyword.unwrap_or_default().trim().to_lowercase(),
        action: action.filter(|value| value != "all"),
        query_type: query_type.filter(|value| value != "all"),
    };
    Ok(server.get_logs_page(offset, limit, &filter))
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
async fn get_cache_stats(state: State<'_, AppState>) -> Result<CacheStats, String> {
    let server = state.server.lock().await;
    Ok(server.get_cache_stats())
}

#[tauri::command]
async fn get_pool_stats(state: State<'_, AppState>) -> Result<PoolStats, String> {
    let server = state.server.lock().await;
    Ok(server.get_pool_stats())
}

/// 应用内存占用信息
#[derive(Debug, Clone, serde::Serialize)]
struct MemoryInfo {
    /// 物理内存 (MB)
    memory_mb: f64,
    /// 虚拟内存 (MB)
    virtual_memory_mb: f64,
}

/// 获取当前进程的内存占用
///
/// 直接读取进程内存计数器，只查自身进程；前端每秒轮询也无需枚举全系统进程。
#[tauri::command]
fn get_memory_usage() -> Result<MemoryInfo, String> {
    use windows::Win32::System::ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS};
    use windows::Win32::System::Threading::GetCurrentProcess;

    let mut counters = PROCESS_MEMORY_COUNTERS {
        cb: std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32,
        ..Default::default()
    };

    unsafe {
        GetProcessMemoryInfo(GetCurrentProcess(), &mut counters, counters.cb)
            .map_err(|e| format!("获取进程内存信息失败: {}", e))?;
    }

    Ok(MemoryInfo {
        memory_mb: counters.WorkingSetSize as f64 / (1024.0 * 1024.0),
        virtual_memory_mb: counters.PagefileUsage as f64 / (1024.0 * 1024.0),
    })
}

#[tauri::command]
async fn update_subscriptions(state: State<'_, AppState>) -> Result<String, String> {
    let server = state.server.lock().await;
    server.update_subscriptions().await;
    // 配置是服务器与界面共享的同一个实例，处理器更新后这里直接可见，无需再同步一次
    Ok("订阅已更新".to_string())
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
    let servers: Vec<_> = {
        let config = lock_config(&state.config);
        config
            .upstream
            .iter()
            .filter(|s| s.enabled)
            .cloned()
            .collect()
    };

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
async fn get_latency_results(
    state: State<'_, AppState>,
) -> Result<(Vec<DnsLatencyResult>, Option<String>), String> {
    let results = state.latency_results.lock().await.clone();
    let last_test = state.latency_last_test.lock().await.clone();
    Ok((results, last_test))
}

/// 检查更新信息
#[derive(serde::Serialize, Clone)]
struct UpdateInfo {
    has_update: bool,
    current_version: String,
    latest_version: String,
    release_url: String,
    release_notes: String,
    published_at: String,
    installer_url: String,
    installer_sha256: String,
}

/// 检查程序是否已注册开机自启动（计划任务）
#[tauri::command]
fn is_autostart_enabled() -> Result<bool, String> {
    let output = std::process::Command::new("schtasks")
        .args(["/Query", "/TN", "DNS Proxy"])
        .output()
        .map_err(|e| format!("查询计划任务失败: {}", e))?;
    Ok(output.status.success())
}

/// 设置或取消开机自启动（计划任务，登录时以最高权限静默启动，不弹 UAC）
#[tauri::command]
fn set_autostart(enabled: bool) -> Result<(), String> {
    let exe_path = std::env::current_exe()
        .map_err(|e| format!("无法获取程序路径: {}", e))?
        .to_string_lossy()
        .to_string();

    if enabled {
        let task_command = format!("\"{}\"", exe_path);
        let output = std::process::Command::new("schtasks")
            .args([
                "/Create",
                "/TN",
                "DNS Proxy",
                "/TR",
                &task_command,
                "/SC",
                "ONLOGON",
                "/RL",
                "HIGHEST",
                "/F",
            ])
            .output()
            .map_err(|e| format!("创建计划任务失败: {}", e))?;
        if !output.status.success() {
            return Err(format!(
                "创建计划任务失败: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            ));
        }
    } else {
        // 删除任务，任务不存在也视为成功
        let _ = std::process::Command::new("schtasks")
            .args(["/Delete", "/TN", "DNS Proxy", "/F"])
            .output();
    }
    Ok(())
}

#[tauri::command]
async fn check_update() -> Result<UpdateInfo, String> {
    let current_version = env!("CARGO_PKG_VERSION").to_string();

    // 调用 GitHub API 获取最新 release（支持系统代理）
    let mut builder = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .user_agent("dns-proxy")
        .danger_accept_invalid_certs(false);

    // 尝试使用系统代理
    if let Ok(proxy_url) = std::env::var("HTTP_PROXY").or_else(|_| std::env::var("http_proxy")) {
        if let Ok(proxy) = reqwest::Proxy::http(&proxy_url) {
            builder = builder.proxy(proxy);
        }
    }
    if let Ok(proxy_url) = std::env::var("HTTPS_PROXY").or_else(|_| std::env::var("https_proxy")) {
        if let Ok(proxy) = reqwest::Proxy::https(&proxy_url) {
            builder = builder.proxy(proxy);
        }
    }

    let client = builder
        .build()
        .map_err(|e| format!("创建 HTTP 客户端失败: {}", e))?;

    let url = "https://api.github.com/repos/PVPWHGAMES/dns-proxy/releases/latest";
    let response = client
        .get(url)
        .header("Accept", "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2022-11-28")
        .send()
        .await
        .map_err(|e| format!("请求 GitHub API 失败，请检查网络连接或代理设置: {}", e))?;

    if !response.status().is_success() {
        let status = response.status();
        let message = response.text().await.unwrap_or_default().trim().to_string();
        let detail = if message.is_empty() {
            status.to_string()
        } else {
            format!("{} {}", status, message)
        };
        return Err(format!("GitHub API 返回错误: {}", detail));
    }

    let release: serde_json::Value = response
        .json()
        .await
        .map_err(|e| format!("解析响应失败: {}", e))?;

    let latest_version = release["tag_name"]
        .as_str()
        .unwrap_or("v0.0.0")
        .trim_start_matches('v')
        .to_string();

    let release_url = release["html_url"].as_str().unwrap_or("").to_string();

    let release_notes = release["body"].as_str().unwrap_or("无更新说明").to_string();

    let published_at = release["published_at"].as_str().unwrap_or("").to_string();

    // 从资产列表中提取安装包下载地址和 SHA256 校验值
    let mut installer_url = String::new();
    let mut installer_sha256 = String::new();
    if let Some(assets) = release["assets"].as_array() {
        for asset in assets {
            let name = asset["name"].as_str().unwrap_or("");
            if name.to_ascii_lowercase().ends_with(".exe") {
                installer_url = asset["browser_download_url"]
                    .as_str()
                    .unwrap_or("")
                    .to_string();
                installer_sha256 = asset["digest"]
                    .as_str()
                    .unwrap_or("")
                    .trim_start_matches("sha256:")
                    .to_string();
                break;
            }
        }
    }

    // 比较版本号
    let has_update = compare_versions(&current_version, &latest_version);

    Ok(UpdateInfo {
        has_update,
        current_version,
        latest_version,
        release_url,
        release_notes,
        published_at,
        installer_url,
        installer_sha256,
    })
}

/// 比较版本号，如果 latest > current 返回 true
fn compare_versions(current: &str, latest: &str) -> bool {
    let parse_version =
        |v: &str| -> Vec<u32> { v.split('.').filter_map(|s| s.parse().ok()).collect() };

    let current_parts = parse_version(current);
    let latest_parts = parse_version(latest);

    for i in 0..std::cmp::max(current_parts.len(), latest_parts.len()) {
        let c = current_parts.get(i).copied().unwrap_or(0);
        let l = latest_parts.get(i).copied().unwrap_or(0);
        if l > c {
            return true;
        }
        if l < c {
            return false;
        }
    }

    false
}

/// 下载安装包、校验 SHA256，校验通过后启动 NSIS 安装器并退出当前程序
#[tauri::command]
async fn download_and_install(
    app: AppHandle,
    url: String,
    sha256: String,
) -> Result<String, String> {
    if url.is_empty() {
        return Err("安装包下载地址为空".to_string());
    }

    // 复用与 check_update 一致的系统代理配置
    let mut builder = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(300))
        .user_agent("dns-proxy")
        .danger_accept_invalid_certs(false);
    if let Ok(proxy_url) = std::env::var("HTTP_PROXY").or_else(|_| std::env::var("http_proxy")) {
        if let Ok(proxy) = reqwest::Proxy::http(&proxy_url) {
            builder = builder.proxy(proxy);
        }
    }
    if let Ok(proxy_url) = std::env::var("HTTPS_PROXY").or_else(|_| std::env::var("https_proxy")) {
        if let Ok(proxy) = reqwest::Proxy::https(&proxy_url) {
            builder = builder.proxy(proxy);
        }
    }
    let client = builder
        .build()
        .map_err(|e| format!("创建 HTTP 客户端失败: {}", e))?;

    let response = client
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("下载安装包失败，请检查网络连接或代理设置: {}", e))?;
    if !response.status().is_success() {
        return Err(format!("下载安装包失败: HTTP {}", response.status()));
    }

    let bytes = response
        .bytes()
        .await
        .map_err(|e| format!("读取安装包数据失败: {}", e))?;

    // SHA256 校验
    if !sha256.is_empty() {
        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();
        hasher.update(&bytes);
        let computed = format!("{:x}", hasher.finalize());
        if !computed.eq_ignore_ascii_case(&sha256) {
            return Err(format!(
                "安装包校验失败：期望 {}，实际 {}",
                sha256, computed
            ));
        }
    }

    // 写入缓存目录
    let file_name = url
        .rsplit('/')
        .next()
        .filter(|s| !s.is_empty())
        .unwrap_or("dns-proxy-update.exe");
    let cache_dir = dirs::cache_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("dns-proxy");
    std::fs::create_dir_all(&cache_dir).map_err(|e| format!("创建下载目录失败: {}", e))?;
    let installer_path = cache_dir.join(file_name);
    std::fs::write(&installer_path, &bytes).map_err(|e| format!("写入安装包失败: {}", e))?;

    // 启动 NSIS 安装器，成功后退出当前程序
    std::process::Command::new(&installer_path)
        .spawn()
        .map_err(|e| format!("启动安装程序失败: {}", e))?;

    app.exit(0);

    Ok("安装程序已启动".to_string())
}

async fn run_latency_test(servers: &[crate::config::DnsServer]) -> Vec<DnsLatencyResult> {
    let mut results = Vec::new();

    // 创建优化的 HTTP 客户端（连接池）
    let http_client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(3))
        .pool_max_idle_per_host(8)
        .pool_idle_timeout(std::time::Duration::from_secs(90))
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
    let tcp =
        match tokio::time::timeout(std::time::Duration::from_secs(3), TcpStream::connect(&addr))
            .await
        {
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

    let mut tls = match tokio::time::timeout(
        std::time::Duration::from_secs(3),
        connector.connect(domain, tcp),
    )
    .await
    {
        Ok(Ok(stream)) => stream,
        Ok(Err(e)) => return Err(format!("TLS握手失败: {}", e)),
        Err(_) => return Err("TLS握手超时".to_string()),
    };

    // 发送 DNS 查询（DoT 使用 TCP 格式：2字节长度前缀 + 查询数据）
    let len = (query.len() as u16).to_be_bytes();
    tls.write_all(&len)
        .await
        .map_err(|e| format!("发送长度失败: {}", e))?;
    tls.write_all(query)
        .await
        .map_err(|e| format!("发送查询失败: {}", e))?;

    // 读取响应长度
    let mut len_buf = [0u8; 2];
    tls.read_exact(&mut len_buf)
        .await
        .map_err(|e| format!("读取响应长度失败: {}", e))?;
    let resp_len = u16::from_be_bytes(len_buf) as usize;

    if resp_len > 4096 {
        return Err(format!("响应长度异常: {}", resp_len));
    }

    // 读取响应数据
    let mut resp_buf = vec![0u8; resp_len];
    match tokio::time::timeout(
        std::time::Duration::from_secs(3),
        tls.read_exact(&mut resp_buf),
    )
    .await
    {
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
    let config = AppConfig::load();
    let log_dir = AppConfig::config_path()
        .parent()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    let log_file = config
        .log
        .file
        .as_deref()
        .map(PathBuf::from)
        .unwrap_or_else(|| log_dir.join("dns-proxy-diagnostics.log"));
    let log_file = if log_file.is_absolute() {
        log_file
    } else {
        log_dir.join(log_file)
    };

    // 日志文件仅用于诊断，按天轮转，避免应用运行期间阻塞启动。
    let _log_guard = match std::fs::create_dir_all(log_file.parent().unwrap_or(&log_dir)) {
        Ok(()) => {
            let file_name = log_file
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("dns-proxy-diagnostics.log");
            let file_dir = log_file.parent().unwrap_or(&log_dir);
            let appender = tracing_appender::rolling::daily(file_dir, file_name);
            let (file_writer, guard) = tracing_appender::non_blocking(appender);
            let file_layer = tracing_subscriber::fmt::layer()
                .with_ansi(false)
                .with_writer(file_writer);
            let filter = tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(&config.log.level));
            let _ = tracing_subscriber::registry()
                .with(filter)
                .with(tracing_subscriber::fmt::layer())
                .with(file_layer)
                .try_init();
            Some(guard)
        }
        Err(error) => {
            eprintln!("创建诊断日志目录失败: {}", error);
            tracing_subscriber::fmt()
                .with_env_filter(
                    tracing_subscriber::EnvFilter::try_from_default_env()
                        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(&config.log.level)),
                )
                .try_init()
                .ok();
            None
        }
    };

    tracing::info!(path = %log_file.display(), "诊断日志已初始化");

    // 全进程只保留一份配置实例：服务器、处理器与界面状态共享同一个 Arc。
    // 订阅规则动辄十几万条，多一份克隆就是十几 MB 常驻内存。
    let shared_config = Arc::new(std::sync::Mutex::new(config.clone()));
    let server = DnsServer::new(shared_config.clone());

    let latency_results = Arc::new(Mutex::new(Vec::new()));
    let latency_last_test = Arc::new(Mutex::new(None));
    let start_minimized = config.start_minimized;
    let startup_delay_seconds = config.startup_delay_seconds;

    let state = AppState {
        server: Arc::new(Mutex::new(server)),
        config: shared_config,
        latency_results: latency_results.clone(),
        latency_last_test: latency_last_test.clone(),
        dns_snapshot: Arc::new(std::sync::Mutex::new(None)),
    };

    // 上次异常退出可能把网卡 DNS 留在 127.0.0.1，先自愈再启动服务
    if let Err(error) = system_dns::recover_pending() {
        // 不中断启动：真正接管前还会再试一次，失败也会被拦下来并把原因报给界面
        tracing::error!(%error, "上次的 DNS 接管未能恢复");
    }

    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .manage(state)
        .setup(move |app| {
            // 创建系统托盘菜单
            let dns_status = MenuItemBuilder::with_id("dns_status", "DNS: 检查中...")
                .enabled(false)
                .build(app)?;
            let latency_info = MenuItemBuilder::with_id("latency_info", "延迟: 未测试")
                .enabled(false)
                .build(app)?;
            let restart_dns =
                MenuItemBuilder::with_id("restart_dns", "重启 DNS 服务").build(app)?;
            let test_latency = MenuItemBuilder::with_id("test_latency", "测试延迟").build(app)?;
            let show_window = MenuItemBuilder::with_id("show_window", "显示窗口").build(app)?;
            let quit = MenuItemBuilder::with_id("quit", "退出").build(app)?;

            let menu = MenuBuilder::new(app)
                .item(&dns_status)
                .item(&latency_info)
                .separator()
                .item(&restart_dns)
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
                        "test_latency" => {
                            let state = app.state::<AppState>();
                            let config = state.config.clone();
                            let latency_results = state.latency_results.clone();
                            let latency_last_test = state.latency_last_test.clone();
                            tauri::async_runtime::spawn(async move {
                                let servers = {
                                    let config = lock_config(&config);
                                    config
                                        .upstream
                                        .iter()
                                        .filter(|s| s.enabled)
                                        .cloned()
                                        .collect::<Vec<_>>()
                                };
                                let results = run_latency_test(&servers).await;
                                let mut saved = latency_results.lock().await;
                                *saved = results;
                                let mut last_test = latency_last_test.lock().await;
                                *last_test =
                                    Some(chrono::Local::now().format("%H:%M:%S").to_string());
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

            if start_minimized {
                if let Some(window) = app.get_webview_window("main") {
                    if let Err(error) = window.hide() {
                        tracing::error!(%error, "启动时隐藏主窗口失败");
                    }
                }
            }

            // 自动启动 DNS 服务：可选等待系统网络与代理组件就绪，避免登录瞬间抢占 DNS。
            let app_handle_startup = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                let delay = std::time::Duration::from_secs(startup_delay_seconds);
                if !delay.is_zero() {
                    tracing::info!(seconds = startup_delay_seconds, "延迟启动 DNS 服务");
                    tokio::time::sleep(delay).await;
                } else {
                    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                }
                let state = app_handle_startup.state::<AppState>();

                {
                    let mut server = state.server.lock().await;
                    if let Err(e) = server.start().await {
                        tracing::error!("自动启动DNS服务失败: {}", e);
                        return;
                    }
                    tracing::info!("DNS服务已自动启动");
                }

                // 服务起来之后再把系统 DNS 指过来，顺序反了会有一段解析真空
                let takeover = lock_config(&state.config).proxy.takeover_system_dns;
                if takeover {
                    match sync_dns_takeover(&state.dns_snapshot, true) {
                        Ok(detail) => tracing::info!(detail = %detail, "系统 DNS 已接管"),
                        Err(error) => {
                            tracing::error!(%error, "接管系统 DNS 失败，本机解析仍走原 DNS")
                        }
                    }
                }
            });

            // 定时重启 DNS 服务：只重启代理服务，不退出桌面应用。
            // 每轮读取配置，用户保存新间隔后无需重启整个应用即可生效。
            let app_handle_restart = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                loop {
                    let interval_hours = {
                        let state = app_handle_restart.state::<AppState>();
                        let interval = lock_config(&state.config).dns_restart_interval_hours;
                        interval
                    };
                    if interval_hours == 0 {
                        tokio::time::sleep(std::time::Duration::from_secs(60)).await;
                        continue;
                    }

                    tokio::time::sleep(std::time::Duration::from_secs(interval_hours * 60 * 60))
                        .await;
                    let state = app_handle_restart.state::<AppState>();
                    // 配置可能在等待期被关闭或修改，重新读取后再决定是否执行。
                    if lock_config(&state.config).dns_restart_interval_hours != interval_hours {
                        continue;
                    }
                    tracing::info!(hours = interval_hours, "执行定时 DNS 服务重启");
                    match restart_dns_service(state.inner()).await {
                        Ok(()) => tracing::info!("定时 DNS 服务重启完成"),
                        Err(error) => tracing::error!(%error, "定时 DNS 服务重启失败"),
                    }
                }
            });

            // 定期更新托盘菜单状态
            let app_handle = app.handle().clone();
            let dns_status_item = dns_status.clone();
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

                    // 更新延迟信息
                    let latency_text = {
                        let results = state.latency_results.lock().await;
                        let last_test = state.latency_last_test.lock().await;
                        if results.is_empty() {
                            "延迟: 未测试".to_string()
                        } else {
                            let fastest = results.iter().find(|r| r.latency_ms.is_some());
                            match fastest {
                                Some(r) => {
                                    format!("最快: {} ({}ms)", r.name, r.latency_ms.unwrap())
                                }
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
                    // 在块内取出副本再返回：守卫借用的是临时的 state，不能带出块外
                    let interval = {
                        let state = app_handle_auto.state::<AppState>();
                        let interval = lock_config(&state.config).latency_test_interval;
                        interval
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
                            let config = lock_config(&state.config);
                            config
                                .upstream
                                .iter()
                                .filter(|s| s.enabled)
                                .cloned()
                                .collect::<Vec<_>>()
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
                    let ecs_enabled = lock_config(&state.config).ecs.enabled;

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
            save_app_settings,
            start_server,
            stop_server,
            get_server_status,
            get_stats,
            get_logs,
            get_logs_since,
            get_logs_page,
            clear_logs,
            clear_cache,
            update_subscriptions,
            get_traffic_stats,
            get_cache_stats,
            get_pool_stats,
            get_dns_takeover_status,
            restore_system_dns,
            test_dns_latency,
            get_latency_results,
            check_update,
            download_and_install,
            get_memory_usage,
            is_autostart_enabled,
            set_autostart
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app_handle, event| {
            // 任何退出路径都必须把系统 DNS 还回去：
            // 进程没了而网卡还指向 127.0.0.1，整机域名解析会全部失效。
            // 还原是幂等的（快照取出后即为 None），重复触发不会重复执行。
            if matches!(
                event,
                tauri::RunEvent::ExitRequested { .. } | tauri::RunEvent::Exit
            ) {
                let state = app_handle.state::<AppState>();
                release_dns_takeover(&state.dns_snapshot);
            }
        });
}
