//! 系统 DNS 接管
//!
//! 把本机各在用网卡的 DNS 指向本地代理，并在停止或退出时精确还原。
//!
//! 这个模块会改动系统网络配置，还原失败会让整机域名解析失效，因此：
//! - 接管前先把原始配置落盘（`%APPDATA%\dns-proxy\dns-takeover.json`）
//! - 进程异常退出后，下次启动会先读该文件还原，避免残留
//! - IPv4 区分「静态配置」与「DHCP 自动下发」，还原时分别用原地址与重置，
//!   不能一律重置为 DHCP —— 那会抹掉用户手工配置的 DNS
//!
//! 所有 PowerShell 交互都走临时文件而不是命令行参数或 stdout：
//! 网卡别名可能含中文、空格与括号，命令行拼接容易被 quoting 破坏，
//! 控制台编码也可能把中文别名写坏。

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::process::Command;
use tracing::{info, warn};

/// 接管后指向的本地代理地址
const LOCAL_V4: &str = "127.0.0.1";
const LOCAL_V6: &str = "::1";

/// 单个网卡的原始 DNS 配置
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AdapterDns {
    pub if_index: u32,
    pub alias: String,
    /// None 表示 DNS 由 DHCP/RA 自动下发；Some 表示静态配置的服务器列表
    pub v4: Option<Vec<String>>,
    /// 原始 IPv6 DNS 列表（仅用于展示与判断是否曾配置）
    pub v6: Vec<String>,
}

/// 一次接管前的系统 DNS 快照
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DnsSnapshot {
    pub taken_at: String,
    pub adapters: Vec<AdapterDns>,
}

impl DnsSnapshot {
    /// 摘要，供界面展示
    pub fn summary(&self) -> String {
        self.adapters
            .iter()
            .map(|adapter| {
                let original = match &adapter.v4 {
                    Some(servers) => servers.join(","),
                    None => "自动获取".to_string(),
                };
                format!("{} ({})", adapter.alias, original)
            })
            .collect::<Vec<_>>()
            .join("; ")
    }
}

/// 待还原快照的落盘位置
pub fn pending_path() -> PathBuf {
    crate::config::AppConfig::config_path()
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."))
        .join("dns-takeover.json")
}

