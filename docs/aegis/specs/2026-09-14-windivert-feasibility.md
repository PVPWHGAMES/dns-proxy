# WinDivert 全局接管可行性探测

日期：2026-09-14
结论：**可行**。驱动在本机（Secure Boot + 管理员）可正常加载，回环/IPv6/ICMP 全部可见，FLOW 层可拿到进程归属；探测全程只读，未对网络造成任何影响。

## 背景

TUN 方案已回退（其问题在 userspace 协议栈，不在 TUN 本身）。为回答"接管全局 TCP/UDP 该用什么技术"，先做 WinDivert 的可行性探测，不做任何实现。

候选技术排序（探测前的判断，探测后维持）：**WinDivert > TUN > WFP Callout**。
WFP 用户态只能过滤/阻断，**不能重定向**（`FwpsApplyRedirect0` 与 callout 均为内核态专属）；自研 WFP callout 需要自有内核驱动 + EV 证书 + WHQL/attestation，且有 BSOD 风险。

## 方法与安全边界

- 一律使用官方 2.2.2-A 包内的样例程序，**不写自定义 FFI、不调用 `WinDivertSend`**。
- 抓包用 `netdump.exe`；进程归属用 `flowtrack.exe`。两者均以镜像内写死的模式打开句柄，`flowtrack` 源码为 `WINDIVERT_LAYER_FLOW` + `WINDIVERT_FLAG_SNIFF | WINDIVERT_FLAG_RECV_ONLY`——包被复制而非取走，进程从不发送、从不丢弃。
- 未使用"无 SNIFF 的直接 divert"：那会把包从协议栈里拿走，一旦程序有 bug 就是全机断网。这是本次不做的动作。

## 验证结果

| 项目 | 结果 | 证据 |
|---|---|---|
| 管理员提权 | 通过 | `管理员: True` |
| 驱动加载（Secure Boot 下） | 通过 | 服务 `WinDivert` = KERNEL_DRIVER / RUNNING；签名含 Sectigo EV 代码签名 + Microsoft attestation |
| 普通 IPv4 抓包 | 通过 | `netdump ip`，4 秒 51987 行 |
| 回环 127.0.0.1 | 通过 | 自造 25 次 TCP 连接 → 1641 行，`Loopback=1` |
| IPv6（::1） | 通过 | 自造 20 次连接 → 1355 行，`Loopback=1` |
| ICMP | 通过 | 6 次 ping → 137 行 |
| FLOW 层 + 进程归属 | 通过 | `flowtrack` 以 FLOW 句柄运行 5 秒未退出、stderr 为空（打开失败会立即报错退出） |
| 探测后网络正常 | 通过 | 解析 example.com 成功；ping 223.5.5.5 = True |

包文件校验：`WinDivert-2.2.2-A.zip` 405137 字节，与 GitHub Releases API 一致；`WinDivert64.sys` SHA256 `8da085332782708d8767bcace5327a6ec7283c17cfb85e40b03cd2323a90ddc2`、MD5 `89ed5be7ea83c01d0de33d3519944aa5`。

## 为什么这些结论对该项目关键

1. **回环可见**：本项目的 DNS 代理监听 `127.0.0.1` 与 `::1`，且系统 DNS 已被接管指向它们。TUN 结构上看不到本机到本机的流量，WinDivert 可以——这意味着"DNS 查询"和"应用连接"两段都在同一层可观测、可介入。
2. **IPv6 完整**：不需要像 TUN 那样维护双栈设备，减少一类 `-AddressFamily` 式的隐性失败。
3. **FLOW 层给出 `ProcessId`**：官方头文件 `WINDIVERT_DATA_FLOW` 含 `Endpoint / ParentEndpoint / ProcessId / LocalAddr / RemoteAddr / LocalPort / RemotePort / Protocol`，可支撑"按进程分流"，这是 TUN + 五元组做不到的。
4. **官方样例已示范目标形态**：`streamdump.exe` 的说明是 "divert outbound TCP connections to a local proxy server"——重定向到本地代理正是我们要做的事，不需要从零发明。

## 未验证 / 未获取（诚实边界）

- **未测我们自己的重定向实现**：本次没有编译任何自定义代码，因此重定向的正确性、吞吐、延迟均未测。51987 行/4 秒只说明"句柄能跟上突发流量"，不等于长期吞吐上限。
- **WFP Provider 清单没拿到**：`netsh wfp show state file=` 导出的文件里没有 `<providers>` 段（只有 subscriptions / ipsecStatistics / ALE 端点 / firewallState），`netsh wfp show` 也没有 providers 子命令。因此"还有哪些第三方 WFP 过滤器在栈里"这一项**未确认**。已导出的 ALE 端点列表显示存在第三方 VPN 类客户端流量（如 Corplink），是否需要避让待后续确认。
- **flowtrack 无法在重定向下取证**：它是纯控制台 UI（`SetConsoleCursorPosition` + `WriteConsole` 画表格），stdout 重定向后没有任何输出路径。因此进程归属能力用"句柄存活 + 官方头文件结构"间接证明，未取到实际 PID 文本。
- 未在多网卡切换、睡眠唤醒、以及第三方安全软件拦截等场景下验证。

## 主要风险

1. **实现阶段必须 divert 而非 sniff**：sniff 只是复制，做不了接管；一旦转为 divert，程序崩溃/卡死就会直接断网。需要"看门狗 + 自动恢复网络"作为硬约束，而不是可选项。
2. **许可**：WinDivert 2.2 为 LGPLv3 / GPLv2 二选一。闭源商业分发需要法律评估（本次不建议由我单方面决定）。
3. **驱动残留**：首次使用会安装内核驱动服务，需在卸载/升级路径中一并处理。

## 本次探测的副作用

