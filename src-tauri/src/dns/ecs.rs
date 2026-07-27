use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use tracing::debug;

/// EDNS Client Subnet (ECS) 配置
#[derive(Debug, Clone)]
pub struct EcsConfig {
    /// 是否启用 ECS
    pub enabled: bool,
    /// 客户端 IP 地址（如果为 None，则从请求中推断）
    pub client_ip: Option<IpAddr>,
    /// IPv4 源掩码长度（默认 24，即 /24 子网）
    pub ipv4_source_mask: u8,
    /// IPv6 源掩码长度（默认 56，即 /56 子网）
    pub ipv6_source_mask: u8,
}

impl Default for EcsConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            client_ip: None,
            ipv4_source_mask: 24,
            ipv6_source_mask: 56,
        }
    }
}

/// EDNS0 OPT 记录类型
const OPT_TYPE: u16 = 41;
/// ECS Option Code
const ECS_OPTION_CODE: u16 = 8;
/// 默认 UDP payload size
const UDP_PAYLOAD_SIZE: u16 = 4096;

/// 在 DNS 查询包中注入 ECS 信息
///
/// # 参数
/// - `query_bytes`: 原始 DNS 查询包
/// - `client_ip`: 客户端 IP 地址
/// - `source_mask`: 源掩码长度
///
/// # 返回
/// 注入 ECS 后的 DNS 查询包
pub fn inject_ecs(query_bytes: &[u8], client_ip: IpAddr, source_mask: u8) -> Vec<u8> {
    let mut packet = query_bytes.to_vec();

    // 构建 EDNS0 OPT 记录
    let opt_record = build_opt_record(client_ip, source_mask);

    // 追加到 DNS 包末尾
    packet.extend_from_slice(&opt_record);

    // 更新 Additional Count（第 10-11 字节）
    if packet.len() >= 12 {
        let additional_count = u16::from_be_bytes([packet[10], packet[11]]);
        packet[10] = ((additional_count + 1) >> 8) as u8;
        packet[11] = ((additional_count + 1) & 0xFF) as u8;
    }

    debug!("已注入 ECS: client_ip={}, mask=/{}/", client_ip, source_mask);
    packet
}

/// 构建 EDNS0 OPT 记录
///
/// OPT 记录格式：
/// - Name: 0x00 (root)
/// - Type: 41 (OPT)
/// - Class: UDP payload size
/// - TTL: Extended RCODE (8 bits) + Version (8 bits) + DO bit (1 bit) + Z (15 bits)
/// - RDLENGTH: RDATA 长度
/// - RDATA: EDNS options
fn build_opt_record(client_ip: IpAddr, source_mask: u8) -> Vec<u8> {
    let mut record = Vec::new();

    // Name: root (0x00)
    record.push(0x00);

    // Type: OPT (41)
    record.extend_from_slice(&OPT_TYPE.to_be_bytes());

    // Class: UDP payload size
    record.extend_from_slice(&UDP_PAYLOAD_SIZE.to_be_bytes());

    // TTL: Extended RCODE + Version + Flags
    // Extended RCODE: 0, Version: 0, DO bit: 0, Z: 0
    record.extend_from_slice(&[0x00, 0x00, 0x00, 0x00]);

    // 构建 ECS Option
    let ecs_option = build_ecs_option(client_ip, source_mask);

    // RDLENGTH
    record.extend_from_slice(&(ecs_option.len() as u16).to_be_bytes());

    // RDATA
    record.extend_from_slice(&ecs_option);

    record
}

