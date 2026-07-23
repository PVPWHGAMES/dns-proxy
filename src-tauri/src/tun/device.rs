use crate::tun::TunConfig;
use anyhow::{anyhow, Result};
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::info;
use std::process::Command;

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
        let mut running = self.running.lock().await;
        if *running {
            return Ok(());
        }

        if !Self::is_admin() {
            return Err(anyhow!("TUN模式需要管理员权限"));
        }

        info!("启动TUN模式...");

        // 加载 WinTun - 优先从可执行文件同目录查找
        let wintun = {
            let exe_dir = std::env::current_exe()
                .ok()
                .and_then(|p| p.parent().map(|p| p.to_path_buf()))
                .unwrap_or_default();
            let dll_path = exe_dir.join("wintun.dll");
            info!("查找 wintun.dll: {:?}", dll_path);

            if dll_path.exists() {
                unsafe { wintun::load_from_path(&dll_path) }
                    .map_err(|e| anyhow!("从 {:?} 加载WinTun失败: {}", dll_path, e))?
            } else {
                // 回退到默认加载方式
                unsafe { wintun::load() }
                    .map_err(|e| anyhow!("加载WinTun失败 (DLL未找到): {}", e))?
            }
        };

        // 创建或打开适配器
        let adapter = match wintun::Adapter::open(&wintun, &self.config.interface_name) {
            Ok(a) => {
                info!("已打开TUN适配器");
                a
            }
            Err(_) => {
                info!("创建TUN适配器...");
                wintun::Adapter::create(&wintun, &self.config.interface_name, "DNS Proxy", None)
                    .map_err(|e| anyhow!("创建适配器失败: {}", e))?
            }
        };

        // 获取接口索引
        let if_index = self.get_interface_index().await?;
        info!("接口索引: {}", if_index);

        // 配置IP地址
        let ip_parts: Vec<&str> = self.config.subnet.split('/').collect();
        let ip_addr = ip_parts[0];
        let prefix_len: u32 = ip_parts.get(1).and_then(|s| s.parse().ok()).unwrap_or(24);
        let mask = Self::prefix_to_mask(prefix_len);

        info!("配置IP: {}/{}", ip_addr, prefix_len);

        // 设置IP
        let output = Self::run_cmd("netsh", &[
            "interface", "ip", "set", "address",
            &if_index, "static", ip_addr, &mask, &self.config.gateway,
        ])?;
        info!("设置IP: {}", if output.status.success() { "成功" } else { "失败" });

        // 设置DNS为本机
        let _ = Self::run_cmd("netsh", &[
            "interface", "ip", "set", "dns",
            &if_index, "static", "127.0.0.1",
        ]);

        // 配置路由 - 将所有DNS流量路由到TUN
        if self.config.auto_route {
            info!("配置路由...");

            // 添加路由到网关
            let _ = Self::run_cmd("route", &[
                "add", "0.0.0.0", "mask", "0.0.0.0", &self.config.gateway,
                "metric", "1", "IF", &if_index,
            ]);

            // 设置系统DNS为TUN网卡的地址
            self.set_system_dns("127.0.0.1").await?;
        }

        // 创建会话
        let session = adapter.start_session(wintun::MAX_RING_CAPACITY)
            .map_err(|e| anyhow!("创建会话失败: {}", e))?;

        self.adapter = Some(adapter);
        self.session = Some(Arc::new(session));
        *running = true;

        info!("TUN启动成功: {} ({})", self.config.interface_name, ip_addr);

        Ok(())
    }

    pub async fn stop(&mut self) {
        let mut running = self.running.lock().await;
        if !*running {
            return;
        }
        *running = false;

        info!("停止TUN...");

        // 恢复DNS
        let _ = self.set_system_dns("dhcp").await;

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
    async fn set_system_dns(&self, dns: &str) -> Result<()> {
        // 获取所有活跃的网络接口
        let output = Self::run_cmd("powershell", &[
            "-Command",
            "Get-NetAdapter | Where-Object {$_.Status -eq 'Up'} | Select-Object -ExpandProperty ifIndex",
        ])?;

        let indices = String::from_utf8_lossy(&output.stdout);
        for index in indices.lines() {
            let index = index.trim();
            if !index.is_empty() && index != &self.get_interface_index().await? {
                if dns == "dhcp" {
                    let _ = Self::run_cmd("netsh", &[
                        "interface", "ip", "set", "dns", index, "dhcp",
                    ]);
                } else {
                    let _ = Self::run_cmd("netsh", &[
                        "interface", "ip", "set", "dns", index, "static", dns,
                    ]);
                }
            }
        }

        Ok(())
    }

    // 获取接口索引
    async fn get_interface_index(&self) -> Result<String> {
        let output = Self::run_cmd("powershell", &[
            "-Command",
            &format!("(Get-NetAdapter -Name '{}').ifIndex", self.config.interface_name),
        ])?;

        let index = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if index.is_empty() {
            // 尝试通过描述获取
            let output = Self::run_cmd("powershell", &[
                "-Command",
                &format!("(Get-NetAdapter | Where-Object {{$_.InterfaceDescription -like '*{}*'}}).ifIndex",
                    self.config.interface_name),
            ])?;
            let index = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !index.is_empty() {
                return Ok(index);
            }
            return Err(anyhow!("无法获取接口索引"));
        }

        Ok(index)
    }

    fn run_cmd(program: &str, args: &[&str]) -> Result<std::process::Output> {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x08000000;

        let output = Command::new(program)
            .args(args)
            .creation_flags(CREATE_NO_WINDOW)
            .output()?;

        Ok(output)
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
        let mask = if prefix_len == 0 { 0 } else { !0u32 << (32 - prefix_len) };
        let b = mask.to_be_bytes();
        format!("{}.{}.{}.{}", b[0], b[1], b[2], b[3])
    }
}