- 探测安装并启动了 `WinDivert` 内核驱动服务（探测前本机没有）。探测结束后已清理。
- 临时文件位于 `%TEMP%\windivert-probe\`，已清理。

## 下一步建议

1. 先在**只读观察模式**下实现一遍 FLOW 层监听，验证能稳定区分进程与流，不碰数据包。
2. 再在可一键回滚的前提下实现最小重定向（单个进程 + 单个端口），验证"应用拿到 IP 后仍能连接"。
3. 许可问题在上述两步产出效果后单独决策，不阻塞技术验证。

## 关联

- `docs/aegis/specs/2026-09-07-ipv4-global-proxy-design.md` 与 `docs/aegis/plans/2026-09-07-ipv4-global-proxy.md` 描述的是已回退的 TUN 方案，**已失效**，仅作历史参考。

---

# 第一步实施记录：FLOW 层只读观察

## 交付物

| 文件 | 作用 |
|---|---|
| `src-tauri/bin/WinDivert.dll`、`WinDivert64.sys`、`LICENSE-WinDivert.txt` | 随包分发的运行库（2.2.2-A，x64） |
| `src-tauri/src/windivert/mod.rs` | 运行时绑定、`WINDIVERT_ADDRESS` 解析、进程名查询 |
| `src-tauri/src/windivert/monitor.rs` | 观察线程与活动流表 |
| `src-tauri/build.rs` | 复制运行库到 exe 同级（`--no-bundle` 不处理 `bundle.resources`） |
| `src-tauri/tauri.conf.json` | 安装包资源，落在资源根目录 |
| `src-tauri/tests/windivert_flow.rs` | 端到端验证（需管理员，默认 `#[ignore]`） |
| `src/pages/FlowMonitor.tsx` | 「流量观察」页面 |

## 关键实现决策

1. **运行时加载而非链接期依赖**。链接期写法下 DLL 一旦缺失（被杀软隔离、被误删）进程在加载阶段就起不来，而那时系统 DNS 可能正指向本程序，整机解析会一起失效。运行时加载还能在用户补上文件后重试。
2. **句柄以 `FLAG_SNIFF | FLAG_RECV_ONLY` 打开**。前者让包「复制后放行」，后者从类型上排除发送路径——这个句柄在结构上不可能改变流量。
3. **观察循环跑在标准库线程上**。`WinDivertRecv` 是阻塞调用，占用 tokio 工作线程不合适；停止时用 `WinDivertShutdown` 把它从阻塞里叫醒。
4. **先摘牌再关闭句柄**。`stop()` 只在持有槽位锁时调用 `Shutdown`，而线程关闭句柄前必须先摘牌，两者互斥保证不会操作已释放的句柄。
5. **进程名查不到就退回 `PID 1234`**，不因为查名字失败而丢掉这条流。

## 验证证据

- `cargo test --lib`：**69 通过 / 0 失败**，其中 9 个是本次新增（结构体尺寸 80 字节、位域位序、FLOW 字段偏移、缺 DLL/缺驱动的报错、状态机幂等）。
- `cargo test --test windivert_flow -- --ignored`（管理员）：**1 通过**，真实数据如下：

  ```
  TCP pid=11668 windivert_flow-….exe  172.16.93.178:62648 -> 223.5.5.5:53   回环=false 出站=true
  TCP pid=11668 windivert_flow-….exe  127.0.0.1:62646 -> 127.0.0.1:62647    回环=true  出站=false
  TCP pid=11668 windivert_flow-….exe  127.0.0.1:62647 -> 127.0.0.1:62646    回环=true  出站=true
  ```

  一次覆盖了位域解码、字段偏移、进程归属、回环可观测与跨网卡出站流。
- **测试有牙的证明**：把 `event()` 的位域改为读取 bit16..24 后重跑，测试在「没看到本进程的流」断言处失败（exit=101）；改回后复验通过。
- **发布布局也验证过**：端到端测试支持 `WINDIVERT_RUNTIME_DIR` 覆盖运行库目录，对着 `src-tauri/target/release`（DLL 与驱动就在 exe 旁边）跑同样 `1 passed`，仓库 `bin/` 布局回归同样通过。
- **第三方进程归属**：同一轮验证里顺带抓到 `java.exe`（pid 29840、15928）的真实回环连接，进程名反查正确——归属能力不只对我们的测试进程成立。
- **DNS 回归**：新构建启动后接管 5 张网卡指向 `127.0.0.1`，`qq.com` 正常解析，`nslookup` 走 `::1`。
- 驱动无残留：最后一个句柄关闭后系统里不再有 `WinDivert` 服务。

## 尚未验证

- **界面目视**：提权窗口无法截屏，页面渲染与交互只能由人确认。
- **安装包路径**：`bundle.resources` 的落地位置未实测（`--no-bundle` 不产生资源目录），代码里已同时覆盖同级与资源目录两种布局。
- `cargo test` 会连带跑 `--bin dns-proxy` 目标，该 exe 带 `requireAdministrator` 清单，测试框架启动它必然报 `os error 740`；这是既有现象，与本次改动无关，日常用 `cargo test --lib`。

---

# 第二步设计前提（读驱动源码与官方文档得出，均为只读检索）

原本计划用一次"窄口径动包实验"来确定下面第 1 条，源码直接给出了答案，因此这次实验不必做。

1. **关闭句柄时，未超时的 divert 包会被重新注入协议栈**。`windivert_cleanup()`（`sys/windivert.c` 1999–2014）逐个摘出队列中的包：非 sniff 模式且未超时的走 `windivert_reinject_packet()`，其余 `windivert_free_packet()`。含义：**程序崩溃或正常退出不会把网络打成黑洞**，最多丢掉已超时的那部分。
2. **队列时限默认 2000 ms，超时即丢**（`WINDIVERT_PARAM_QUEUE_TIME`）。含义：句柄仍然打开但循环卡死时，包会持续被丢弃——TCP 靠重传可恢复，UDP 不可恢复。看门狗依然必要，但它的失败模式是可恢复的。
3. **注入时校验和由驱动代算**。驱动发送路径（`sys/windivert.c` 5222–5227）在对应 `*Checksum` 标志为 0 时调用 `WinDivertHelperCalcChecksums()`；文档明确 "Injected packets must have the correct checksums **or have the corresponding pAddr->*Checksum flag unset**"。含义：改写地址后把校验和标志清掉即可，不必自研校验和。
4. **注入的包可能被再次捕获**（文档明言 WinDivert 无法阻止）。对策是过滤器加 `not impostor`，注入时把 `Impostor` 位置 1；驱动对 impostor 包会自动递减 TTL 作为兜底，TTL 归零时报 `ERROR_HOST_UNREACHABLE (1232)`。
5. **回环没有被禁用的注入路径**：驱动里 loopback 只是一组标志位（`packet->loopback`、`addr->Loopback`），未见针对回环注入的特殊拦截。

（本节结论来自静态检索，尚未在动包实验中复验。）

---

# 第二步 2a 实施与验证：回环沙盒里的最小重定向

## 交付物

| 文件 | 作用 |
|---|---|
| `src-tauri/src/windivert/worker.rs` | 句柄 + 工作线程的生命周期骨架，观察与重定向共用 |
| `src-tauri/src/windivert/redirect.rs` | 规则、**纯改写函数**、重定向器 |
| `src-tauri/src/windivert/mod.rs` | 新增 `WinDivertSend`/`WinDivertHelperCalcChecksums` 绑定与 `recv_packet` |
| `src-tauri/tests/windivert_redirect.rs` | 回环沙盒端到端验证 |

