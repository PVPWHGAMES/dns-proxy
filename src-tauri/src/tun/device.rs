use crate::tun::TunConfig;
use anyhow::{anyhow, Result};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::Mutex;
use tracing::{info, warn};
use windows::core::{GUID, PWSTR};
use windows::Win32::Foundation::{ERROR_BUFFER_OVERFLOW, NO_ERROR};
use windows::Win32::NetworkManagement::IpHelper::{
    ConvertInterfaceLuidToGuid, GetAdaptersAddresses, SetInterfaceDnsSettings,
    DNS_INTERFACE_SETTINGS, DNS_INTERFACE_SETTINGS_VERSION1, DNS_SETTING_NAMESERVER,
    GAA_FLAG_SKIP_ANYCAST, GAA_FLAG_SKIP_DNS_SERVER, GAA_FLAG_SKIP_MULTICAST,
    IP_ADAPTER_ADDRESSES_LH,
};
use windows::Win32::Networking::WinSock::AF_UNSPEC;

static NEXT_RUN_ID: AtomicU64 = AtomicU64::new(1);

struct AdapterInfo {
    if_index: u32,
    guid: GUID,
    friendly_name: String,
    description: String,
    oper_status: i32,
}

fn get_adapters() -> Result<Vec<AdapterInfo>> {
    unsafe {
        let flags = GAA_FLAG_SKIP_ANYCAST | GAA_FLAG_SKIP_MULTICAST | GAA_FLAG_SKIP_DNS_SERVER;

        // 第一次调用获取所需缓冲区大小
        let mut size: u32 = 0;
        let ret = GetAdaptersAddresses(AF_UNSPEC.0 as u32, flags, None, None, &mut size);
        if ret != ERROR_BUFFER_OVERFLOW.0 {
            return Err(anyhow!("获取网络适配器信息失败，错误码: {}", ret));
        }

        // 分配缓冲区并第二次调用
        let mut buffer = vec![0u8; size as usize];
        let ret = GetAdaptersAddresses(
            AF_UNSPEC.0 as u32,
            flags,
            None,
            Some(buffer.as_mut_ptr() as *mut IP_ADAPTER_ADDRESSES_LH),
            &mut size,
        );
        if ret != NO_ERROR.0 {
            return Err(anyhow!("获取网络适配器信息失败，错误码: {}", ret));
        }

        // 遍历链表
        let mut adapters = Vec::new();
        let mut ptr = buffer.as_ptr() as *const IP_ADAPTER_ADDRESSES_LH;
        while !ptr.is_null() {
            let adapter = &*ptr;
            let friendly_name = if adapter.FriendlyName.is_null() {
                String::new()
            } else {
                adapter.FriendlyName.to_string().unwrap_or_default()
            };
            let description = if adapter.Description.is_null() {
                String::new()
            } else {
                adapter.Description.to_string().unwrap_or_default()
            };
            let mut guid = GUID::zeroed();
            if ConvertInterfaceLuidToGuid(&adapter.Luid, &mut guid) != NO_ERROR {
                ptr = adapter.Next;
                continue;
            }
            adapters.push(AdapterInfo {
                if_index: adapter.Anonymous1.Anonymous.IfIndex,
                guid,
                friendly_name,
                description,
                oper_status: adapter.OperStatus.0,
            });
            ptr = adapter.Next;
        }

        Ok(adapters)
    }
}

pub struct TunDevice {
    config: TunConfig,
    adapter: Option<Arc<wintun::Adapter>>,
    session: Option<Arc<wintun::Session>>,
    running: Arc<Mutex<bool>>,
}

impl TunDevice {
    pub fn new(config: TunConfig) -> Self {
        Self {
            config,
            adapter: None,
            session: None,
            running: Arc::new(Mutex::new(false)),
        }
    }

