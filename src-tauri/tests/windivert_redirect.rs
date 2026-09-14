//! 回环沙盒里的最小重定向验证
//!
//! 需要管理员权限（打开的是非嗅探句柄）：
//!
//! ```text
//! cargo test --test windivert_redirect -- --ignored --nocapture
//! ```
//!
//! 全部流量都是本测试自己造的回环连接，不涉及任何外部目标：
//! 客户端连的是「原目标端口」，断言它实际上被改投到了「目标端口」，
//! 并且原目标端口自始至终没有收到连接；停止重定向后再连一次，应恢复直连。

use std::io::{Read, Write};
use std::net::{Ipv4Addr, TcpListener, TcpStream};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use dns_proxy_lib::windivert::{locate, RedirectRule, Redirector};

mod common;
use common::{bind_low_port, runtime_dir};

/// 等待 accept 的上限
const ACCEPT_TIMEOUT: Duration = Duration::from_secs(5);
/// 「确认某端口没有收到连接」需要等多久
const SILENCE: Duration = Duration::from_millis(700);

/// 起一个只 accept 一次的监听线程，接受后回一句招呼
fn spawn_once(listener: TcpListener, greeting: &'static [u8]) -> mpsc::Receiver<()> {
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        if let Ok((mut stream, _)) = listener.accept() {
            let _ = sender.send(());
            let _ = stream.write_all(greeting);
            let _ = stream.flush();
            thread::sleep(Duration::from_millis(400));
        }
    });
    receiver
}

/// 连上去并读取对端写来的招呼
fn read_greeting(port: u16) -> std::io::Result<String> {
    let mut stream = TcpStream::connect((Ipv4Addr::LOCALHOST, port))?;
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    let mut buffer = [0u8; 32];
    let length = stream.read(&mut buffer)?;
    Ok(String::from_utf8_lossy(&buffer[..length]).to_string())
}

#[test]
#[ignore = "需要管理员权限"]
fn loopback_connection_is_redirected_and_can_be_rolled_back() {
    let runtime_dir = runtime_dir();
    assert!(
        locate(&runtime_dir).is_some(),
        "缺少 {}，无法验证",
        runtime_dir.join("WinDivert.dll").display()
    );

    // 两个真实监听：original 是客户端以为自己在连的目标，redirected 是实际接手的服务
    let original = bind_low_port();
    let original_port = original.local_addr().unwrap().port();
    let redirected = bind_low_port();
    let redirected_port = redirected.local_addr().unwrap().port();
    // 本测试没有中继参与，哨兵端口只要求与目标端口、中继端口都不同
    let sentinel_port = redirected_port + 1;
    assert_ne!(sentinel_port, original_port);
    println!("原目标 127.0.0.1:{original_port} → 改投 127.0.0.1:{redirected_port}");

    let original_accepted = spawn_once(original, b"direct-ok");
    let redirected_accepted = spawn_once(redirected, b"redirected-ok");

    let redirector = Redirector::new(runtime_dir);
    let status = redirector
        .start(RedirectRule {
            target_addr: Ipv4Addr::LOCALHOST,
            target_port: original_port,
            relay_port: redirected_port,
            sentinel_port,
        })
        .unwrap_or_else(|error| panic!("启动重定向失败：{error}（该测试需要管理员权限）"));
    assert!(status.running, "启动后应处于运行中：{status:?}");
    assert!(status.available);

    // 第一段：连接应被改投
    let greeting = match read_greeting(original_port) {
        Ok(greeting) => greeting,
        Err(error) => {
            // 失败时先把证据打出来，否则只知道「连不上」，不知道卡在哪一步
            let status = redirector.status();
            println!("握手失败：{error}");
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
            println!(
                "目标端口是否收到连接：{}",
                redirected_accepted.try_recv().is_ok()
            );
            println!(
                "原目标端口是否收到连接：{}",
                original_accepted.try_recv().is_ok()
            );
            panic!("客户端无法完成握手：{error}");
        }
    };
    assert_eq!(
        greeting, "redirected-ok",
        "连接没有被改投到目标端口，实际读到：{greeting:?}"
    );
    redirected_accepted
        .recv_timeout(ACCEPT_TIMEOUT)
        .expect("目标端口应当收到这条连接");
    assert!(
        original_accepted.recv_timeout(SILENCE).is_err(),
        "原目标端口不该收到连接，说明改投没有生效"
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
    assert!(
        status.to_client > 0,
        "分支 2 应有流量（否则客户端读不到数据）：{status:?}"
    );
    assert_eq!(status.send_failed, 0, "不应有注入失败：{status:?}");

    // 第二段：停止后必须恢复直连，这就是「一键回滚」
    let stopped = redirector.stop();
    assert!(!stopped.running, "停止后状态应为未运行：{stopped:?}");

    let greeting = read_greeting(original_port).expect("回滚后应能直连原目标");
    assert_eq!(
        greeting, "direct-ok",
        "回滚后连接应直达原目标，实际读到：{greeting:?}"
    );
    original_accepted
        .recv_timeout(ACCEPT_TIMEOUT)
        .expect("回滚后原目标端口应当收到连接");
}
