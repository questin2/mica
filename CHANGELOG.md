# CHANGELOG

## [阶段2-42] 2026-10-18 —— **安全审计闭环：审批链加固**

- **只读/自动放行改为保守白名单形状**：管道 / 分号 / 重定向 / 子表达式 / 脚本块 / 写入类 cmdlet / 多行
  一律判 danger，不再进入会话信任或 `autoApprovePatterns`；`Out-File` 等写别名不再算只读。
- **后端敏感路径兜底**：审批文本涉及 `config\`、`secrets\`、已知配置文件名、`EAGENT_*` 时强制人工确认，
  并在审批卡上加安全警告；`tool_decision` 写可信审计（命令哈希/forced/age）。
- **审批 id 随机化**：u32 范围（避免 JS `Number` 精度丢失），登记表保存命令哈希与 forced 标记。
- **堵 debug 探针旁路**：`run_tool_probe` 只允许 `calc`/`now`，不再能直接执行 `web_read`。
- **MCP 供应链**：只保留 workspace 根目录的 `fs` + `memory` + `sequential-thinking`，所有包钉版本；
  子进程 `env_clear()` + Windows 环境白名单；`server-git` / `server-sqlite`（npm 404）移除；
  `mcp_save_server` / `mcp_import` 只允许 `npx`/`node`，其他 command 强制停用。
- **安全配置快照**：`autoApprovePatterns` / `replyEndpoint` / `approval.json` / `mcp.json` / `provider.json`
  启动时冻结；外部改动需在设置页「重新加载安全配置」确认，未确认时回退安全默认。
- **密钥迁移**：`secrets/` 移出仓库到 `%APPDATA%\EAgent\secrets`，发布脚本默认路径同步。
- 验证：前端 **244** 用例全绿、`tsc && vite build` 通过；Rust **143** 用例在干净运行下全绿
  （`chat::ollama` 的两个 mock 测试在本机偶发本地连接失败，单跑通过，与本次改动无关）。

## [阶段2-41] 2026-10-17 —— **发布硬化：Q1 根因查清 + 路径回退 / 崩溃落盘 / 单实例**

### Q1 结论：os error 5 不是 Defender 受控文件夹访问

- `Get-MpComputerStatus`：Defender **整体关闭**（`AntivirusEnabled=False`、`AMServiceEnabled=False`）；
- `Get-MpPreference`：`0x800106ba`（服务未运行，CFA 无法启用）；
- Defender CFA 事件 1123/1124/1127：**无记录**；
- ACL 证据：`%LOCALAPPDATA%` 对 `CodexSandboxUsers` 只有 `RX`，`%TEMP%` 含沙箱 SID，
  `D:\EAgent` 含 `S-1-4-357510-576819066`（沙箱能力 SID）与 `Everyone:(DENY)(DC)`；
- 应用自身写探针：`LOCALAPPDATA=✓ TEMP=✓ data_dir=✓`。

**结论**：历史 os error 5 是**开发期沙箱/ACL 环境**造成的，不是产品在普通用户环境下的行为。
仍需保留路径回退链以应对 ACL 损坏/EDR/企业策略，但不需要 Defender 白名单或提权。

### 已落地的发布硬化

- **数据目录解析链 + 写探针**：`EAGENT_DATA_DIR → 开发布局 → %APPDATA% → %LOCALAPPDATA% → exe 旁 → %TEMP%`；
  每级建目录+写探针+删除，结果进启动日志与自检（`path_probe_report()`）；开发布局优先，升级不丢现有数据。
- **默认工作目录**：`%USERPROFILE%\Desktop → %USERPROFILE% → 当前目录`；移除前端/后端硬编码开发机路径。
- **panic hook**：崩溃写 `<data_dir>\crash.log`（失败回退 `%TEMP%`），含 backtrace；下次启动检测 24h 内日志并发 `crash` 事件，前端弹告警。
- **单实例**：`tauri-plugin-single-instance`；第二实例聚焦已有窗口后退出。实测连续启动两次始终只有 1 个进程。
- **数据/缓存分离**：数据走 `%APPDATA%\EAgent`，缓存（WebView2 profile / npm）走
  `%LOCALAPPDATA%\EAgentCache`；缓存与 per-user 安装目录 `%LOCALAPPDATA%\EAgent` 是兄弟目录，
  卸载后安装目录不残留。
- **MCP 缓存目录**：未显式配置 `npm_config_cache` 时默认注入 `<cache_dir>\npm-cache`，不再依赖 `%LOCALAPPDATA%`。
- **NSIS per-user**：`bundle.targets=["nsis"]` + `installMode=currentUser`，免 UAC；安装目录只读对待。
- **UI**：设置页显示真实数据目录/WebView2 profile/路径探测；移除写死的 `D:\EAgent\app\data\webview`。

### 真实安装回归（2026-10-17）

- 成功构建 NSIS：`EAgent_0.1.0_x64-setup.exe`（6.12 MiB，NotSigned）；
- **R01 全新安装 ✅**：exit 0，实际安装到 `%LOCALAPPDATA%\EAgent`（per-user，无 UAC），开始菜单 1 项；
- **R03 卸载 ✅**：exit 0，安装目录与快捷方式删除，**`%APPDATA%\EAgent` 数据保留**；
- **R12 单实例 ✅**：安装版连续启动两次始终只有 1 个进程；
- **R02 覆盖安装 ⚠️**：同版本重装通过、数据保留、快捷键不重复；真实版本升级未测；
- **R06 MCP 子进程 ⚠️**：安装包未附带 `config/mcp.json`，MCP 默认不启用，需配置后单独测；
- 安装版路径实测：`[OK] APPDATA: ...\Roaming\EAgent` + `[OK] LOCALAPPDATA-cache: ...\Local\EAgentCache`；
  DB/日志/知识库在 Roaming，WebView2 profile/npm 缓存在 Local；卸载后安装目录删除、数据与缓存保留。

### 自动更新 / 真实升级 / 安装版 MCP / panic（2026-10-17）

- **版本升至 0.1.1**；updater 密钥对已生成（`secrets/` 已 gitignore）：
  `tauri-plugin-updater` + `tauri-plugin-process`，权限 `updater:default` / `process:allow-restart`；
- 设置页新增「更新」卡片（检查/下载/安装/重启 + 进度）；`scripts/release/build_release.ps1`
  一键构建并生成 `latest.json`；
- **R05 E2E ✅**：0.1.0 检查本地 latest.json → 下载 **6,829,241 字节** → minisign 校验 →
  NSIS 安装 → 重启后 exe FileVersion = **0.1.1**；
- **R02 真实升级 ✅**：0.1.0 → 0.1.1 数据目录与缓存保留，开始菜单仍 1 项；
- **R06 安装版 MCP ✅**：`EAGENT_MCP_PROBE=1` → npx 拉起 filesystem server，
  握手 15,648ms，工具清单 **2,827 字符**；npm 缓存落在 `%LOCALAPPDATA%\EAgentCache\npm-cache`；
- **R15 panic 落盘 ✅**：`EAGENT_PANIC_TEST=1` → `%APPDATA%\EAgent\crash.log`（513 字节含 backtrace）；
  下次启动检测并写日志/发 `crash` 事件；
- **MCP 子进程 Job Object ✅**：`win_job.rs` kill-on-close Job；
  日志 `mcp: pid 19432 已加入 kill-on-close Job Object`，强杀应用后该 pid 立即 `GONE`；
- **安装前杀进程 ✅**：`installer-hooks.nsh` 的 PREINSTALL/PREUNINSTALL 先 `taskkill /F /T`；
  在应用（含 MCP 子进程）运行中直接覆盖安装 0.1.1 → exit 0，旧进程终止、数据保留；
- **R04 Authenticode 流水线 ✅（自签名验证）**：`scripts/release/sign_windows.ps1` 成功签名安装包，
  证书可见；状态 UnknownError 仅因根不受信。OV 证书采购是唯一剩余阻塞。
- 本地 updater E2E 使用 `tauri.test-updater.conf.json`（localhost HTTP + 危险传输开关）；
  生产配置仍只允许 HTTPS 的 GitHub Releases。

### 仍未完成（需决策/证书/隔离环境）

- Authenticode **OV 证书采购**（流水线已用自签名验证）、Defender 开启环境扫描、
  125%/150% 缩放回归、卸载“彻底清除数据”选项、更新回滚入口。
  详见 `docs/18-release-hardening.md`。

## [阶段2-40] 2026-10-17 —— **Phase 2–4 全部落地：双通道 / SQLite 记忆 / 知识库 / 流程编排**

### Phase 2：对话层（双通道）

- `chat/mod.rs`：`ChatProvider` trait（`send_turn` 一轮完整语义 + `TurnReply` 原生 tool_calls 预留）；
  `EngineState.chat` 按配置键（通道 + 模型 + 地址）懒建/重建；切换前 `cancel + reset`。
- `chat/website.rs`：现役 `send_and_wait_full` **原样收编**，选择器/网络层/完成判定零改动。
- `chat/ollama.rs`：`GET /api/tags` + `POST /api/chat`（blocking，非流式）；
  会话历史、connect/request 双超时、明确错误映射；模型不支持 `tools` 时自动回退文本协议；
  404 `model not found` 返回可执行的中文修复提示（`ollama pull <模型名>`）。
- `config/provider.json` + 运行时读取/原子写；`list_providers` / `set_provider` 命令。
- 前端 `ModelSwitcher`（DeepSeek 网页 / Ollama 模型列表）；切换 = 新会话（清视图 + 复位引擎）。
- `ProviderCaps.max_tool_list_chars`：工具清单按行裁剪，超限明确标注"已省略"。
- `web_read` 内置工具：仅 http/https、DNS 解析后拒绝私网/环回/链路本地、逐跳校验重定向、
  20s 超时、2MB 上限、输出 8000 字符；**每次调用人工审批**并展示完整 URL。

### Phase 3：记忆层

- `memory/db.rs`：`rusqlite`(bundled) + WAL + busy_timeout；`PRAGMA user_version` 迁移；
  表：sessions / messages / tool_calls / memory / audit / workflow_runs / kb_docs / kb_chunks。
- 回合落库：`run_agent_loop_traced` 返回 `AgentTurn { reply, tool_calls }`，`agent_turn` 持久化；
  失败只记日志，不影响对话。
- 长期记忆 KV：仅显式写入；新会话注入 ≤800 字符；设置页 `MemoryPanel` 增删。
- 知识库：txt/md/csv/json/代码/html/docx/pdf 抽取（GBK 回退）；700 字符分块 + 80 overlap；
  BM25（k1=1.2,b=0.75）+ 中文 unigram/bigram；每轮检索 top 3 注入 ≤2400 字符；`KnowledgePage` 导入/检索/删除。
- 多会话：`HistoryPage` 列表/查看/删除/载入视图；`open_session` 切换当前会话。
- 审计双写：`audit.log` 保持权威副本，同时并行写 `audit` 表。

### Phase 4：自动化与打磨

- `workflows.rs`：3 个内置模板（资料整理/项目巡检/文档摘要），顺序执行、状态机、
  `engine://workflow` 进度事件、取消标记；每步复用 `run_agent_loop_traced`，审批门不旁路。
- Ollama 原生 `tool_calls`：请求附带 `tools`（内置 + PowerShell + 插件模板），
  响应解析后走**同一审批/执行路径**；文本协议保留为回退。
- `.github/workflows/ci.yml`：Windows runner，npm test/build + cargo test/check。
- 硬化：每轮记录输入/输出字符数（提示词体量指标）；MCP 启用/保存/导入前展示完整启动命令并要求确认。
- **MCP / web_read 审批文本带 `[强制逐次确认]` 前缀**，前端归为 danger：
  不提供「本会话全部允许」，配置自动放行规则也不生效（避免外部工具被会话信任静默放行）。
- **对话头部响应式修复**：浏览器面板展开时标题/元信息不再被挤成竖排；
  窄主区自动隐藏次要信息并允许换行。初始窗口尺寸按显示器逻辑工作区收敛（高 DPI 下不再超出屏幕）。

### 验证证据（2026-10-17）

```
cargo test       132 passed / 0 failed（0 警告）
npm test         222 passed (7 files)
npm run build    tsc + vite 通过
cargo build      debug 二进制通过
```

- **真机网页链路（经新 ChatProvider）**：
  `chat: provider=deepseek-web（DeepSeek 网页（免费））发送 11 字符` → `CHAIN OK: reply="OK"`
- **记忆层启动**：`memory: 数据库已就绪 → D:\EAgent\app\data\eagent.db` →
  `boot: 记忆层 → SQLite 就绪（知识库 0 篇 / 0 块）`
- **Ollama 真机链路**：已安装 0.40.5 + 拉取 `qwen2.5:0.5b`（397MB）；
  `chat: provider=ollama（Ollama 本地模型）发送 11 字符` → `CHAIN OK: reply="OK"`
  （`ollama ps`：100% GPU，4K 上下文）。
  另有本地 mock HTTP 测试覆盖 tags / chat / 原生 tool_calls / 500 回滚历史 / 404 修复提示。

### 已知边界（诚实标注）

- Ollama 首版**非流式**：长回复期间只有"生成中"状态，没有逐字输出；
- 知识库采用**本地 BM25**，不是原案优先的 `repo-rag-mcp`（该 crate 未验证；ADR-0005 记录取舍）；
- MCP 工具仍走文本协议，未进入 Ollama 原生 `tools`（schema 由 server 动态提供，后续再接）；
- 历史会话"载入到对话"只恢复显示，**不迁移模型上下文**；
- 本机已拉取 `qwen2.5:0.5b` 用于链路验证（能力有限）；需要更强模型时
  `ollama pull qwen2.5:7b`，然后在顶部切换到该模型即可。

## [阶段2-39] 2026-10-17 —— **阶段 C 完成：MCP 工具生态真机跑通**（策划案 2.0 · Phase 1 达成）

### ★ 决定性证据（audit.log 原文）

```
[1792227565714] 审批来源：人工允许 #28（类别 normal）
[1792227565720] MCP[fs] tool=list_allowed_directories 5ms result=ok killed=false
```

**批准在 .5714，执行在 .5720 —— 相隔 6ms。** 完整链路：
模型发出 `mcp:fs.list_allowed_directories` → 引擎路由 → `request_approval` →
**用户点允许** → 桥接 → 真实 filesystem server 执行 → 结果回传 → 模型据实作答。

模型的实际回复也印证了正确性：它准确报告"MCP 的文件系统服务只放行 D:\\EAgent 一个目录"，
与 `config/mcp.json` 里配置的根目录一致 —— **结果是真实的，不是编的**。

### ⑤ / ⑦ 同时达成

- **⑤ 端到端**：配置 → 桥接 → npx 拉起真实 server → initialize → tools/list → 提示词注入
  （实测 2827 字符，prompt 18394 → 21221）→ 模型调用 → 审批 → 执行 → 回传 → 即席作答；
- **⑦ 审批门无旁路**：审计显示**批准那一刻之前没有任何执行记录**，批准后 6ms 才执行。

### 过程中修复的三个真 bug（都是真机才暴露的）

1. **连接静默死亡**：MCP server 空闲约 3 分钟即 `Transport closed`，
   原实现走"缓存 client 快速失败"→ **静默给出空工具清单**（模型完全不知道有 MCP 工具）。
   这是"看起来正常但其实没生效"的最坏情形。已加**自愈**：失败即丢弃桥接、重建、重试一次
   （实测触发成功：319ms 失败 → 重建 → 2181ms 成功）；
2. **可见控制台窗口**：`mcp_bridge` 拉起 npx/node 时漏了 `CREATE_NO_WINDOW`
   （`exec.rs`/`commands.rs` 都设了，这里漏了）→ 用户看到空黑框，关掉等于杀掉 server。
   已补 `#[cfg(windows)] c.creation_flags(0x0800_0000)`，实测可见 Console 窗口 = 0；
3. **`.gitignore` 缺口**：`git add -A` 曾把 `_npmcache/`（4495 个文件）提交进去，
   已回退并加规则。教训：本仓库有大目录，**`git add` 一律显式列文件**。

### 阶段 C 全景

| 步骤 | 状态 |
|---|---|
| ① 配置层（`config/mcp.json`，空则完全不生效） | ✅ |
| ② 全局桥接（`EngineState.mcp` 懒建 + `TauriObserver`） | ✅ |
| ③ 路由 + **审批门**（`mcp:<server>.<tool>`，无旁路） | ✅ 真机验证 |
| ④ 工具清单注入提示词 | ✅ 实测 2827 字符 |
| ⑤ 真机 E2E | ✅ **6ms 证据** |
| ⑥ 移除两处过渡 `allow(dead_code)` | ⬜ 收尾 |
| ⑦ 未审批不执行 | ✅ 审计证明 |

### 策划案 Phase 1 的实质进展

策划案 §2.2 差距清单原第 1 条"**无 MCP Client → 工具生态为 0**" **已消除**：
现在可接任意 MCP server，工具清单自动进提示词，调用经统一审批门。
剩余差距：多模型（Phase 2）、持久化记忆（Phase 3）、自动化编排（Phase 4）。