改写逻辑刻意做成纯函数：这段代码最容易出错，而它不需要驱动、不需要管理员权限就能被完整测到。

## 踩到的坑：回环包只能按「出站」注入

文档原文：

> WinDivert considers loopback packets to be **outbound only**, and will not capture loopback packets on the inbound [path].

照搬官方 `streamdump` 的"出站反射成入站"在回环上会把包送进不支持回环的入站路径，表现为**客户端 SYN 一直等不到响应**（首次实现 21 秒超时失败）。修正为「回环保留出站方向，非回环才反射成入站」后，0.72 秒通过。

## 验证证据

两次连续运行结果**完全一致**（可重复，非偶发）：

```
原目标 127.0.0.1:56234 → 改投 127.0.0.1:56235
计数器：改写出 5 / 改写入 4 / 原样放行 0 / 跳过 0 / 注入失败 0
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
```

断言强度（每一条都能单独否定"改投成功"这个结论）：

- 客户端读到 `redirected-ok`——这句话只有被改投的目标端口会发；改投没生效就会读到 `direct-ok`。
- 原目标端口全程**没有**收到连接。
- 双向计数器都 > 0：证明请求与应答两个方向都真的被改写并送达（只有单向改写则读不到数据）。
- 注入失败为 0。
- **回滚**：停止重定向后重连，读到 `direct-ok` 且原目标端口收到连接。

单元测试：`cargo test --lib` **79 通过 / 0 失败**（新增 10 个覆盖纯改写逻辑：端口/地址改写、未匹配包一个字节都不变、分片跳过、TCP 选项、自指规则被拒、缺运行库时的提示）。

回归：重构 `FlowMonitor` 到共用骨架后重跑第一步的端到端测试，`1 passed`，并照例抓到 `java.exe`、`msedge.exe` 的真实流量。运行结束后系统里无 `WinDivert` 服务残留。

## 尚未验证

- **非回环的真实流量重定向（2b）**：那条"反射成入站"的代码路径还没有跑过。回环沙盒证明了改写与注入的机械链路，但没有证明非回环场景。
- 本地中继（真正代连出去）与界面接线、以及"退出即回滚"在真实流量下的表现。

---

# 2b 设计：哨兵端口映射（来自官方 streamdump）

## 环路问题

中继接受被改投的连接后要连回原目标，**而那个连接会被我们自己的过滤器再抓一次**，形成自噬。这不是实现细节，是设计必须解决的第一问题。

## 官方解法：哨兵端口

`streamdump` 用三个端口绕开它：中继拨 `目标:alt_port`，重定向把 `alt_port` 映射回真实端口 `port`；真实服务的回包（源端口 `port`）再被映射成源端口 `alt_port`，于是中继的拨号套接字看到的收发两端是自洽的。**全程不需要连接跟踪**，因为没有任何流量会以 `port` 为目的端口从中继发出。

## 我们的四个分支

与 streamdump 相同，但去掉它的地址交换（它用交换把"原目标"藏进源地址好让中继从 `accept()` 得知目标；我们的规则里已经知道目标）：

| # | 匹配 | 改写 | 注入 |
|---|---|---|---|
| 1 | 出站 `dst == T:P` | `dst = 127.0.0.1:R` | 注入（客户端 → 中继） |
| 2 | 出站 `src == 127.0.0.1:R` | `src = T:P` | 注入（中继 → 客户端） |
| 3 | 出站 `dst == T:S` | 目的端口 `= P` | 原样放行（中继拨出去 → 真实服务） |
| 4 | `src == T:P` | 源端口 `= S` | 注入（真实服务回包 → 中继拨号套接字） |

## 一个让沙盒重新可用的发现

第 4 分支**不必要求入站方向**：在回环上它以出站形态出现，在真实网卡上它才是入站。把它写成方向无关的条件后，**整套四分支设计可以在回环沙盒里被完整验证**——真实的中继、真实的四段映射、真实的自噬规避，全程零外部流量。

（真实网卡上"源地址等于远端服务器"的出站包不存在，所以方向无关不会误伤。）

## 已交付：本地中继

`src-tauri/src/relay.rs`：纯用户态的字节转发（监听 → 连接目标 → 双向搬运），带接受数/活动数/上下行字节数/错误记录。不需要管理员权限，因此测试是常规测试而非 `#[ignore]`：`cargo test --test local_relay` **2 通过**（双向转发一致性、停止后监听器关闭并拒绝新连接、目标不可达时记录错误）。

## 已验证：完整链路（四段映射 + 真实中继）

`cargo test --test windivert_full_chain -- --ignored`（管理员）**1 通过**：

```
服务端口 33200 / 中继端口 33201 / 哨兵端口 33202
中继状态：accepted: 1, bytes_up: 16, bytes_down: 10
重定向计数器：客户端→中继 5 / 中继→客户端 4 / 拨号映射 5 / 回包映射 5
              / 原样放行 0 / 跳过 0 / 注入失败 0
```

客户端先发 16 字节请求、服务读到后才回 10 字节招呼，因此**两个方向都有真实数据穿过链路**：

- 四段映射全部命中，且没有出现"原样放行"——说明过滤器与改写分支的条件完全对齐，没有包掉进无法归类的缝里。
- 服务侧直接断言收到的字节数等于请求长度，证明上游方向确实走完了 客户端 → 改写 → 中继 → 哨兵映射 → 服务。
- 中继计数器独立佐证了同一条链路。
- **自噬规避有效**：中继拨出去的那条连接被分支 3 正确映射，而不是又被当成"客户端连接"改投回来（否则测试会因无限循环超时）。
- 回滚后客户端直连服务，服务再次原样收到请求。

同一轮里第一步的端到端测试复跑仍为 `1 passed`，运行结束后系统无 `WinDivert` 服务残留。

## 已验证：非回环真对端路径（原先的最后一块结构性缺口）

回环只有出站形态，因此"反射成入站"这条路径在回环沙盒里**结构上覆盖不到**。解决办法是找一个"本机之外、但不出网"的对端：**WSL2**。

- WSL2（Ubuntu）虚拟机地址 `172.20.74.14`，Windows 侧网卡 `172.20.64.1`（`vEthernet (WSL)`），
  路由 `172.20.64.0/20 → vEthernet (WSL)`，对 Windows 协议栈是货真价实的非回环收发。
- 对端服务是一个 python 回显服务（收请求后回 `service-ok:` + 请求原文），
  因此**一次交换就同时证明上行到达与下行返回**。
