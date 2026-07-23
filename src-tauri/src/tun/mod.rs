pub mod device;
pub mod dns_intercept;

use serde::{Deserialize, Serialize};


#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TunConfig {
    pub enabled: bool,
    pub interface_name: String,
    pub subnet: String,
    pub gateway: String,
    pub dns_servers: Vec<String>,
    pub auto_route: bool,
}

impl Default for TunConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            interface_name: "DNS-Proxy-TUN".to_string(),
            subnet: "10.10.0.0/24".to_string(),
            gateway: "10.10.0.1".to_string(),
            dns_servers: vec!["10.10.0.1".to_string()],
            auto_route: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TunStatus {
    pub active: bool,
    pub interface_name: String,
    pub ip_address: String,
    pub dns_redirected: bool,
    pub packets_processed: u64,
}
