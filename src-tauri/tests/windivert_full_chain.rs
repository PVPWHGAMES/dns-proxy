//! 完整链路的回环端到端验证（四段映射 + 真实中继）
//!
//! 需要管理员权限：
//!
//! ```text
//! cargo test --test windivert_full_chain -- --ignored --nocapture
//! ```
//!
//! 链路（全部在 127.0.0.1 上，零外部流量）：
//!
//! ```text
//! 客户端 → 服务端口 P
//!   分支 1：改投到中继端口 R
//! 中继 → 拨号哨兵端口 S
//!   分支 3：映射回 P，真正连到"服务"
//! 服务 → 回包（源端口 P）
//!   分支 4：源端口映射成 S，中继的拨号套接字才认得来包
//! 中继 → 转发给客户端（源端口 R）
//!   分支 2：源端口还原成 P，客户端才认为来自它连的目标
//! ```
//!
//! 这条链路同时验证了四段映射、真实中继，以及**自噬规避**：中继拨出去的连接
//! 如果没有被映射而是又被当成"客户端连接"改投回来，测试会因为无限循环而超时。

use std::io::{Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use dns_proxy_lib::relay::{LocalRelay, RelayConfig};
use dns_proxy_lib::windivert::{locate, RedirectRule, Redirector};

mod common;
use common::{free_low_ports, runtime_dir};

const ACCEPT_TIMEOUT: Duration = Duration::from_secs(10);
const CHAIN_TIMEOUT: Duration = Duration::from_secs(10);

/// 客户端发给服务的请求：非空才能验证上游方向真的通了
const REQUEST: &[u8] = b"ping-from-client";

/// 起一个「真实服务」：先读请求，再回招呼；把每次读到的字节数记到通道
///
/// 先读后写是刻意的——只回招呼的话上游方向一个字节都不会经过链路，
/// 那条路径就白测了。
fn spawn_service(listener: TcpListener, greeting: &'static [u8]) -> mpsc::Receiver<usize> {
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { break };
            let sender = sender.clone();
            thread::spawn(move || {
                let mut buffer = [0u8; 128];
                if let Ok(length) = stream.read(&mut buffer) {
                    let _ = sender.send(length);
                    let _ = stream.write_all(greeting);
                    let _ = stream.flush();
                }
                thread::sleep(Duration::from_millis(600));
            });
        }
    });
    receiver
}

/// 连上去发请求、读招呼，返回（收到请求字节数由服务侧断言，这里返回招呼）
fn exchange(port: u16) -> std::io::Result<String> {
    let mut stream = TcpStream::connect((Ipv4Addr::LOCALHOST, port))?;
    stream.set_read_timeout(Some(CHAIN_TIMEOUT))?;
    stream.write_all(REQUEST)?;
    stream.flush()?;
    let mut buffer = [0u8; 32];
    let length = stream.read(&mut buffer)?;
    Ok(String::from_utf8_lossy(&buffer[..length]).to_string())
}

#[test]
#[ignore = "需要管理员权限"]
fn full_chain_redirects_through_a_real_relay_and_rolls_back() {
    let runtime_dir = runtime_dir();
    assert!(
        locate(&runtime_dir).is_some(),
        "缺少 {}，无法验证",
        runtime_dir.join("WinDivert.dll").display()
    );

    let ports = free_low_ports(3);
    let (service_port, relay_port, sentinel_port) = (ports[0], ports[1], ports[2]);
    println!(
        "服务端口 {service_port} / 中继端口 {relay_port} / 哨兵端口 {sentinel_port}"
    );

    let service = TcpListener::bind((Ipv4Addr::LOCALHOST, service_port))
        .expect("重新绑定服务端口失败");
    let service_accepted = spawn_service(service, b"service-ok");

    // 中继拨的是哨兵端口，由重定向映射回真实服务端口
    let relay = LocalRelay::new();
    let relay_status = relay
        .start(RelayConfig {
            bind_addr: Ipv4Addr::LOCALHOST,
            listen_port: relay_port,
            destination: format!("127.0.0.1:{sentinel_port}")
                .parse::<SocketAddr>()
                .unwrap(),
            // 分支 1 会交换源/目的地址，把原目标地址写进源地址，
            // 所以中继看到的对端必然是本机回环地址
            allowed_peer: Some(Ipv4Addr::LOCALHOST.into()),
        })
        .expect("启动中继失败");
    assert!(relay_status.running, "中继应处于运行中：{relay_status:?}");

    let redirector = Redirector::new(runtime_dir);
    let status = redirector
        .start(RedirectRule {
            target_addr: Ipv4Addr::LOCALHOST,
            target_port: service_port,
            relay_port,
            sentinel_port,
        })
        .unwrap_or_else(|error| panic!("启动重定向失败：{error}（该测试需要管理员权限）"));
    assert!(status.running, "重定向应处于运行中：{status:?}");

    // 第一段：客户端连服务端口，数据应当绕一整圈回到它手上
    let greeting = match exchange(service_port) {
        Ok(greeting) => greeting,
        Err(error) => {
            let status = redirector.status();
            println!("链路失败：{error}");
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
            println!("服务是否收到连接：{}", service_accepted.try_recv().is_ok());
            panic!("完整链路未能把数据送回客户端：{error}");
        }
    };
    assert_eq!(
        greeting, "service-ok",
        "应读到真实服务的招呼，实际读到：{greeting:?}"
    );

    let received_by_service = service_accepted
        .recv_timeout(ACCEPT_TIMEOUT)
        .expect("真实服务应当收到中继拨出的连接");
    assert_eq!(
        received_by_service,
        REQUEST.len(),
        "服务应原样收到客户端请求（这说明上游方向也穿过了整条链路）"
    );

    let relay_status = relay.status();
    println!("中继状态：{relay_status:?}");
    assert_eq!(relay_status.accepted, 1, "中继应只接受一条连接");
    assert!(
        relay_status.bytes_up >= REQUEST.len() as u64,
        "上行字节数应不少于请求长度：{relay_status:?}"
    );
    assert!(
        relay_status.bytes_down > 0,
        "下行字节数应大于 0：{relay_status:?}"
    );

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
    assert!(status.to_relay > 0, "分支 1 应有流量：{status:?}");
    assert!(status.to_client > 0, "分支 2 应有流量：{status:?}");
    assert!(
        status.dial_mapped > 0,
        "分支 3 应有流量（否则中继拨不到真实服务）：{status:?}"
    );
    assert!(
        status.reply_mapped > 0,
        "分支 4 应有流量（否则中继读不到服务回包）：{status:?}"
    );
    assert_eq!(status.send_failed, 0, "不应有注入失败：{status:?}");

    // 第二段：一键回滚后必须恢复直连
    let stopped = redirector.stop();
    assert!(!stopped.running, "停止后应为未运行：{stopped:?}");

    let greeting = exchange(service_port).expect("回滚后应能直连服务");
    assert_eq!(greeting, "service-ok", "回滚后应直连到真实服务");
    assert_eq!(
        service_accepted
            .recv_timeout(ACCEPT_TIMEOUT)
            .expect("回滚后真实服务应当再次收到连接"),
        REQUEST.len(),
        "回滚后服务应直接收到客户端请求"
    );

    relay.stop();
}