- 中继绑到面向对端的网卡地址 `172.20.64.1`（重定向交换源/目的地址后，注入包的目的地址就是客户端本机地址，绑回环收不到）。

`cargo test --test windivert_real_peer -- --ignored`（管理员 + 两个环境变量）**1 通过**：

```
基线通过："service-ok:ping-from-client"
目标 172.20.74.14:34000 / 中继 172.20.64.1:33200 / 哨兵 33201
重定向计数器：客户端→中继 6 / 中继→客户端 4 / 拨号映射 5 / 回包映射 4
              / 原样放行 0 / 跳过 0 / 注入失败 0
中继状态：accepted: 1, bytes_up: 16, bytes_down: 27
```

含义：`if !address.loopback() { address.set_outbound(false) }` 这条"把非回环出站包反射成入站包"的路径**实测可用**，四个分支在真实网卡收发下全部命中，自噬规避依然有效。复现步骤写在 `src-tauri/tests/windivert_real_peer.rs` 的头注释里。

至此，重定向机制的每一条代码路径都有了实测证据，且**全程没有触碰任何互联网目标**。

## 仍未验证

- **应用接线**：Tauri 命令、界面开关、以及"应用拿到 IP 后仍能连接"的真实应用场景（浏览器/客户端经本程序 DNS 解析后连接被接管的目标）。
  **（已过时：见下文第三步——Tauri 命令与界面开关已就位，"应用拿到 IP 后仍能连接"已由
  `windivert_dns_connect` 与 `windivert_internet` 覆盖。仍未做的只剩"人从界面点一次"。）**
- **中继的暴露面策略**：真实接管时中继要绑到面向客户端的网卡；绑 `0.0.0.0` 等于把它开放给整个网段（可被当作开放代理），需要决定绑定范围与是否加来源校验。
  **（部分过时：来源校验已落地为"放行集合严格等于接管集合"，绑定范围仍是待决的产品选择。）**
- 长时间运行下的稳定性（计数器上限、连接表增长、队列打满时的表现）。

---

# 会话顺序与来源校验

## 把启动/停止顺序抽成可验证的代码

顺序原先写在 Tauri 命令里，而命令带 `State` 因而无法自动化验证。抽成 `RedirectSession` 之后就能用普通测试覆盖：

- **启动：先中继，后重定向。** 中继没起来就接管流量，等于把连接改投到一个没人监听的端口——那是纯粹的中断。
- **停止：先重定向，后中继。** 反过来会让还在途中的报文继续被改投到已经关闭的端口。

`cargo test --test redirect_session`（不需要管理员）**3 通过**：失败后不留中继、规则校验发生在加载运行库与绑定端口之前、停止幂等。

**有牙证明**：把失败路径里的 `self.relay.stop()` 注释掉后，测试立刻失败并打印出那个半启动状态——
`relay: running: true, listen_addr: "127.0.0.1:34567"` 而重定向并未运行。

## 来源校验：顺手把"开放中继"这个隐患关掉

分支 1 的地址交换会把**原目标地址写进源地址**，因此中继看到的对端必然等于原目标。据此可以拒掉一切别的来源（例如同网段的其他机器把我们当中继用），不需要额外配置——`RelayConfig.allowed_peer` 即为此。

证据：

- `relay_rejects_connections_from_unexpected_peers`（常规测试）通过：来源不符时 `accepted = 0`、`rejected = 1`，且连接被关闭（读立刻拿到 EOF）。
- 两个提权端到端测试的 `rejected` 都是 **0**——回环链路（目标 `127.0.0.1`）与真对端链路（目标 `172.20.74.14`）都没有被误杀。这也顺带实测确认了"对端必然等于原目标"这条推断在两条路径上都成立。

## 仍未验证（本轮新增之外）

- 从界面发起的一次真实接管：界面交互驱动不了 WebView，只能由人确认。
- 真实互联网目标。
- 运行中的二进制落后于源码一轮（本轮改动尚未重新构建进 exe）。

---

# 已验证：应用拿到 IP 后仍能连接

这是本目标原话里的验收点。`cargo test --test windivert_dns_connect -- --ignored`（管理员）**1 通过**：

```
应用解析 www.example.org:8999 -> 172.66.157.237:8999
面向该目标的源地址是 172.16.93.178，本机服务绑在它上面
计数器：客户端→中继 6 / 中继→客户端 3 / 拨号映射 0 / 回包映射 0
        / 原样放行 0 / 跳过 0 / 注入失败 0
```

流程与真实应用完全一致：

1. 走 `getaddrinfo`（系统解析器 → 本程序接管的 DNS）解析域名，拿到公网 IPv4；
2. 对该 IP:端口建立重定向，把连接改投到本机服务；
3. 用普通 `TcpStream`（应用会做的事）连上去，断言拿到本机服务的应答、且本机服务
   原样收到了请求（上行也穿过改写）。

**一个字节都没有发往互联网**：客户端发往那个公网 IP 的 SYN 被分支 1 取走，注入出去
的是改写后的副本（交换地址后目的地址是客户端本机），原包从未离开；因此分支 3/4
为零是预期结果，测试也把它断言成 0。

顺带说明：解析出的是真实 DNS 应答（`www.example.org` 由 Cloudflare 承载），
不是构造的假数据。

# 证据总表

| 套件 | 权限 | 结果 | 覆盖 |
|---|---|---|---|
| `cargo test --lib` | 无 | 83 通过 | 四分支改写、地址交换、过滤器、规则校验、状态机 |
| `local_relay` | 无 | 3 通过 | 双向转发、来源校验、目标不可达 |
| `redirect_session` | 无 | 3 通过 | 启动/停止顺序、失败不留半启动态、停止幂等 |
| `windivert_flow` | 管理员 | 1 通过 | FLOW 只读观察、进程归属 |
| `windivert_redirect` | 管理员 | 1 通过 | 回环改投 + 回滚 |
| `windivert_full_chain` | 管理员 | 1 通过 | 四段映射 + 真实中继 + 自噬规避 |
| `windivert_real_peer` | 管理员 | 1 通过 | 非回环"反射成入站"路径（WSL 对端） |
| `windivert_dns_connect` | 管理员 | 1 通过 | 应用解析出 IP 后仍能连接，零外网流量 |

# 二进制与源码对齐

第 1 步与 2a/2b 的改动已全部构建进 `src-tauri/target/release/dns-proxy.exe`，
并通过"改名旧映像 → 构建 → 提权停旧实例 → 只启一次 → 逐项核对"的方式完成切换。
核对结果：**单实例**、53 端口独占、6 张网卡指向 `127.0.0.1`、域名可解析、无驱动残留。

