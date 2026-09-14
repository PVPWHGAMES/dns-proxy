fn main() {
    // 让 exe 运行时请求管理员权限（双击弹 UAC）：
    // 监听本地 53 端口、释放被占用的端口和写入系统级配置都需要提升权限
    let mut windows = tauri_build::WindowsAttributes::new();
    windows = windows.app_manifest(include_str!("app-manifest.xml"));
    let attrs = tauri_build::Attributes::new().windows_attributes(windows);
    tauri_build::try_build(attrs).expect("failed to run build script");

    copy_windivert_runtime();
}

/// 把 WinDivert 运行库复制到 exe 同级目录
///
/// `tauri build --no-bundle` 不处理 `bundle.resources`，而运行期是运行时加载
/// （LoadLibraryExW 按 DLL 自身目录解析驱动），所以必须显式放到 exe 旁边，
/// 否则开发构建里观察功能永远报「未找到 WinDivert.dll」。
///
/// DLL 与驱动必须同目录，且两者都不可省：只有 DLL 时驱动服务能建出来但启动失败。
fn copy_windivert_runtime() {
    const FILES: [&str; 2] = ["WinDivert.dll", "WinDivert64.sys"];

    let manifest_dir = std::path::PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let source_dir = manifest_dir.join("bin");

    println!("cargo:rerun-if-changed=bin/WinDivert.dll");
    println!("cargo:rerun-if-changed=bin/WinDivert64.sys");

    // OUT_DIR 形如 target/<profile>/build/<pkg>-<hash>/out，上溯三层即 target/<profile>
    let Some(profile_dir) = std::path::PathBuf::from(std::env::var("OUT_DIR").unwrap())
        .ancestors()
        .nth(3)
        .map(std::path::Path::to_path_buf)
    else {
        return;
    };

    for file in FILES {
        let source = source_dir.join(file);
        if !source.is_file() {
            continue;
        }
        let target = profile_dir.join(file);
        // 目标文件可能是正在运行的旧副本（被占用），复制失败不应让整个构建失败
        if let Err(error) = std::fs::copy(&source, &target) {
            println!("cargo:warning=复制 {file} 到 {} 失败：{error}", profile_dir.display());
        }
    }
}