    pub async fn start(&mut self) -> Result<()> {
        let started_at = Instant::now();
        let run_id = NEXT_RUN_ID.fetch_add(1, Ordering::Relaxed);
        let mut running = self.running.lock().await;
        if *running {
            info!(run_id = %run_id, "TUN已在运行，跳过启动");
            return Ok(());
        }

        info!(run_id = %run_id, interface = %self.config.interface_name, "开始启动TUN");
        let admin_started = Instant::now();
        let is_admin = Self::is_admin();
        info!(run_id = %run_id, stage = "admin_check", elapsed_ms = admin_started.elapsed().as_millis() as u64, success = is_admin, "TUN启动阶段完成");
        if !is_admin {
            warn!(run_id = %run_id, elapsed_ms = started_at.elapsed().as_millis() as u64, "TUN启动失败：需要管理员权限");
            return Err(anyhow!("TUN模式需要管理员权限"));
        }

        // 加载 WinTun - 优先从可执行文件同目录查找
        let wintun_started = Instant::now();
        let wintun = {
            let exe_dir = std::env::current_exe()
                .ok()
                .and_then(|p| p.parent().map(|p| p.to_path_buf()))
                .unwrap_or_default();
            let dll_path = exe_dir.join("wintun.dll");
            info!(run_id = %run_id, path = %dll_path.display(), "查找 wintun.dll");

            if dll_path.exists() {
                unsafe { wintun::load_from_path(&dll_path) }
                    .map_err(|e| anyhow!("从 {:?} 加载WinTun失败: {}", dll_path, e))?
            } else {
                unsafe { wintun::load() }
                    .map_err(|e| anyhow!("加载WinTun失败 (DLL未找到): {}", e))?
            }
        };
        info!(run_id = %run_id, stage = "wintun_load", elapsed_ms = wintun_started.elapsed().as_millis() as u64, "TUN启动阶段完成");

        // 创建或打开适配器
        let adapter_started = Instant::now();
        let (adapter, adapter_action) =
            match wintun::Adapter::open(&wintun, &self.config.interface_name) {
                Ok(a) => (a, "reuse"),
                Err(_) => {
                    info!(run_id = %run_id, "未找到TUN适配器，开始创建");
                    (
                        wintun::Adapter::create(
                            &wintun,
                            &self.config.interface_name,
                            "果冻网络加速",
                            None,
                        )
                        .map_err(|e| anyhow!("创建适配器失败: {}", e))?,
                        "create",
                    )
                }
            };
        info!(run_id = %run_id, stage = "adapter", action = adapter_action, elapsed_ms = adapter_started.elapsed().as_millis() as u64, "TUN启动阶段完成");

        // 获取接口索引
        let index_started = Instant::now();
        let if_index = self.get_interface_index().await?;
        info!(run_id = %run_id, stage = "interface_index", elapsed_ms = index_started.elapsed().as_millis() as u64, "TUN启动阶段完成");

        let ip_parts: Vec<&str> = self.config.subnet.split('/').collect();
        let ip_addr = ip_parts[0];
        let prefix_len: u32 = ip_parts.get(1).and_then(|s| s.parse().ok()).unwrap_or(24);
        let mask = Self::prefix_to_mask(prefix_len);
        info!(run_id = %run_id, interface_index = %if_index, "配置TUN网络地址");

        let ip_started = Instant::now();
        let output = Self::run_cmd(
            "netsh",
            &[
                "interface",
                "ip",
                "set",
                "address",
                &if_index,
                "static",
                ip_addr,
                &mask,
                &self.config.gateway,
            ],
        )?;
        info!(run_id = %run_id, stage = "set_ip", elapsed_ms = ip_started.elapsed().as_millis() as u64, success = output.status.success(), "TUN启动阶段完成");

        if self.config.auto_route {
            let route_started = Instant::now();
            let route_result = Self::run_cmd(
                "route",
                &[
                    "add",
                    "0.0.0.0",
                    "mask",
                    "0.0.0.0",
                    &self.config.gateway,
                    "metric",
                    "1",
                    "IF",
                    &if_index,
                ],
            );
            info!(run_id = %run_id, stage = "add_route", elapsed_ms = route_started.elapsed().as_millis() as u64, success = route_result.as_ref().map(|o| o.status.success()).unwrap_or(false), "TUN启动阶段完成");
        }

        let dns_started = Instant::now();
        let tun_index_u32: u32 = if_index.parse().map_err(|_| anyhow!("接口索引解析失败"))?;
        self.set_system_dns_native("127.0.0.1", tun_index_u32, self.config.auto_route)
            .await?;
        info!(run_id = %run_id, stage = "configure_dns", elapsed_ms = dns_started.elapsed().as_millis() as u64, "TUN启动阶段完成");

        let session_started = Instant::now();
        let session = adapter
            .start_session(wintun::MAX_RING_CAPACITY)
            .map_err(|e| anyhow!("创建会话失败: {}", e))?;
        info!(run_id = %run_id, stage = "start_session", elapsed_ms = session_started.elapsed().as_millis() as u64, "TUN启动阶段完成");

        self.adapter = Some(adapter);
        self.session = Some(Arc::new(session));
        *running = true;

        info!(run_id = %run_id, elapsed_ms = started_at.elapsed().as_millis() as u64, interface = %self.config.interface_name, "TUN启动成功");
        Ok(())
    }