# 本目标之外、产品化还需要的事

- **真实互联网目标的端到端**：本目标只要求"应用拿到 IP 后仍能连接"，已达成；
  但"中继代连到真实远端并取回内容"需要真实外网流量，尚未验证。
- **长跑稳定性**：队列打满、连接表增长、长时间接管下的表现。
- **许可**：WinDivert 为 LGPLv3 / GPLv2 二选一，闭源分发需法律评估。
- 中继绑定范围与来源校验已实现（`allowed_peer`），但绑 `0.0.0.0` 的暴露面仍需产品决策。

---

# 更正：驱动残留的结论是错的

上文第 109、174、233 行写的"运行结束后系统里无 `WinDivert` 服务残留"**与实测不符**，
按当前机器上的实际状态更正如下：

```
Get-Service WinDivert
  Status   : Running
  StartType: Disabled
  ServiceType: KERNEL_DRIVER
  BINARY_PATH_NAME: \??\C:\Dev\dns-proxy\src-tauri\bin\WinDivert64.sys
```

驱动服务**在最后一个句柄关闭后依然存在**，只是启动类型为 `Disabled`（不会开机自启）。
之前"无残留"的判断很可能来自一次错误的查询方式（例如只看服务是否 `Running` 而没看它是否
仍然注册，或在清理脚本执行后立刻查询）。含义：卸载/升级路径必须显式处理这个服务，
不能依赖"关掉句柄它就没了"。

同时更正一条相关观察：**非提权下句柄能打开，但收不到任何包**。驱动服务处于 `Running`
时，非管理员进程的 `WinDivertOpen` 会成功返回句柄，可 `WinDivertRecv` 永远没有数据——
`windivert_flow` 与 `windivert_internet` 两条套件在非提权下都复现了这个现象（句柄就位、
计数器全为 0、连接直接走直连）。这解释了此前"err=5 表示语法合法但无权限"只在**驱动尚未
加载**时成立：驱动一旦在跑，失败方式就从"打不开"变成"打开了但空转"。

后果是界面不能把 `running`（= 句柄已打开）当成"接管在生效"。产品上由 `build.rs` 的
`requireAdministrator` 清单保证始终提权，所以这不是线上问题，但状态语义值得记一笔。

---

# 第三步 3.0：动态规则集的过滤器策略

问题：接管集合要由 DNS 动态驱动，目标数不固定，需要确定"一个过滤器能装多少目标"。

## 结论

| 写法 | 结果 |
|---|---|
| 集合语法 `in {…}` / `in (…)` / `in a, b` | **不支持**，`WinDivertOpen` 返回 `err=87`（语法错误） |
| OR 链 `(a or b or …)` | 支持，长度不是瓶颈：4426 字符 / 90 个目标可正常打开 |
| **真正的上限是编译后的字节码** | 非紧凑写法 41 个目标通过（6190 字符）/ 42 个被拒（6340） |
| **紧凑写法**（把端口提到括号外，只列地址） | **81 个目标通过（7192 字符）/ 82 个被拒（7279）** |

把目标端口提到括号外（`(dst port == P and dst in {A,B,C})` 改成
`dst port == P and (dst in {A,B,C})` 的形式）语义完全相同，容量直接翻倍。
`RuleTable::filter()` 因此按目标端口分组生成子句。

重开句柄的开销（25 个目标 / 3806 字符，100 次开-关循环，0 失败）：

```
min 0.85 ms / avg 1.69 ms / p50 1.68 ms / p95 2.23 ms / max 5.16 ms
```

含义：**"改规则就重开句柄"这条路是可行的**——1.7 ms 的中位开销让"先开新句柄、
再关旧句柄"的并存窗口小到可以忽略，不需要实现增量更新。

## 顺带确认的优先级语义

官方文档原文：

> A packet is only diverted once per priority level, so handles should not share priority
> levels unless they use mutually exclusive filters. Otherwise it is not defined which
> handle will receive the packet first.

因此分片之间**必须用不同优先级**；反过来，只要过滤器里带 `!impostor`，不同优先级的
句柄就是互斥的，多句柄方案在语义上成立。

---

# 第三步 3.1：多句柄分片

## 设计

单片实测上限 81 个目标，取 **40** 作为每片容量（留一倍余量，避免贴着字节码上限运行）。
超过容量就再开一片，每片一个独立句柄、一个独立优先级。

- `src-tauri/src/windivert/table.rs`：多目标 `RuleTable` + 纯函数 `rewrite()`。
  纯函数意味着这段最容易错的逻辑不需要驱动、不需要管理员权限就能被完整测到。
- `src-tauri/src/windivert/shard.rs`：`ShardedRedirector` + `Shard` + `PriorityPool`。
- `PriorityPool` 管优先级槽位：加目标时挑一个**更高**的槽（新句柄赢下并存期），
  删目标时挑一个**更低**的槽（旧句柄赢下并存期，被删目标上的连接可以自然结束）。
  每个 `ShardedRedirector` 实例再用 `INSTANCE_SEQ` 分到互不重叠的优先级段
  （`PRIORITY_BAND = 64`），保证同一进程里多个实例也不会撞优先级。

`Shard::reload()` 的顺序是**先开新句柄、再关旧句柄**，失败时保留旧状态——
反过来的话，两次操作之间会有一个"谁都没在接管"的窗口。

## 踩到的三个坑

1. **句柄泄漏会级联污染后续测试**。`ShardedRedirector` 与 `Worker` 都没有 `Drop`，
   一个 panic 的测试会留下一个仍在接管全机流量的句柄，下一个测试复用同一端口就超时
   （`code: 10060, kind: TimedOut`）。补上 `impl Drop for Worker` 后消失。
2. **同优先级重开会收到零个包**。修好上一条后计数器完全冻结（`to_relay: 5, to_client: 4`
   前后不变）——新句柄和旧句柄同优先级，文档语义下"谁先拿到包"未定义。这就是
   `PriorityPool` 存在的原因。
3. **中继的真实 bug**：`last_close: Some("上行(调用方→目标)=读失败(WouldBlock)
   下行(目标→调用方)=对端关闭")` 指出真正的原因——**Windows 上 `accept()` 返回的套接字
   会继承监听套接字的非阻塞模式**，于是 `WouldBlock` 被当成致命错误，连接在第一批数据
   之后就被拆掉。修法是 `client.set_nonblocking(false)`，并把
   `WouldBlock | TimedOut` 当成"暂时没数据"重试。

   这条有牙证明：把修复临时改回去，`local_relay` 的新用例
   `relay_keeps_an_idle_connection_alive_for_a_second_roundtrip` 立刻失败并打印
   `Os { code: 10053, kind: ConnectionAborted }`；改回来通过。

