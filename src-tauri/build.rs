fn main() {
    // 让 exe 运行时请求管理员权限（双击弹 UAC）：
    // 监听本地 53 端口、释放被占用的端口和写入系统级配置都需要提升权限
    let mut windows = tauri_build::WindowsAttributes::new();
    windows = windows.app_manifest(include_str!("app-manifest.xml"));
    let attrs = tauri_build::Attributes::new().windows_attributes(windows);
    tauri_build::try_build(attrs).expect("failed to run build script");
}