/// 运行 PowerShell，返回其标准输出
fn run_powershell(script: &str) -> Result<String, String> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x08000000;

    let output = Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", script])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map_err(|e| format!("启动 PowerShell 失败: {}", e))?;

    if !output.status.success() {
        return Err(format!(
            "PowerShell 执行失败（退出码 {:?}）: {}",
            output.status.code(),
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }

    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

/// 一次调用用的临时文件路径
fn temp_file(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("dns-proxy-{}-{}", std::process::id(), name))
}

/// 读取 PowerShell 用 UTF8 写出的 JSON 文件
fn read_json_file(path: &Path) -> Result<serde_json::Value, String> {
    let content = std::fs::read_to_string(path)
        .map_err(|e| format!("读取 {} 失败: {}", path.display(), e))?;
    let _ = std::fs::remove_file(path);
    serde_json::from_str(content.trim_start_matches('\u{feff}'))
        .map_err(|e| format!("解析结果失败: {}（原始内容: {}）", e, content))
}

/// 把可能是单个对象或数组的 JSON 值统一成数组
///
/// PowerShell 的 ConvertTo-Json 会把单元素数组塌缩成对象，必须在此兼容。
fn as_array(value: serde_json::Value) -> Vec<serde_json::Value> {
    match value {
        serde_json::Value::Array(items) => items,
        serde_json::Value::Null => Vec::new(),
        other => vec![other],
    }
}

/// 按逗号切分 PowerShell 传来的服务器列表
fn split_servers(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(|item| item.trim().to_string())
        .filter(|item| !item.is_empty())
        .collect()
}

/// 采集当前在用网卡的 DNS 配置
pub fn capture() -> Result<DnsSnapshot, String> {
    let out = temp_file("capture.json");
    let _ = std::fs::remove_file(&out);
    let out_literal = out.to_string_lossy().replace('\'', "''");

    let script = format!(
        r#"
$ErrorActionPreference = 'Stop'
$v4 = @{{}}
Get-CimInstance Win32_NetworkAdapterConfiguration -Filter 'IPEnabled=True' -ErrorAction SilentlyContinue | ForEach-Object {{
  $v4[[int]$_.InterfaceIndex] = $_.DNSServerSearchOrder
}}
$v6 = @{{}}
Get-DnsClientServerAddress -AddressFamily IPv6 -ErrorAction SilentlyContinue | ForEach-Object {{
  $v6[[int]$_.InterfaceIndex] = @($_.ServerAddresses)
}}
# 只有存在 DNS 客户端配置对象的网卡才能被 Set-DnsClientServerAddress 修改。
# 网桥、未取得 IP 的无线网卡虽然状态是 Up，却没有该对象，对它们设置会直接报错；
# 若把这种报错算作失败，整次接管都会被回滚。
$settable = @{{}}
Get-DnsClientServerAddress -ErrorAction SilentlyContinue | ForEach-Object {{
  $settable[[int]$_.InterfaceIndex] = $true
}}
$items = @()
Get-NetAdapter -ErrorAction SilentlyContinue | Where-Object {{ $_.Status -eq 'Up' -and $_.ifType -ne 24 -and $settable.ContainsKey([int]$_.ifIndex) }} | ForEach-Object {{
  $i = [int]$_.ifIndex
  $raw = if ($v4.ContainsKey($i)) {{ $v4[$i] }} else {{ $null }}
  $items += [pscustomobject]@{{
    if_index = $i
    alias    = [string]$_.Name
    v4_auto  = ($null -eq $raw)
    v4       = if ($null -eq $raw) {{ '' }} else {{ ($raw -join ',') }}
    v6       = if ($v6.ContainsKey($i)) {{ ($v6[$i] -join ',') }} else {{ '' }}
  }}
}}
@{{ adapters = @($items) }} | ConvertTo-Json -Compress -Depth 5 | Set-Content -Path '{out}' -Encoding UTF8
"#,
        out = out_literal
    );

    run_powershell(&script)?;
    let value = read_json_file(&out)?;

    let raw_adapters = value
        .get("adapters")
        .cloned()
        .ok_or_else(|| "采集结果缺少 adapters 字段".to_string())?;

    let mut adapters = Vec::new();
    for item in as_array(raw_adapters) {
        let if_index = item
            .get("if_index")
            .and_then(|v| v.as_u64())
            .ok_or_else(|| "采集结果缺少 if_index".to_string())? as u32;
        let alias = item
            .get("alias")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();
        let v4_auto = item
            .get("v4_auto")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);
        let v4_raw = item.get("v4").and_then(|v| v.as_str()).unwrap_or_default();
        let v6_raw = item.get("v6").and_then(|v| v.as_str()).unwrap_or_default();

        adapters.push(AdapterDns {
            if_index,
            alias,
            v4: if v4_auto {
                None
            } else {
                Some(split_servers(v4_raw))
            },
            v6: split_servers(v6_raw),
        });
    }

    info!(
        count = adapters.len(),
        detail = %DnsSnapshot {
            taken_at: String::new(),
            adapters: adapters.clone(),
        }
        .summary(),
        "已采集系统 DNS 配置"
    );

    Ok(DnsSnapshot {
        taken_at: chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string(),
        adapters,
    })
}

/// 执行一份计划脚本，返回失败信息
fn run_plan_script(plan: &DnsSnapshot, body: &str) -> Result<(), String> {
    let plan_file = temp_file("plan.json");
    let result_file = temp_file("result.json");
    let _ = std::fs::remove_file(&result_file);

    let plan_json = serde_json::to_string(plan).map_err(|e| format!("序列化计划失败: {}", e))?;
    std::fs::write(&plan_file, plan_json).map_err(|e| format!("写入计划失败: {}", e))?;

    let script = format!(
        r#"
$ErrorActionPreference = 'Continue'
$plan = Get-Content -Raw -Encoding UTF8 '{plan}' | ConvertFrom-Json
$ok = 0
$fail = 0
$msgs = @()
{body}
@{{
  ok = $ok
  fail = $fail
  messages = @($msgs)
}} | ConvertTo-Json -Compress -Depth 4 | Set-Content -Path '{result}' -Encoding UTF8
"#,
        plan = plan_file.to_string_lossy().replace('\'', "''"),
        result = result_file.to_string_lossy().replace('\'', "''"),
        body = body
    );

    let outcome = run_powershell(&script);
    let _ = std::fs::remove_file(&plan_file);

    // 脚本自身失败时也要把结果文件清掉，避免残留影响下一次读取
    let value = match outcome {
        Ok(_) => read_json_file(&result_file)?,
        Err(e) => {
            let _ = std::fs::remove_file(&result_file);
            return Err(e);
        }
    };

    let fail = value.get("fail").and_then(|v| v.as_u64()).unwrap_or(0);
    let messages: Vec<String> = value
        .get("messages")
        .cloned()
        .map(as_array)
        .unwrap_or_default()
        .iter()
        .filter_map(|item| item.as_str().map(str::to_string))
        .collect();

    if fail > 0 || !messages.is_empty() {
        return Err(format!("{} 项操作失败: {}", fail, messages.join(" ;; ")));
    }

    Ok(())
}

