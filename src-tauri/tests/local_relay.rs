//! 本地中继的转发验证
//!
//! 纯用户态、不需要管理员权限，因此作为常规测试运行（非 `#[ignore]`）：
//! 中继监听一个由系统分配的端口，把连接转给一个回显服务，验证字节双向可达。

use std::io::{Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::thread;
use std::time::Duration;

use dns_proxy_lib::relay::{LocalRelay, RelayConfig};

/// 起一个回显服务，作为中继要连的目标
fn spawn_echo() -> SocketAddr {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).expect("绑定回显服务失败");
    let address = listener.local_addr().unwrap();
    thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { break };
            thread::spawn(move || {
                let mut buffer = [0u8; 1024];
                loop {
                    match stream.read(&mut buffer) {
                        Ok(0) | Err(_) => break,
                        Ok(length) => {
                            if stream.write_all(&buffer[..length]).is_err() {
                                break;
                            }
                        }
                    }
                }
            });
        }
    });
    address
}

/// 监听回环、由系统分配端口、不限制来源
fn loopback_config(destination: SocketAddr) -> RelayConfig {
    RelayConfig {
        bind_addr: Ipv4Addr::LOCALHOST,
        listen_port: 0,
        destination,
        allowed_peer: None,
    }
}

#[test]
fn relay_forwards_bytes_in_both_directions() {
    let echo_address = spawn_echo();

    let relay = LocalRelay::new();
    let status = relay
        .start(loopback_config(echo_address))
        .expect("启动中继失败");
    assert!(status.running);
    let listen_address: SocketAddr = status
        .listen_addr
        .as_ref()
        .expect("应报告监听地址")
        .parse()
        .expect("监听地址应可解析");
    assert_ne!(listen_address.port(), 0, "端口 0 应由系统分配成真实端口");

    let mut client = TcpStream::connect(listen_address).expect("连接中继失败");
    client
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();

    let payload = b"relay-check-0123456789";
    client.write_all(payload).expect("写入中继失败");
    client.flush().unwrap();

    let mut received = vec![0u8; payload.len()];
    client
        .read_exact(&mut received)
        .expect("从回显服务读回数据失败");
    assert_eq!(&received, payload, "回显内容应与写入完全一致");

    // 计数器在转发线程里更新，留一点时间落地
    thread::sleep(Duration::from_millis(200));
    let status = relay.status();
    println!("中继状态: {status:?}");
    assert_eq!(status.accepted, 1, "应当只接受了这一条连接");
    assert_eq!(status.rejected, 0, "不应有拒绝");
    assert!(
        status.bytes_up >= payload.len() as u64,
        "上行字节数应不少于写入量：{status:?}"
    );
    assert!(
        status.bytes_down >= payload.len() as u64,
        "下行字节数应不少于回显量：{status:?}"
    );
    assert!(
        status.last_error.is_none(),
        "不应出现错误：{:?}",
        status.last_error
    );

    drop(client);
    let stopped = relay.stop();
    assert!(!stopped.running, "停止后状态应为未运行：{stopped:?}");

    // 监听器已随线程退出而关闭，新连接必须被拒
    assert!(
        TcpStream::connect(listen_address).is_err(),
        "停止后不应还能连上中继"
    );
}

#[test]
fn relay_reports_the_destination_it_could_not_reach() {
    // 目标端口用 0：它是无效端口，连接会在毫秒级确定失败。
    // 不用「某个关闭的端口」是因为本机上那要等约 2 秒才失败（疑似被 VPN 的
    // 过滤器先吞掉 SYN 再放行），会让测试变得又慢又不稳。
    let relay = LocalRelay::new();
    let status = relay
        .start(loopback_config("127.0.0.1:0".parse().unwrap()))
        .expect("启动中继失败");
    let listen_address: SocketAddr = status.listen_addr.unwrap().parse().unwrap();

    let client = TcpStream::connect(listen_address).expect("连接中继失败");
    thread::sleep(Duration::from_millis(400));

    let status = relay.status();
    assert_eq!(status.accepted, 1);
    assert!(
        status.last_error.is_some(),
        "连不上目标时应记录错误：{status:?}"
    );
    drop(client);
    relay.stop();
}

#[test]
fn relay_rejects_connections_from_unexpected_peers() {
    // 这是"别把自己变成开放中继"的那道闸：重定向的分支 1 会交换源/目的地址，
    // 把原目标地址写进源地址，所以中继看到的对端必然是原目标；来源不符的一律拒掉。
    let echo_address = spawn_echo();
    let relay = LocalRelay::new();
    let status = relay
        .start(RelayConfig {
            bind_addr: Ipv4Addr::LOCALHOST,
            listen_port: 0,
            destination: echo_address,
            allowed_peer: Some("10.0.0.1".parse().unwrap()),
        })
        .expect("启动中继失败");
    let listen_address: SocketAddr = status.listen_addr.unwrap().parse().unwrap();

    let mut client = TcpStream::connect(listen_address).expect("连接中继失败");
    thread::sleep(Duration::from_millis(300));

    let status = relay.status();
    println!("中继状态: {status:?}");
    assert_eq!(status.accepted, 0, "来源不符的连接不该被接受：{status:?}");
    assert_eq!(status.rejected, 1, "应当记一次拒绝：{status:?}");

    // 被拒的连接应当已被关闭：读会立刻拿到 EOF
    client
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    let mut buffer = [0u8; 8];
    let read = client.read(&mut buffer);
    assert!(
        matches!(read, Ok(0)) || read.is_err(),
        "被拒的连接不应有数据可读：{read:?}"
    );

    drop(client);
    relay.stop();
}
