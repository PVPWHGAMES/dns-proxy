//! 重定向会话的启动/停止顺序验证
//!
//! 不需要管理员权限：这些用例全部走"启动失败"或"参数不自洽"的路径，
//! 恰恰是最容易留下半启动状态的路径——而半启动状态（中继在监听但没人往里
//! 改投）比"什么都没启动"更糟，因为连接会被改投到一个死端点。

use std::net::Ipv4Addr;
use std::path::PathBuf;

use dns_proxy_lib::redirect_session::RedirectSession;
use dns_proxy_lib::windivert::RedirectRule;

mod common;
use common::runtime_dir;

fn rule() -> RedirectRule {
    RedirectRule {
        target_addr: Ipv4Addr::new(10, 0, 0, 1),
        target_port: 80,
        relay_port: 34567,
        sentinel_port: 34568,
    }
}

#[test]
fn start_failure_leaves_nothing_running() {
    // 运行库不在这个目录里，重定向必然起不来；此时中继必须已经被撤掉
    let session = RedirectSession::new(PathBuf::from("Z:\\definitely-not-here"));
    let error = session
        .start(rule(), Ipv4Addr::new(127, 0, 0, 1))
        .err()
        .expect("运行库缺失时应当启动失败");
    assert!(error.contains("WinDivert.dll"), "实际错误：{error}");

    let overview = session.overview();
    assert!(!overview.redirect.running, "重定向不该在运行：{overview:?}");
    assert!(
        !overview.relay.running,
        "失败后不该留下中继在监听（半启动状态）：{overview:?}"
    );
    assert!(
        overview.relay.listen_addr.is_none(),
        "监听地址应已清空：{overview:?}"
    );
}

#[test]
fn invalid_rule_is_rejected_before_the_relay_starts() {
    // 这里故意用真实的运行库目录：参数校验必须在加载运行库、绑定端口之前就拦下
    let session = RedirectSession::new(runtime_dir());
    let mut broken = rule();
    broken.relay_port = broken.target_port;

    let error = session
        .start(broken, Ipv4Addr::LOCALHOST)
        .err()
        .expect("端口自相矛盾时应当被拒");
    assert!(error.contains("环路"), "实际错误：{error}");

    let overview = session.overview();
    assert!(
        !overview.relay.running,
        "校验失败时不该动中继：{overview:?}"
    );
    assert!(!overview.redirect.running);
}

#[test]
fn stop_is_idempotent_and_clears_the_rule_view() {
    let session = RedirectSession::new(PathBuf::from("Z:\\definitely-not-here"));
    session.stop();

    let overview = session.overview();
    assert!(!overview.redirect.running);
    assert!(
        overview.redirect.rule.is_none(),
        "停止后不该再报告规则：{overview:?}"
    );
    assert!(!overview.relay.running);

    // 再停一次不应出错，也不应改变状态
    session.stop();
    assert!(!session.overview().relay.running);
}
