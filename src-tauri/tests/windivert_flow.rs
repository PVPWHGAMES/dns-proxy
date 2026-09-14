//! FLOW 层观察的端到端验证
//!
//! 需要管理员权限（打开 WinDivert 句柄会加载内核驱动），因此默认忽略：
//!
//! ```text
//! cargo test --test windivert_flow -- --ignored --nocapture
//! ```
//!
//! 这个测试是结构体布局的唯一硬证据：`WINDIVERT_ADDRESS` 的位域若解错，
//! 事件类型根本匹配不上，下面的断言会一条流都看不到。

use std::net::{Ipv4Addr, TcpListener, TcpStream};
use std::path::PathBuf;
use std::thread;
use std::time::Duration;

use dns_proxy_lib::windivert::{locate, FlowMonitor};

/// 运行库所在目录
///
/// 默认指向仓库里的 `bin/`；用 `WINDIVERT_RUNTIME_DIR` 可以改指别处，
/// 便于验证「DLL 与驱动放在 exe 同级」这种发布布局是否真的能被解析到。
fn runtime_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("WINDIVERT_RUNTIME_DIR") {
        return PathBuf::from(dir);
    }
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("bin")
}

#[test]
#[ignore = "需要管理员权限"]
fn flow_monitor_sees_our_own_connections() {
    let runtime_dir = runtime_dir();
    assert!(
        locate(&runtime_dir).is_some(),
        "缺少 {}，无法验证",
        runtime_dir.join("WinDivert.dll").display()
    );

    let monitor = FlowMonitor::new(runtime_dir);
    let status = monitor
        .start()
        .unwrap_or_else(|error| panic!("启动观察失败：{error}（该测试需要管理员权限）"));
    assert!(status.running, "启动后状态应为运行中：{status:?}");
    assert!(status.available);

    // 造两类流量：回环 TCP（本机到本机）和一条跨网卡的 TCP
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).expect("绑定回环端口失败");
    let port = listener.local_addr().unwrap().port();
    let client = TcpStream::connect((Ipv4Addr::LOCALHOST, port)).expect("回环连接失败");
    let (server, _) = listener.accept().expect("接受回环连接失败");
    let outbound = TcpStream::connect(("223.5.5.5", 53)).ok();

    thread::sleep(Duration::from_millis(1500));

    let flows = monitor.snapshot();
    println!("共观察到 {} 条流", flows.len());
    for flow in flows.iter().take(10) {
        println!(
            "  [{}] {} pid={} {} {}:{} -> {}:{} 回环={} 出站={}",
            flow.established_at,
            flow.protocol,
            flow.process_id,
            flow.process_name,
            flow.local_addr,
            flow.local_port,
            flow.remote_addr,
            flow.remote_port,
            flow.loopback,
            flow.outbound
        );
    }

    let own_pid = std::process::id();
    let mine: Vec<_> = flows.iter().filter(|flow| flow.process_id == own_pid).collect();
    assert!(
        !mine.is_empty(),
        "没看到本进程（pid={}，端口 {}）的流：位域布局或事件类型解析可能有误",
        own_pid,
        port
    );
    assert!(
        mine.iter().any(|flow| flow.protocol == "TCP"),
        "本进程的流里没有 TCP：{mine:?}"
    );
    assert!(
        mine.iter().any(|flow| !flow.process_name.is_empty()),
        "进程名不应为空：{mine:?}"
    );
    assert!(
        mine.iter().any(|flow| flow.local_port == port || flow.remote_port == port),
        "没看到回环端口 {} 的流：{:?}",
        port,
        mine.iter().map(|flow| flow.local_port).collect::<Vec<_>>()
    );
    assert!(
        outbound.is_some(),
        "跨网卡连接未建立，测试前提不成立"
    );

    let stopped = monitor.stop();
    assert!(!stopped.running, "停止后状态应为未运行：{stopped:?}");

    drop((client, server, outbound));
}