## [阶段2-38] 2026-10-17 —— B-bridge 集成验收：6 条用例全绿（93/93），修掉一个真实设计缺陷

### fake server 加 `--mark-file <path>`（先决条件）

收到 `tools/call` 时**先写**该文件（内容 = pid）。
**这是 cancellation 用例的前提** —— 只用 `hang` 无法区分"bridge 没发"和"发了但 server 卡住"，
那会写成**伪测试**。审查指出的这条是实质新增。

### 6 条集成用例（真跑 fake server，`mcp_bridge` 10/10）

| # | 用例 | 断言 |
|---|---|---|
| 1 | `normal` | 回显成功；审计 `result=ok`、reason 含 `initial handshake` |
| 2 | `crash` → 再调一次 | 首次 `crashed`；第二次审计含 `lazy restart`（证明真的重拉了） |
| 3 | `hang` + 1500ms | `Err(超时)`；**mark 里的 pid 已死**；审计 `timeout` / `killed=true` |
| 4 | `crash` + 100KB stderr 洪泛 | reason 截断 ≤1000 字符且标注 `truncated` |
| 5 | 外层超时（注入 hook 睡 6s） | `recv_timeout` 在 slack 后返回"桥接失效" |
| 6 | cancellation | 外层超时后桥接线程恢复时**跳过执行**（mark 未产生）+ 审计 `cancelled` |

### ★ 修掉一个真实设计缺陷（it6 暴露）

原实现只在**出队时**检查 `cancelled` flag，但最关键的窗口是**握手之后、真正 `call_tool` 之前**。
握手可能已花掉几百毫秒，期间调用方放弃 → 原实现仍会执行 = **未报告的成功**。
已在 `dispatch` 内该临界点补检查。

顺带修正语义：首次连接不再标成 `lazy restart`（`was_connected` 在 handshake 之前读取）。

### ⚠ 实测发现一个未定论的环境问题（记录，不掩盖）

本机上 **fake server 子进程写 `%TEMP%`、`%LOCALAPPDATA%`、甚至 `app/src-tauri/target/` 都得到
`拒绝访问 (os error 5)`**，而写 `_spike_mcp/` 完全正常 —— 同样是子进程、同样的启动方式。
已把测试临时文件改到 `_spike_mcp/`（加 `.gitignore`），并在代码注释里写明原因。

**这个差异很可能是之前 npm `EPERM` 的真正根因** —— 此前归因为"npm 缓存目录权限"可能不准确。
属于进程令牌/ACL 层面，尚未定论；若它真实存在于用户环境，则 MCP server 的缓存目录配置
（§ 每 server `env`）必须指向**已验证可写**的路径，不能想当然用 `%LOCALAPPDATA%`。

### Verified
- `cargo test` **93/93**（+6 集成）；`cargo check --tests` **0 警告**

## [阶段2-37] 2026-10-17 —— B-bridge 四条设计闭合 + `mcp_bridge.rs` 落地（`f2a49ad`）

### 文档闭合（评审四组钉子，全部写进 `docs/15`，670 行）

**§4.5 cancellation flag —— 副作用安全**（四条里最严重的一条）：
外层 `recv_timeout` 超时返回时 `Request` **已入队**；桥接线程之后恢复会照常 dispatch，
→ 调用方以为失败、实际 tool 已执行 = **"未报告的成功"**。
解法：`Request` 带 `Arc<AtomicBool> cancelled`，同步侧超时即置位，桥接线程 **dispatch 前**检查。
**如实写明边界**：flag 只阻止"尚未开始执行"的请求；外层超时语义是
「桥接子系统失效，无法保证」，**不是**「调用被撤销」。

**§4.6 双 deadline**：崩溃重拉要先握手再执行，一个预算不够。
`Request` 带 `call_deadline` + `handshake_deadline`，caller 从配置读两个值传入。
冷启动首次可能吃 30+30=60s，**这是正确的**（§7 预热就是为避免用户撞到）。

**§3.5 `Bridge` 必须是可构造实例**：全局单例只是 wrapper。
否则 `hang` 用例会串行阻塞其他单测。同时定死 `Drop` 顺序：
drop tx → join 带超时 → 子进程由桥接线程在自己退出路径清理。

**实现细节 5 条**：catch_unwind 包在 `rt.block_on` **外层**（panic 后 runtime 已坏 → 标记 failed 退出 loop，不恢复）；
stderr **持续 drain** 只保留前 64KB（读满就停会重新堵管道）；
`args` 定为 `Map` 而非 `Value`；重拉阶段审计 `tool="-"` + reason 带 `lazy restart` 摘要；
`BridgeHandle` 不持 `AppHandle`。

**已知缺口显式化 3 条**：外层超时后子进程可能残留（桥接线程卡死时清理也失效，进程退出统一清理）；
cancellation 只防"未开始执行"；pid 缺失时的孤儿进程由下次懒恢复清理。

### 代码：`mcp_bridge.rs`（`f2a49ad`，3 文件 +693/−5）

按上述定案实现，并附带一处重构：
**`exec::kill_process_tree(pid)` 抽出** —— 原先 `taskkill /T /F` 在 PowerShell 超时路径里内联，
MCP 清理要再写一遍。现在两个调用方共用一个 owner（且明确"必须用 pid，绝不按进程名杀"）。

### Verified
- `cargo check --tests` **0 警告**；`cargo test` **87/87**（新增 4 条）
- 新增测试含**以 A′ 实测 EPERM stderr 为样本**的回归：断言保留 `EPERM` 关键行、
  标注 `...(truncated N lines)`、总长 ≤ 1000 字符

### 未完成（下一步，如实记录）
- **8 行单测矩阵的集成部分**（normal / hang 内层超时 / crash 重拉 / stderr-flood /
  外层超时 / cancellation / pid 缺失 / EPERM fixture）—— 需先落 `fake_server_path()` 助手
  （方案 A：`CARGO_MANIFEST_DIR` + `../../_spike_mcp`）。当前只完成纯函数部分（第 8 行与截断逻辑）；
- **接入 `tools.rs` 路由**（阶段 C），届时移除 `mcp_bridge.rs` 顶部的过渡性 `allow(dead_code)`。

## [阶段2-36] 2026-10-17 —— 27 阶段积压清零（8 个逻辑组）+ 修掉 `CARGO_BIN_EXE` 跨 crate 坑

### 积压提交：26 个阶段未提交的 80 个文件，按 7 个逻辑组入库（不回溯 26 个阶段）

```
50cc773 docs: design docs and changelog evolution (phases 2-8 ~ 2-34)      8 文件
a22fa45 test(spike): MCP transport verification harness (phase A/A-prime)  9 文件
7c7a7d1 chore(baseline): UI evidence screenshots (phases 2-8 ~ 2-34)      34 文件
a46ebb3 chore(config): runtime configuration evolution (phases 2-8 ~ 2-34) 4 文件
24bc9dc test(frontend): 219 tests and vitest toolchain (phases 2-8 ~ 2-34) 3 文件
e2a67b1 feat(frontend): components, hooks and styles (phases 2-8 ~ 2-34)   38 文件
551e097 feat(backend): phases 2-8 ~ 2-34 backend modules                 17 文件
d9a0ca5 build(deps): add tokio + rmcp for MCP bridge (no code yet)          2 文件
```

**工作区现在干净（0 项未提交）**。提交前完整回归：前端 **219/219**、后端 **83/83**。

**刻意不按 26 个阶段回溯**：阶段边界已模糊，硬拆 26 个 commit 是"伪精确"
（每个 message 都是事后编的），不如按逻辑分组诚实记录。

### 清理：`_spike_mcp/` 的 14,562 个未跟踪文件

根 `.gitignore` 没有 `_spike_mcp` 规则，`target/`（3.3GB）与三份 npm 缓存（各 41.9MB）
都会被误入库。已删缓存（共 125.7MB）、补 `.gitignore` 规则 → **14,562 → 8 个文件**。

### ★ 修掉评审指出的实质缺口：`CARGO_BIN_EXE` 跨 crate 失效

文档 §8 原写"二进制路径用 `env!("CARGO_BIN_EXE_fake_mcp_server")` 定位，复用阶段 A 的构建方式"——
**这行只在 `_spike_mcp` 这个 package 内成立**。B-bridge 的单测在主工程，主工程没有该 bin，
`env!` 会**编译期报错**。

选定**方案 A**（不动主工程 bin 结构、不污染 `Cargo.toml`）：
主工程测试用 `CARGO_MANIFEST_DIR` 相对定位 + 缺产物时 `--manifest-path` 构建。

> **实测踩了一次坑**：第一次写 `../../../_spike_mcp` → 解析成 `D:\_spike_mcp`（差一级全错）；
> 正确是 **`../../_spike_mcp`**（`app/src-tauri` 只比 `_spike_mcp` 深两层）。

### fake server 4 种 mode 已验证（`_spike_mcp/tests/fake_server_modes.rs` → 4 passed）

| mode | 实测 |
|---|---|
| `normal` | ✓ 回显 `echo:hi` |
| `hang` | ✓ 802ms 超时仍未返回（供内层 deadline 单测） |
| `crash` | ✓ `Err(Transport closed)`，**1.93ms**，不挂起 |
| `stderr-flood` | ✓ drain 后握手与调用均正常 |

顺带纠正了一个错误的冒烟方法：**用空 stdin 重定向会让 server 读到 EOF 正常 `exit 0`**，
看起来"四种 mode 都秒退"，其实什么也没证明 —— 必须用管道驱动走完 initialize。

### B-1 逐行读源码确认（不只信文档）

`append_log` 确为纯同步 `std::fs`；`app.emit` 是 Tauri `Emitter::emit`（同步）；
`EngineState` 全为 `Mutex`/`Arc`；`AppHandle` 未存入状态（可安全跨线程持有）。
发现一处细节：`std::sync::mpsc::Sender` 是 `Send` 但**非 `Sync`**，现有代码靠 `Mutex` 包住；
桥接层 `reply_tx` 随 `Request` **移动**进桥接线程，设计上已避开这个约束。

### 两条固化进 checklist 的流程经验
1. **改 `Cargo.toml` 的提交，合并前必跑 `cargo check --locked`**（toml/lock 不同步的最便宜探针）；
2. **构建前必须彻底停掉 app**，否则 `failed to remove ...exe`（文件占用）会被误读成编译错误。

### 体积基线更新
B-bridge 的体积对比基线改为 **5,051,392 字节**（不是 5,044,736）——
接入后 rmcp/tokio 的 process/time 路径真正被链接，增量会跳升。

## [阶段2-35] 2026-10-17 —— **B-cargo 已提交**（`d9a0ca5`）：tokio + rmcp 依赖就位，9 条判定全过

### 提交内容（纯依赖，零 `.rs`）

```
d9a0ca5 build(deps): add tokio + rmcp for MCP bridge (no code yet)
 app/src-tauri/Cargo.lock | 107 +++++++++++++++++++++++++++++++++++
 app/src-tauri/Cargo.toml |  25 ++++++++++
 2 files changed, 132 insertions(+)   ← 注意：0 行删除
```

```toml
tokio = { version = "1", default-features = false, features = ["rt", "process", "time", "io-util"] }
rmcp  = { version = "3.5.1", default-features = false, features = ["client", "transport-child-process", "which-command"] }
```

### 9 条判定（全部通过）

| # | 检查 | 结果 |
|---|---|---|
| 1 | release 构建 | 成功。增量 **194s** / 全量 **230s**（基线全量 219s → **+11s**） |
| 2 | `cargo test` | **83/83** |
| 3 | debug 警告 | **0** |
| 4 | release 警告 | **0** |
| 5 | `cargo tree -i tokio` | **只有 1.53.3**（与 tauri 复用，无第二份） |
| 6 | `cargo tree --duplicates` | 基线 25 → 现在 25，**新增 0** |
| 7 | `Cargo.lock` 逐包比对 | 454 → 462（**+8**），**移除/变更 0**，关键版本全未动；`--locked` 通过 |
| 8 | release 体积 | 5,044,736 → **5,051,392**（**+6,656 字节**） |
| 9 | 启动冒烟 | ✅ 扩展工具与工具注册表与之前逐字一致 |

新增的 8 个包：`rmcp`、`process-wrap`、`tokio-stream`、`which`、`futures`、
`nix`（Unix-only，Windows 不编译）、`cfg_aliases`、`tokio-macros`。

### ★ 两个反直觉的观测（已写入文档）

1. **体积只涨 6.5KB** —— 因为本提交还没有任何代码使用这些依赖，
   `lto` + `strip` + `panic = abort` 把未引用代码全剪掉了。
   **真实增量要等 B-bridge 接入后才显现**，不能把 6.5KB 当成"MCP 只花这么点"。
2. **编译时间 +11s（+5%）** —— 这是引入 rmcp 的隐性成本，已记录以便评估后续迭代速度。

### 过程中的自我纠正

取"重复包基线"时我写错了还原逻辑（`Cargo.toml` 被还原成基线、而 lock 是新的，导致两者不一致），
已用 `cargo check --locked` 复核修正：**lock 与 toml 一致、无需重新解析**。
教训：比对类操作改文件前先确认"恢复到哪个状态"，`--locked` 是这类不一致的可靠探针。

### ⚠ 需要你决定的一件事（未擅自处理）

仓库最后一次提交是 **phase2-7**，**之后 26 个阶段（阶段2-8 ~ 2-34）的改动全部未提交**，
包括 Rust 16 个模块、前端 14 组件 8 hooks、219 个前端测试、`config/` 与 `docs/` 的大量演进。
本次 B-cargo 是"只暂存 2 个文件"的纯净提交，未触碰其余未提交内容。
**是否要按阶段把这批积压补成提交，需要你决定**（我没擅自打包成一个大 commit）。

## [阶段2-34] 2026-10-17 —— 动主工程前的"最后几个钉子"：env 等效性验证通过 + B-5~B-8 定案

### ★ 关键前置验证：`npm_config_cache` env **等效于** `--cache`（决定性对照实验）

`_spike_mcp/tests/phase_a_prime_env_cache.rs` → 1 passed

| 组 | 配置 | 结果 |
|---|---|---|
| 对照 | 不设 env、不传 `--cache` | **失败**，1.2s，stderr 2641 字节（EPERM） |
| 实验 | **只设** `env("npm_config_cache", <空目录>)` | **成功**，5.3s，stderr 0 字节 |

→ **`McpServerConfig.env` 方案成立**，不需要区分"npx 包装层参数"与"server 参数"，
**架构级变更风险解除**，阶段 B 按原设计进行。
（若 env 不生效，就必须改用 args 传 `--cache`，而 npx 的 args 层会被 server args 覆盖 ——
那是要重新设计的，所以这条必须先验、且已验。）

### B-5 的关键发现：`tokio v1.53.3` **已经在依赖树里**

`cargo tree -i tokio` → `tokio v1.53.3 ← tauri v2.12.4`。
所以显式加 tokio 会**复用同一版本**，**不会出现两份 tokio 或版本冲突** —— B-cargo 的最大风险点消失。
体积增量只来自额外 feature（`rt`/`process`/`time`/`io-util`）与 rmcp。

### 文档定案（`docs/15-mcp-bridge-design.md`，456 行）

**§5.2 `reason` 的 stderr 截断策略**（否决"取前 1000 字"）：
优先保留 `error`/`EPERM`/`ENOENT`/`EACCES` 关键行（≤5 行）→ 补首尾 → 标注 `...(truncated N lines)`；
用 A′ 实测的 EPERM stderr 样本做 fixture 回归。

**§5.3 审计写入策略**：写失败**直接丢弃 + stderr 警告，绝不重试**（不能让磁盘拖住桥接线程）。

**§5.4 pid 缺失的降级**（按评审意见修正）：**禁止**按名字杀 `node.exe`/`npx.exe`（会误杀用户其他进程）→
改为**不 taskkill、drop client 靠 stdio EOF 退出、标记 failed、审计 `killed=false`**，
孤儿进程由下次懒恢复清理（第一阶段不做孤儿列表，记为已知缺口）。这条分支**必须被单测覆盖**。

**B-5 Cargo 依赖最小化**：`tokio` 用 `default-features = false` + `rt`/`process`/`time`/`io-util`
（**不要 `macros`**，桥接线程用手写 `Builder::new_current_thread()`）；
`rmcp` 只开 `client` + `transport-child-process` + `which-command`；
回归含 **`build --release` 体积对比**。

**B-6** stderr 截断见 §5.2。
**B-7** `deadline` 由 caller 传入，桥接层只做 `timeout(deadline, dispatch(req))`，**不查配置表**；
好处是单测传 100ms 就能测超时，不必等 30s。
**B-8** 外层 `recv_timeout` 超时：优先 `#[cfg(test)] inject_dispatch_hook` 注入卡死 hook，
不引入测试专用 hook 则降级为"代码审查 + 手动故障注入一次" —— **不为此把架构搞复杂**。

**提交边界**：拆成 **B-cargo**（只加依赖，回归 83/83 + 体积对比）与 **B-bridge**（桥接层 + 单测）两个 commit，
让"依赖问题"与"逻辑问题"不互相掩盖。