## 证据

提权全量回归（`windivert_sharded` 3 个用例 + 全链路 + 回环改投 + FLOW + DNS 连通）：

```
windivert_sharded: 3 passed（41 个目标 → 2 片，每片上限 40）
```

关键一条 `adding_a_target_does_not_break_an_established_connection`：重开分片后
`active: 1, last_close: None`；结束时 `accepted: 2, active: 2, bytes_up: 16, bytes_down: 16`
（16 = 改动前 6 + 改动后 5 + 新连接 5），证明**重开分片不会打断已建立的连接**。

---

# 第三步 3.2：DNS 应答联动

## 接线

`src-tauri/src/takeover.rs`（新增）把"解析到某个 IP"和"接管发往该 IP 的连接"接成一条链：

- `DnsHandler` 增加一个回调槽 `takeover_hook`，用回调而不是直接持有接管管理器，
  让 DNS 这一层不必知道 WinDivert 的存在。
- 挂钩在 `handle_query_inner` 里、**应答返回客户端之前**触发，且只对
  `forward_group == Some("proxy")` 的应答触发。直连域名走本地网络，接管它们只是平白多一跳。
- 只取 A 记录（重定向只处理 IPv4），TTL 取应答里 A 记录的**最短**值。
- 挂钩出错一律只记日志，绝不影响 DNS 应答本身：DNS 挂了整个网络就挂了，
  接管失败最多是"没有走中继"。

`TakeoverManager` 负责地址的进出：

- 到期 = TTL + 30 秒宽限。宽限是必要的：DNS 记录到期后浏览器未必立刻重新解析，
  没有宽限会出现"浏览器还在用旧地址、我们已经不管了"的空档。
- 超长 TTL 被 `MAX_LIFETIME`（6 小时）截断，防止个别域名给一天以上的 TTL 把地址长期占住。
- 超过 `MAX_TARGETS`（256 个目标）时按到期时间从早到晚淘汰——最早到期的就是最久没被
  DNS 确认的地址，先撤它最不容易打断在用的连接。
- 中继的放行集合每次整体重算而不是增量增删。增量维护要考虑"这个地址是否还被别的
  目标端口用着"，算错一次就会把合法连接拒掉，而重算只有几百个元素。
- `start()` 本身**不打开任何 WinDivert 句柄**：句柄在第一个目标进来时才开。
  所以"打开了开关但还没解析到任何 proxy 域名"是完全无副作用的。

## 一个必须写下来的边界

接管按 **IP** 进行，而一个 CDN 地址上通常挂着很多域名。所以"域名 A 在 proxy 分组"
会连带接管同一地址上域名 B 的流量。这一点无法通过规则表消除，只能靠中继侧的后续策略
（按 SNI 分流）收敛。界面文案里明说了这条。

## 证据

- 单元测试（不需要管理员）：挂钩收到全部地址与最短 TTL、无地址时不触发、
  重复地址去重、没装挂钩时静默、到期回收、超容量淘汰最久未确认的、TTL 上限截断。
- 界面接线：`start_takeover` / `stop_takeover` / `get_takeover_status` /
  `get_takeover_targets` 四个 Tauri 命令，`src/pages/Redirect.tsx` 新增「DNS 联动接管」面板。
  接管开关**刻意不写进配置文件**：一个"已启用"的接管设置会在下次启动时自动拦截流量，
  而这是实验性能力，默认值必须是"不碰任何流量"。

---

# 第三步 3.3：真实互联网端到端

`src-tauri/tests/windivert_internet.rs`（新增）。不复用任何回环假对端，
整条链路跑在真实网卡上：DNS 解析 `example.com`（默认分组 = proxy）→ 挂钩把 A 记录交给
接管管理器 → 客户端连真实 IP:80 → 四段映射 → 中继连回真实远端 → 读回真实内容。

`cargo test --test windivert_internet -- --ignored`（管理员 + 外网）**4 通过**：

```
本机网卡地址 192.168.0.89
挂钩收到 example.com → [172.66.147.243, 104.20.23.154]（TTL 260s）
预检 example.com → [172.66.147.243, 104.20.23.154]
接管已启动，中继监听 ["192.168.0.89:34000"]

接管状态：address_count: 2, target_count: 2, shard_count: 1,
          port_pairs: [(80, 34000, 34001)], relay_accepted: 0, to_relay: 0 …   ← 还没连接

收到响应开头：HTTP/1.1 200 OK
接管状态：relay_accepted: 1, relay_active: 1,
          to_relay: 5, to_client: 5, dial_mapped: 5, reply_mapped: 4,
          passed_through: 0, skipped: 0, send_failed: 0

回滚后直连成功：HTTP/1.1 200 OK
```

断言强度：读到的是真实 `HTTP/1.1 200 OK`（内容来自 Cloudflare 承载的 example.com）；
四个分支的计数器都 > 0，证明四个方向都真的被改写并送达，而不是走了直连；
`passed_through == 0` 说明过滤器与改写分支的条件完全对齐，没有包掉进无法归类的缝里；
`relay_rejected == 0` 说明来源校验没有误杀真实目标。

另一条 `an_expired_target_leaves_the_takeover_set` 验证**撤销**路径：

```
到期后接管状态：address_count: 0, target_count: 0, shard_count: 0,
                targets: [], running: false
```

TTL + 宽限过后地址自动退出接管集合，分片句柄被关掉，规则表清空。

## 这一轮抓到的真实产品 bug

`TakeoverManager::allocate_port_pair()` 原先用 `127.0.0.1` 探测空闲端口，而中继绑的是
**网卡地址**。两者在同一端口上互不冲突，于是第二个目标端口探到的还是第一对已经用掉的
端口 —— 也就是说**默认的 `[443, 80]` 双端口配置在真实网卡上根本起不来**。

为什么单元测试没抓到：测试里中继绑的是 `127.0.0.1`，探测地址和中继地址恰好相同，
掩盖了这个不一致。只有把中继绑到真实网卡才暴露。

修法：`allocate_port_pair(bind)` 按中继真正要绑的地址探测。回归测试
`consecutive_allocations_never_reuse_a_port_the_relay_already_holds` 必须用网卡地址，
并且在用例里额外断言"回环上能绑上已被网卡占住的端口"——把 bug 的成因也钉进测试，
免得后人把测试改回回环地址后它悄悄失效。另加
`the_default_target_ports_can_all_start_at_once` 作为默认配置的冒烟测试。

