//! 「应用拿到 IP 后仍能连接」的端到端验证
//!
//! 链路：
//!
//! 1. 像真实应用一样解析域名——走系统解析器，也就是本程序接管的 DNS；
//! 2. 对它解析出的 IPv4 建立重定向，让连接落到本机的一个服务上；
//! 3. 真的连上去（普通 `TcpStream`，就是应用会做的事），断言拿到本机服务的应答。
//!
//! **一个字节都不会发往互联网。** 客户端发往那个公网 IP 的 SYN 会被重定向的分支 1
//! 取走，注入出去的是改写后的副本（交换地址后目的地址是客户端本机），原来的包从未
//! 离开本机。因此这个测试既能验证"应用拿到 IP 后仍能连接"，又不需要任何外网流量。
//!
//! 需要管理员权限，并且本程序正在运行以提供 DNS：
//!
//! ```text
//! cargo test --test windivert_dns_connect -- --ignored --nocapture
//! ```

use std::io::{Read, Write};
use std::net::{IpAddr, SocketAddr, TcpListener, TcpStream, ToSocketAddrs, UdpSocket};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use dns_proxy_lib::windivert::{locate, RedirectRule, Redirector};

mod common;
use common::{free_low_ports, runtime_dir};

const TIMEOUT: Duration = Duration::from_secs(10);
const REQUEST: &[u8] = b"GET / HTTP/1.1\r\nHost: placeholder\r\n\r\n";

/// 本机服务：先读请求，再回一句招呼；把每次读到的字节数记到通道
fn spawn_service(listener: TcpListener, greeting: &'static [u8]) -> mpsc::Receiver<usize> {
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { break };
            let sender = sender.clone();
            thread::spawn(move || {
                let mut buffer = [0u8; 256];
                if let Ok(length) = stream.read(&mut buffer) {
                    let _ = sender.send(length);
                    let _ = stream.write_all(greeting);
                    let _ = stream.flush();
                }
                thread::sleep(Duration::from_millis(400));
            });
        }
    });
    receiver
}

/// 像普通客户端那样连上去发请求、读应答
fn exchange(target: SocketAddr) -> std::io::Result<String> {
    let mut stream = TcpStream::connect(target)?;
    stream.set_read_timeout(Some(TIMEOUT))?;
    stream.write_all(REQUEST)?;
    stream.flush()?;
    let mut buffer = [0u8; 64];
    let length = stream.read(&mut buffer)?;
    Ok(String::from_utf8_lossy(&buffer[..length]).to_string())
}

#[test]
#[ignore = "需要管理员权限，且本程序需正在运行以提供 DNS"]
fn application_can_still_connect_to_an_ip_it_resolved() {
    let runtime_dir = runtime_dir();
    assert!(
        locate(&runtime_dir).is_some(),
        "缺少 {}，无法验证",
        runtime_dir.join("WinDivert.dll").display()
    );

    let name = std::env::var("WINDIVERT_DNS_NAME").unwrap_or_else(|_| "www.example.org".to_string());
    let port: u16 = std::env::var("WINDIVERT_DNS_PORT")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(8999);

    // 第一步：真实应用的解析路径（getaddrinfo → 系统 DNS → 本程序）
    let target = (name.as_str(), port)
        .to_socket_addrs()
        .unwrap_or_else(|error| panic!("解析 {name} 失败：{error}（本程序的 DNS 在运行吗？）"))
        .find(|address| matches!(address.ip(), IpAddr::V4(_)))
        .unwrap_or_else(|| panic!("{name} 没有解析出 IPv4 地址"));
    println!("应用解析 {name}:{port} -> {target}");

    // 第二步：本机服务。先用一次 UDP connect 问内核会选哪个源地址——UDP 的
    // connect 只设置默认对端，不发送任何报文，所以这一步同样零外网流量。
    let probe = UdpSocket::bind("0.0.0.0:0").expect("探测套接字创建失败");
    probe.connect(target).expect("探测路由失败");
    let bind_ip = match probe.local_addr().expect("读取本地地址失败").ip() {
        IpAddr::V4(address) => address,
        IpAddr::V6(_) => panic!("本测试仅支持 IPv4"),
    };
    drop(probe);
    println!("面向该目标的源地址是 {bind_ip}，本机服务绑在它上面");

    let ports = free_low_ports(2);
    let (service_port, sentinel_port) = (ports[0], ports[1]);
    let service = TcpListener::bind((bind_ip, service_port)).expect("绑定本机服务失败");
    let service_received = spawn_service(service, b"local-service-ok");

    let target_addr = match target.ip() {
        IpAddr::V4(address) => address,
        IpAddr::V6(_) => unreachable!(),
    };

    // 第三步：把该公网目标的连接改投到本机服务
    let redirector = Redirector::new(runtime_dir);
    let status = redirector
        .start(RedirectRule {
            target_addr,
            target_port: port,
            relay_port: service_port,
            sentinel_port,
        })
        .unwrap_or_else(|error| panic!("启动重定向失败：{error}（该测试需要管理员权限）"));
    assert!(status.running, "重定向应处于运行中：{status:?}");

    // 第四步：应用真的去连它解析出来的那个 IP
    let reply = match exchange(target) {
        Ok(reply) => reply,
        Err(error) => {
            let status = redirector.status();
            println!("连接失败：{error}");
            println!("计数器：{status:?}");
            panic!("应用无法连接它解析出来的地址：{error}");
        }
    };

    assert_eq!(
        reply, "local-service-ok",
        "应答应来自本机服务，实际收到：{reply:?}"
    );
    let received = service_received
        .recv_timeout(TIMEOUT)
        .expect("本机服务应当收到应用发出的请求");
    assert_eq!(
        received,
        REQUEST.len(),
        "本机服务应原样收到请求，说明上行也穿过了改写"
    );

    let status = redirector.status();
    println!(
        "计数器：客户端→中继 {} / 中继→客户端 {} / 拨号映射 {} / 回包映射 {} / 原样放行 {} / 跳过 {} / 注入失败 {}",
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
    assert_eq!(
        status.dial_mapped, 0,
        "本用例没有中继拨号，不该出现分支 3：{status:?}"
    );
    assert_eq!(
        status.send_failed, 0,
        "不应有注入失败：{status:?}"
    );

    // 回滚：停止后不该再接管。这里不再向公网目标发起连接——
    // 那会产生真实的出网报文，而本用例的前提就是零外网流量。
    let stopped = redirector.stop();
    assert!(!stopped.running, "停止后应为未运行：{stopped:?}");
    assert!(stopped.rule.is_none(), "停止后不该再报告规则：{stopped:?}");
}