    pub async fn stop(&mut self) {
        let mut running = self.running.lock().await;
        if !*running {
            return;
        }
        *running = false;

        info!("停止TUN...");

        let system_dns_started = Instant::now();
        if let Err(error) = self.set_system_dns("dhcp", None, true, false).await {
            warn!(error = %error, "恢复系统DNS失败");
        }

        info!(
            stage = "restore_dns",
            elapsed_ms = system_dns_started.elapsed().as_millis() as u64,
            "TUN停止阶段完成"
        );
        self.session = None;
        self.adapter = None;

        info!("TUN已停止");
    }

    pub async fn is_running(&self) -> bool {
        *self.running.lock().await
    }

    pub fn get_session(&self) -> Option<Arc<wintun::Session>> {
        self.session.clone()
    }

    // 设置系统DNS
    async fn set_system_dns(
        &self,
        dns: &str,
        tun_index: Option<&str>,
        configure_tun: bool,
        configure_system: bool,
    ) -> Result<()> {
        let tun_index = match tun_index {
            Some(index) => index.to_string(),
            None => self.get_interface_index().await?,
        };
        let targets = if configure_system {
            "$targets=@(Get-NetAdapter | Where-Object {$_.Status -eq 'Up' -and $_.ifIndex -ne $tun})"
        } else {
            "$targets=@()"
        };
        let tun_command = if configure_tun {
            if dns == "dhcp" {
                "try { Set-DnsClientServerAddress -InterfaceIndex $tun -ResetServerAddresses -ErrorAction Stop; $ok++ } catch { $failed++ };"
            } else {
                "try { Set-DnsClientServerAddress -InterfaceIndex $tun -ServerAddresses @('127.0.0.1') -ErrorAction Stop; $ok++ } catch { $failed++ };"
            }
        } else {
            ""
        };
        let target_command = if dns == "dhcp" {
            "try { Set-DnsClientServerAddress -InterfaceIndex $adapter.ifIndex -ResetServerAddresses -ErrorAction Stop; $ok++ } catch { $failed++ }"
        } else {
            "try { Set-DnsClientServerAddress -InterfaceIndex $adapter.ifIndex -ServerAddresses @('127.0.0.1') -ErrorAction Stop; $ok++ } catch { $failed++ }"
        };
        let script = format!(
            "$tun=[int]'{}'; $ok=0; $failed=0; {}; foreach ($adapter in $targets) {{ {} }}; {}; Write-Output ('dns_result targets={{0}} success={{1}} failed={{2}}' -f $targets.Count,$ok,$failed)",
            tun_index, targets, target_command, tun_command
        );

        let output = Self::run_cmd(
            "powershell",
            &["-NoProfile", "-NonInteractive", "-Command", &script],
        )?;
        info!(
            dns = dns,
            configure_tun,
            configure_system,
            result = %Self::summarize_output(&output.stdout),
            "批量DNS配置完成"
        );
        if !output.status.success() {
            return Err(anyhow!(
                "批量设置系统DNS失败，退出码: {:?}",
                output.status.code()
            ));
        }
        Ok(())
    }

