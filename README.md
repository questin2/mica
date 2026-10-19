<div align="center">

<img src="docs/assets/banner.svg" alt="Mica — 以浏览器为推理内核的桌面 AI Agent" width="100%">

**简体中文** · [English](README.en.md)

[![Platform](https://img.shields.io/badge/Windows-10%2B-0078D6?logo=windows11&logoColor=white)](#)
[![Tauri](https://img.shields.io/badge/Tauri-2.12-24C8DB?logo=tauri&logoColor=white)](#)
[![Rust](https://img.shields.io/badge/Engine-Rust-F74C00?logo=rust&logoColor=white)](#)
[![React](https://img.shields.io/badge/Frontend-React%2019-61DAFB?logo=react&logoColor=white)](#)
[![License](https://img.shields.io/badge/License-MIT-3DA639)](LICENSE)

[快速开始](#快速开始) · [安全模型](#安全模型) · [威胁模型与已知局限](#威胁模型与已知局限) · [FAQ](#faq) · [架构详情](docs/ARCHITECTURE.md)

</div>

---

> **风险提示**：本项目通过浏览器自动化协议（CDP）驱动第三方网站（DeepSeek 网页版）并读取其内部接口。这种使用方式**可能违反对应服务的用户条款，存在账号被限制的风险**。请自行评估合规风险，风险自负；仅建议用于个人学习研究。本项目与 DeepSeek 及任何模型服务商均无关联。

## 这是什么

Mica 是一个 Windows 桌面 AI Agent：**不调用模型 API**，而是把**你已登录的 DeepSeek 网页版**内嵌为推理引擎（Tauri WebView2），由本地 Rust 引擎通过 CDP 在网络层读取模型回复，驱动完整的多轮工具调用循环。

- **零 API 成本** — 复用网页会话，无 Key、无按量计费（受网页版自身额度与频率限制）
- **数据留本地** — 对话、工具结果、审计日志全部在本机，无遥测
- **网络层读回** — 从模型的 SSE 回复流直接取结果，不依赖页面 DOM 实现，读回可靠
- **fail-closed 审批门** — 非只读命令默认拦截，人工逐条批准后执行，全程审计留痕

架构与技术实现的完整说明见 **[docs/ARCHITECTURE.md](docs/ARCHITECTURE.md)**（[English](docs/ARCHITECTURE.en.md)）。

## 快速开始

**环境要求**：Windows 10+（含 WebView2 Runtime，Win11 自带）· Rust 1.99 · Node 18+ · VS Build Tools 2022（VC + Windows SDK）；Python 3 仅质量门禁工具需要。

```powershell
git clone https://github.com/questin2/mica.git
cd mica/app

npm install
npm run build                # 前端产物 -> dist/

cd src-tauri
cargo build                  # 生成 target\debug\eagent-app.exe
```

运行（二选一）：

```powershell
# 方式一：开发模式（前端热更新）
cd mica\app
npm run tauri dev

# 方式二：直接运行构建产物
.\app\src-tauri\target\debug\eagent-app.exe
```

**首次运行**：在内嵌 WebView 中登录你的 DeepSeek 网页账号 → 左栏填入工作目录 → 点击「注入提示词」初始化会话，之后即可对话并下达需要工具执行的任务。

质量门禁（可选，修改代码后建议跑）：

```powershell
# 仓库根目录；-ExecutionPolicy Bypass 仅用于运行仓库内自带的测试脚本
powershell -NoProfile -ExecutionPolicy Bypass -File tests\run_tests.ps1
python tools\verify_assets.py --all
cd app\src-tauri && cargo test
cd app && npm test
```

> 注：项目更名自 EAgent，部分代码级标识符（可执行文件名 `eagent-app.exe`、数据目录、`EAGENT_*` 环境变量前缀）仍为历史命名，将在后续版本统一迁移。

## 安全模型

设计原则：**默认拒绝、单一审批入口、执行留痕可审计**。

- **审批门** — 所有需要执行命令的路径收敛到同一道审批门；审批卡展示**代入后的完整命令**，你确认的就是实际执行的
- **风险分级** — 管道 / 分号 / 重定向 / 脚本块 / 写入类 cmdlet 等模式判为危险；危险类**不提供**「全部允许」选项
- **审计留痕** — 每条命令的哈希、风险分级、审批来源（人工 / 自动）写入本地审计日志，界面可查
- **配置保护** — 安全相关配置（审批规则、MCP 清单等）启动时冻结快照，文件被外部改动后需在设置页确认才生效，否则回落安全默认
- **密钥隔离** — 密钥存于用户数据目录（历史命名 `%APPDATA%\EAgent\secrets`），不入仓库

## 威胁模型与已知局限

明确它**防什么、不防什么**——对一个会执行本地命令的工具，这比"绝对安全"的承诺更重要：

**它防的**：
- 模型幻觉或误解指令产生的危险命令被直接执行（默认拒绝 + 人工逐条确认）
- "全部允许"类的一键放行被扩大到危险操作（危险类不提供该选项）
- 安全配置被静默篡改（快照 + 显式确认机制）
- 执行了什么却说不清（全量审计日志）

**它不防的（已知局限）**：
- **风险分级基于命令文本的模式匹配**，存在被绕过的可能：PowerShell 别名、编码命令、混淆写法可能被误判为低危。**最终防线是你在审批卡上阅读完整命令**——请把它当作必须认真看的确认步骤，而不是形式
- **网页内容中的提示词注入**：模型浏览的网页可能包含诱导其生成危险命令的内容。这类命令仍会经过审批门，但如果你不细看就批准，防线就在你这一层失效
- **自动放行**（默认关闭的可选功能）一旦启用，其规则覆盖范围内的命令不再有人工确认——启用即表示你接受这一代价，界面会常驻红色警告徽标
- 审批门是**设计目标上的单一收敛点**，不构成形式化安全证明；如需强保证，请在隔离环境（虚拟机 / 沙箱）中运行

## FAQ

**Q：它会在我不知情时执行命令吗？**
设计上不会。所有非只读命令默认逐条审批；唯一的自动执行面（`autoApprovePatterns`）默认为空且完全不生效，启用后界面常驻红色警告徽标、危险类永不进入自动放行、每条放行留痕并可一键撤销。但请阅读上方[威胁模型](#威胁模型与已知局限)理解局限。

**Q：为什么零成本？用的是我自己的账号吗？**
是。Mica 驱动**你自己已登录**的 DeepSeek 网页会话，不调用 API——没有 Key、没有 token 计费，代价是受网页版使用限制，且存在上文所述的条款风险。

**Q：支持哪些模型？**
内置适配两条通道：**DeepSeek 网页版**（默认，经实测适配）与**本地 Ollama 模型**。并非"任意网页模型"开箱即用——接入其他站点需要适配其页面选择器与接口，属扩展开发。

**Q：我的数据存在哪？**
全部本地：会话与配置在工作区数据目录，审计日志在应用数据目录（`app/data/audit.log`，已从 git 跟踪中排除），密钥在 `%APPDATA%\EAgent\secrets`。无遥测。

**Q：为什么 Windows only？**
引擎依赖 WebView2 与 PowerShell 工具链，是刻意的范围取舍而非技术限制；跨平台在 2.0 规划中评估。

## Roadmap

- [x] 内嵌 WebView2 技术验证（CDP 直调 / 文件上传 / 截屏 / 登录持久化 / 完整回合）
- [x] Agent 主循环 + fail-closed 审批门 + 审计闭环
- [x] 网络层拦截引擎（替代 DOM 猜测）
- [x] 技能 / 插件 / 会话持久化 / 配置全外置
- [x] 发布加固：路径回退 / 崩溃落盘 / 单实例
- [ ] MCP 桥接全量落地（**开发中**）
- [ ] 引擎重写收尾与品牌统一（移除 CDP 通道、迁移历史命名标识符）

## 文档

| 文档 | 内容 |
|---|---|
| [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) | 架构与技术实现详情（工作原理 / 模块 / 配置参考 / 质量门禁） |
| [docs/ARCHITECTURE.en.md](docs/ARCHITECTURE.en.md) | Architecture & implementation details (English) |

## License

[MIT](LICENSE) © 2026 questin2

---

<div align="center">

<sub>Rust · Tauri · 0 tokens spent</sub>

</div>