/// 把各网卡 DNS 指向本地代理
pub fn takeover(snapshot: &DnsSnapshot) -> Result<(), String> {
    if snapshot.adapters.is_empty() {
        return Err("没有可接管的在用网卡".to_string());
    }

    let indices = snapshot
        .adapters
        .iter()
        .map(|adapter| adapter.if_index.to_string())
        .collect::<Vec<_>>()
        .join(",");

    let body = format!(
        r#"
foreach ($i in @({indices})) {{
  try {{
    Set-DnsClientServerAddress -InterfaceIndex ([int]$i) -ServerAddresses @('{v4}') -ErrorAction Stop
    $ok++
  }} catch {{
    $fail++
    $msgs += "IPv4 index ${{i}}: $($_.Exception.Message)"
  }}
}}
foreach ($a in @($plan.adapters)) {{
  $out = netsh interface ipv6 set dnsservers "$($a.alias)" static {v6} primary 2>&1
  if ($LASTEXITCODE -ne 0) {{
    $fail++
    $msgs += "IPv6 $($a.alias): $out"
  }} else {{
    $ok++
  }}
}}
"#,
        indices = indices,
        v4 = LOCAL_V4,
        v6 = LOCAL_V6
    );

    run_plan_script(snapshot, &body)
}

/// 按快照还原各网卡 DNS
pub fn restore(snapshot: &DnsSnapshot) -> Result<(), String> {
    let body = r#"
foreach ($a in @($plan.adapters)) {
  $index = [int]$a.if_index
  try {
    if ($a.v4_auto) {
      Set-DnsClientServerAddress -InterfaceIndex $index -ResetServerAddresses -ErrorAction Stop
    } else {
      $servers = @($a.v4 -split ',' | Where-Object { $_ -ne '' })
      if ($servers.Count -eq 0) {
        Set-DnsClientServerAddress -InterfaceIndex $index -ResetServerAddresses -ErrorAction Stop
      } else {
        Set-DnsClientServerAddress -InterfaceIndex $index -ServerAddresses $servers -ErrorAction Stop
      }
    }
    $ok++
  } catch {
    $fail++
    $msgs += "IPv4 $($a.alias): $($_.Exception.Message)"
  }

  # IPv6 统一交回自动获取：本机上各网卡的 IPv6 DNS 都来自 DHCP/RA
  $out = netsh interface ipv6 set dnsservers "$($a.alias)" source=dhcp 2>&1
  if ($LASTEXITCODE -ne 0) {
    $fail++
    $msgs += "IPv6 $($a.alias): $out"
  } else {
    $ok++
  }
}
"#;

    run_plan_script(snapshot, body)
}

/// 落盘待还原快照，供异常退出后自愈
pub fn save_pending(snapshot: &DnsSnapshot) -> Result<(), String> {
    let path = pending_path();
    let json =
        serde_json::to_string_pretty(snapshot).map_err(|e| format!("序列化快照失败: {}", e))?;
    std::fs::write(&path, json).map_err(|e| format!("写入 {} 失败: {}", path.display(), e))
}

