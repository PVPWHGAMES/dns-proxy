//! WinDivert 运行时绑定
//!
//! 只声明本项目用到的入口，并且一律在运行时加载 WinDivert.dll：若改成链接期依赖，
//! DLL 一旦缺失（被杀软隔离、被误删）进程会在加载阶段直接起不来，而那时系统 DNS
//! 可能正指向本程序，整机域名解析会跟着一起失效。
//!
//! 两条实测得到的部署约束（见 docs/aegis/specs/2026-09-14-windivert-feasibility.md）：
//! 1. `WinDivert64.sys` 必须与 `WinDivert.dll` 同目录。2.2.2 的 DLL 不内嵌驱动：
//!    只有 DLL 时驱动服务能建出来但启动失败（错误码 2），表现为「打开设备失败(2)」。
//! 2. 驱动路径按 DLL 自身所在目录解析，与当前工作目录无关，因此计划任务启动
//!    （工作目录为 System32）同样可用。
//!
//! kernel32 的几个入口 ABI 长期稳定，直接声明即可，不必为它们引入额外 feature。

use std::ffi::c_void;
use std::path::{Path, PathBuf};

pub mod monitor;
pub mod redirect;
pub(crate) mod worker;

pub use monitor::{FlowEntry, FlowMonitor, FlowMonitorStatus};
pub use redirect::{RedirectRule, RedirectStatus, Redirector};

#[link(name = "kernel32")]
extern "system" {
    fn LoadLibraryExW(file: *const u16, h_file: *mut c_void, flags: u32) -> *mut c_void;
    fn GetProcAddress(module: *mut c_void, name: *const u8) -> *mut c_void;
    fn FreeLibrary(module: *mut c_void) -> i32;

    pub(crate) fn GetLastError() -> u32;
    pub(crate) fn OpenProcess(access: u32, inherit: i32, pid: u32) -> *mut c_void;
    pub(crate) fn QueryFullProcessImageNameW(
        process: *mut c_void,
        flags: u32,
        name: *mut u16,
        size: *mut u32,
    ) -> i32;
    pub(crate) fn CloseHandle(handle: *mut c_void) -> i32;
}

/// 只在 DLL 所在目录及其子目录里找依赖，避免从当前目录或 PATH 误加载同名文件
const LOAD_WITH_ALTERED_SEARCH_PATH: u32 = 0x0000_0008;
pub(crate) const PROCESS_QUERY_LIMITED_INFORMATION: u32 = 0x1000;

pub(crate) const ERROR_FILE_NOT_FOUND: u32 = 2;
pub(crate) const ERROR_ACCESS_DENIED: u32 = 5;
pub(crate) const ERROR_INVALID_HANDLE: u32 = 6;
pub(crate) const ERROR_NO_DATA: u32 = 232;

/// WinDivert 层：`WINDIVERT_LAYER_NETWORK`
pub const LAYER_NETWORK: i32 = 0;

/// WinDivert 层：`WINDIVERT_LAYER_FLOW`
pub const LAYER_FLOW: i32 = 2;

/// `WINDIVERT_FLAG_SNIFF | WINDIVERT_FLAG_RECV_ONLY`
///
/// SNIFF 让包「复制后放行」而不是「取走后放行」，RECV_ONLY 则从类型上排除了发送；
/// 两者相加意味着这个句柄无论如何都不会改变、丢弃或注入任何数据包。
pub const FLAGS_OBSERVE_ONLY: u64 = 0x0001 | 0x0004;

/// `WINDIVERT_EVENT_FLOW_ESTABLISHED`
pub const EVENT_FLOW_ESTABLISHED: u8 = 1;
/// `WINDIVERT_EVENT_FLOW_DELETED`
pub const EVENT_FLOW_DELETED: u8 = 2;

/// `WINDIVERT_SHUTDOWN_RECV`：用于打断阻塞中的 `WinDivertRecv`
const SHUTDOWN_RECV: u32 = 0x0001;

type OpenFn = unsafe extern "system" fn(*const u8, i32, i16, u64) -> *mut c_void;
type RecvFn =
    unsafe extern "system" fn(*mut c_void, *mut c_void, u32, *mut u32, *mut WindivertAddress) -> i32;
type ShutdownFn = unsafe extern "system" fn(*mut c_void, u32) -> i32;
type CloseFn = unsafe extern "system" fn(*mut c_void) -> i32;
type FormatAddressFn = unsafe extern "system" fn(*const u32, *mut u8, u32) -> i32;
type SendFn =
    unsafe extern "system" fn(*mut c_void, *const c_void, u32, *mut u32, *const WindivertAddress) -> i32;