**缓存目录**：用应用专属路径 `%LOCALAPPDATA%\EAgent\npm-cache`，**不要 `%TEMP%`**
（清理工具会删，删了下次又吃 6s 冷启动）。

### 材料与顺手修的警告
- **release 基线已建立**：`eagent-app.exe` = **5,044,736 字节（4.81 MB）**，耗时 3m39s；
  B-cargo 引入 tokio+rmcp 后直接与这个数比体积增量；
- 顺手修掉 **release 构建独有的 2 个 `dead_code` 警告**（debug 构建看不到）：
  `chain_flag_path` 只在 `#[cfg(debug_assertions)]` 里用，release 分支编译掉后就成了死代码。
  已给该函数与对应 `use` 都加 cfg 门控 → **debug 与 release 现在都是零警告**；
- 测试 **83/83**，应用已重启验证正常（`boot: 扩展工具 → extendedTools=true，内置 [now, calc]`）；
- 主工程仍**零改动**（`Cargo.toml` 无 rmcp/tokio）。

## [阶段2-33] 2026-10-17 —— 策划案 2.0 · **阶段 A′ 完成**（真实 npx 耗时/路径/stderr 量级）+ 阶段 B 前置确认

### ★ 阶段 A′：`_spike_mcp/tests/phase_a_prime_npx.rs` → 2 passed

**刻意用裸 stdio 直连真实 server**（不借 rmcp 测 rmcp），把耗时拆成三段：

| 轮次 | `which_command("npx")` | `spawn` | spawn→首响应 | **总计** | stderr |
|---|---|---|---|---|---|
| **冷**（删空缓存） | 1.21ms | 19.2ms | ≈6.0s（含下载） | **6.02s** | 0 字节 |
| **热**（已缓存） | 1.17ms | 22.7ms | ≈1.58s | **1.60s** | 0 字节 |

- **A′3 `which_command` 双向都通**，无需 fallback：
  `npx` → `...\Volta\npx.exe`(1.2ms)、`npx.cmd` → `...\Volta\npx.cmd`(0.3ms)、`cmd` → 可解析(5.4ms，留作最后回退)；
- **A′4 stderr 量级**：成功 0 字节、失败 2.64KB → **A2 的 8MB 洪泛确实过虑**，
  drain 仍保留（防御）但**缓冲截断到 64KB 即可**；
- **结论：§4.2 的 30s 对冷/热都足够**（5 倍余量），保持 30s + 每 server 可配。

### 实测撞到的真实环境坑（比数字更有价值）

本机 **npm 默认缓存目录写不进去**：
`npm error code EPERM ... path C:\Users\<local-user>\AppData\Local\npm-cache\_cacache\tmp\***`，
导致 MCP server **根本无法启动**。绕开方式：显式指定可写缓存（`--cache <tmp>`）后一切正常。

→ 两条设计结论：**桥接层必须支持每 server 配 `env`**（如 `npm_config_cache`）；
**必须把 stderr 摘要写进审计 `reason`** —— 正是这段 stderr 定位了问题，反证了 pipe+drain 的必要性。

### 阶段 B 前置：两个原本被当"隐藏工作量"的点，确认后都不是问题

1. **跨线程写审计/通知 UI —— 现有系统已安全**：
   `audit_log`/`chain_log` → `append_log` 是**纯同步 `std::fs` 追加**（不碰 async，桥接线程可直接调）；
   `emit_ev` → `app.emit()`，Tauri Emitter 同步且线程安全；`EngineState` 全为 `Mutex`/`Arc`。
   → **不会出现"同步调 async"违规**。
2. **§4.3 的 pid 来源定死**：`TokioChildProcess::id(&self)` 是公开方法，而 `serve()` 会消费 transport，
   所以顺序固定为 **`let pid = transport.id();` 先记下，再 `serve(transport)`** —— 无需破坏 rmcp 封装。

### 阶段 B checklist 已写入 `docs/15`
- **B-1** 桥接线程直接调 `audit_log`/`emit_ev`，UI 状态走共享 `Arc<Mutex>`；
- **B-2** 超时后 `reply_tx.send` 失败是**正常路径**：静默忽略，**清理必须发生在 send 之前**，
  **绝不把 `?` 用在 `send` 上**；
- **B-3** 懒启动用 `Mutex<Option<BridgeHandle>>` + `AtomicBool failed`（`OnceLock` 崩溃后不可重置）；
- **B-4 fake server 参数化行为已实现**：`--mode normal|hang|crash|stderr-flood` + `--flood-bytes N`，
  阶段 A 的 6 个用例**回归全绿**（确认加 mode 未破坏原行为）。

### 边界
- **主工程仍零改动**：`Cargo.toml` 无 rmcp/tokio、83/83 测试通过、回合循环未动；
- 全部改动在 `_spike_mcp/`（scratch）与 `docs/15-mcp-bridge-design.md`。

## [阶段2-32] 2026-10-17 —— 策划案 2.0 · 方案定稿 + **阶段 A 传输层验证完成（A1–A5 全部有实测答案）**

### 方案 v2 定稿（按用户四项裁决）

`docs/15-mcp-bridge-design.md` 已按裁决重写：
- **选 B**（专用线程 + 独立 `current_thread` runtime + `std::sync::mpsc`），不走整条异步化；
- 串行契约**写进 `mcp_bridge.rs` 接口注释**（不只留在文档），接口按 `Request { server, tool, args, reply_tx }` 设计，
  让"按 server 分发"成为内部实现细节，将来换 `multi_thread` / 每 server 一线程上层零改动；
- 超时 30s/30s + 每 server 可配 + **审计记录实际耗时** + 预热失败必须显式暴露；
- **§6 相邻修正（`spawn_blocking`）定为"必须做、单独提交、排在阶段 B 之后 / 阶段 C 之前"**；
  理由：阶段 C 要真机验并发行为，回合循环占着 tokio worker 会**污染验证结论**；
- 审计统一五字段：`server / tool / elapsed_ms / result / reason`；
- `notifications/cancelled` **发完立即继续，不等回复**（否则超时路径又引入一次等待）。

### ★ 阶段 A 完成：`_spike_mcp/tests/phase_a_stdio_transport.rs` → 6 passed

**fake server 刻意手写裸 JSON-RPC（不用 rmcp）** —— 避免"用 SDK 验证 SDK"，
验证的才是真实边界「rmcp 客户端 ↔ 裸协议 server」。

| # | 实测答案 |
|---|---|
| A1 换行符 | `\n` 与 `\r\n` **都被接受**（各自握手 + 发现工具成功） |
| A2 stderr 堵管道 | **确认会堵并可复现**：piped 但不读 + 8MB 洪泛 → 握手超时；加 drain 后同样洪泛正常。<br>→ 桥接层若为审计 pipe stderr **必须同时 drain** |
| A3 child pid | ✅ `TokioChildProcess::id()` 与子进程自报 pid **完全一致** → `taskkill /PID /T /F` 可靠 |
| A4 崩溃表现 | **`Err(Transport closed)`，1.7ms 内返回，不挂起**（比预期好；外层兜底保留但正常用不上） |
| A5 耗时 | spawn 5.4ms / initialize 3.6ms / tools.list 0.45ms / tools.call 0.45ms（**原生二进制基线**） |

**顺带发现的三个 API 事实**（Windows 专属坑）：
1. `TokioChildProcess` 默认 `stderr: Stdio::inherit()`，默认安全；
2. Windows 上 `npx.cmd` 这类 shim **不被 `tokio::process::Command` 可靠找到**，
   必须用 `which_command("npx")`（需 `which-command` 特性）；
3. `CallToolRequestParams` 是 `#[non_exhaustive]`，须用 `new().with_arguments()`。

### 边界
- **主工程零改动**：`Cargo.toml`、回合循环、线协议、`net.rs`/`session.rs`/`exec.rs` 全部未动；
- 阶段 A 产物是**可重复的 cargo test**（非一次性手测），阶段 B 直接复用它当子进程。

## [阶段2-31] 2026-10-17 —— 策划案 2.0 · **P1.7 端到端验证通过** + P1.1 MCP 可行性确认

### ★ P1.7：工具调用闭环**真机跑通**（这是 Phase 1 最大的未知项）

策划案 §12 把「模型不遵守文本标记格式」列为**头号风险**。为回答它，我加了一个
**一次性开机探针**（debug 构建、跑一次落 `.tool_probe_done` flag，不重复打扰）：
要求模型按 JSON 标记调用 `calc` → 引擎解析 → 执行 → 把结果回传 → 看模型能否据此作答。

真机日志（原样）：

```
TOOL PROBE: 回复 114 字符 → "# eagent_tool_调用工具_识别头_本行注释一定要完全保留
                             {"name":"calc","arguments":{"expr":"1+2*3"}}
                             # eagent_tool_调用工具_识别尾_本行注释一定要完全保留"
TOOL PROBE: 识别到工具「calc」参数 {"expr":"1+2*3"}
TOOL PROBE: 内置工具执行成功 → 7
TOOL PROBE E2E OK: 回传后模型回答 → "7"
```

**结论：模型确实照格式发出工具调用；解析→执行→回传→作答整条闭环成立。**
头号风险实测不成立 —— 这正是"免费 WebView 通道走文本工具协议"路线的关键前提。

### P1.1 可行性：`rmcp` 3.5.1 可落地

- 独立 spike 工程（`_spike_mcp/`，**不动主工程 Cargo.toml**）实测：
  `rmcp = "3"` + `tokio` **默认特性即可编译通过**（32 秒，锁定 98 个包）；
- 需要的特性都在：**`client`** + **`transport-child-process`**（stdio 子进程，正是拉起
  `npx @modelcontextprotocol/server-filesystem` 需要的那种）；
- 依赖代价：98 个包、编译 32s（可接受）。

**结论：MCP Client 路线可行**，不需要绕过 SDK 手写 JSON-RPC。
（注：`rmcp` 最新为 3.5.1，比策划案里写的 API 更新；下一步做握手测试时以 3.x 为准。）

### 本轮未做
- **stdio 握手 / tools/list / tools/call 的真实验证**（策划案 P1.1 的验收项）——
  spike 工程已具所需依赖，下一轮直接写 fake stdio server 完成；
- 把 `rmcp` 接进主工程（会引入 tokio 运行时，需与现有同步代码共存，要设计桥接方式）；
- P1.6 前端工具市场。

## [阶段2-30] 2026-10-17 —— 策划案 2.0 · Phase 1 开工：通用工具协议 + 内置工具（P1.2/P1.3/P1.4/P1.5）

> 依据：`C:\Users\<local-user>\Desktop\报表\EAgent2.0多功能AIAgent策划案.md`

### 开工前的关键发现（决定了本轮切法）

策划案把 **P1.1 MCP Client 集成**列为首要任务，但实测现有工程：
**完全没有异步运行时、没有 HTTP 客户端**（依赖只有 tauri/serde/webview2-com/windows/encoding_rs/base64）。
而首选 SDK `rmcp` 是 **tokio 异步**的 —— 那是给一个纯同步代码库引入 tokio，属架构级变更，
正是 P1.1 需要先回答的问题，不该顺手塞进来。

所以本轮先做**不依赖 MCP 的那半地基**：工具协议与工具执行。MCP 将来插的就是这个插座。

### Added

**通用 JSON 工具协议（P1.3/P1.4）** —— `tools.rs`
- 新标记 `# eagent_tool_调用工具_识别头/识别尾`，中间是**结构化 JSON**：
  `{"name":"calc","arguments":{"expr":"(1+2)*3"}}`
- **与旧 PowerShell 标记协议并行**：老协议逐字保留、优先级最高，策划案要求的兼容性成立；
- `parse_json_call()` 的返回值刻意区分三种情况：`None`（没有标记）/ `Ok`（合法）/ `Err`（**有标记但内容坏**）——
  后者必须把错误**回传给模型**，否则它永远不知道自己格式写错了；
- 未知工具名 → 明确报错回传，**不猜测、不静默**；
- `tool_list_prompt_block()`：工具清单由注册表生成（不是手写），并明确告知模型"旧 PowerShell 协议仍然有效"。

**内置工具（P1.5）** —— `builtin.rs`
- `now`：当前时间（UTC，unix 毫秒 + ISO-8601），零依赖（自写 civil_from_days 换算）；
- `calc`：算术表达式求值，**手写递归下降解析器，不 eval**；支持 `+ - * / % ^`、括号、小数、一元负号，
  右结合乘方；拒绝非法字符/未闭合括号/除 0/超长输入；
- 二者是**纯计算无副作用**，故不经审批（策划案 P1.5 明确"不审批"）；
- 其余一切工具（powershell 本体、插件模板工具）仍走**同一道审批门** —— 三条路径收敛到 `run_approved_powershell()`，无旁路。

**安全阀**
- `config/agent.json` 新增 `extendedTools`（默认 `true`）。设为 `false` 即回到"只有 PowerShell 协议"的老行为，**无需重编译**。

### Verified
- **Rust 83/83**（新增 19 条：JSON 协议的三种返回、坏 JSON 必须报错不能静默、name 必填、
  arguments 必须是对象、围栏剥离、标记常量稳定性、工具清单内容；
  calc 的优先级/括号/右结合乘方/**拒绝 eval 类输入**/除 0/超长；utc_iso 已知时刻；now 产出合法 JSON）
- **前端 219/219**；`tsc && vite build` 全绿
- **真机启动日志确认生效**：
  `boot: 工具注册表 → 共 5 个（powershell + 内置 2 + 插件 2）；插件说明 655 字符`
  `boot: 扩展工具 → extendedTools=true，内置 [now, calc]；清单 676 字符；通用协议标记已启用`

### 未做（下一轮，按策划案顺序）
- **P1.1 MCP Client**：需先评估给这个同步库引入 tokio 的代价（`rmcp` vs 官方 rust-sdk 的 Windows stdio 兼容性），
  这是策划案 §13 决策点 3；
- **P1.6 前端工具市场**：把内置/插件/MCP 工具列出来（可复用刚做完的 PluginsPage 模式）；
- **P1.7 端到端**：需要一次真实模型回合让它**主动发出 JSON 工具调用** —— 本轮只验证了协议解析、
  路由与提示词注入，**没有验证模型是否照格式输出**。这是本阶段真正的未知项。

## [阶段2-29] 2026-10-17 —— 插件从「后端有能力」变成「界面上能用」

> 用户反馈：「现在没有插件这一类的功能」

### 诊断：能力在，入口不在

后端 `plugins.rs` 早就齐了（清单解析、技能/工具贡献、故障隔离），但前端只有技能页上一行
`插件 N 个` 计数条 + 一句「把文件夹放进来」——
**没有列表、没有创建方式、没有删除、没有导航入口**，插件目录还是空的。
对用户而言这确实等于"这个能力不存在"。

### Added

**后端**
- `plugins::details()` + `PluginDetail`：把每个插件**贡献了什么**（技能明细 / 工具明细 / 加载错误 / 目录）摊开；
- `plugins::create()`：新建插件，可勾选附带**开箱即用的示例技能与示例工具**（建完立刻能在技能库与工具注册表里看到效果，而不是只得到空壳）；
- `plugins::safe_dir_name()`：插件名 → 安全目录名（只留字母数字 `-` `_`），**阻断路径穿越**；
- `plugins::delete()`：**后端强制两道闸** —— 必须是插件根目录的**直接子目录**，且必须含 `plugin.json`；
- 新增 4 个命令：`plugin_details` / `create_plugin` / `delete_plugin` / `open_plugins_dir`（用资源管理器打开，带 `CREATE_NO_WINDOW`）。

