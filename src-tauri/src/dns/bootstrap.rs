//! DoH 主机名的引导解析
//!
//! DoH 上游只给了域名时，必须先把这个域名解析出地址才能建立 DoH 连接。而此时系统
//! DNS 往往已经指向本程序——交给系统解析器就会递归回自身，表现为"无法解析 DoH
//! bootstrap 地址"，最终连 DoH 都用不起来。
//!
//! 所以这里绕开系统解析器，直接向配置好的引导服务器发一次普通 UDP 查询。
//! 用 UDP 而不是再套一层加密：引导解析只用来拿到 IP，链路本身很短，而且这一步
//! 若还要依赖别的加密通道，就又多了一层先有鸡还是先有蛋。

use std::net::{IpAddr, Ipv4Addr, SocketAddr, UdpSocket};
use std::time::Duration;

use tracing::{debug, warn};
use trust_dns_proto::op::{Message, Query};
use trust_dns_proto::rr::{Name, RData, RecordType};
use trust_dns_proto::serialize::binary::{BinDecodable, BinEncodable};

/// 单台引导服务器的查询超时
const QUERY_TIMEOUT: Duration = Duration::from_secs(2);

/// 接收响应的缓冲区
const RECV_BUFFER: usize = 1232;

/// 解析一条引导服务器配置
///
/// 支持 `1.1.1.1` 与 `1.1.1.1:5353` 两种写法，缺省端口为 53。
pub fn parse_server(entry: &str) -> Option<SocketAddr> {
    let entry = entry.trim();
    if entry.is_empty() {
        return None;
    }
    if let Ok(address) = entry.parse::<SocketAddr>() {
        return Some(address);
    }
    entry
        .parse::<IpAddr>()
        .ok()
        .map(|ip| SocketAddr::new(ip, 53))
}

/// 依次向引导服务器查询主机名的 A 记录，返回第一个可用地址
///
/// 只查 A 记录：引导用到的公共 DNS 都是 IPv4 地址，而 DoH 主机名只有 AAAA 的
/// 情况极少；为它多写一条 v6 查询路径不划算，真有需要时配置里直接给 IP 即可。
pub fn resolve_ipv4(host: &str, servers: &[String], timeout: Duration) -> Option<Ipv4Addr> {
    let name = match Name::from_ascii(host) {
        Ok(name) => name,
        Err(error) => {
            debug!(%host, %error, "引导解析：域名不合法");
            return None;
        }
    };

    let mut query = Message::new();
    query
        .set_recursion_desired(true)
        .add_query(Query::query(name, RecordType::A));
    let request = query.to_bytes().ok()?;

    for entry in servers {
        let Some(server) = parse_server(entry) else {
            warn!(entry = %entry, "引导服务器地址无效，已跳过");
            continue;
        };
        if let Some(address) = query_server(server, &request, timeout) {
            debug!(%host, %server, %address, "引导解析成功");
            return Some(address);
        }
        debug!(%host, %server, "引导解析未取得结果，试下一台");
    }

    None
}

/// 用默认超时解析
pub fn resolve_ipv4_default(host: &str, servers: &[String]) -> Option<Ipv4Addr> {
    resolve_ipv4(host, servers, QUERY_TIMEOUT)
}

