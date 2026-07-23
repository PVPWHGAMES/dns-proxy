use crate::dns::handler::DnsHandler;
use crate::tun::device::TunDevice;
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::{info, warn, error};

pub struct DnsInterceptor {
    tun_device: Arc<Mutex<TunDevice>>,
    running: Arc<Mutex<bool>>,
    packet_count: Arc<Mutex<u64>>,
}

impl DnsInterceptor {
    pub fn new(tun_device: Arc<Mutex<TunDevice>>) -> Self {
        Self {
            tun_device,
            running: Arc::new(Mutex::new(false)),
            packet_count: Arc::new(Mutex::new(0)),
        }
    }

    pub async fn start(&self, dns_handler: Arc<DnsHandler>) {
        let running = self.running.clone();
        let tun = self.tun_device.clone();
        let count = self.packet_count.clone();
        *running.lock().await = true;

        info!("DNS拦截器启动");

        tokio::spawn(async move {
            loop {
                if !*running.lock().await {
                    break;
                }

                let session = {
                    let device = tun.lock().await;
                    device.get_session()
                };

                let session = match session {
                    Some(s) => s,
                    None => {
                        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                        continue;
                    }
                };

                // 接收数据包（阻塞）
                let packet = match tokio::task::spawn_blocking({
                    let session = session.clone();
                    move || {
                        // 使用超时避免永久阻塞
                        match session.receive_blocking() {
                            Ok(pkt) => Ok(pkt),
                            Err(e) => Err(e),
                        }
                    }
                }).await {
                    Ok(Ok(pkt)) => pkt,
                    Ok(Err(e)) => {
                        // 只在运行时记录错误
                        if *running.lock().await {
                            warn!("接收数据包: {}", e);
                        }
                        continue;
                    }
                    Err(e) => {
                        error!("任务失败: {}", e);
                        continue;
                    }
                };

                let packet_data = packet.bytes().to_vec();

                // 更新计数
                {
                    let mut c = count.lock().await;
                    *c += 1;
                    if *c % 100 == 0 {
                        info!("已处理 {} 个数据包", *c);
                    }
                }

                // 检查是否是DNS请求
                if let Some(dns_query) = Self::extract_dns_query(&packet_data) {
                    let handler = dns_handler.clone();
                    let tun_clone = tun.clone();

                    tokio::spawn(async move {
                        let src_addr = "10.10.0.2:0".parse().unwrap();

                        match handler.handle_query(&dns_query, src_addr).await {
                            Some(response) => {
                                if let Some(response_packet) = Self::build_dns_response(&packet_data, &response) {
                                    let device = tun_clone.lock().await;
                                    if let Some(session) = device.get_session() {
                                        match session.allocate_send_packet(response_packet.len() as u16) {
                                            Ok(mut allocator) => {
                                                allocator.bytes_mut().copy_from_slice(&response_packet);
                                                session.send_packet(allocator);
                                            }
                                            Err(e) => {
                                                error!("发送失败: {}", e);
                                            }
                                        }
                                    }
                                }
                            }
                            None => {
                                // 没有响应，可能是规则阻止
                            }
                        }
                    });
                }
            }

            info!("DNS拦截器停止");
        });
    }

    pub async fn stop(&self) {
        *self.running.lock().await = false;
    }

    fn extract_dns_query(packet: &[u8]) -> Option<Vec<u8>> {
        if packet.len() < 20 {
            return None;
        }

        // IPv4
        let version = (packet[0] >> 4) & 0x0F;
        if version != 4 {
            return None;
        }

        let ihl = (packet[0] & 0x0F) as usize * 4;
        if packet.len() < ihl + 8 {
            return None;
        }

        // UDP
        let protocol = packet[9];
        if protocol != 17 {
            return None;
        }

        // 目标端口53
        let dst_port = u16::from_be_bytes([packet[ihl + 2], packet[ihl + 3]]);
        if dst_port != 53 {
            return None;
        }

        info!("捕获DNS请求: {} -> {}",
            std::net::Ipv4Addr::new(packet[12], packet[13], packet[14], packet[15]),
            std::net::Ipv4Addr::new(packet[16], packet[17], packet[18], packet[19])
        );

        let udp_offset = ihl + 8;
        if packet.len() > udp_offset {
            Some(packet[udp_offset..].to_vec())
        } else {
            None
        }
    }

    fn build_dns_response(query_packet: &[u8], dns_response: &[u8]) -> Option<Vec<u8>> {
        if query_packet.len() < 20 {
            return None;
        }

        let ihl = (query_packet[0] & 0x0F) as usize * 4;
        let total_len = ihl + 8 + dns_response.len();

        let mut response = vec![0u8; total_len];

        // IP头
        response[0] = 0x45;
        response[2..4].copy_from_slice(&(total_len as u16).to_be_bytes());
        response[4..6].copy_from_slice(&query_packet[4..6]);
        response[6..8].copy_from_slice(&query_packet[6..8]);
        response[8] = 64;
        response[9] = 17;

        // 交换IP
        response[12..16].copy_from_slice(&query_packet[16..20]);
        response[16..20].copy_from_slice(&query_packet[12..16]);

        // UDP头 - 交换端口
        let udp_offset = ihl;
        response[udp_offset..udp_offset + 2].copy_from_slice(&query_packet[udp_offset + 2..udp_offset + 4]);
        response[udp_offset + 2..udp_offset + 4].copy_from_slice(&query_packet[udp_offset..udp_offset + 2]);
        response[udp_offset + 4..udp_offset + 6].copy_from_slice(&((8 + dns_response.len()) as u16).to_be_bytes());

        // DNS响应
        response[ihl + 8..].copy_from_slice(dns_response);

        // 校验和
        let checksum = Self::checksum(&response[..ihl]);
        response[10..12].copy_from_slice(&checksum.to_be_bytes());

        Some(response)
    }

    fn checksum(data: &[u8]) -> u16 {
        let mut sum: u32 = 0;
        let mut i = 0;
        while i < data.len() - 1 {
            sum += u16::from_be_bytes([data[i], data[i + 1]]) as u32;
            i += 2;
        }
        if i < data.len() {
            sum += (data[i] as u32) << 8;
        }
        while sum > 0xFFFF {
            sum = (sum & 0xFFFF) + (sum >> 16);
        }
        !sum as u16
    }
}