## 仍未验证

- **从界面发起的一次真实接管**：界面交互驱动不了 WebView，只能由人确认。
  命令与面板已就位，但"点按钮 → 真的接管 → 浏览器能上网"这条没人走完。
- **长跑稳定性**：队列打满、连接表增长、长时间接管下的表现。
- **UDP 与 IPv6**：`rewrite()` 对非 TCP 与分片包一律原样放行，只有 IPv4/TCP 被接管。
- **许可**：WinDivert 为 LGPLv3 / GPLv2 二选一，闭源分发需法律评估。
- **中继绑定范围**：绑 `0.0.0.0` 等于把中继开放给整个网段。来源校验（放行集合严格等于
  接管集合）已经关掉了"被同网段当成开放代理"这个隐患，但绑定范围本身仍是产品决策。

# 证据总表（第三步更新）

| 套件 | 权限 | 结果 | 覆盖 |
|---|---|---|---|
| `cargo test --lib` | 无 | 132 通过 | 含新增 14 个接管用例、4 个 DNS 挂钩用例、分片与规则表用例 |
| `local_relay` | 无 | 4 通过 | 双向转发、来源校验、目标不可达、空闲连接不被误拆 |
| `redirect_session` | 无 | 3 通过 | 启动/停止顺序、失败不留半启动态、停止幂等 |
| `windivert_flow` | 管理员 | 1 通过 | FLOW 只读观察、进程归属 |
| `windivert_redirect` | 管理员 | 1 通过 | 回环改投 + 回滚 |
| `windivert_full_chain` | 管理员 | 1 通过 | 四段映射 + 真实中继 + 自噬规避 |
| `windivert_sharded` | 管理员 | 3 通过 | 多目标分片、加目标不断连接、跨片拆分 |
| `windivert_dns_connect` | 管理员 | 1 通过 | 应用解析出 IP 后仍能连接，零外网流量 |
| `windivert_internet` | 管理员 + 外网 | 4 通过 | **DNS 应答驱动接管 + 真实互联网四段映射 + 到期回收 + 停止释放资源** |
| `windivert_real_peer` | 管理员 + 手工对端 | 需设 `WINDIVERT_REAL_PEER` | 非回环"反射成入站"路径（WSL 对端），选入式 |

`windivert_real_peer` 需要手工准备一个 WSL 回显服务并设置两个环境变量，不设就 panic，
因此不进默认回归列表；非回环路径已由 `windivert_internet` 在真实互联网上覆盖。

# 第三步 3.4：中继侧 SNI 白名单分流

## 要解决的问题

DNS 联动接管是按**地址**接管的。一个 CDN 地址（例如 `104.20.x.x`）往往同时承载多个
域名，其中只有 proxy 分组的域名需要走中继。按地址接管会把同一地址上**不相关**的域名
一起接管 —— 这就是"连带接管"。

## 为什么 SNI 是唯一可行的判据

TLS ClientHello 是明文（除非启用 ECH），其中的 SNI 扩展直接给出目标域名。共享同一 IP
的不同域名在 TCP 层完全无法区分（目的 IP、目的端口都一样），只有 SNI 能把它们分开。

## 实现

三部分：

1. `windivert/sni.rs`：纯函数 `extract(&[u8]) -> Option<String>`，从字节流里挖 SNI。
   不依赖任何状态，可以脱离驱动做单测（8 个用例，含跨段、截断、非 TLS、IP 字面量、
   无 SNI 扩展）。
2. 域名随目标一起登记：`TargetRegistry` 记录 `地址 -> {域名}` 集合，
   由 `TakeoverManager::register(domain, addresses, ttl)` 接收，调用方是 DNS 应答钩子。
3. 中继侧判定：连接建立后 `peek` 首个数据段，解析 SNI 决定放行还是拒绝。

## 判定规则（保守优先）

| 情况 | 判定 | 理由 |
|---|---|---|
| 该地址没有任何白名单记录 | 不判定，直接放行 | 与加分流之前的行为完全一致；否则"开了功能但一个域名都没登记"会变成全拒 |
| SNI 命中白名单 | 放行 | |
| SNI 不命中白名单 | 拒绝连接 + 撤销该地址的接管 | 这个地址上的客户端不是 proxy 域名，继续接管只会持续失败 |
| 读不出 SNI（超时 / 非 TLS / 不完整） | 放行，单独计数 | 绝不误杀本来能用的连接 |

## 两个实现细节

**用 `peek` 而不是 `read`**：判定之后同一个套接字还要交给转发循环。用 `read` 把
ClientHello 读走，客户端就会卡在等 ServerHello 上。`peek` 只观察不消费。

**判定必须在 `set_nonblocking(false)` 之前**：Windows 上 accept 出来的套接字会继承
监听套接字的非阻塞状态。这里刻意复用这个状态做有界轮询
（`SNI_PEEK_POLL` 5ms × `SNI_PEEK_ATTEMPTS` 100 次 = 500ms 上限）。

## 证据：5 个用例，真实 rustls ClientHello

客户端用 `ClientConnection::write_tls` 生成**真实的** ClientHello，不是手拼字节：

| 场景 | accepted | sni_allowed | sni_denied | sni_unknown | 撤销请求 |
|---|---|---|---|---|---|
| SNI 命中白名单 | 1 | 1 | 0 | 0 | 无 |
| SNI 未命中白名单 | 0 | 0 | 1 | 0 | `(127.0.0.1, "other.example.com")` |
| 该地址无白名单记录 | 1 | 0 | 0 | 0 | 无 |
| 非 TLS 流量 | 1 | 0 | 0 | 1 | 无 |
| 空闲连接（不发数据） | 1 | 0 | 0 | 1 | 无 |

未命中时客户端读到 `ConnectionReset (10054)` —— 拒绝确实生效，不是只改了计数。

## 计数器语义修正（本轮自查发现）

`sni_allowed` 原先把"没做判定"也算成命中，后果是一个域名都没登记的部署会显示
"命中率 100%"，看不出分流其实没在跑。改成四态 `Allowed` / `NotFiltered` / `Unknown` /
`Denied`，只有真正命中白名单才计 `sni_allowed`。

## 诚实边界：分流今天还不是"选路"

**实测确认：`relay.rs` 拨上游用的是裸 `TcpStream::connect_timeout`，全项目没有任何
SOCKS / 代理出网代码（grep `socks` 零匹配）。因此"走中继"与"直连"当前的字节流向完全一致。**