fn query_server(server: SocketAddr, request: &[u8], timeout: Duration) -> Option<Ipv4Addr> {
    // 绑定要与目标同族，否则 IPv6 引导服务器上发不出去
    let bind: SocketAddr = if server.is_ipv4() {
        SocketAddr::from(([0, 0, 0, 0], 0))
    } else {
        SocketAddr::from(([0u16; 8], 0))
    };
    let socket = UdpSocket::bind(bind).ok()?;
    socket.set_read_timeout(Some(timeout)).ok()?;
    socket.send_to(request, server).ok()?;

    let mut buffer = [0u8; RECV_BUFFER];
    let (length, _) = socket.recv_from(&mut buffer).ok()?;
    let response = Message::from_bytes(&buffer[..length]).ok()?;

    response
        .answers()
        .iter()
        .find_map(|record| match record.data() {
            Some(RData::A(address)) => Some(address.0),
            _ => None,
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;
    use trust_dns_client::rr::rdata::A;
    use trust_dns_proto::op::MessageType;
    use trust_dns_proto::rr::Record;

    #[test]
    fn parses_ip_and_ip_with_port() {
        assert_eq!(
            parse_server("223.5.5.5"),
            Some(SocketAddr::from(([223, 5, 5, 5], 53)))
        );
        assert_eq!(
            parse_server(" 119.29.29.29:5353 "),
            Some(SocketAddr::from(([119, 29, 29, 29], 5353)))
        );
        assert_eq!(
            parse_server("2400:3200::1"),
            Some("[2400:3200::1]:53".parse().unwrap())
        );
        assert_eq!(parse_server(""), None);
        assert_eq!(parse_server("   "), None);
        assert_eq!(parse_server("dns.example.com"), None);
    }

    /// 起一台只回一条 A 记录的假 DNS 服务器，返回它的地址
    fn spawn_fake_dns(address: Ipv4Addr) -> SocketAddr {
        let socket = UdpSocket::bind(("127.0.0.1", 0)).expect("绑定假 DNS 失败");
        let server = socket.local_addr().unwrap();
        thread::spawn(move || {
            let mut buffer = [0u8; 512];
            if let Ok((length, peer)) = socket.recv_from(&mut buffer) {
                let Ok(request) = Message::from_bytes(&buffer[..length]) else {
                    return;
                };
                let mut response = Message::new();
                response
                    .set_id(request.id())
                    .set_message_type(MessageType::Response)
                    .set_recursion_desired(true)
                    .set_recursion_available(true);
                for query in request.queries() {
                    response.add_query(query.clone());
                    response.add_answer(Record::from_rdata(
                        query.name().clone(),
                        60,
                        RData::A(A(address)),
                    ));
                }
                if let Ok(bytes) = response.to_bytes() {
                    let _ = socket.send_to(&bytes, peer);
                }
            }
        });
        server
    }

    #[test]
    fn resolves_through_the_configured_bootstrap_server() {
        let server = spawn_fake_dns(Ipv4Addr::new(203, 0, 113, 9));
        let answer = resolve_ipv4(
            "doh.example.com",
            &[server.to_string()],
            Duration::from_secs(2),
        );
        assert_eq!(answer, Some(Ipv4Addr::new(203, 0, 113, 9)));
    }

    #[test]
    fn skips_invalid_entries_and_reports_nothing_when_all_fail() {
        // 无效项被跳过；剩下一个没人监听的端口，最终返回 None 而不是 panic
        let answer = resolve_ipv4(
            "doh.example.com",
            &["不是地址".to_string(), "127.0.0.1:1".to_string()],
            Duration::from_millis(300),
        );
        assert_eq!(answer, None);
    }

    #[test]
    fn empty_server_list_resolves_to_nothing() {
        assert_eq!(resolve_ipv4("doh.example.com", &[], QUERY_TIMEOUT), None);
    }

    /// 用真实公共 DNS 解析一次，确认引导链路在现实网络里也通
    ///
    /// 会向 223.5.5.5 与 119.29.29.29 各发一次普通 UDP 查询，所以默认忽略。
    #[test]
    #[ignore = "需要外网：向 223.5.5.5 / 119.29.29.29 各发一次 UDP 查询"]
    fn resolves_through_the_default_public_servers() {
        let servers = vec!["223.5.5.5".to_string(), "119.29.29.29".to_string()];
        let answer = resolve_ipv4_default("dns.alidns.com", &servers);
        println!("dns.alidns.com -> {answer:?}");
        assert!(
            answer.is_some(),
            "默认引导服务器应能解析出 DoH 主机名，否则设置里配了也没用"
        );
    }
}