type CalcChecksumsFn = unsafe extern "system" fn(*mut c_void, u32, *mut WindivertAddress, u64) -> i32;

/// 与 `windivert.h` 中 `WINDIVERT_ADDRESS` 逐字节对应（共 80 字节）
///
/// C 侧是位域，MSVC 从最低位开始分配合成 32 位单元，所以：
/// `Layer` 占 bit0..8、`Event` 占 bit8..16、`Sniffed`/`Outbound`/`Loopback` 依次占
/// bit16/17/18。位序写错的后果不是崩溃而是「永远收不到事件」，所以有单测钉住它。
#[repr(C)]
#[derive(Clone, Copy)]
pub struct WindivertAddress {
    pub timestamp: i64,
    pub flags: u32,
    pub reserved2: u32,
    pub data: [u8; 64],
}

impl Default for WindivertAddress {
    fn default() -> Self {
        // 数组长度超过 32 就没有 Default 实现，手工给零值
        Self {
            timestamp: 0,
            flags: 0,
            reserved2: 0,
            data: [0u8; 64],
        }
    }
}

impl WindivertAddress {
    pub fn layer(&self) -> u8 {
        (self.flags & 0xFF) as u8
    }

    pub fn event(&self) -> u8 {
        ((self.flags >> 8) & 0xFF) as u8
    }

    pub fn outbound(&self) -> bool {
        self.flags & (1 << 17) != 0
    }

    pub fn loopback(&self) -> bool {
        self.flags & (1 << 18) != 0
    }

    /// 改写收发方向
    ///
    /// 重定向的关键一步：出站包必须翻成入站包才能被本机套接字接收
    /// （官方 `streamdump` 的 "reflecting outbound into inbound" 即为此）。
    pub fn set_outbound(&mut self, outbound: bool) {
        if outbound {
            self.flags |= 1 << 17;
        } else {
            self.flags &= !(1 << 17);
        }
    }

    /// 标记为「本程序注入的包」
    ///
    /// 文档明言 WinDivert 无法阻止注入的包被再次捕获，对策就是配合过滤器里的
    /// `!impostor`：这样即便某个改写结果又落回过滤器条件，也不会形成自噬。
    /// 代价是驱动会对 impostor 包自动递减 TTL（它自带的防环兜底）。
    pub fn set_impostor(&mut self, impostor: bool) {
        if impostor {
            self.flags |= 1 << 19;
        } else {
            self.flags &= !(1 << 19);
        }
    }

    /// 解析联合体里的 `WINDIVERT_DATA_FLOW`
    ///
    /// 联合体起始于地址结构偏移 16 处，即 `data[0]`；字段偏移与 `windivert.h` 一一对应。
    pub fn flow(&self) -> FlowData {
        let read_u16 = |offset: usize| {
            u16::from_le_bytes([self.data[offset], self.data[offset + 1]])
        };
        let read_u32 = |offset: usize| {
            u32::from_le_bytes([
                self.data[offset],
                self.data[offset + 1],
                self.data[offset + 2],
                self.data[offset + 3],
            ])
        };
        let read_u64 = |offset: usize| {
            let mut bytes = [0u8; 8];
            bytes.copy_from_slice(&self.data[offset..offset + 8]);
            u64::from_le_bytes(bytes)
        };
        let read_addr = |offset: usize| {
            [
                read_u32(offset),
                read_u32(offset + 4),
                read_u32(offset + 8),
                read_u32(offset + 12),
            ]
        };

        FlowData {
            endpoint_id: read_u64(0),
            parent_endpoint_id: read_u64(8),
            process_id: read_u32(16),
            local_addr: read_addr(20),
            remote_addr: read_addr(36),
            local_port: read_u16(52),
            remote_port: read_u16(54),
            protocol: self.data[56],
        }
    }
}

/// `WINDIVERT_DATA_FLOW`
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FlowData {
    pub endpoint_id: u64,
    pub parent_endpoint_id: u64,
    pub process_id: u32,
    pub local_addr: [u32; 4],
    pub remote_addr: [u32; 4],
    pub local_port: u16,
    pub remote_port: u16,
    pub protocol: u8,
}

/// WinDivert 句柄：裸指针本身不满足 Send，但句柄语义上可在其他线程收发
#[derive(Clone, Copy)]
pub(crate) struct RawHandle(pub(crate) *mut c_void);

