fn main() {
    // 让 exe 运行时请求管理员权限（双击弹 UAC），TUN 模式需要改系统 DNS 和创建虚拟网卡
    let mut windows = tauri_build::WindowsAttributes::new();
    windows = windows.app_manifest(include_str!("app-manifest.xml"));
    let attrs = tauri_build::Attributes::new().windows_attributes(windows);
    tauri_build::try_build(attrs).expect("failed to run build script");
}