**前端**
- 左栏新增 **「插件」导航项**（自绘 plug 图标 + 数量徽标）；
- 新建 `components/PluginsPage.tsx`：插件卡片列表（名称/版本/描述/目录/**技能与工具明细**）、
  「新建插件」内联表单（可勾示例技能/工具）、「打开插件目录」、「刷新」、每卡「删除」（二次确认）；
- 加载失败的插件用单独样式与错误条显示原因；
- 插件增删后**联动刷新**技能徽标（插件会贡献技能）。

### Verified
- **Rust 64/64**（新增 5 条：目录名安全与**路径穿越阻断**、空名/无可用字符拒绝、
  **删除的两道闸**（根外目录 / 缺 plugin.json 都拒绝））
- **前端 219/219**；`tsc && vite build` 全绿
- **真机实拍**：左栏出现「插件 1」，页面正确显示 `demo-toolkit` 及其 **1 个技能 + 2 个工具**明细

### 过程中的两个自我纠正（如实记录）
1. **前端改动没生效**：`cargo build` 未重新嵌入 `dist`（`tauri-build` 的构建脚本没重跑），
   导致连改两轮界面都没变化。改用 `cargo clean -p eagent-app` 强制重新嵌入后才生效。
2. **我误判了一次「布局溢出」**：截图里卡片右侧像被截断，我据此加了 `min-width: 0`。
   但搭测量页实测（`getBoundingClientRect`）显示**卡片 706px / 右边界 738px，可用宽度 770px，根本没溢出** ——
   之前的判断是错的。`min-width: 0` 保留（本身无害且正确），但"溢出"结论已撤回；
   同时把删除按钮改成有可见描边（暗色卡片上 `btn ghost` 对比度太低），并让表头可换行以免按钮被挤出去。

## [阶段2-28] 2026-10-17 —— 「cmd 命令窗口还是弹」的真实根因：桌面放着一份修复前的旧拷贝

> 用户反馈：「现在还是会出来 cmd 命令窗口」

### 根因（产物层，**不在源码**）

排查过程（每个假设都被证伪，最后靠 PE 子系统 + 时间线定案）：

| 假设 | 结论 |
|---|---|
| 应用自身子系统没修好 | ✗ 实测 `eagent-app.exe` Subsystem = **2 (GUI)** |
| 工具执行弹窗 | ✗ 源码只有两处 `Command::new`（`powershell` / `taskkill`），**都带 `CREATE_NO_WINDOW` 且在 `spawn()` 之前** |
| 脚本里有会自己开控制台的命令 | ✗ 审计日志里 `Start-Process` / `cmd /c` / `conhost` **零命中** |
| 旧版易语言 `D:\EAgent\EAgent.exe` | ✗ 它也是 GUI 子系统 |

**真相**：`C:\Users\<local-user>\Desktop\EAgent.exe`（13.9 MB）是**我们程序的一份旧拷贝** ——
版本信息 `ProductName=EAgent / CompanyName=eagent`，含 `chat.deepseek.com` 等特征串，
构建于 **01:44:43**；而子系统修复提交于 `main.rs` **03:14:22**。
→ **它比修复早 1.5 小时，PE 子系统仍是 `3 (CONSOLE)`**，Windows 启动它时必然分配控制台窗口。

因果证据同族、差异点恰好就是控制该症状的那个属性：

| 文件 | Subsystem | 构建时间 |
|---|---|---|
| 桌面旧拷贝 | **3 = CONSOLE** | 01:44:43 |
| 当前构建 | **2 = GUI** | 05:03:08 |

### 修复（改在**产物层**，不是源码 —— 源码本来就是对的）

拷贝式分发会产生"用的还是旧东西"这一类问题。改成**快捷方式**：

1. 旧拷贝**改名保留**为 `EAgent.exe.旧版-console-20261017-0144.bak`（不删除）；
2. 桌面新建 `EAgent.lnk` 指向 `target\debug\eagent-app.exe`，`WorkingDirectory = D:\EAgent`。
   —— 这样既**永远是最新构建**，又能让配置发现走到 `D:\EAgent\config`（实测确认，见下）。

### Verified（运行时证据，不只靠静态推断）
- **启动前后可见控制台窗口数：1 → 1，无新增**（枚举 `ConsoleWindowClass` / Windows Terminal 窗口类）
- 应用子进程只有 `msedgewebview2.exe`，**无 `conhost`**（说明没有分配控制台）
- 通过**快捷方式**启动后，配置来源正确解析到运行时文件（非内嵌副本）：
  `selectors.json ← D:\EAgent\config\selectors.json ｜ agent.json (1432 字节) ｜ approval.json (1297 字节)`
- 会话存档与技能库为同一份（`app\data`），未因换启动方式分叉

### 给用户的建议（同类风险）
- **不要再把 exe 拷到桌面** —— 拷贝会随源码演进而陈旧，且从桌面启动时找不到 `D:\EAgent\config`（会退回内嵌配置）。
- 工作目录里的 `D:\EAgent\EAgent.exe`（2.3 MB）是**旧版易语言参考实现**，不是本客户端；
  它也是 GUI 子系统不弹黑窗，但功能与当前版本完全不同，注意别启动错。

## [阶段2-27] 2026-10-17 凌晨 —— 持久自动放行规则（「读法 A」，经用户明确同意后实施）

> 用户就"要不要让命令免确认自动执行"明确答复：**做**。
> 此前我连续 3 轮询问并承诺未获答复前不动它 —— 本轮在拿到明确同意后才实施。

### 定位与风险声明（这一条与前面的改动性质不同）

**这是整个项目里唯一一处会让命令不经用户确认就执行的功能。**
启用后用户**看不到这些命令**，只在审计页留下痕迹。因此实现上把安全边界写死在代码里，
**不依赖配置是否"听话"**。

### 安全设计（承诺的 4 条，全部落实）

| 承诺 | 落实方式 | 测试守护 |
|---|---|---|
| **默认空规则** | `agent.json` 的 `autoApprovePatterns` 默认 `[]`；空数组 = 功能完全不生效 | `agent_cfg` 默认断言 + 内嵌兜底副本为空断言 + 前端空规则恒 `ask` |
| **危险类永不入白名单** | 顺序固定在一个纯函数 `decideApproval()` 里：**危险判定永远最先**，危险一律 `ask` | **17 条危险命令 × 通配规则 `.*` × 三类全部信任 → 必须全部 `ask`**（逐条覆盖红名单） |
| **每次自动放行写审计** | `auditConfigAutoApproved()` 记录**命中的规则原文**，与会话信任分列标注 | 断言文案含规则原文、"用户未逐条看到该命令"、"跨重启持续生效" |
| **界面显式警告** | 状态条常驻**红色徽标**「⚠ 持久自动放行 N 条（你看不到这些命令）」，悬停列出全部规则；启用时另弹一次 danger toast | 真机双向实拍（关闭无徽标 / 启用出现徽标） |

### Added
- **`config/agent.json`**：新增 `autoApprovePatterns`（正则数组，默认空）+ 大段自述注释
  （写清风险、三条硬性边界与启用示例）。
- **`config.rs`**：`AgentCfg.auto_approve_patterns`，只收非空字符串，缺字段回落空数组。
- **`auto_approve_rules` IPC 命令**。
- **`src/shell/autoApprove.ts`**：规则编译（非法项跳过并计数）、
  `matchAutoApprove()`、以及**把顺序固定死的纯函数 `decideApproval()`** ——
  抽成纯函数的唯一目的就是让"危险类永不自动放行"这条属性**可被测试穷举**。
- **`audit.ts`**：`configAutoApprovedMessage()` —— 与会话信任分列，注明**跨重启持续生效**。
- **`StatusBar`**：新增持久自动放行警告徽标（`.autoapprove-badge`）。

### 顺序契约（不可颠倒）

```
危险类 → 一律 ask（无论会话信任怎么设、配置规则怎么写）
会话信任命中 → auto-session（本会话内，随会话消失）
配置规则命中 → auto-config（持久，跨重启生效）
否则 → ask
```

### Verified
- **前端 219/219**（新增 57 条：默认不生效、**危险类穷举不可放行**、决策顺序、
  规则容错"全非法 = 关闭而非放行一切"、审计文案；另含既有 162 条）
- **Rust 59/59**（新增 4 条：默认空、内嵌副本为空、规则清洗、缺字段回落）
- `tsc && vite build` 全绿
- **真机双向实测**（同一二进制，只改配置）：
  - 空规则 → 右上角**无**徽标，功能不生效（`baseline/ui_autoapprove_off.png`）
  - 1 条规则 → 右上角出现**红色警告徽标**（`baseline/ui_autoapprove_on.png`）
  - **验证完已把配置恢复为空**，用户机器回到安全状态（`baseline/ui_autoapprove_restored.png`），
    且会话存档与 164 项技能库完好

### 仍未验证到的一环（如实说明）
自动放行的**审计留痕**与**实际放行行为**需要一次真实工具调用才能观察；
本轮未做该交互（需要会话 + 模型产出工具调用）。该路径由纯函数穷举测试 +
编译期命令注册 + 真机 UI 双向实测覆盖，**但不等于端到端跑通过**。

## [阶段2-26] 2026-10-17 凌晨 —— 审计日志增长可见化

### 为什么

阶段2-25 让审批留痕看得见了，但顺带发现一件**未做处理**的事：`audit.log` **无自动清理**，
实测用户环境已 **3202 行 / 158.8 KB**，会一直长下去。

**"要不要自动清理旧审计记录"是一个会影响审计完整性的产品决定 —— 我不擅自实现。**
理由：审计留痕的价值在于"事后可追责"，自动删旧记录会**静默销毁证据**；
这与我一路坚持的原则（危险类永不自动放行、审计不静默丢内容）是同一类事。

因此本轮只做**明确安全**的那一半：**让增长可见**，把决定权留给用户。

### Added
- **`log::format_bytes()`（纯函数）** + `audit_log_size()` / `audit_log_size_human()`：
  人类可读的字节数（B / KB / MB，含边界处理）。
- **`audit_log_path_info` 命令扩展**：由"只返回路径"改为
  `「<路径> · 当前 158.8 KB · 无自动清理」` —— 让用户在审计页直接看到文件在长。
- 审计页文案相应调整（「路径：」→「日志：」，因为返回的不再只是路径）。

### Verified
- **Rust 56/56**（新增 2 条 `format_bytes` 用例：B/KB/MB 换算与四舍五入、
  **KB→MB 边界**（差 1 字节仍为 KB））
- **前端 162/162**；`tsc && vite build` 全绿
- **真机**：应用正常启动，**用户会话存档与技能库完好**；
  实测 `audit.log` 为 **158.8 KB**，界面现在会显示该数字

## [阶段2-25] 2026-10-17 凌晨 —— 让审批留痕**看得见**（补完「逐条审计」）

### 为什么

阶段2-23 已经把**审批来源**写进磁盘 `audit.log`（人工允许 / 会话信任自动放行 / 信任集变更），
但发现只做了一半：**用户在界面上看不到它**。审计留痕如果没人能看见，价值就大打折扣 ——
"能追责"的前提是"能查阅"。

### Added
- **后端 `log::read_audit_tail()` + `read_audit_log` / `audit_log_path_info` 命令**（**只读**）：
  读取磁盘审计日志尾部供界面展示。
- **审计页新增「磁盘审计日志」区块**：显示留痕尾部、路径、刷新与复制；
  默认开启**「只看审批来源」**开关 —— 因为审计日志是混合内容（命令 / 执行结果 / 留痕同文件），
  实测用户环境已 **3202 行 / 162KB**，直接看尾部很难找到关键几行。
  切到「显示全部内容」可看原始尾部。

### 过程中发现并修掉的两个自身问题
1. **注释与实现不一致**：`read_audit_tail` 的注释写着"绝不整文件读入内存"，
   实现却是 `read_to_string` **整读**。审计日志**无上限增长**（已 162KB），这是真实隐患。
   → 改为**从文件末尾 `seek` 读取**（只读末尾 `max_bytes` 窗口），
   并处理两个边界：**切在 UTF-8 字符中间**（lossy 兜底）与
   **切出半行**（丢掉首个不完整行）。
2. **界面文案一开始不准确**：区块原本叫「磁盘审计留痕（审批来源）」，
   但真实日志尾部是工具执行输出。→ 改名「磁盘审计日志」并如实说明是混合内容。

### Verified
- **Rust 54/54**（新增 10 条：尾部行数/字节双上限与显式截断提示、
  **中文按字节截尾不 panic**、空输入、无尾换行、丢弃半行、
  以及一条**真实临时文件端到端读尾**用例 —— 断言起点为文件头时不丢首行、
  尾部读取必含最后一行、且不残留 UTF-8 替换字符）
- **前端 162/162**（新增 8 条 `pickProvenance` 用例：只留留痕行、人工与自动放行都可对照、
  工具输出被剔除、保持原有行序、无留痕返回空串）
- **真机**：应用正常启动，**用户会话存档与技能库完好**；
  实测尾部窗口对真实日志充分（最后 400 行仅 21KB，上限 64KB）
- **诚实说明**：当前 `audit.log` 里「审批来源」留痕为 **0 行** ——
  自该功能上线后用户尚未触发过审批。因此**该路径仍未在实机端到端跑通**，
  目前由纯函数单测 + 真实临时文件读尾测试 + 编译期命令注册三重覆盖。
  你在审批卡上点一次「允许」或「本会话内此类全部允许」，留痕就会出现在审计页。

## [阶段2-24] 2026-10-17 凌晨 —— 修掉「claude 徽标错位」：一个隐藏已久的 CSS 类名撞车

> 用户反馈原文：「这里的 claude 有点错位」（附技能页截图）。

### 复现与定位
用户 138 个技能带 `source: "claude"`，界面把来源渲染成徽标。截图里该徽标与相邻的
「许可：未知」**视觉重叠**。

排查路径（每一步都在否定前一个假设）：
1. 先查 CSS —— `.skill-tags` 是 flex + gap，**理论上不可能重叠**；
2. 全表扫 `position: absolute` / 负 margin —— **没有一条落在徽标上**；
3. 于是搭了一个**只引用项目真实 `tokens.css` / `shell.css` 的对照页**，用 Edge 打开截图，
   并给每个徽标描出**真实盒子的品红描边**、放大 4 倍 —— 真相立刻显形：
   `claude` 徽标被压成了一个**又窄又高的方格**，文字从框里溢出来压到邻居上。

**根因：CSS 类名撞车。** 徽标变体写的是 `badge brand`，而 `.brand` 另有一条
**状态条 logo「E」**的规则（`width: 26px; height: 26px; display: grid; place-items: center`
+ 渐变底 + 白色粗体）。两者**同优先级（各 1 个类）**，`.brand` 在样式表里更靠后 →
**按层叠规则胜出**，把徽标压成 26×26 方格。

### 修复
- 徽标变体改名 **`.badge.accent`**，与 logo 的 `.brand` 彻底解耦（原 `.badge.brand` 规则删除并留注释说明原因）；
- `.badge` 补 **`flex: 0 0 auto`** —— 徽标不应被 flex 容器压窄（这是同类问题的第二道防线）；
- `.skill-tags` 间距 `--s-1`(4px) → `--s-2`(8px)，相邻浅底色块不再糊成一片；
- 来源徽标新增 **`.badge.source`** 中性描边样式，避免与「许可」的琥珀色块粘连；
- 徽标文案 `claude` → **`来源：claude`**（裸文本看不出是来源还是分类；插件来源自带前缀则不重复加）。

### ★ 受影响的**不止**用户看到的那一处
同一撞车在别处也一直存在（此前无人发现）：
| 位置 | 表现 |
|---|---|
| `SkillsPage` 「自建」徽标 | 被压成方格 |
| `HistoryPage` 「进行中」徽标 | 被压成方格 |
| `AuditPage` tool 阶段的阶段徽标 | 被压成方格 |

### 回归防护
新增 **`__tests__/cssClassCollision.test.ts`**：扫描全部源码，**禁止任何元素同时挂 `badge` 与 `brand`**，
并断言样式表里 `.badge.brand` 已不存在、`.badge` 带 `flex: 0 0 auto`、`.brand` 仍保留 logo 语义。

**这条守卫上线即立功**：它立刻抓出一处**人工 grep 漏掉的**三元表达式写法
（`AuditPage.tsx` 的 `l.phase === "tool" ? "brand" : ...`）—— 证明扫描式守卫比肉眼可靠。

### Verified
- **前端 154/154**（新增 6 条：类名撞车守卫 6 + 来源徽标文案 6，另含既有 142 条）
- **Rust 44/44**；`tsc && vite build` 全绿（tsconfig 补 `types: ["node"]` 以支持 fs 型测试）
- **视觉决定性验证** `baseline/ui_badge_rootcause.png`：**同一段文字 `claude`，只换类名** ——
  ① `badge brand` 被压成方格、文字溢出压到相邻徽标；② `badge accent` 完全正常。
- 真机重启后**用户会话存档与 164 项技能库均完好**；残留撞车类名 **0** 处。

## [阶段2-23] 2026-10-17 凌晨 —— 审批来源可审计 + 自动放行面可见可撤销

### 这一条解决的是我自己此前标注的真实缺口

我在阶段2-11 加过「本会话内此类命令全部允许」（会话信任自动放行），并在报告里标它为 HIGH 风险。
本轮盘代码时确认缺口比原先描述的更具体：

> **会话信任的自动放行只写进前端内存日志（`logs` state），完全没进磁盘 `audit.log`。**
> 而引擎侧只看到"被批准了" —— 因此 `audit.log` **无法区分「人工点击允许」与「会话信任自动放行」**。
> 审计链最该留痕的一处，恰恰没有留痕。

另外确认：该自动放行面**只能开启、不能撤销，且界面上完全看不见**。

### Added
- **后端 `log_audit` IPC 命令 + `log::prepare_audit_text()`**：
  前端把**审批来源**写入磁盘审计。输入关口做了纯函数化（去空白 / 拒绝空内容 /
  按**字符**上限 1000 截断并显式标注"已截断"，不静默丢内容）。
- **前端 `src/shell/audit.ts`**：文案构造（纯函数，可单测）与写盘分离，提供三类来源标注：
  - `autoApprovedMessage()` —— **明确写出「非人工逐条确认」与「危险类永不进入此路径」**；
  - `manualDecisionMessage()` —— 人工允许/拒绝；
  - `trustChangedMessage()` —— 信任集开启 / 撤销 / 清空。
  **写盘尽力而为**：审计失败绝不影响审批与执行主流程。
- **状态条新增「自动放行」徽标**：显示当前被信任的类别，**点击即清空并恢复逐条询问**。
  自动放行面从"隐形且不可逆"变为"可见且随时可撤销"。
- 急停（Ctrl+.）也会留痕一条来源记录。

### 明确未做（等用户拍板）
「让命令**免确认自动执行**并配到 `agent.json`」（即"审批白名单"的另一种读法）**本轮仍未做** ——
那会扩大自动执行面、并让用户再也看不到将要执行的命令。
本轮的改动**一点也没放宽**放行条件：审批门一道不减，只是把"已经存在的自动放行"变成了可审计、可撤销的。

### Verified
- **Rust 44/44**（新增 6 条 `prepare_audit_text` 用例：拒绝空/空白、去空白、
  恰好等于上限不报截断、超限必须显式标注、按字符而非字节计数、多行来源文案原样保留）
- **前端 142/142**（新增 12 条审计文案用例，其中一条专门钉住
  **★「自动放行」与「人工确认」在文本上必须可区分且互不串味**）
- **真机**：`tsc && vite build` 全绿；产物含 `log_audit` / `审批来源` / `信任集变更` / `自动放行`；
  启动正常，**用户既有会话存档与 165 项技能库均完好**
- 诚实说明：**实机的"自动放行"审计行需要交互式会话触发**（点开审批卡选「全部允许」），
  本轮未做该交互触发；该路径由纯函数单测 + 编译期命令注册 + 产物字符串三重覆盖。

## [阶段2-22] 2026-10-17 凌晨 —— 审批风险规则配置化（"审批白名单"的安全版）

### 先讲清楚做了什么、**以及刻意没做什么**

用户提的后续项之一是"审批白名单配置化"。这里有两种读法，**安全性差别很大**：

| 读法 | 含义 | 我的处置 |
|---|---|---|
| A. 让命令**免确认自动执行** | 引擎侧绕过审批门 | ❌ **刻意不做** —— 这会扩大自动执行面、让用户再也看不到将要执行的命令。需要用户明确点头才谈 |
| B. 把**风险规则**变成配置 | 只改"分级"，不改"是否放行" | ✅ **做了这个** |

B 还是 `constants.ts` 里 DANGER 列表**自己写的注释**要求的：
> `/* 危险红名单（策划案 §5.1 R6 初版，需人工复核后固化到配置） */`

### Added
- **`config/approval.json`（新）**：`danger[]`（危险模式，逐条正则）+ `readonly`（只读动词）。
  运行时读取 → **改完下一回合即生效，无需重新编译**；已纳入启动配置体检。
- **`config::approval_json()`** + **`approval_rules` IPC 命令**：把规则原文交给前端编译。
- **`format.ts: setRiskRules()`**：用配置覆盖分级规则，返回 `RiskRulesReport`
  （采用的危险条数 / 是否自定义只读 / 跳过的非法项 / 是否回退内置）。

### ★ 安全兜底（本条目最重要的部分）
**若自定义危险列表为空或全部非法 → 回退内置列表并把 `dangerFallback` 置真。**
理由：danger 分级一旦整体失效，两条保护会跟着失效 ——
①"危险类不提供「全部允许」"；②"危险类永不进入会话信任"。
宁可回到内置规则，也**不能静默地把危险分级关掉**。前端会弹告警告知用户。

同时明确：本机制**只影响分级，不影响放行** —— 审批门一道不减，所有工具照样需要用户确认。
测试里专门有一条把这个意图写成断言（返回值里没有任何"自动放行"能力）。

### Verified
- **前端 130/130**（新增 11 条 setRiskRules 用例：自定义生效 / 清空回退 / 全非法回退 /
  部分非法 / 非字符串跳过 / 只读非法回退 / 非对象不抛异常 / 重置 / 意图守卫）
- **Rust 38/38**（config 测试新增 approval.json 检查）
- **真机**：`boot: config 来源 → … | approval.json ← D:\EAgent\config\approval.json (1297 字节)`

## [阶段2-21] 2026-10-17 凌晨 —— M-01 工具抽象（可扩展性报告 v2 关键路径起点）

### 背景
原状：「模型 → 工具」协议被**写死**成一个 PowerShell 代码块 + 一对魔法注释标记，
工具种类只能是 1 种，插件也无从贡献能力（我在阶段2-18 的 `docs/13` 里如实标注过这条边界）。

### Added
- **后端 `src/tools.rs`（新模块）**：`ToolSpec` 工具注册表。
  - 每个工具自带「名字 / 说明 / 标记对 / 执行方式 / 来源」；
  - 内置 `powershell` 工具**逐字沿用原协议**（标记、解析、执行全部不变）；
  - 插件可贡献 `PluginTemplate` 型工具：一段带 `{{arg}}` 占位的命令模板；
  - `parse_call()` **内置优先**（插件工具永不抢占 PowerShell 协议）；
  - 未知工具名 → **拒绝执行、不猜测**，且不消耗一轮审批。
- **`plugins.rs` 扩展**：清单新增 `tools[]`（`name` / `title` / `description` / `command`）；
  `valid_tools()` 过滤掉名字或命令为空的半成品条目（避免"看得见但一调用就执行空命令"）；
  `scan()` 返回结构体 `ScanResult`（plugins / skills / tools 一次扫完）。
- **提示词注入**：`extra_prompt_block()` 生成插件工具说明（含调用格式与命令模板），
  会话初始化时**追加**到提示词末尾。
- **可观测**：启动日志新增 `boot: 工具注册表 → …`；技能页插件状态条新增工具计数。

### ★ 最关键的设计取舍：零风险增量
**没有任何插件工具时，`extra_prompt_block()` 返回空串**，提示词与调用协议与升级前逐字一致 ——
因此这次抽象对**当前可用的回路零影响**。这条性质有单测守护，并在真机上双向实测（见下）。

### 安全性（未放宽任何东西）
插件工具的执行**最终仍落到同一条 PowerShell 通道**，因此**同样经过审批门**；
审批卡展示的是**代入参数后的完整命令**（所见即所执行）。
`render()` **刻意不做任何转义** —— 若偷偷改写会造成"用户看到的 ≠ 实际执行的"，
那比不转义更危险。本模块**没有引入任何绕过审批的路径**。

### Verified
- **Rust 单测 38/38 通过**（新增 12 个 tools 测试 + 5 个 plugins 测试）
- **真机双向实测**（同一二进制，只改插件目录）：
  - 放入含 2 工具的插件 → `boot: 工具注册表 → 共 3 个（内置 powershell + 插件 2）；附加说明 669 字符：[say-hi←插件：demo-toolkit, file-count←插件：demo-toolkit]`
  - 移除插件后重启 → `共 1 个（内置 powershell + 插件 0）；附加说明 0 字符：[无（协议与升级前逐字一致）]`
- 去重：删除 `agent.rs` 中已无生产调用的 `parse_powershell_tool` 与 lib.rs 的重复协议常量/测试
  （等价覆盖已由 `tools::tests` 提供，且更全），`cargo test` 零警告。

## [阶段2-20] 2026-10-17 凌晨 —— 前端测试基线（可扩展性报告 v2 · S-04）

### Added
- **vitest 5.0.5**（devDependency）+ `npm test` / `npm run test:watch` 脚本。
- **`app/src/shell/__tests__/format.test.ts`（90 用例）**：重点覆盖**安全关键**的 `classify` 风险分级器 ——
  - 只读 / 普通 / 危险三类各自的样本；
  - **16 条危险红名单逐条覆盖**；
  - 大小写不敏感；
  - **不变式**：同时像只读又踩红名单时必须判 danger（危险优先级最高）；
  - 另有 `esc` / `highlight` / `fmt` / `mmss` / `baseName` / `summarizeCode`，以及
    constants 自洽性（PHASE_TEXT 与 DOT_STATE 覆盖同一组阶段、BUSY 均有定义、SEND_LIMIT=4000）。
- **`app/src/shell/__tests__/markdown.test.tsx`（29 用例）**：
  块级解析（标题/段落/围栏/列表/引用/分割线/表格/CRLF）+ **安全属性**
  （脚本标签、`onerror`、行内 HTML 一律转义为惰性文本；非 http(s) 协议不产出链接）。

### 测试当场揪出的两个真实缺陷（均已修）
1. **单列表格分隔行不识别**：`RE_TBL_SEP` 用了 `+`（要求至少两组 `|…|`），
   导致 `| A |` + `|---|` 被降级成段落。改为 `*`。
   安全性：分割线在表格判断**之前**处理，`---` 不会被误当表头分隔（已加测试守护）。
2. **非 http(s) 链接仍写入原始 href**：原实现把 `javascript:` 原样写进 `href`，靠 React 兜底拦截。
   改为**纵深防御** —— 渲染器自己只对 http(s) 产出 `<a>`，其余留作惰性文本。
   顺带修掉"看着可点、点了没反应"的假链接。

### 死代码清理
`constants.ts` 的 `SKILLS` / `LICENSED` 在技能数据化（阶段2-16）之后**已无任何消费者**
（区分大小写全仓检索确认只剩定义本身）→ 删除 33 行，`constants.ts` 89 → 56 行。
单一事实来源应为 Rust 侧 `skills.rs` 的内置清单。

### Verified
- `npm test` → **119 passed**；`npm run build`（`tsc && vite build`）→ 全绿。
- 过程中我自己的断言写错了 4 处（段落软换行用空格、孤立 `\r` 被规整、标题层级刻意下移、
  `onerror=` 仍在转义文本里）—— 均按**实现的实际行为**修正，未改动实现去迁就测试。

## [阶段2-19] 2026-10-17 凌晨 —— 前端拆分完成（S-01 收官）：Shell.tsx 1538 → 487 行【架构升级】

> 目标：`Shell.tsx` < 500 行并拆出 Composer / MessageList / StatusBar / LeftRail / 内容页 / Hook。
> **达成**：1538 → **487 行**（-68%），抽出 **14 个组件 + 8 个 Hook**。

### Added（本轮新增抽出）
- **内容页**：`AuditPage` / `HistoryPage` / `SettingsPage`（筛选状态与派生行随组件内聚，不再挂在 Shell 上）
- **会话流**：`ChatHeader` / `MessageList` / `DiagDrawer` / `Composer`
  - `MessageList` 用 **children** 承接审批卡与已决策命令 → 容器装配，避免把 risk/approvalOptions 等状态灌进去
  - 输入框自动增高移入 `Composer`（它才是 textarea 的所有者，容器不再持有只为此用的 ref）
- **状态条 / 左栏**：`StatusBar` / `LeftRail`（`NavItem` 类型提升到 `types.ts`）
- **Hook 层（8 个）**：`useKeyboard`（111 行）/ `useShellActions`（193）/ `useEngineEvents`（82）/
  `useSessionPersistence`（71）/ `useBrowserRect`（53）/ `useAppearance`（40）/ `useNarrow`（32）/ `useElapsed`（26）

### 顺带修正的过时文案（代码即文档，读到就改）
- 历史页：原写「会话持久化在 M2 落地」→ **S-02 已落地**，改为如实说明存档位置与原子写/备份机制
- 设置页「完成判定」：原写「主信号 = 发送键状态（stop→arrow）」→ 网络层早已接管，
  改为「主信号 = SSE loadingFinished；读回 = Network.getResponseBody；兜底 = DOM 文本稳定 + 回声守卫」
- 设置页「允许自动执行」：原写「无自动批准旁路」→ 与实际不符，改为说明会话内按类信任 + 危险类永不自动放行

### Verified
- `tsc && vite build` 全绿（48 模块）；`cargo test` **25/25**；`cargo build` 通过
- **真机渲染验证** `baseline/ui_s01_final.png`：状态条 / 左栏 / 导航徽标 / 空态全部正常
- **IPC 通路验证**：启动后 `session.json`（持久化 Hook）与 `skills.json`（技能 Hook）均由前端调用生成，
  证明重构后的 `invoke` 链路完好

### 过程中的一次自伤与修复（如实记录）
第 8 步脚本的**切片边界算错**（`$src[0..71]` 只到第 72 行，漏掉了第 73 行的 `view` 状态；
同时多跳过了 116–155 的持久化/pin/矩形 Hook 三块），写出了一个**编译不过的 582 行版本**。
因前置的边界断言只覆盖了部分行号，脚本没能拦住。修复方式：按编译器报错逐条定位
（`view` 缺失、`narrow` 重复声明、`pin`/`pushRect`/`useSessionPersistence` 缺失），
**手工补齐三块 Hook 调用**并删除重复声明。教训：切片脚本的断言应覆盖**每一个**被跳过或被保留的边界，
而不只是首尾。

## [阶段2-18] 2026-10-17 凌晨 —— 最小可行插件系统（可扩展性报告 v2 · L-01 第一步）【用户要求】

> 用户问："插件功能呢，加不了吗"。
> 诚实前提：报告把**完整**插件系统（沙箱/签名/市场）列为 3–6 个月的高难度项，依赖 M-01 工具抽象 + M-02 MCP 客户端。
> 本轮**不假装做到那些**，只落地一个「插件能贡献能力」的最小闭环，且清单格式为后续扩展预留。

### Added
- **后端 `src/plugins.rs`（新模块）**：扫描 `<data_dir>/plugins/<目录>/plugin.json`。
  - 清单字段：`name`（**必填**）/ `version` / `description` / `skills[]`；
  - **故障隔离**：逐个插件独立解析，一个坏掉不拖垮其他；
    错误在界面上按**目录**单独列出（"缺少 plugin.json" / "缺少 name" / "解析失败：…"）；
  - `ensure_dir()` 首次调用即创建插件目录，用户有明确位置放插件。
- **`skills::load_all()`**：完整技能视图 = 本地 `skills.json` **+** 插件贡献。
  - 插件技能标注 `source: "插件：<名>"`，界面显示徽标、**不可从本地库删除**；
  - **同名以本地为准** —— 用户可显式覆盖插件提供的技能；
  - 增删只作用于本地库，插件技能绝不写进 `skills.json`。
- **2 个 Tauri 命令**：`list_plugins` / `plugin_dir_info`。
- **前端**：技能页新增插件状态条（`插件 N 个（贡献技能 M 项）· K 个加载失败` + 插件目录路径），
  加载失败的插件单独一条告警；技能卡片对插件来源显示徽标并隐藏删除按钮。
- **`docs/13-plugin-and-skills.md`（新）**：技能机制 + 插件格式 + **已知边界**的完整说明。

### Verified
- **单元测试 25/25 通过**（累计）：新增 `plugins.rs` 3 个（清单解析容错 / name 必填 / 完整往返）。
- **端到端（真机）**：放入 1 个正常插件（2 技能）+ 1 个损坏插件 → 重启后
  - 左栏「技能」徽标 **26 → 28**；
  - 页头「共 28 项（内置 26 · 自建 2）」；
  - 分类标签新增插件带来的 **演示**；
  - 状态条「插件 2 个（贡献技能 2 项）· 1 个加载失败」；
  - 损坏插件单独报错并显示其目录。
  证据：`baseline/ui_plugin_skills.png`。验证后已清理演示插件。

### 诚实标注的边界（详见 docs/13）
1. 插件**只能贡献技能**，不能注册工具/注入提示词片段 —— 需先有 M-01 工具抽象；
2. **无沙箱无权限**：当前清单是纯数据（不含可执行代码），风险面小；
   一旦支持 `tools`/`mcp`，**必须**同步设计权限声明与审批粒度；
3. 无安装/签名/更新流程，靠用户手工放置文件夹；
4. 技能是"告诉模型有哪些能力"，**不等于**自动加载能力实现。

## [阶段2-17] 2026-10-17 凌晨 —— 会话持久化（可扩展性报告 v2 · S-02）【用户要求】

> 用户要求："ab都做吧" —— 这是其中的 **B（S-02 会话持久化）**。

### Added
- **后端 `src/store.rs`（新模块）**：会话快照落盘 `<data_dir>/session.json`。
  - **原子写**：先写 `session.json.tmp` 再 `rename` —— 掉电/崩溃不会留下半截文件；
  - **写前备份**：覆盖前把上一版留成 `session.bak.json`；
  - **读时回退**：主文件损坏/非法 JSON → 自动回退备份；两者都不可用 → 返回空串走初始状态；
  - **写前校验**：只接受合法 JSON，绝不让损坏内容固化到磁盘；
  - `save` / `load` / `clear`（clear 供「开新会话」用）。
- **4 个 Tauri 命令**：`save_session` / `load_session` / `clear_session` / `session_path_info`。
- **前端接入（Shell.tsx）**：启动时 `load_session` 恢复消息流 / 已决策工具 / 工作目录；
  状态变更后 **800ms 防抖自动落盘**（避免每条引擎事件都写盘）。恢复成功会弹提示告知条数。

### Verified
- **单元测试 22/22 通过**（新增 3 个）：非法 JSON 被拒、原子写 + 备份语义、主文件损坏回退备份。
- **端到端（真机）**：
  1. 启动前无 `session.json` → 启动后自动生成，字段 `version/workdir/messages/doneTools/savedAt` 完整；
  2. 写入含消息的存档 → 重启 → **界面恢复「1 个用户回合 · 2 条消息」**，
     助手消息的 Markdown（标题 / 粗体 / 有序列表 / 表格）全部复原 ——
     证据 `baseline/ui_session_restore.png`。
- 验证后已清理测试存档，用户下次启动是干净状态。

## [阶段2-16] 2026-10-17 凌晨 —— 技能数据化：可增删 + 真注入模型【用户反馈】

> 用户反馈原文："技能这里，现在不能加入，只能调用原来有的。"

### 根因（两个）
1. **技能是前端硬编码**：26 项写死在 `constants.ts` 的 `SKILLS` 数组里 —— 只能看，不能加。
2. **技能对模型根本不可见**：提示词只提了一句 `skill_all.zip`，而引擎**从不生成这个 zip**，
   也没把技能清单喂给模型。所以即便能加，模型也用不到 —— 技能此前实质是 UI 摆设。

### Added
- **后端 `src/skills.rs`（新模块）**：技能库数据化，落盘 `<data_dir>/skills.json`。
  - 首次运行从内置清单**播种**；文件损坏/为空自动回退播种并修复文件；
  - `list` / `add` / `remove` / `restore_builtin` / `prompt_block`；
  - 校验：名称去空白、大小写不敏感去重、名/描述非空；分类留空归入「自定义」；
  - 内部实现**路径可注入**（`*_at(path, …)`），因此测试完全隔离、不污染真实数据目录。
- **4 个 Tauri 命令**：`list_skills` / `add_skill` / `remove_skill` / `restore_builtin_skills`
  （增删均返回新列表，前端一次往返即可刷新）。
- **技能真注入**：会话初始化时把技能清单写入 `<tmp>/skills.md`，并替换提示词模板新增的
  **`{{SKILLS}}`** 占位符 —— 模型这才真正"看得见"技能（含用户新增的）。
- **前端 `components/SkillsPage.tsx`（新，拆分自 Shell.tsx）**：数据驱动技能页。
  - 「添加技能」内联表单（名/描述/分类）、卡片悬停出现删除按钮（二次确认）、「恢复内置」；
  - 分类 chips **由实际数据推导**（实测从旧硬编码 7 个变为 11 个真实分类）；
  - 内置/自建徽标区分；左栏导航徽标改为读取后端技能数。

### Verified
- **单元测试 19/19 通过**（新增 4 个）：内置清单良构与去重、**增删改查+落盘+提示词注入完整往返**、
  **损坏文件回退播种并自修复**。
- **端到端**：启动前 `app/data/skills.json` 不存在 → 启动后自动生成 **26 条**，`licensed` / `builtin` 字段完整
  （证明前端 `list_skills` → 后端播种 → 落盘链路打通）。
- 界面实拍 `baseline/ui_skills_data_driven.png`（技能页正常渲染，分类由数据推导）。
- `tsc && vite build` 全绿（31 模块，CSS 38.05KB / JS gzip 86.32KB）。

### 附带（S-01 进度）
`renderPage` 的技能分支抽出为独立组件；`Shell.tsx` 1259 → 1233 行（页内新增的 skillCount 状态抵消了部分）。
S-01 余量：Composer / MessageList / StatusBar / LeftRail / 其余三个内容页 / 三个 Hook。

## [阶段2-15] 2026-10-17 凌晨 —— 前端拆分（可扩展性报告 v2 · S-01 第一批）【架构升级】

> 依据：`C:\Users\<local-user>\Desktop\报表\EAgent可扩展性分析与架构优化报告.md` 的 **S-01（前端 Shell.tsx 拆分）**。
> 该报告把 S-01 列为「低成本高收益的 immediate action」，且是「关键路径 S-01 → M-01 → M-02 → L-01」的起点。

### Changed
**`Shell.tsx`：1538 行 → 1259 行**（本轮 -279 行）。抽出 5 个模块：

| 新文件 | 行数 | 职责 |
|---|---|---|
| `shell/types.ts` | 10 | 9 个共享类型（Msg / ToolReq / DoneTool / Risk / View…） |
| `shell/constants.ts` | 89 | PHASE_TEXT / DOT_STATE / BUSY / SEND_LIMIT / **DANGER 危险红名单** / READONLY / SKILLS / LICENSED / KEYS |
| `shell/format.ts` | 69 | `classify` 风险分级、`highlight` 语法高亮、`fmt`/`hm`/`mmss`、`lsGet`/`lsSet`、`baseName`、`summarizeCode` |
| `shell/components/ApprovalCard.tsx` | 80 | 审批卡（DSH/Claude 式内联编号选项）——纯展示，交互回调给容器 |
| `shell/components/ToolGroup.tsx` | 111 | 已决策命令的折叠汇总行 + 明细 —— 纯展示 |

拆分方式：按行区间**脚本化抽取**（避免手工转写 1500 行出错）+ 为叶子模块补 `export`。
`ApprovalCard` 的键盘导航仍由 Shell 的全局键盘处理器驱动（组件不持有状态，`activeIdx` 由容器传入）。

### Verified
- `tsc && vite build` 全绿（30 模块，CSS 37.12KB / JS gzip 86.31KB）；gzip 与拆分前基本持平（86.20 → 86.31KB）。
- 产物含全部组件串（`已决策` / `审批选项` / `命令全文` / `toolgroup` / `appr-opts` 均命中）→ 组件确已进包。
- **真机渲染验证** `baseline/ui_s01_split.png`：状态条 / 左栏 / 空态正常，**无白屏、无 JS 错误**（重构最大风险点）。

### 未完成（S-01 余量）
`Shell.tsx` 仍有 1259 行、约 7 个职责。后续可继续抽出：
`Composer`（输入区）、`MessageList`（消息流）、`StatusBar`（顶部状态条）、`LeftRail`（左栏导航）、
`SkillsPage` / `AuditPage` / `SettingsPage`（内容页）、`DiagDrawer`（诊断抽屉），
以及 `useTheme` / `useEngineEvents` / `useKeyboard` 三个 Hook。

## [阶段2-14] 2026-10-17 凌晨 —— 按可扩展性报告 v2 执行 S1/S2/S3【架构升级】

> 依据：`C:\Users\<local-user>\Desktop\报表\EAgent可扩展性分析报告-优化版.md` 的短期项（S1 拆分 / S2 配置外置 / S3 参数外置）。
> 报告原文的两条关键发现：**配置被 `include_str!` 编译期固化**、**lib.rs 单文件承担 8 职责**。

### Added
- **`app/src-tauri/src/config.rs`（新）**：配置与路径的**运行时优先、内嵌兜底**加载器。
  - 查找顺序：`$EAGENT_CONFIG_DIR` → `<exe>/config` 及上溯 5 级 → `<cwd>/config` → **编译期内嵌副本**；
  - 每次调用重读磁盘 → **改配置下一回合即生效，无需重新编译**；
  - `config_source_report()`：报出每份配置**实际来自哪个文件**，专治"改了配置怎么没生效"。
- **`config/agent.json`（新）**：引擎行为参数外置——`toolTimeoutSec` / `toolOutputCap` /
  `maxToolIterations` / `replyEndpoint`（后者原为硬编码常量 `REPLY_ENDPOINT`）。
- **启动即报配置体检**：`boot: config 来源 → …` 与 `boot: agentCfg → …` 写入 `chain_test.log`。
- `selfcheck` 命令返回值新增 `configSources` 与 `agentCfg` 两个字段。

### Changed（S1：单文件拆分为模块）
`lib.rs` 由 **1761 行 → 378 行**，拆出 9 个模块（`use super::*` 继承根模块导入，条目提升为 `pub(crate)`）：

| 模块 | 行数 | 职责 |
|---|---|---|
| `net.rs` | 395 | CDP Network 事件订阅 + SSE 回复流解析 |
| `session.rs` | 402 | 注入 → 发送 → 多信号完成判定 → 读回 |
| `commands.rs` | 204 | Tauri IPC 命令 + 会话初始化 |
| `config.rs` | 203 | 配置/路径运行时加载 |
| `exec.rs` | 131 | PowerShell 执行（超时/上限/进程树回收） |
| `agent.rs` | 119 | 工具标记解析、审批门、自检 |
| `cdp.rs` | 68 | 进程内 CDP 直调基建 |
| `state.rs` | 58 | 共享状态（回合锁/审批通道/网络缓冲） |
| `selectors.rs` | 51 | 选择器配置加载 |
| `log.rs` | 30 | 日志与时间戳 |

### Verified
- **`cargo check` 零错误零告警**；**单元测试 15/15 通过**（12 原有 + 3 新增配置测试）。
- **「改配置免重编译」实测**：手改 `config/agent.json` 为 `timeout=77 / cap=3333 / iter=9`，
  **不做任何编译**直接重启，启动日志即输出
  `boot: agentCfg → timeout=77s cap=3333 iter=9` —— 特性成立（验证后已还原为 120/4000/6）。
- 真机运行：网络层订阅 7 事件就绪、选择器探测正常、启动日志显示配置来源为
  `D:\EAgent\config\*.json`（不再走内嵌副本）。

### 过程中发现并修正的两个自身缺陷
1. **配置查找少上溯一层**：exe 在 `app\src-tauri\target\debug`，而配置在**仓库根** `\config`，
   需上溯 **4 级**；初版只写了 3 级，启动日志明确报"未找到运行时文件"→ 修正为 5 级（留余量）。
2. 拆分脚本在 **PowerShell 5.1** 下因 UTF-8 无 BOM 被按 ANSI 误读而解析失败
   （与 README「新增 .ps1 必须 UTF-8 with BOM」的既有约定一致）→ 改用 PowerShell 7 执行。

## [阶段2-13] 2026-10-17 凌晨 —— 启动不再弹出黑色命令窗【用户反馈】

> 用户反馈原文："能不能在启动程序之后不会弹出命令窗？"

### Fixed
- **根因**：`src/main.rs` 的子系统开关写的是模板默认的 `not(debug_assertions)`——
  **debug 构建因此始终带一个控制台窗口**（双击启动即弹出黑窗）；只有 release 才隐藏。
- **改法**：改为 `#![cfg_attr(all(windows, not(feature = "console")), windows_subsystem = "windows")]`，
  即**默认始终走窗口子系统**；同时在 `Cargo.toml` 增加空 feature `console` 作为排障逃生口
  （`cargo build --features console` 可恢复控制台）。