unsafe impl Send for RawHandle {}
unsafe impl Sync for RawHandle {}

/// 已加载的 WinDivert 绑定
///
/// 模块句柄故意不释放：进程生命周期内只加载一次，释放后再有人用到函数指针就是野指针，
/// 而这点内存不值得为它引入引用计数。
pub struct Windivert {
    open: OpenFn,
    recv: RecvFn,
    send: SendFn,
    calc_checksums: CalcChecksumsFn,
    shutdown: ShutdownFn,
    close: CloseFn,
    format_address: FormatAddressFn,
}

unsafe impl Send for Windivert {}
unsafe impl Sync for Windivert {}

impl Windivert {
    /// 以嗅探 + 只收模式打开指定层
    pub(crate) fn open(&self, filter: &str, layer: i32, priority: i16, flags: u64) -> Result<RawHandle, u32> {
        let mut filter_bytes = filter.as_bytes().to_vec();
        filter_bytes.push(0);

        let handle = unsafe {
            (self.open)(
                filter_bytes.as_ptr(),
                layer,
                priority,
                flags,
            )
        };
        if handle.is_null() {
            return Err(unsafe { GetLastError() });
        }
        Ok(RawHandle(handle))
    }

    /// 收一个事件
    ///
    /// FLOW 层没有数据包载荷，因此包缓冲区传空、长度为 0（官方 `flowtrack` 同样如此）。
    pub(crate) fn recv(&self, handle: RawHandle, address: &mut WindivertAddress) -> Result<(), u32> {
        let ok = unsafe {
            (self.recv)(
                handle.0,
                std::ptr::null_mut(),
                0,
                std::ptr::null_mut(),
                address,
            )
        };
        if ok == 0 {
            return Err(unsafe { GetLastError() });
        }
        Ok(())
    }

    /// 收一个数据包（NETWORK 层）
    pub(crate) fn recv_packet(
        &self,
        handle: RawHandle,
        buffer: &mut [u8],
        address: &mut WindivertAddress,
    ) -> Result<usize, u32> {
        let mut received = 0u32;
        let ok = unsafe {
            (self.recv)(
                handle.0,
                buffer.as_mut_ptr() as *mut c_void,
                buffer.len() as u32,
                &mut received,
                address,
            )
        };
        if ok == 0 {
            return Err(unsafe { GetLastError() });
        }
        Ok(received as usize)
    }

    /// 注入一个数据包
    pub(crate) fn send(
        &self,
        handle: RawHandle,
        packet: &[u8],
        address: &WindivertAddress,
    ) -> Result<(), u32> {
        let mut sent = 0u32;
        let ok = unsafe {
            (self.send)(
                handle.0,
                packet.as_ptr() as *const c_void,
                packet.len() as u32,
                &mut sent,
                address,
            )
        };
        if ok == 0 {
            return Err(unsafe { GetLastError() });
        }
        Ok(())
    }

    /// 重算校验和并同步更新地址里的校验和标志
    ///
    /// 照官方 `streamdump` 的做法显式调用，而不是依赖驱动在标志为 0 时自行重算：
    /// 抓到的包在开启校验和卸载时可能带着无效校验和，显式重算不留歧义。
    pub(crate) fn calc_checksums(&self, packet: &mut [u8], address: &mut WindivertAddress) {
        unsafe {
            (self.calc_checksums)(
                packet.as_mut_ptr() as *mut c_void,
                packet.len() as u32,
                address,
                0,
            );
        }
    }

    /// 打断阻塞中的 [`Windivert::recv`]
    pub(crate) fn shutdown(&self, handle: RawHandle) {
        unsafe { (self.shutdown)(handle.0, SHUTDOWN_RECV) };
    }

    pub(crate) fn close(&self, handle: RawHandle) {
        unsafe { (self.close)(handle.0) };
    }

    /// 格式化地址。IPv4 也走这个入口：官方用法就是同一个函数（`flowtrack` 即如此），
    /// 自己按字节序拼串反而容易在 IPv4 映射到 IPv6 的形态上出错。
    pub fn format_address(&self, addr: &[u32; 4]) -> String {
        let mut buffer = [0u8; 64];
        let ok = unsafe { (self.format_address)(addr.as_ptr(), buffer.as_mut_ptr(), buffer.len() as u32) };
        if ok == 0 {
            return "?".to_string();
        }
        let len = buffer.iter().position(|b| *b == 0).unwrap_or(buffer.len());
        String::from_utf8_lossy(&buffer[..len]).to_string()
    }
}

