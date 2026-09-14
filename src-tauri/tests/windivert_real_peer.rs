//! 真对端（非回环）路径验证
//!
//! 这是回环沙盒**在结构上覆盖不到**的那条路径：非回环包要"反射成入站"才会被
//! 本机套接字接收（`if !address.loopback() { address.set_outbound(false) }`）。
//! 回环只有出站形态，所以必须有一个真正的对端主机才能跑到这里。
//!
//! 需要一个本机之外、但**不出网**的对端，WSL2 正好合适：流量走虚拟交换机，
//! 对 Windows 协议栈而言是货真价实的非回环收发。
//!
//! 准备（管理员 PowerShell）：
//!
//! ```text
//! wsl -d Ubuntu -- sh -c "nohup python3 /mnt/c/.../wsl-echo.py 34000 > /tmp/echo.log 2>&1 &"
//! ```
//!
//! 运行：
//!
//! ```text
//! $env:WINDIVERT_REAL_PEER = "172.20.74.14:34000"   # WSL 侧地址（hostname -I 可得）
//! $env:WINDIVERT_RELAY_BIND = "172.20.64.1"         # Windows 侧该网卡地址
//! cargo test --test windivert_real_peer -- --ignored --nocapture
//! ```
//!
//! 全部配置都在环境变量里，测试本身不猜地址、也不碰除该目标外的任何流量。

use std::io::{Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpStream};
use std::time::Duration;

use dns_proxy_lib::relay::{LocalRelay, RelayConfig};
use dns_proxy_lib::windivert::{locate, RedirectRule, Redirector};

mod common;
use common::{free_low_ports, runtime_dir};

const CHAIN_TIMEOUT: Duration = Duration::from_secs(10);
const REQUEST: &[u8] = b"ping-from-client";

fn env_value(name: &str) -> Option<String> {
    match std::env::var(name) {
        Ok(value) if !value.trim().is_empty() => Some(value.trim().to_string()),
        _ => None,
    }
}

/// 发请求、读回显
fn exchange(peer: SocketAddr) -> std::io::Result<String> {
    let mut stream = TcpStream::connect(peer)?;
    stream.set_read_timeout(Some(CHAIN_TIMEOUT))?;
    stream.write_all(REQUEST)?;
    stream.flush()?;
    let mut buffer = [0u8; 128];
    let length = stream.read(&mut buffer)?;
    Ok(String::from_utf8_lossy(&buffer[..length]).to_string())
}

#[test]
#[ignore = "需要管理员权限，并设置 WINDIVERT_REAL_PEER / WINDIVERT_RELAY_BIND"]
fn non_loopback_traffic_can_be_redirected_and_rolled_back() {
    let Some(peer) = env_value("WINDIVERT_REAL_PEER") else {
        panic!("请设置 WINDIVERT_REAL_PEER=ip:port 指向一个非回环对端（例如 WSL 里的回显服务）");
    };
    let peer: SocketAddr = peer.parse().expect("WINDIVERT_REAL_PEER 应为 ip:port");
    let relay_bind: Ipv4Addr = env_value("WINDIVERT_RELAY_BIND")
        .unwrap_or_else(|| "0.0.0.0".to_string())
        .parse()
        .expect("WINDIVERT_RELAY_BIND 应为 IPv4 地址");

    let runtime_dir = runtime_dir();
    assert!(
        locate(&runtime_dir).is_some(),
        "缺少 {}，无法验证",
        runtime_dir.join("WinDivert.dll").display()
    );

    // 先把基线跑一遍：不经任何重定向必须能通，否则后面的失败无法归因
    let baseline = exchange(peer).unwrap_or_else(|error| {
        panic!("基线不通：{peer} 上没有可用的服务（{error}）")
    });
    assert!(
        baseline.starts_with("service-ok:"),
        "对端应当回显 service-ok: 前缀，实际：{baseline:?}"
    );
    println!("基线通过：{baseline:?}");

    let ports = free_low_ports(2);
    let (relay_port, sentinel_port) = (ports[0], ports[1]);
    println!(
        "目标 {peer} / 中继 {relay_bind}:{relay_port} / 哨兵 {sentinel_port}"
    );

    // 中继拨哨兵端口，由重定向映射回真实目标端口；绑定到面向对端的那张网卡
    let relay = LocalRelay::new();
    let relay_status = relay
        .start(RelayConfig {
            bind_addr: relay_bind,
            listen_port: relay_port,
            destination: SocketAddr::new(peer.ip(), sentinel_port),
            // 注入包会把原目标地址写进源地址，因此对端必然是它
            allowed_peer: Some(peer.ip()),
        })
        .unwrap_or_else(|error| panic!("启动中继失败：{error}"));
    assert!(relay_status.running, "中继应处于运行中：{relay_status:?}");

    let redirector = Redirector::new(runtime_dir);
    let status = redirector
        .start(RedirectRule {
            target_addr: match peer.ip() {
                std::net::IpAddr::V4(address) => address,
                std::net::IpAddr::V6(_) => panic!("本测试仅支持 IPv4 对端"),
            },
            target_port: peer.port(),
            relay_port,
            sentinel_port,
        })
        .unwrap_or_else(|error| panic!("启动重定向失败：{error}（该测试需要管理员权限）"));
    assert!(status.running, "重定向应处于运行中：{status:?}");

    let echoed = match exchange(peer) {
        Ok(echoed) => echoed,
        Err(error) => {
            let status = redirector.status();
            println!("非回环链路失败：{error}");
            println!(
                "重定向计数器：客户端→中继 {} / 中继→客户端 {} / 拨号映射 {} / 回包映射 {} / 原样放行 {} / 跳过 {} / 注入失败 {}",
                status.to_relay,
                status.to_client,
                status.dial_mapped,
                status.reply_mapped,
                status.passed_through,
                status.skipped,
                status.send_failed
            );
            println!("中继状态：{:?}", relay.status());
            panic!("非回环路径未能把数据送达客户端：{error}");
        }
    };

    // 对端是回显服务：能读回自己的请求，就同时证明上行到达、下行返回
    let expected = format!("service-ok:{}", String::from_utf8_lossy(REQUEST));
    assert_eq!(echoed, expected, "回显内容应与请求一致");

    let status = redirector.status();
    println!(
        "重定向计数器：客户端→中继 {} / 中继→客户端 {} / 拨号映射 {} / 回包映射 {} / 原样放行 {} / 跳过 {} / 注入失败 {}",
        status.to_relay,
        status.to_client,
        status.dial_mapped,
        status.reply_mapped,
        status.passed_through,
        status.skipped,
        status.send_failed
    );
    println!("中继状态：{:?}", relay.status());

    assert!(status.to_relay > 0, "分支 1（含反射成入站）应有流量：{status:?}");
    assert!(status.to_client > 0, "分支 2（含反射成入站）应有流量：{status:?}");
    assert!(status.dial_mapped > 0, "分支 3 应有流量：{status:?}");
    assert!(status.reply_mapped > 0, "分支 4 应有流量：{status:?}");
    assert_eq!(status.send_failed, 0, "不应有注入失败：{status:?}");
    assert_eq!(relay.status().accepted, 1, "中继应只接受一条连接");

    // 一键回滚：停止后必须恢复直连
    let stopped = redirector.stop();
    assert!(!stopped.running, "停止后应为未运行：{stopped:?}");
    let direct = exchange(peer).expect("回滚后应能直连对端");
    assert_eq!(direct, expected, "回滚后应直达对端");

    relay.stop();
}