- 日志能力不受影响：引擎本就落文件（`app\data\chain_test.log` / `audit.log`）+ UI 有「审计」页；
  以 `-RedirectStandardOutput` 启动时 stdout 重定向照常写入。

### Verified（含一次自我纠错）
- 首次改完**看似成功但实际无效**：编辑时误把换行删掉，属性被吞进注释行尾，编译照过、黑窗照旧。
  改用 **PE 头子系统字段**做确定性判定后暴露——`Subsystem = 3 (WINDOWS_CUI)`。
- 修正后复测：**`PE Subsystem = 2 (WINDOWS_GUI)`**；启动后枚举该进程全部顶层窗口，
  **无 `ConsoleWindowClass`**；主窗口 1295×877 正常；`run15_stdout.log` 仍写入 473 字节。
- 教训：这类"开关是否真的生效"必须看产物（PE 头/窗口枚举），不能凭编译通过推断。

## [阶段2-12] 2026-10-17 凌晨 —— 助手输出 Markdown 渲染 + 消息一左一右【用户反馈】

> 用户反馈原文："这个输出好难看，你可以弄成Claude那种输出吗，还有对话放一左一右。"
> 症状：助手回复里的 `##`、`**粗体**`、`| 表格 |`、`- 列表` 全是**原始符号**挤成一段；用户与 AI 消息全部左对齐。