/// DLL 候选路径
///
/// `--no-bundle` 构建由 `build.rs` 复制到 exe 同级；打成安装包时由
/// `bundle.resources` 落在资源目录。两种布局都覆盖，省得为路径差异反复排查。
pub fn candidates(exe_dir: &Path) -> Vec<PathBuf> {
    vec![
        exe_dir.join("WinDivert.dll"),
        exe_dir.join("bin").join("WinDivert.dll"),
        exe_dir.join("resources").join("WinDivert.dll"),
        exe_dir.join("resources").join("bin").join("WinDivert.dll"),
    ]
}

/// 定位 WinDivert.dll，不触发加载
pub fn locate(exe_dir: &Path) -> Option<PathBuf> {
    candidates(exe_dir).into_iter().find(|path| path.is_file())
}

/// 加载 WinDivert.dll 并取回所需入口
pub fn load(exe_dir: &Path) -> Result<Windivert, String> {
    let dll = locate(exe_dir).ok_or_else(|| {
        let tried = candidates(exe_dir)
            .iter()
            .map(|path| path.display().to_string())
            .collect::<Vec<_>>()
            .join("、");
        format!("未找到 WinDivert.dll，已查找：{}", tried)
    })?;

    // 提前拦住「只有 DLL 没有驱动」这种最难定位的情况
    let driver = dll.with_file_name("WinDivert64.sys");
    if !driver.is_file() {
        return Err(format!(
            "缺少驱动文件 {}，它必须与 WinDivert.dll 放在同一目录",
            driver.display()
        ));
    }

    let wide = to_wide(&dll);
    let module = unsafe {
        LoadLibraryExW(
            wide.as_ptr(),
            std::ptr::null_mut(),
            LOAD_WITH_ALTERED_SEARCH_PATH,
        )
    };
    if module.is_null() {
        let code = unsafe { GetLastError() };
        return Err(format!("加载 {} 失败（错误 {}）", dll.display(), code));
    }

    let resolve = |name: &[u8]| -> Option<*mut c_void> {
        let pointer = unsafe { GetProcAddress(module, name.as_ptr()) };
        if pointer.is_null() {
            None
        } else {
            Some(pointer)
        }
    };

    let bindings = (|| -> Result<Windivert, String> {
        let require = |name: &[u8]| {
            resolve(name).ok_or_else(|| {
                format!(
                    "{} 缺少导出函数 {}",
                    dll.display(),
                    String::from_utf8_lossy(&name[..name.len() - 1])
                )
            })
        };

        Ok(Windivert {
            open: unsafe { std::mem::transmute::<*mut c_void, OpenFn>(require(b"WinDivertOpen\0")?) },
            recv: unsafe { std::mem::transmute::<*mut c_void, RecvFn>(require(b"WinDivertRecv\0")?) },
            send: unsafe { std::mem::transmute::<*mut c_void, SendFn>(require(b"WinDivertSend\0")?) },
            calc_checksums: unsafe {
                std::mem::transmute::<*mut c_void, CalcChecksumsFn>(require(
                    b"WinDivertHelperCalcChecksums\0",
                )?)
            },
            shutdown: unsafe {
                std::mem::transmute::<*mut c_void, ShutdownFn>(require(b"WinDivertShutdown\0")?)
            },
            close: unsafe {
                std::mem::transmute::<*mut c_void, CloseFn>(require(b"WinDivertClose\0")?)
            },
            format_address: unsafe {
                std::mem::transmute::<*mut c_void, FormatAddressFn>(require(
                    b"WinDivertHelperFormatIPv6Address\0",
                )?)
            },
        })
    })();

    match bindings {
        Ok(windivert) => Ok(windivert),
        Err(error) => {
            unsafe { FreeLibrary(module) };
            Err(error)
        }
    }
}

fn to_wide(path: &Path) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;
    path.as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

