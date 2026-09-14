//! 端到端测试共用的脚手架
//!
//! 各测试文件通过 `mod common;` 引入。这里只放与具体场景无关的部分：
//! 运行库目录、低位端口分配。

#![allow(dead_code)]

use std::net::{Ipv4Addr, TcpListener};
use std::path::PathBuf;

/// WinDivert 运行库所在目录
///
/// 默认指向仓库里的 `bin/`；用 `WINDIVERT_RUNTIME_DIR` 可以改指别处，
/// 便于验证「DLL 与驱动放在 exe 同级」这种发布布局。
pub fn runtime_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("WINDIVERT_RUNTIME_DIR") {
        return PathBuf::from(dir);
    }
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("bin")
}

/// 取一个低位空闲端口并一直持有它
///
/// 必须避开 Windows 的动态端口范围（49152+）：中继/哨兵端口落在那里就可能与
/// 某个客户端的临时源端口撞上，重定向的规则校验会直接拒绝。
pub fn bind_low_port() -> TcpListener {
    for port in 33_000..33_100 {
        if let Ok(listener) = TcpListener::bind((Ipv4Addr::LOCALHOST, port)) {
            return listener;
        }
    }
    panic!("33_000 起找不到空闲端口");
}

/// 取若干个互不相同、当前空闲的低位端口
///
/// 扫描期间一直持有监听，保证取到的端口互不相同且确实可用；返回前统一释放，
/// 交给被测代码去绑定。
pub fn free_low_ports(count: usize) -> Vec<u16> {
    let mut held = Vec::new();
    let mut ports = Vec::new();
    for port in 33_200..33_300 {
        if ports.len() >= count {
            break;
        }
        if let Ok(listener) = TcpListener::bind((Ipv4Addr::LOCALHOST, port)) {
            ports.push(port);
            held.push(listener);
        }
    }
    drop(held);
    assert_eq!(ports.len(), count, "低位端口不足");
    ports
}