    // 设置系统DNS（原生 API，用于启动热路径）
    async fn set_system_dns_native(
        &self,
        dns: &str,
        tun_index: u32,
        configure_system: bool,
    ) -> Result<()> {
        let adapters = get_adapters()?;
        let mut success = 0usize;
        let mut failed = 0usize;

        for adapter in &adapters {
            let is_tun = adapter.if_index == tun_index;
            if !is_tun {
                // 系统网卡仅在需要时配置，且必须处于 Up 状态
                if !configure_system {
                    continue;
                }
                // 1 = IfOperStatusUp
                if adapter.oper_status != 1 {
                    continue;
                }
            }

            match Self::set_dns_server(&adapter.guid, dns) {
                Ok(()) => success += 1,
                Err(e) => {
                    warn!(if_index = adapter.if_index, error = %e, "设置接口DNS失败");
                    failed += 1;
                }
            }
        }

        info!(dns = dns, configure_system, success, failed, "批量DNS配置完成");

        if failed > 0 {
            return Err(anyhow!("批量设置系统DNS失败，失败 {} 个接口", failed));
        }
        Ok(())
    }

    // 用原生 API 设置单个接口的静态 DNS 服务器
    fn set_dns_server(guid: &GUID, dns: &str) -> Result<()> {
        let dns_wide: Vec<u16> = dns.encode_utf16().chain(std::iter::once(0)).collect();

        let mut settings: DNS_INTERFACE_SETTINGS = unsafe { std::mem::zeroed() };
        settings.Version = DNS_INTERFACE_SETTINGS_VERSION1;
        settings.Flags = DNS_SETTING_NAMESERVER as u64;
        settings.NameServer = PWSTR(dns_wide.as_ptr() as *mut u16);

        unsafe {
            let ret = SetInterfaceDnsSettings(*guid, &settings);
            if ret != NO_ERROR {
                return Err(anyhow!("SetInterfaceDnsSettings 失败，错误码: {}", ret.0));
            }
        }
        Ok(())
    }

    // 获取接口索引
    async fn get_interface_index(&self) -> Result<String> {
        let adapters = get_adapters()?;

        // 精确匹配 FriendlyName
        if let Some(adapter) = adapters
            .iter()
            .find(|a| a.friendly_name == self.config.interface_name)
        {
            return Ok(adapter.if_index.to_string());
        }

        // 模糊匹配 Description
        if let Some(adapter) = adapters
            .iter()
            .find(|a| a.description.contains(&self.config.interface_name))
        {
            return Ok(adapter.if_index.to_string());
        }

        Err(anyhow!("无法获取接口索引"))
    }

    fn run_cmd(program: &str, args: &[&str]) -> Result<std::process::Output> {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        let started_at = Instant::now();

        let output = Command::new(program)
            .args(args)
            .creation_flags(CREATE_NO_WINDOW)
            .output();

        match &output {
            Ok(output) => {
                let stderr = Self::summarize_output(&output.stderr);
                if output.status.success() {
                    info!(
                        command = program,
                        elapsed_ms = started_at.elapsed().as_millis() as u64,
                        exit_code = output.status.code().unwrap_or(-1),
                        stderr = %stderr,
                        "系统命令完成"
                    );
                } else {
                    warn!(
                        command = program,
                        elapsed_ms = started_at.elapsed().as_millis() as u64,
                        exit_code = output.status.code().unwrap_or(-1),
                        stderr = %stderr,
                        "系统命令失败"
                    );
                }
            }
            Err(error) => {
                warn!(
                    command = program,
                    elapsed_ms = started_at.elapsed().as_millis() as u64,
                    error = %error,
                    "系统命令启动失败"
                );
            }
        }

        output.map_err(Into::into)
    }

    fn summarize_output(output: &[u8]) -> String {
        const MAX_OUTPUT_LENGTH: usize = 256;
        let text = String::from_utf8_lossy(output).replace(['\r', '\n', '\t'], " ");
        if text.chars().count() > MAX_OUTPUT_LENGTH {
            let truncated: String = text.chars().take(MAX_OUTPUT_LENGTH).collect();
            format!("{}...", truncated)
        } else {
            text
        }
    }

    fn is_admin() -> bool {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x08000000;

        Command::new("net")
            .args(["session"])
            .creation_flags(CREATE_NO_WINDOW)
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    fn prefix_to_mask(prefix_len: u32) -> String {
        let mask = if prefix_len == 0 {
            0
        } else {
            !0u32 << (32 - prefix_len)
        };
        let b = mask.to_be_bytes();
        format!("{}.{}.{}.{}", b[0], b[1], b[2], b[3])
    }
}