/// 读取待还原快照
pub fn load_pending() -> Option<DnsSnapshot> {
    let path = pending_path();
    if !path.exists() {
        return None;
    }
    let content = std::fs::read_to_string(&path).ok()?;
    match serde_json::from_str::<DnsSnapshot>(&content) {
        Ok(snapshot) => Some(snapshot),
        Err(error) => {
            warn!(path = %path.display(), %error, "待还原快照无法解析，忽略");
            None
        }
    }
}

/// 清除待还原快照
pub fn clear_pending() {
    let _ = std::fs::remove_file(pending_path());
}

/// 启动时自愈：上次异常退出留下的快照先还原掉
///
/// 否则网卡 DNS 会一直停在 127.0.0.1，而代理进程已经不在，整机解析全废。
pub fn recover_pending() {
    let Some(snapshot) = load_pending() else {
        return;
    };

    warn!(
        taken_at = %snapshot.taken_at,
        detail = %snapshot.summary(),
        "发现上次未还原的 DNS 接管记录，正在恢复"
    );

    match restore(&snapshot) {
        Ok(()) => {
            clear_pending();
            info!("上次的 DNS 接管已恢复");
        }
        Err(error) => warn!(%error, "恢复上次 DNS 接管失败，将保留快照以便重试"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot_with(adapters: Vec<AdapterDns>) -> DnsSnapshot {
        DnsSnapshot {
            taken_at: "2026-01-01 00:00:00".to_string(),
            adapters,
        }
    }

    #[test]
    fn single_element_json_array_is_normalized() {
        // PowerShell 的 ConvertTo-Json 会把单元素数组塌缩成对象
        let single = serde_json::json!({ "if_index": 4 });
        assert_eq!(as_array(single).len(), 1);

        let many = serde_json::json!([{ "if_index": 4 }, { "if_index": 20 }]);
        assert_eq!(as_array(many).len(), 2);

        assert!(as_array(serde_json::Value::Null).is_empty());
    }

    #[test]
    fn split_servers_ignores_empty_entries() {
        assert_eq!(split_servers(""), Vec::<String>::new());
        assert_eq!(
            split_servers("223.5.5.5,202.103.224.68"),
            vec!["223.5.5.5".to_string(), "202.103.224.68".to_string()]
        );
        assert_eq!(split_servers(" , "), Vec::<String>::new());
    }

    /// 摘要要区分「静态配置」与「自动获取」，否则还原错误无从察觉
    #[test]
    fn summary_distinguishes_static_from_automatic() {
        let snapshot = snapshot_with(vec![
            AdapterDns {
                if_index: 32,
                alias: "vEthernet (vlan_wan)".to_string(),
                v4: Some(vec!["223.5.5.5".to_string()]),
                v6: Vec::new(),
            },
            AdapterDns {
                if_index: 20,
                alias: "vEthernet (vlan_lan)".to_string(),
                v4: None,
                v6: Vec::new(),
            },
        ]);

        let summary = snapshot.summary();
        assert!(summary.contains("223.5.5.5"), "摘要应含静态地址");
        assert!(summary.contains("自动获取"), "摘要应标明自动获取项");
    }

    /// 快照必须能完整往返，否则异常退出后无法还原
    #[test]
    fn snapshot_round_trips_through_json() {
        let snapshot = snapshot_with(vec![AdapterDns {
            if_index: 4,
            alias: "以太网 2".to_string(),
            v4: Some(vec!["10.99.0.1".to_string()]),
            v6: vec!["fec0:0:0:ffff::1".to_string()],
        }]);

        let json = serde_json::to_string(&snapshot).expect("应能序列化");
        let restored: DnsSnapshot = serde_json::from_str(&json).expect("应能反序列化");

        assert_eq!(restored.adapters, snapshot.adapters);
        assert_eq!(restored.adapters[0].alias, "以太网 2");
    }

    /// 自动获取项必须序列化成 null，不能被写成空列表而丢掉语义
    #[test]
    fn automatic_dns_is_serialized_as_null() {
        let snapshot = snapshot_with(vec![AdapterDns {
            if_index: 20,
            alias: "auto".to_string(),
            v4: None,
            v6: Vec::new(),
        }]);

        let json = serde_json::to_string(&snapshot).expect("应能序列化");
        assert!(
            json.contains("\"v4\":null"),
            "自动获取必须落成 null，还原时才知道该重置而不是写空地址: {}",
            json
        );
    }
}