### Added
- **新模块 `app/src/shell/Markdown.tsx`**：轻量 Markdown 渲染器（Claude 风格消息排版）。
  - 覆盖：标题 `#`–`######`、段落、**粗体**、*斜体*、行内 `` `代码` ``、围栏代码块、
    有序/无序列表、引用、分割线、**表格**、链接。
  - **零新依赖**（延续项目"图标都自绘"的取舍）；**不使用 `dangerouslySetInnerHTML`**，
    全部产出 React 元素 —— 模型输出属不可信内容，XSS 面必须为零。
  - 链接仅放行 `http(s)`，经 `@tauri-apps/plugin-opener` 交给系统浏览器打开；
    `javascript:` / `data:` 等一律忽略。

### Changed
- **消息一左一右**：用户消息整行反向（头像 + 气泡靠右，气泡按内容收缩，上限 `min(78%, 560px)`）；
  助手消息保持左侧通栏。用户气泡内时间戳右对齐。
- 助手回复走 Markdown 渲染；**用户消息仍按原文预格式显示**（避免用户输入里的符号被误解析）。
- `.md-*` 样式族（标题/列表/表格/代码/引用/链接），表格可横向滚动、代码块最高 380px。

### Verified
- `tsc && vite build` 全绿（CSS 37.12KB / JS gzip 86.17KB）；`cargo build` + 重启通过。
- 对比图 `baseline/ui_markdown_layout.png`（用真实 tokens.css/shell.css 渲染，
  同屏展示"纯文本全左对齐" → "Markdown + 一左一右"）。

## [阶段2-11] 2026-10-17 凌晨 —— 审批交互改为 DSH / Claude 式「内联编号选项」【用户反馈】

> 用户要求原文："你可以把审核的方式改成dsh或者claude那种。"

### Changed
- **审批卡重做为单一内联选项卡**，替代原先「危险卡 + 工具卡 + 底部按钮」三处散落结构：
  - 顶部：工具名 + 风险徽标（只读/普通/危险）+ `#id` + 字符计数；
  - 中部：命令预览（可滚动，最高 200px）；危险类额外显示**红色判危条**；
  - 询问行：`是否执行这条命令？` + 作用域说明；
  - **编号选项列表**（`role="listbox"`，鼠标悬停即高亮）；
  - 底部键盘提示条。
- **键盘驱动**（对齐 DSH / Claude 交互）：
  - `↑` `↓` 循环移动选择；
  - `Enter` 确认当前选项；
  - `1`–`n` 数字键直选；
  - `Esc` 拒绝。
- **选项按风险分级**：
  | 风险 | 选项 | 默认高亮 |
  |---|---|---|
  | 只读 / 普通 | ① 允许执行　② 本会话内此类命令全部允许　③ 拒绝 | **① 允许**（Enter 即批准，Claude 手感） |
  | 危险 | ① 允许执行　② 拒绝 | **② 拒绝**（fail-closed，Enter 不会误批危险操作） |
- **会话信任（对应 `docs/07` 的「同类本会话全允」）**：仅 `readonly` / `normal` 可被授予；
  授予后同类命令不再逐条打断（走审计日志记录"按会话信任自动允许"）。
  **危险类永不入集**——`classify()` 与入集校验双重保证，`request_approval` 引擎侧依旧无自动批准旁路。

### Verified
- `tsc && vite build` 全绿（CSS 35.31KB / JS gzip 85.06KB）；`cargo build` + 重启通过。
- 视觉验证 `baseline/ui_approval_claude_style.png`（真实 tokens.css/shell.css 渲染，含两种风险态）。

## [阶段2-10] 2026-10-17 凌晨 —— 已决策命令「折叠汇总」+ 一键清空【用户反馈】

> 用户反馈原文："现在很多执行后的命令不会去除，一直保留在程序里，然后导致对话框冗杂。"
> （承 2-9：上一轮已把「完整卡」压成「一行摘要」，但 n 条命令仍占 n 行。）

### Changed
- **已决策命令改为「默认折叠的单行汇总」**：无论执行多少条，对话流底部只占**一行**——
  `已决策 N 条命令 · X 允许 / Y 拒绝 · 时间范围`，右侧带 ✕。点开才看逐条明细，
  单条明细再可展开命令全文。彻底解决"命令一直堆在对话里"的冗杂。
- **新增「清空」按钮**：一键把已决策记录从对话流移除（明细仍完整保留在「审计」页与磁盘 `audit.log`），
  直接回应"不会去除"的诉求。

### Verified
- `tsc && vite build` 全绿（CSS 32.51KB / JS gzip 84.74KB）；`cargo build` 通过。
- 视觉验证 `baseline/ui_toolgroup_fix.png`（用真实 tokens.css/shell.css 渲染）：
  折叠态 1 行（对比修复前 6 行）；展开态显示明细且单条可再展开。
- 修复前实拍 `baseline/ui_before_group.png`（6 条各占一行）。

## [阶段2-9] 2026-10-17 凌晨 —— UI 修复：已决策工具卡不再堆积 + 健康告警去误导【用户反馈】

> 用户反馈原文："现在这个对话框太乱了，PowerShell 命令确认后不会删除，然后导致对话乱死了。"

### Fixed
- **已决策工具从「完整卡」改为「一行摘要（点开看全文）」**：此前 `doneTools.map()` 为**每一条**已决策命令
  渲染一张完整工具卡（含命令全文，实测单条 570–1059 字符），连续多轮后对话流被彻底撑爆。
  现改为紧凑行：`状态（已执行/已拒绝）+ #id + 操作摘要 + 字符数 + 时间`，默认收起，点击展开命令全文
  （保留复制按钮）。实测 3 条命令的垂直占用从 ~700px 降到 ~160px。
  - 新增 `summarizeCode()`：优先取引擎提示词要求的 `# 操作：…` 注释，其次 `# 描述：…`，再次首行代码。
  - 新增 `.toolrow / .tr-main / .tr-summary / .tr-body` 样式；窄窗（≤860px）自动隐藏次要元信息防换行。
  - 折叠状态用 `Set<number>` 记录，展开/收起为纯前端状态，不影响引擎事件流。
  - `renderToolCard()` 的 `decided` 参数退役（待审批态专用），`#id` 移到待审批卡头。