/// 查进程可执行文件名
///
/// 查不到就退回 `PID 1234`：界面上显示什么都可以，唯独不能因为查不到名字就丢掉这条流。
pub(crate) fn process_name(pid: u32) -> String {
    if pid == 0 {
        return "System Idle".to_string();
    }
    if pid == 4 {
        return "System".to_string();
    }

    let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if handle.is_null() {
        return format!("PID {}", pid);
    }

    let mut buffer = [0u16; 512];
    let mut size = buffer.len() as u32;
    let ok = unsafe { QueryFullProcessImageNameW(handle, 0, buffer.as_mut_ptr(), &mut size) };
    unsafe { CloseHandle(handle) };

    if ok == 0 || size == 0 {
        return format!("PID {}", pid);
    }

    let path = String::from_utf16_lossy(&buffer[..size as usize]);
    path.rsplit(['\\', '/'])
        .next()
        .filter(|name| !name.is_empty())
        .unwrap_or(&path)
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn address_layout_matches_windivert_header() {
        // 与 windivert.h 对齐关系：i64 + u32 位域字 + u32 保留 + 64 字节联合体
        assert_eq!(std::mem::size_of::<WindivertAddress>(), 80);
        assert_eq!(std::mem::align_of::<WindivertAddress>(), 8);
    }

    #[test]
    fn bitfield_decodes_layer_event_and_flags() {
        let mut address = WindivertAddress::default();

        // Layer=2(FLOW)、Event=1(ESTABLISHED)、Outbound=1、Loopback=0
        address.flags = 2 | (1 << 8) | (1 << 17);
        assert_eq!(address.layer(), LAYER_FLOW as u8);
        assert_eq!(address.event(), EVENT_FLOW_ESTABLISHED);
        assert!(address.outbound());
        assert!(!address.loopback());

        address.flags = 2 | (2 << 8) | (1 << 18);
        assert_eq!(address.event(), EVENT_FLOW_DELETED);
        assert!(!address.outbound());
        assert!(address.loopback());
    }

    #[test]
    fn flow_fields_are_read_from_header_offsets() {
        let mut address = WindivertAddress::default();
        address.data[0..8].copy_from_slice(&0x1122_3344_5566_7788u64.to_le_bytes());
        address.data[8..16].copy_from_slice(&42u64.to_le_bytes());
        address.data[16..20].copy_from_slice(&4242u32.to_le_bytes());
        // 本地地址 127.0.0.1：网络字节序下按 32 位读是 0x0100007F
        address.data[20..24].copy_from_slice(&0x0100_007Fu32.to_le_bytes());
        address.data[36..40].copy_from_slice(&0x0808_0808u32.to_le_bytes());
        address.data[52..54].copy_from_slice(&51000u16.to_le_bytes());
        address.data[54..56].copy_from_slice(&53u16.to_le_bytes());
        address.data[56] = 17;

        let flow = address.flow();
        assert_eq!(flow.endpoint_id, 0x1122_3344_5566_7788);
        assert_eq!(flow.parent_endpoint_id, 42);
        assert_eq!(flow.process_id, 4242);
        assert_eq!(flow.local_addr[0], 0x0100_007F);
        assert_eq!(flow.remote_addr[0], 0x0808_0808);
        assert_eq!(flow.local_port, 51000);
        assert_eq!(flow.remote_port, 53);
        assert_eq!(flow.protocol, 17);
        // 联合体只用到前 57 字节，后面必须是原样
        assert_eq!(flow.remote_addr[3], 0);
    }

    #[test]
    fn candidates_prefer_exe_directory() {
        let exe_dir = Path::new("C:\\Program Files\\DNS Proxy");
        let list = candidates(exe_dir);
        assert!(list[0].ends_with("WinDivert.dll"));
        assert_eq!(list[0].parent().unwrap(), exe_dir);
        assert!(list.iter().any(|path| path.ends_with("bin\\WinDivert.dll")));
    }

    #[test]
    fn load_reports_missing_dll_with_paths() {
        let empty = std::env::temp_dir().join("windivert-test-missing");
        let _ = std::fs::create_dir_all(&empty);
        let error = load(&empty).err().expect("空目录里不应加载成功");
        assert!(error.contains("未找到 WinDivert.dll"), "实际错误：{error}");
        let _ = std::fs::remove_dir_all(&empty);
    }

    #[test]
    fn load_refuses_dll_without_driver_next_to_it() {
        // 用真实 DLL 复制到临时目录，但故意不放 WinDivert64.sys。
        // 这条规则是实测出来的：只有 DLL 时驱动服务能建但启动失败（错误 2），
        // 报错信息会淹没在「打开设备失败」里，所以必须提前拦住。
        let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("bin").join("WinDivert.dll");
        if !source.is_file() {
            return; // 未随仓库提供二进制时跳过，避免把测试写成环境依赖
        }

        let dir = std::env::temp_dir().join("windivert-test-no-driver");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::copy(&source, dir.join("WinDivert.dll")).unwrap();

        let error = load(&dir).err().expect("缺驱动时不应加载成功");
        assert!(error.contains("WinDivert64.sys"), "实际错误：{error}");

        let _ = std::fs::remove_dir_all(&dir);
    }
}