/// 构建 ECS Option
///
/// ECS Option 格式：
/// - Option Code: 8 (Client Subnet)
/// - Option Length: 数据长度
/// - Family: 1 (IPv4) 或 2 (IPv6)
/// - Source Netmask: 源掩码长度
/// - Scope Netmask: 作用域掩码长度（查询时为 0）
/// - Address: 客户端 IP 地址（按掩码截断，填充到字节边界）
fn build_ecs_option(client_ip: IpAddr, source_mask: u8) -> Vec<u8> {
    let mut option = Vec::new();

    // Option Code: ECS (8)
    option.extend_from_slice(&ECS_OPTION_CODE.to_be_bytes());

    match client_ip {
        IpAddr::V4(ipv4) => {
            let mask = source_mask.min(32);
            let addr_bytes = truncate_ipv4(ipv4, mask);
            let option_data_len = 4 + addr_bytes.len(); // Family(2) + Mask(1) + Scope(1) + Address

            // Option Length
            option.extend_from_slice(&(option_data_len as u16).to_be_bytes());

            // Family: IPv4 (1)
            option.extend_from_slice(&1u16.to_be_bytes());

            // Source Netmask
            option.push(mask);

            // Scope Netmask (查询时为 0)
            option.push(0);

            // Address
            option.extend_from_slice(&addr_bytes);
        }
        IpAddr::V6(ipv6) => {
            let mask = source_mask.min(128);
            let addr_bytes = truncate_ipv6(ipv6, mask);
            let option_data_len = 4 + addr_bytes.len();

            // Option Length
            option.extend_from_slice(&(option_data_len as u16).to_be_bytes());

            // Family: IPv6 (2)
            option.extend_from_slice(&2u16.to_be_bytes());

            // Source Netmask
            option.push(mask);

            // Scope Netmask (查询时为 0)
            option.push(0);

            // Address
            option.extend_from_slice(&addr_bytes);
        }
    }

    option
}

/// 截断 IPv4 地址到指定掩码长度
///
/// 返回截断后的字节数组（填充到字节边界）
fn truncate_ipv4(ip: Ipv4Addr, mask: u8) -> Vec<u8> {
    let octets = ip.octets();
    let full_bytes = (mask / 8) as usize;
    let remaining_bits = mask % 8;

    let mut result = Vec::new();

    // 复制完整的字节
    for i in 0..full_bytes.min(4) {
        result.push(octets[i]);
    }

    // 处理部分字节
    if full_bytes < 4 && remaining_bits > 0 {
        let mask_byte = 0xFF << (8 - remaining_bits);
        result.push(octets[full_bytes] & mask_byte);
    }

    result
}

/// 截断 IPv6 地址到指定掩码长度
fn truncate_ipv6(ip: Ipv6Addr, mask: u8) -> Vec<u8> {
    let segments = ip.segments();
    let mut result = Vec::new();

    // 将 16 位段转换为字节
    let mut bytes = Vec::new();
    for seg in &segments {
        bytes.push((seg >> 8) as u8);
        bytes.push((seg & 0xFF) as u8);
    }

    let full_bytes = (mask / 8) as usize;
    let remaining_bits = mask % 8;

    // 复制完整的字节
    for i in 0..full_bytes.min(16) {
        result.push(bytes[i]);
    }

    // 处理部分字节
    if full_bytes < 16 && remaining_bits > 0 {
        let mask_byte = 0xFF << (8 - remaining_bits);
        result.push(bytes[full_bytes] & mask_byte);
    }

    result
}

/// 从 DNS 响应中提取 ECS 信息
///
/// 返回 (scope_netmask, client_ip) 或 None
pub fn extract_ecs_from_response(response: &[u8]) -> Option<(u8, IpAddr)> {
    // 解析 DNS 响应，查找 OPT 记录中的 ECS option
    // 这里简化处理，实际实现需要完整的 DNS 解析
    // 暂时返回 None，后续可以扩展
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_truncate_ipv4() {
        let ip = Ipv4Addr::new(192, 168, 1, 100);
        let result = truncate_ipv4(ip, 24);
        assert_eq!(result, vec![192, 168, 1]);
    }

    #[test]
    fn test_truncate_ipv4_partial() {
        let ip = Ipv4Addr::new(192, 168, 1, 100);
        let result = truncate_ipv4(ip, 20);
        assert_eq!(result, vec![192, 168, 0]); // 20 bits = 2 full bytes + 4 bits
    }

    #[test]
    fn test_inject_ecs() {
        // 构造一个简单的 DNS 查询包
        let query = vec![
            0x00, 0x01, // Transaction ID
            0x01, 0x00, // Flags
            0x00, 0x01, // Questions
            0x00, 0x00, // Answer RRs
            0x00, 0x00, // Authority RRs
            0x00, 0x00, // Additional RRs
            // Query name: example.com
            0x07, 0x65, 0x78, 0x61, 0x6d, 0x70, 0x6c, 0x65,
            0x03, 0x63, 0x6f, 0x6d,
            0x00, // Root label
            0x00, 0x01, // Type A
            0x00, 0x01, // Class IN
        ];

        let client_ip = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 100));
        let result = inject_ecs(&query, client_ip, 24);

        // 验证 Additional Count 增加了 1
        assert_eq!(result[10], 0x00);
        assert_eq!(result[11], 0x01);

        // 验证包长度增加了
        assert!(result.len() > query.len());
    }
}