- **启动健康告警去误导**：此前每次启动必然弹出红色 `错误 · 选择器健康告警：以下组 0 命中 —— user`。
  M1b 起回复读取已走网络层（SSE），选择器已非关键路径 → 网络层就绪时该组 0 命中**降级为兜底提示**
  （不再 emit `error`）；仅当**网络层未就绪且选择器失效**时才报 error。
- **新增网络层健康上报**：启动后上报 `网络层就绪：回复读取走 SSE（/api/v0/chat/completion），不依赖页面类名`，
  状态条显示为绿色「空闲」；网络层未就绪时改为 error，让用户一眼知道真实风险在哪。
- 修正 `net_subscribe` 的订阅日志：`with_webview` 闭包为异步派发，原先在闭包外读取订阅集合恒为空
  （日志恒显示"0 events subscribed"，与实际不符）→ 改为闭包内打印，现准确显示 7 个事件。

### Verified
- `tsc && vite build` 全绿（CSS 31.17KB / JS gzip 84.40KB）；`cargo build` + 单测 12/12。
- 真机截图：`baseline/ui_toolrow_run2.png`（状态条显示绿色"网络层就绪"）；
  前后对比图 `baseline/ui_toolrow_fix.png`（同屏展示修复前后，用真实 tokens.css/shell.css 渲染）。
- 启动日志：`net_subscribe: Network.enable + 7 个事件已订阅: …`、`SELECTOR UNHEALTHY on boot: user（网络层已就绪，降级为兜底）`。

## [阶段2-8] 2026-10-17 凌晨 —— 引擎重构：DOM 抓取 → 网络层拦截（SSE）【内核级】

> 承接 `docs/11`（空回复故障）与 `docs/10`（E-04 / F-10）。**把「读回复 + 判完成」从比对第三方 CSS 类名/SVG 图标，换成直接读 DeepSeek 自己的 SSE 回复流。** 详见新文档 `docs/12-network-layer-engine.md`。

### Discovered（实测协议，决定性发现）
- **DeepSeek 网页版不用 WebSocket 传回复**（`webSocketFrameReceived` 计数恒 0）；回复走
  **`https://chat.deepseek.com/api/v0/chat/completion`，`mimeType=text/event-stream`（SSE）**。
- SSE 为 JSON-Patch 风格补丁流：`{"p":path,"o":"APPEND"|"SET","v":..}`、粘性路径 `{"v":..}`、
  批量 `o:"BATCH"`；`fragments[].type` 区分 **`THINK`（思考）/ `RESPONSE`（真正回答）**；
  `response/status=FINISHED` 为流内权威完成标志。**纯文本，无需逆向 protobuf。**

### Added
- **CDP 网络事件订阅基建**：`Network.enable` + `GetDevToolsProtocolEventReceiver` +
  `add_DevToolsProtocolEventReceived`（进程内、无端口）。事件写入 `NetCapture` 环形缓冲（4096 上限）。
- **完成判定网络化**：`Network.loadingFinished`（本回合 `/chat/completion` 的 requestId）作为**主信号**，
  彻底摆脱 `<svg><path d="M8.3125 0.980206…">` 这类魔法字符串。
- **读回网络化**：`Network.getResponseBody` 直取 SSE 正文 → `parse_sse_assistant_text` 重放补丁流、
  **只取 RESPONSE 片段**（排除 THINK 思考内容）。不受 DeepSeek 改类名影响。
- **DOM 回声守卫** `looks_like_echo`：兜底路径若读回恰好是自己刚发的用户消息（长提示词注入时高发）
  或页面固定文案 → 丢弃重等，不再把「用户消息/页脚」当回复。
- **显式失败**：网络层与 DOM 皆空时返回 `Err`（修复 `docs/11` F-01「静默返回空串」）。
- **超时诊断**：超时信息包含「是否捕获到回复流 / 已收字节数」，可一眼区分网络问题与页面问题。
- **可观测**：`net_probe` 命令 + 每回合 `net_probe.json` / `sse_body.txt` 落盘，协议再变也能快速取证。
- 新增依赖 `base64 = "0.22"`（`getResponseBody` 的 base64 分支）。

### Verified（真机实测）
- 链路测试：`done: 网络层读回 2 字符, tail="OK"` → `CHAIN OK: reply="OK"`（修复前恒为 `0 字符`）。
- 多轮 Agent 会话（含真实人工审批）：连续读到 673 / 699 / 875 / 920 字符的正确回复（含 `powershell_tool` 标记）。
- `audit.log` 出现「执行命令」与「工具被用户拒绝」两类记录 → 注入→工具解析→审批→执行→回传闭环成立。
- 单元测试 **12/12 通过**（新增 6 个：SSE 片段提取、THINK 排除、FINISHED 判定、OpenAI 风格容错、回声守卫 ×2）。

### Security
- **审批门逐字未改**：`recv_timeout(120s).unwrap_or(false)` → 超时即拒绝（fail-closed），无自动批准旁路。
- 网络层只读事件与响应正文；不注入页面脚本、不新增监听端口。

## [阶段2-7] 2026-10-17 凌晨 —— Claude 原生两区化 + 双会话协调（UI 层）

### Changed（布局：三列 → 两区）
- **DeepSeek 网页从常驻第三列改为按需观察窗**：默认收起（Ctrl+J / 状态条 monitor 钮开合）；收起时 Rust 侧 `hide()` 真隐藏；折叠态细条化 + 隐藏分栏把手
- 对话阅读列收窄至 **780px 居中**（Claude 同款阅读宽度）；对话区头部去边框透明
- 文案随新形态更新（"观察窗"表述）

### Discovered / Process（重要记录）
- **双会话并行修改同一仓库**：另一会话并行修复引擎选择器问题，与本会话引擎修复在 lib.rs 中共存。用户裁决：**引擎（含 selectors.json）归对方会话，前端归本会话**；重建/重启前互相避让。
- 空回复故障根因（据对方 `docs/11`）：**DeepSeek 页面改版导致 assistant 选择器整组 0 命中**，读回恒为空；模型实际已生成工具调用。选择器修复与健康探测归对方会话（15s/30s 可读性告警、结构化自检、启动健康探测）。
- 本机 DPI 坐标系混淆（tauri 报的"物理"尺寸带 1.5× 系数）：任何手动 `set_size` 会被放大 → 已全部回滚（窗口尺寸交系统默认 + auto_resize）。
- 今晚多次"窗口灵异"（首启降级/尺寸漂移/最小化幽灵）疑为双会话重建+重启竞争 WebView2 profile 锁所致。

### Note
- 本提交仅含 UI 层（Shell.tsx / styles）；`lib.rs` / `selectors.json` 的混合改动由对方会话验证后自行提交。
- 证据：`baseline/ui_claude_native_run2.png`（两区化实拍）

## [阶段2-6] 2026-10-16 深夜 —— 暖调人文改版（配色 + 布局精修；用户拍板方向）

### Changed（视觉方向：全站换血为暖调人文 / CDS 风）
- **品牌色 #4D6BFE → 陶土橙 #D97757**；暖炭黑表面阶梯（canvas `#100F0E` / panel `#161513` / surface `#1F1D1A` / inset `#0C0B0A`）；暖白文字 `#F0EEE8`；描边发丝线暖化（`rgba(240,238,232,…)`）
- **暗/浅双主题同源**：浅色重配为暖纸系（`#F3F0EA` / `#FBFAF7` / 深陶土 `#C15F3C`）
- 布局与降噪精修：
  - 左栏 CTA 改**描边式**（拒绝大面积填充色块；空态 CTA 仍保留填充作为焦点）
  - 空态重做：**圆形发光徽章**（陶土 halo 双环）+ 层次留白
  - 顶部**环境暖光**（radial 氛围渐变）；卡片顶缘微高光 + 暖化投影
  - 细滚动条（内容盒内缩）；圆角微增（`--r-lg` 12 / `--r-xl` 16）
- 裸色值清剿：toolcard / approval / composer-focus 全部改 `color-mix(in srgb, var(--…))` 主题自适应；新增 `--c-brand-line` / `--c-ambient`；`--glass-tint` 暖化

### Verified
- `tsc && vite build` 全绿；真机截图 `baseline/ui_warm_run1.png`（暖调渲染确认：描边 CTA / 发光空态 / 橙色 logo 芯片 / 导航激活态）

### Note
- 配色决策与 `docs/07` §4.1 旧的品牌蓝取值存在出入：以 `app/src/styles/tokens.css`（唯一信源）为准，策划案配色章节待下轮文档回填。

## [阶段2-5] 2026-10-16 深夜 —— open-claude-ui 采编 Batch 1（交互物理 + 中文输入守卫）

