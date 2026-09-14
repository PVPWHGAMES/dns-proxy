//! 重定向会话：把「中继 + 重定向器」的启动/停止顺序固定在一处
//!
//! 顺序不是风格问题：
//!
//! - **启动时必须先把中继拉起来**。中继没起来就接管流量，等于把连接改投到一个
//!   没人监听的端口——那是纯粹的中断，而不是"没生效"。
//! - **停止时必须先停重定向**。反过来会让还在途中的报文继续被改投到已经关闭的端口。
//!
//! 这段顺序原先写在 Tauri 命令里，而命令带 `State` 因而无法自动化验证；
//! 抽到这里之后就能用普通测试覆盖（不需要管理员，也不需要界面）。

use std::net::{IpAddr, Ipv4Addr};
use std::path::PathBuf;
use std::sync::Arc;

use crate::relay::{LocalRelay, RelayConfig, RelayStatus};
use crate::windivert::{RedirectRule, RedirectStatus, Redirector};

/// 会话对外暴露的组合状态
#[derive(Clone, Debug, serde::Serialize)]
pub struct RedirectOverview {
    pub redirect: RedirectStatus,
    pub relay: RelayStatus,
}

/// 一次接管会话
pub struct RedirectSession {
    relay: Arc<LocalRelay>,
    redirector: Arc<Redirector>,
}

impl RedirectSession {
    pub fn new(exe_dir: PathBuf) -> Self {
        Self {
            relay: Arc::new(LocalRelay::new()),
            redirector: Arc::new(Redirector::new(exe_dir)),
        }
    }

    /// 启动接管：先中继，再重定向；任一步失败都不留半启动状态
    pub fn start(&self, rule: RedirectRule, relay_bind: Ipv4Addr) -> Result<(), String> {
        // 规则校验放在最前面：连参数都不自洽时不该动任何东西
        rule.validate()?;

        self.relay.start(RelayConfig {
            bind_addr: relay_bind,
            listen_port: rule.relay_port,
            destination: (rule.target_addr, rule.sentinel_port).into(),
            // 分支 1 会交换源/目的地址，把原目标地址写进源地址，
            // 因此中继看到的对端必然是那个目标——据此拒掉一切别的来源
            allowed_peer: Some(IpAddr::V4(rule.target_addr)),
        })?;

        if let Err(error) = self.redirector.start(rule) {
            // 中继在监听但没人往里改投，是最糟的中间态：先撤掉再报错
            self.relay.stop();
            return Err(error);
        }

        Ok(())
    }

    /// 停止接管：先重定向，再中继。幂等。
    pub fn stop(&self) {
        self.redirector.stop();
        self.relay.stop();
    }

    pub fn overview(&self) -> RedirectOverview {
        RedirectOverview {
            redirect: self.redirector.status(),
            relay: self.relay.status(),
        }
    }
}