也就是说，SNI 分流今天的实际效果只是**决定要不要继续接管这个地址**，而不是在多个出口
之间选路。这点必须写清楚，否则容易误以为它在做智能分流。要让它真正成为出口选择器，
前提是先有代理出网能力。

# 第三步 3.5：接管看门狗

## 要解决的问题

分片工作线程在连续读失败后会 `break` 退出。此时规则表原封不动：`shard_count` 与
`target_count` 看起来完全正常，`status().running` 也报 `true`，但那个分片上的包
**一个都不会被改写** —— 接管静默失效。

失效本身不会断网（包会原样放行，退回直连），但用户看到的是"接管开着却没效果"，
而界面报着"运行中"。这属于最难查的一类问题。

## 三处乐观状态一起修

1. `ShardedRedirector::status()`：`running: shard_count > 0` → 改为「有片**且全部健康**」
2. `Running::status()`：`running: true` 是硬编码 → 改为由健康度决定
3. 新增 `unhealthy_shards()`，报出「异常分片数 + 受影响目标数」

## 停机动作

reaper 线程每 `REAP_INTERVAL` 醒一次，发现异常就：释放重定向器与全部中继 → 摘掉
`Running` → 把停机原因单独存一份 → 置停机标志并退出。

停机原因必须**单独存**（`TakeoverManager::watchdog_stop`）：`Running` 被丢弃后挂在它
上面的错误信息也跟着没了，界面只会显示"未运行"，用户分不清是自己关的还是程序自动关的。

**为什么不能直接调用 `TakeoverManager::stop()`**：reaper 线程自己就是 `stop()` 要 join
的那个线程，调用它等于自己等自己。

**`REAP_INTERVAL` 从 5s 改到 1s**：这个周期同时决定撤销请求的响应延迟 —— 中继判定
SNI 不命中后把地址投进队列，要等 reaper 醒来才真正撤销，在那之前该地址上的连接会持续
被拒。1s 是"用户几乎无感"与"线程开销可忽略"的折中。

## 证据

`the_watchdog_stops_the_takeover_and_releases_every_resource`：用回环启动真实
`TakeoverManager`（真中继、真线程、真监听端口），注入健康问题后验证三件事：

- `is_running()` 转为 false
- 中继端口不再可连接（资源确实释放了，不是只改了个标志）
- `status().health_problem` 保留了停机原因

**证伪验证**：把停机逻辑条件置假后该用例立刻变红，且失败在最直接的断言上
（"看门狗必须在发现异常后自动停止接管"）。测试确有牙齿，不是恒真。

## 为什么"检测"那一半要管理员权限

制造"分片工作线程已退出"必须真开一个 WinDivert 句柄。所以把验证拆成两半：

- **反应**（非提权可测）：注入问题 → 验证真的停机回滚
- **检测**（需管理员）：真实线程退出 → 验证被判定为不健康
  （`a_shard_whose_worker_finished_is_reported_as_unhealthy`，无 DLL 或无权限时
  **明确打印 SKIPPED，不做假通过**）

# 本轮抓到的第二个真实产品 bug：句柄失败判定写错

`Windivert::open()` 原先判断 `handle.is_null()`。但官方文档写得很明确：

> A valid WinDivert handle on success, or `INVALID_HANDLE_VALUE` if an error occurred.

`INVALID_HANDLE_VALUE` 是 `(HANDLE)-1`，**不是 NULL**。所以打开失败时，代码会把一个
`0xFFFF...` 的假句柄当成有效句柄返回 `Ok`。

后果：调用方拿到假句柄一路走到 `recv` 才以"读取报文失败"的形式炸掉，而真正的错误原因
（驱动没起来、权限不足）在那时已经丢失。**这个 bug 在管理员环境下永远不出现**（句柄总能
开成功），只有非提权运行才暴露 —— 这也是它一直没被测试抓到的原因。

发现路径：给看门狗写"真实线程退出"用例时，测试报告"成功启动了工作线程"，但本机
`IsAdmin=False` 且驱动服务不存在 —— 这个矛盾直接指向了失败判定。查证靠官方文档
原文 + 实测错误码。

修法：`handle.is_null() || handle as usize == usize::MAX`。回归用例
`open_never_hands_back_an_invalid_handle_as_success` 在非提权环境下断言错误码为
5（ERROR_ACCESS_DENIED），修复前该用例会因假句柄而变红。

# 本轮抓到的第三个问题：停机排序让状态不诚实

看门狗最初是「先 `take()` 摘掉 `Running`，再停资源」。而 `is_running()` 与 `status()`
读的就是 `Running` 是否存在 —— 于是外部会在**中继还在监听**的时候就看到"未运行"。

这个窗口是写测试时撞出来的：断言"停机后中继端口不可连接"间歇性失败。加诊断输出后
看到那一刻连接确实连上了（而不是期望的拒绝），证明是排序问题而非测试写错。

修法：整个停机过程持有同一把锁，**先释放资源再摘掉 `Running`**。持锁期间
`is_running()` 会阻塞，观察者只可能看到"停之前"或"停完了"两个状态。`stop()` 里同样的
排序也一并改了。

修完串行、并行各连跑 3 次，共 6 次全绿。

# 证据总表（SNI 分流与看门狗）

| 套件 | 权限 | 结果 | 覆盖 |
|---|---|---|---|
| `cargo test --lib` | 无 | **146 通过 / 1 忽略** | 本轮新增 6 个：SNI 计数器语义、分片健康检测 ×3、句柄判定回归、看门狗停机回滚 |
| `sni_diversion` | 无 | **5 通过** | 真实 rustls ClientHello 下的命中放行 / 未命中拒绝并撤销 / 无白名单不判定 / 非 TLS / 空闲连接 |
| `local_relay` | 无 | 4 通过 | 无回归 |
| `windivert_full_chain` / `windivert_sharded` / `windivert_internet` / `windivert_real_peer` | 管理员 | 忽略 | 本环境 shell 非提权，未运行 |

基线 140 通过 + 1 忽略 → 现在 146 通过 + 1 忽略。

## 本环境的限制（诚实边界）

- 当前 shell **非管理员**（实测 `IsAdmin=False`），且 WinDivert 驱动服务未安装
  （`sc query WinDivert` 报 1060；注册表 `Services` 与 `driverquery` 都查不到）。
  因此所有需要驱动的端到端用例在本轮**未运行**，本轮结论只建立在非提权路径上。
- `cargo test`（不带参数）会在 `--bin dns-proxy` 上失败：该二进制带
  `requireAdministrator` 清单，测试进程起不来（os error 740）。**这与本轮改动无关**，
  基线同样如此。常规回归请用 `cargo test --lib` 加按名指定的集成套件。