> 学习开源项目 [open-claude-ui](https://github.com/Caho1/open-claude-ui)（MIT，Claude 桌面端逆向还原）并择优采编。全量评估与采编映射见 `docs/09-open-claude-ui-notes.md`；参考副本 `D:\refs\open-claude-ui`。

### Fixed
- **中文输入法误发消息（真实 bug）**：Enter 选词确认（IME 合成中）会触发发送 —— 采编 Claude 官方断言，键盘处理器顶部加合成守卫（`isComposing || keyCode===229` 一律放行）

### Added
- `app/src/shell/useScrollPinning.ts`：滚动图钉 Hook（150ms 用户输入冷却窗 / 40px 底部吸附容差 / >120px 显示「跳至最新」/ `behavior:auto` 防抖对齐 —— 物理常数原样保留）
- `app/src/shell/ScrollFade.tsx`：7 档非线性 Alpha 羽化遮罩（y≈x^1.5 感知曲线，28px；触顶/触底自动归零；仅用 alpha 通道，暗/浅主题通用）
- `tokens.css`：CDS_EASING 四曲线（OVERSHOOT / EASE_OUT_EXPO / DECEL / SHARP_PRESS）+ `--d-press`

### Changed
- 消息流：接入图钉滚动 + 边缘羽化 + 「跳至最新」悬浮按钮（新消息自动贴底，用户上翻时不抢视口）
- Composer：`field-sizing: content` 原生自适应伸缩（rows=1，46–220px）；双环投影 + focus 品牌微光
- 全局交互物理：按钮按压微缩（active:scale .97 / 150ms 弹簧曲线）；消息上浮入场（320ms）；审批卡待授权呼吸点 + 琥珀微晕
- 减弱动效双通道切断（`data-motion=reduced` + `prefers-reduced-motion`）
- `tools/capture_window.ps1` 升级：EnumWindows 按进程+面积选窗（根治 MainWindowTitle 启发式抖动）；最小化态自动「还原 → 截图 → 放回最小化」

### Discovered（运维记录）
- **紧跟在 cargo build 后的首次启动可能进入"无主窗"状态**（WebView2 profile 锁竞争，表现为仅有 15×15 辅助窗 / 158×26 残窗）；`Stop-Process` 后重拉即恢复。建议重建后间隔数秒再启动。

### Verified
- `tsc && vite build` 全绿（CSS 27.1KB / JS gzip 83.2KB）
- 真机截图 `baseline/ui_cds_run1.png`（1295×877，新会话空态正常渲染）
- 本次重建同时携带上一轮引擎修复（DS 新对话导航回退 + 完成判定复核门）生效

## [阶段2-4] 2026-10-16 深夜 —— Shell UI 构建验收 + 适配修复（新 UI 成为默认；旧骨架退役）

> 承接上一条 [UI实现]：该条目声明的"未经构建校验"已在本轮完成 —— 构建全绿、真机验收通过、并修复 3 处首次编译暴露的问题。

### Fixed（新 UI 首次 `tsc` strict 构建，3 处）
- **`READONLY` 运行时炸弹**：写为 `RegExp[]` 数组却按单正则 `.test()` 调用 —— 编译报错；即使绕过编译，任何非危险命令渲染审批卡时都会崩（数组无 `test`）→ 改为单 `RegExp`
- **分栏拖拽静默失效**：`getElementById("workbench")` 但 workbench 容器缺 `id` → 补 `id="workbench"`
- JSX 收窄后 `risk !== "danger"` 恒真比较（TS2367）→ 该分支内无条件 `autoFocus`

### Changed
- **默认 UI 切换完成**：`App.tsx` → `Shell`；旧单面板骨架样式 `app/src/App.css` **删除**（回滚见 `docs/08` §8.3）
- `tools/capture_window.ps1`：窗口截图工具（PrintWindow 离屏，仅抓目标窗口区域）

### Fixed（引擎，用户实测日志发现；已 check + 单测，随下次重建生效）
- `ds_new_conversation` 按钮匹配失败（480px 右栏下 DS 侧边栏疑似图标模式无文本，日志 ×2）→ 三态返回 `click/navigate/none`；失败回退 **`Page.navigate` 根路径**（DS 根 = 新对话入口）+ 就绪轮询（textarea 就绪且助手消息清空）
- 完成判定疑似中途暂停误判截断（某回合回复 102 字符断于「我」）→ **完成复核门**：候选完成后采样文本 + 停止键状态，文本增长或 stop 重现则继续轮询；完成时 `chain_test.log` 记录字数与尾 24 字（取证）

### Verified
- `tsc && vite build` 全绿（vite 8.3.4 / 195ms / CSS 25.1KB / JS gzip 82.6KB）；`cargo check` + 引擎单测 6/6
- **真机验收（新 Shell UI）**：① 注入 8,169 字符 → 模型按协议侦察（桌面目录树 + skill 检查）→ 审批「允许」→ PowerShell 执行（如实报出 `claude/unity/报表/简历/论文`）→ 回传 → 完成入消息流；证据 `baseline/ui_shell_run{1,2}.png`
- 历史对照（接入前旧骨架 UI，同日 23:09–23:20）：6 次工具执行，含 `自我介绍.docx` 真实任务（目标/可写性/TEMP 探测）

## [UI实现] 2026-10-16 深夜 —— UI 按策划案重构（正式工程 `app/` + spike 零构建参考实现）

### Added（正式工程 `app/` —— 产品本体 UI）
- `app/src/styles/tokens.css`：策划案 §4 全量设计 token（暗/浅双主题、间距/圆角/阴影/层级/动效、密度、玻璃强度 0–3、reduced-motion）。
- `app/src/styles/shell.css`：Shell 布局与组件样式（状态条/左栏/消息/工具卡/危险审批卡/输入区/分栏/内容页/空态/Toast）。
- `app/src/shell/Shell.tsx`：三栏工作台 UI。**接线沿用既有契约，未新增/改名任何命令或事件**：`set_browser_rect` / `start_agent_session` / `agent_turn` / `test_roundtrip` / `tool_decision` / `selfcheck`，事件 `engine://state`（阶段标签+计时+状态点）与 `engine://tool`（解析 `id|code` → 审批卡）。
- `app/src/shell/icons.tsx`：30 个自绘图标（24 网格 / 1.5 线宽 / currentColor，**零新依赖**）。
- 能力与状态：工具**三级策略**（危险红名单 16 条启发式判定；危险类不给「同类全允」；**审批卡默认焦点仍在「拒绝」**，fail-closed 语义不变）、4,000 字符计数、通道自检结果解析（0 命中 → 红色 Banner）、历史/技能（26 项 + 许可未知标注）/审计（事件时间线 + 筛选 + 复制诊断）/设置（主题/玻璃/密度/动效/回车发送；「允许自动执行」标注为**待引擎支持**而非假开关）、诊断抽屉（保留联调所需引擎日志）。
- `spike/embed-webview/src/`：同款 UI 的**零构建参考实现**（`app.html`/`app.js`/`app.css`/`tokens.css`/`icons.js`/`theme-boot.js` + 10 个状态样张 + `EAgent.wiringCheck()` 接线自检），供无构建环境做设计走查。

### Changed
- `app/src/App.tsx`：入口改为渲染 `Shell`（旧单面板实现见 git 历史；回滚 `git checkout app/src/App.tsx`）。
- `spike/embed-webview/src-tauri/src/lib.rs`：三栏 → **双栏**（shell + deepseek，矩形由 UI 驱动），新增 `set_browser_rect` / `estop`，窗口 1280×840 / 最小 1040×720 / 禁止最大化；`tauri.conf.json` 命名按 ADR-0003；`run_experiments.ps1` 产物名按候选解析。
- `docs/08-ui-implementation-notes.md`：实施记录（文件↔章节对照、A/B 运行路径、8 项验收清单、未实现项、回滚、未验证声明 + §8 `app/` 接入说明）。

### Verified
- ❌ **未验证（诚实声明）**：本会话 shell 通道报 `SetNamedSecurityInfoW failed (Win32 5): grantWrite(D:\EAgent)` → 无法执行 `npm run build`（tsc）、`cargo build` 或截图。因此 `app/` 改动**未经类型/打包校验**、Rust 改动未经编译；已按 `strict` + `noUnusedLocals` + `noUnusedParameters` 逐项自查。**第一道验收 = `app` 下 `npm run build` → `cargo build` → `Start-Process` 启动**（8 项清单见 `docs/08` §4）。

## [阶段2-3a] 2026-10-16 深夜 —— 首日点验修正：会话懒启动（用户实测反馈）

### Fixed
- **实测发现**：未点「① 注入提示词」直接点「② 发送指令」→ 模型以普通聊天身份回答"无法访问本地文件"（引擎日志实证：仅 39 字符注入，8k 工具协议从未发出）。修正：
  - **② 未开会话时自动先执行会话初始化**（懒启动）：新对话 → 注入提示词（等待完成）→ 再发用户消息；初始化失败则明确报错，不与用户消息混发
  - **会话初始化先开 DS 新对话**（`ds_new_conversation`：文本匹配「开启新对话/新对话/新建对话」+ 可见性过滤；找不到则沿用当前会话并记录警告）——避免旧上下文（含普通聊天的"我无法访问"痕迹）污染 Agent 人设（Legacy 同为"新对话 + 注入"流程）
  - `session_started` 状态：inject/send 硬失败不计入已启动（可安全重试）；超时计入（提示词已送达）
- 工作目录归会话：① 时锁定；② 在已开会话时忽略后续输入框改动

### Verified
- 用户点验截图实证：**回复全文显示 353 字符无截断**（对照修复前：160 字符尾部截断）✓
- 单元测试 6/6 复跑通过

## [阶段2-3] 2026-10-16 深夜 —— M1a 主链路：Agent 循环（提示词注入 → 工具执行 → 审批 → 回传）

### Added
- **Agent 三件套（M1a）**——让应用真正「操作电脑」（回应用户实测反馈"只传话不干活"）：
  - ① **提示词注入**：`start_agent_session(workdir)` 把 `config/prompt.initial.md`（工具协议，约 8k 字）注入会话；`{{WORKDIR}}` / `{{TEMP_WORKDIR}}` 实填
  - ② **工具标记解析**：提取 `# powershell_tool_调用工具_识别头/尾_本行注释一定要完全保留` 之间的代码（容忍 ``` 围栏；缺尾视为未完成，不作工具调用）
  - ③ **执行 + 回传**：临时 `.ps1`（UTF-8 BOM）+ `-NoProfile -ExecutionPolicy Bypass -File`、**CREATE_NO_WINDOW**（不弹黑窗）、120s 超时（超时杀子进程）、**4000 字符输出上限**（沿用 Legacy 限制）、stdout/stderr 合并（UTF-8 优先 / GBK 兜底解码）；结果以 `[工具回传][PowerShell 执行结果]` 回注会话；单回合最多 6 次工具迭代
- **审批门（用户安全指令落地）**：工具执行前 emit `engine://tool` → UI 弹审批卡（**默认焦点在「拒绝」**，显示代码预览）→ `tool_decision(id, allow)` 下达；**10 分钟未操作按拒绝处理（fail-closed）；代码内无任何自动批准旁路**（含 debug 构建）
- **审计（proto-audit）**：工具决策与执行（命令全文 + 结果前 300 字）落 `app\data\audit.log`；回合轨迹落 `chain_test.log`
- 引擎命令：`start_agent_session` / `agent_turn` / `tool_decision`；会话级**回合锁**（并发回合直接拒绝）；新增依赖 `encoding_rs`
- UI：工作目录输入 + 「① 注入提示词」「② 发送指令」「测试回合」（裸回合，诊断用）；审批卡（代码预览/拒绝/允许）；`tool` 阶段状态色（琥珀）

### Fixed
- **回复被截断到 160 字符**（完成判定读回尾部仅用于稳定性比较，误把尾部当结果返回）→ 完成时单独做一次**全文读回**
- **临时脚本文件名毫秒级时间戳并行冲突**（单元测试抓出：同毫秒两次执行互相覆盖）→ 文件名追加全局序号
- 注入步骤偶发 `ERR: CDP timeout`：失败后 1 秒重试一次

### Verified
- 引擎单元测试 **6/6 通过**（`cargo test --lib`）：工具标记解析（正/负/多行去围栏）+ 执行器（ASCII 回传 / 中文 UTF-8 回传 / 错误捕获）

### Next
- 真机端到端点验（用户手动）：① 注入提示词 → ② 发指令 → 审批卡点「允许」→ 结果回传。基准样例：`C:\Users\<local-user>\Desktop\报表`（实测为空文件夹——若 Agent 答"空"，即为工具真实执行的铁证）
- docs/07 M1 其余项：消息流/工具卡三级审批细化（红名单）、审计落库、设置持久化（D 盘）

## [阶段2-2] 2026-10-16 深夜 —— 正式工程骨架 + 链路闭环（`app/`）

### Added
- **`app/` 产品本体工程**（create-tauri-app react-ts；Tauri 2.12.4 / React 19 / Vite 8 / Rust 引擎）：
  - **2-WebView 收敛**（docs/07 §1.2 约束 2）：shell 全窗 + DeepSeek 右栏；`ResizeObserver → rAF → invoke("set_browser_rect")` 同步矩形
  - 引擎 `lib.rs`：`cdp_call` 进程内 CDP 基建 + `selectors.json` 编译期内嵌 + **完成判定 v2**（stop→arrow 主判 + 文本兜底 + 完成后复核）+ `engine://state` 事件流
  - 命令：`set_browser_rect` / `test_roundtrip` / `selfcheck`；启动 15s 后**一次性自动链路测试**（flag 防重复）
  - `WEBVIEW2_USER_DATA_FOLDER=D:\EAgent\app\data\webview`（**登录态自 spike profile 迁移**，重启免登录）
  - CSP 按 docs/07 §8.2 收紧；capabilities 最小（`core:default`+`opener`，`webviews:["shell"]`）
- 证据：`baseline/chain_test_run1.log`、`baseline/skeleton_stdout_run1.log`、`baseline/skeleton_screenshot.png`（窗口实拍）

### Verified
- **链路闭环首跑通过**（正式工程内）：`auto: href=https://chat.deepseek.com/`（登录态迁移成功）→ `CHAIN OK: reply="OK"`（约 4.3s 往返）
- 窗口渲染正常（PrintWindow 实拍：标题栏 `EAgent`、玻璃状态条、左面板 UI、右侧 DeepSeek 栏）

### Discovered & Fixed（骨架期踩坑，供团队与 M1 参考）
1. **debug 构建下 `WebviewUrl::App` 解析到 `devUrl`**（`localhost:1420`）→ shell 显示 ERR_CONNECTION_REFUSED。已移除 conf 的 `devUrl`/`beforeDevCommand`（本工作流 = `npm run build` + `cargo build` 直连 frontendDist；如需 `tauri dev` 热更再恢复）。
2. **从 MSYS bash 直接 `.\eagent-app.exe &` 启动 GUI 会出现"无窗口"**（进程存活、WebView 正常、但无顶层窗口可枚举；属启动环境问题）→ **一律用 `Start-Process` 启动**（README 已记录）。
3. 构建前必须先关闭运行中的实例（exe 被占用 → link 拒绝访问 os error 5）。
4. 依赖版本对齐：app 直接依赖 `webview2-com =0.39.1` / `windows =0.62.2` 与 tauri 解析一致（锁版于 `app/src-tauri/Cargo.lock`）。

### Next（M1 按 docs/07）
- 手动「发送（测试回合）」按钮 + `set_browser_rect` 已接线（待用户点验）；`engine://state` 事件已驱动状态条
- 待办：消息流/工具卡/审批（docs/07 §5）、审计落库、设置持久化（D 盘，不经 appDataDir）

## [UI策划] 2026-10-16 深夜 —— 新 UI 策划案 v1.0 + 产品命名与术语冻结

### Added
- `docs/07-ui-design-plan.md`：完整 UI 策划案 v1.0 —— 项目结构分析（参考实现 34 控件全量盘点 / 7 条技术硬约束）、**§0 术语与命名约定**、设计目标与原则、现代美学方向、设计系统（色彩/排版/间距/圆角/阴影/层级/图标/基础与业务组件/状态矩阵/**设计 token**）、**§1.3 能力迁移对照（F1–F11）**、关键页面与模块方案、交互与动效、响应式与可访问性、实施与交付（交付物清单 / 工程结构 / M0–M3 里程碑 / 14 项风险）。
- `docs/adr/0003-naming-and-terminology.md`：**产品命名与术语约定** —— `EAgent` = 产品本体（本工作区）；`EAgent Legacy` = 参考实现 / 能力 donor；含术语替换规范与命名落地清单。

### Changed
- **定位纠正（用户指令）**：旧版易语言实现是**参考实现 / 能力 donor，不是产品谱系上的上游**；**本工作区即产品本体**。全局替换措辞：上游产品 → 参考实现 / 旧版 EAgent；继承品牌 → 沿用品牌色；保留 F1–F11 → **迁移 F1–F11 能力**。
- `README.md`：标题与定位改为"EAgent 产品本体与工作区"；`EAgent.exe`/`README.html`/附件标注为参考实现发布物；阶段 2 条目补 `docs/07` + ADR-0003 索引；自检命令注明需 Legacy 的调试浏览器。
- `docs/06-ui-design.md`：F1–F11 措辞改为"迁移"；功能映射表头改为"参考实现形态"；补术语指引（以 ADR-0003 + `docs/07` §0 为准）。
- `docs/adr/0001`：加术语注记（本文"上游发布物" = 参考实现发布物），决策内容不变。

### Decided
- **命名冻结**：产品显示名 / `productName` = `EAgent`，`identifier` = `com.eagent.app`，安装包 = `EAgent-Setup-<version>-x64.exe`；参考实现磁盘文件名 `EAgent.exe` **不重命名**（哈希冻结于 `baseline/MANIFEST.sha256`），`EAgent Legacy` 标签只出现在文档与 UI。
- **F1–F11 能力清单**在 `docs/07` §1.3 依证据重建并冻结为**能力迁移唯一清单**（原清单仅存在于聊天版，仓库内无规范文本）。
- 前端 WebView 由 spike 的 3 个收敛为 **2 个**（1 shell + 1 DeepSeek），矩形容器经 `ResizeObserver → IPC → set_position/set_size` 同步；理由是左/中两页无法共享 token/状态，且 `auto_resize()` 无法实现"右栏固定、中栏伸缩"。
- **沿用** `README.html` 品牌色 `#4D6BFE`（与 `docs/06` 的 `#3B6CFF` 合并），避免出现两个品牌蓝。
- 旧版"整窗透明度"改为 **面板玻璃强度 0–3 档**（正文区 α ≥ 92%），以保住对比度。
- **更正** `docs/06` 的急停热键：`Ctrl+Shift+Esc` 是 Windows 保留的系统热键，应用无法接管 → 建议 `Ctrl+Alt+Q` + UI 常驻急停按钮。
- 不提供窗口最大化（系统提示词硬规则），以"专注布局"替代。

## [阶段2-1] 2026-10-16 深夜 —— UI 设计文档 + 完成判定 v2 + profile 重定向

### Added
- `docs/06-ui-design.md`：阶段 2 UI 设计文档 v0.1（三栏信息架构、交互流、功能映射 F1-F11、视觉/可访问性规范、验收标准）
- Spike E7：完成判定 v2（发送按钮状态主判 + 文本稳定兜底 + 完成后 +3s 零增长复查）

### Verified
- **完成判定 v2 全链路**（run10）：`stop@0.7s → arrow@3.9s → PRIMARY 判定@4.3s → +3s growth=0`（0 截断 ✓）；主判定比文本兜底快约 2.5s/回合；svg 前缀双向现场验证
- **WebView2 profile 重定向 D 盘**：`WEBVIEW2_USER_DATA_FOLDER` 生效；原 C 盘 EBWebView（20MB）迁移后**登录态保留**（重启直达 chat.deepseek.com）；C 盘 spike 目录已清空

### Fixed
- E7 状态探测的**双重 JSON.stringify** 缺陷（内层返回字符串 → Rust 侧字段全部丢失）；踩坑与三条引擎实现注意点记入 docs/05 §8.3

### Changed
- `config/selectors.json`：svg 前缀升格为**已验证的主状态信号**（aria 仍不可用）

## [Spike] 2026-10-16 晚 —— 内嵌 WebView2 全绿 + 工具链落盘

### Added
- 工具链：Rust 1.99.1（`D:\rust`，用户级环境变量）+ VS BuildTools 2022（`D:\VSBuildTools`）+ rsproxy 源镜像
- `spike/embed-webview/`：内嵌 WebView2 实验工程（官方 create-tauri-app + examples/multiwebview 模式）+ 运行器 + 四轮实验日志 + 证据截图
- `docs/05-spike-report.md`：实验矩阵、实测回填、阶段 2 输入

### Verified（E1-E6，四轮实测，原始日志在 spike/embed-webview/）
- 三栏 multiwebview（unstable）｜CDP `Runtime.evaluate` 带回值｜`DOM.setFileInputFiles` 返回 OK（nodeId=29）｜`Page.captureScreenshot` ~48-85KB PNG｜登录持久化（重启直达 chat.deepseek.com）｜**完整回合 4 秒：注入→发送→等待→读回"OK"**

### Changed
- `config/selectors.json` → v2.1：按实测回填 verified；**确认 aria 候选全部不可用** → 按钮状态判定改以 svg path 前缀为主
- legacy 真机自检（channel_selfcheck -Port 9222）建议跳过（已被 E5 实测替代）；20 轮"0 截断"回归改在新引擎上执行
- 清理 `baseline/smoke_profile`（15MB，legacy 冒烟产物）

## [阶段1收尾-1] 2026-10-16 晚 —— 双层结构 + 方向变更

### Decided
- **方向变更（ADR-0002）**：阶段 2/3 采用 Tauri v2 内嵌 WebView2，不再驱动外部 Chrome；R2 决策点收敛为"直接新引擎"。
- Q&A 澄清：阶段1"5 个资产"= 3 脚本 + 提示词 + `1_启动调试浏览器.ps1` 的**旧版备份副本**（后两者存在于两份 `.e` 源、编译产物中无备份副本）；`smoke_profile` 已在 .gitignore；随机端口为每次运行重新抽取（32750 仅为当次样本）。

### Changed
- `scripts/` 重构为双层：`frozen/`（防漂移基线）+ `working/`（工作版本），哈希清单 `scripts/MANIFEST.json`。
- `tools/verify_assets.py` v3：frozen 强制三源校验 + 双层漂移报告 + `--manifest`。
- `tests/run_tests.ps1`：T0/T1 覆盖双层；T2a（frozen 强一致）/T2b（工作层漂移报告）；新增 T5（清单校验）。

### Added
- `docs/adr/0002-embedded-webview.md`（方向变更与风险）
- `docs/03-asset-domains.md`（资产域×编码×来源 + git 入/不入清单）
- `docs/04-ops-cards.md`（真机自检 / 20 轮回归 / 内嵌 spike 运行手册）

## [阶段1] 2026-10-16 —— 文本化与模块化（首批）

### Added
- `tools/extract_assets.py`：易语言工程/二进制内嵌资产逐字节提取器（脚本×3 + 系统提示词；多副本感知）。
- `tools/verify_assets.py`：漂移校验器（scripts/ ↔ `_EAgent.e` / `code.e` / `EAgent.exe` 三源 SHA256 比对）。
- `scripts/`：3 个 PowerShell 引擎脚本的字节级副本（UTF-8 BOM，三源一致，含 `scripts/README.md` 同步协议）。
- `config/selectors.json`：选择器外置 v1（语义锚点优先 + 混淆类名兜底）。
- `config/prompt.initial.md`：系统提示词可读版（来源：逐字节提取，GB18030 原件存 `baseline/`）。
- `tools/channel_selfcheck.ps1`：通道自检（只读，统计选择器命中与发送按钮状态）。
- `tests/run_tests.ps1` + `tests/parse_check.ps1`：静态测试套件 T0（BOM）/T1（语法）/T2（字节一致）/T3（配置有效）/T4（提示词资产）。
- `tests/smoke_launch.ps1`：启动冒烟（随机端口 + D 盘 profile + 无 allow-origins 实验；自开自关）。
- `baseline/`：证据目录（上游副本、三源提取对照、窗体 JSON、字符串转储、MANIFEST.sha256）。
- `docs/`：阶段 0 评估、阶段 1 报告、E2TXT 覆盖评估、ADR-0001。

### Verified
- 脚本与提示词在 `_EAgent.e`、`code.e`、`EAgent.exe` 三处 **SHA256 完全一致**（无版本漂移）。
- 静态测试 T0-T4 全部通过；漂移校验三源全 PASS。
- 冒烟实验：Chrome 155 随机端口 32750 无 `--remote-allow-origins=*` 情况下，CDP 页面级/浏览器级 WebSocket 均连通且 `Runtime.evaluate` 成功 → 该旗标可在新引擎中移除（P0-5 路径验证）。

### Discovered
- 内嵌脚本实为 **UTF-8 with BOM**（非 GBK）；提示词常量则为 GBK。
- `1_启动调试浏览器.ps1` 在源码中存在旧版备份副本，与当前版唯一差异 = 缺少 `--remote-allow-origins=*`（该旗标为后加）。
- 编译产物不包含备份副本；`code.e` 与 `_EAgent.e` 在主资产上一致。
