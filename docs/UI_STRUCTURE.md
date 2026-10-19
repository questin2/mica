# Mica 前端 UI 结构文档（供精修 / 重构参考）

> 生成日期：2026-10-18 ｜ 方式：只读通读当前源码 ｜ 范围：`d:\Mica\app\src`
> 用途：交给外部专家做 UI 精修/重构。**本文只描述现状与结构，不含改动建议的实施**。
> 行号基于 2026-10-18 当前工作树，**重构前请再对一次源码**。

---

## 0. 一页速览

- **技术栈**：React 19 + TypeScript + Vite；样式为**两个全局 CSS 文件**（`tokens.css` 设计令牌 + `shell.css` 组件样式），**无 CSS Modules / 无 Tailwind / 无 styled-components**。
- **UI 骨架**：单窗口三栏工作台 —— 顶部状态条（StatusBar）+ 左导航轨（LeftRail）+ 主区（对话或某个能力页）+ 分栏拖拽（splitter）+ 右侧浏览器面板（DeepSeek WebView2 原生视图覆盖区）。
- **状态模型**：**无 Context / 无外部 store**。全部 UI/引擎状态集中在 `Shell.tsx` 一个组件的 ~25 个 `useState`，再以 props 下发到各子组件，以 setter 注入各 hook。
- **设计令牌**：`tokens.css` 是唯一色彩/间距/字号/圆角/层级/动效信源，**禁止裸色值**（文件头 `tokens.css:4` 明确）。
- **主题/密度/动效/玻璃** 由 `html` 上的 `data-theme|data-density|data-motion|data-glass` 切换（`tokens.css:122-184`）；**布局折叠态** 由 `.app` 上的 `data-rail|data-browser|data-narrow` 切换（`shell.css:310-325`、`938-990`、`1383-1397`）。
- **安全逻辑在 UI 侧**：审批分级与自动放行是纯函数（`format.ts` / `autoApprove.ts`），**属安全边界，重构不得改动其语义**（详见 §7）。

---

## 1. 文件清单

| 文件 | 行数(约) | 角色 |
|---|---|---|
| `src/main.tsx` | 8 | 挂载 |
| `src/App.tsx` | 72 | `AppErrorBoundary` + `<Shell/>` |
| `src/shell/Shell.tsx` | 668 | **状态中心 + 布局装配 + 页面路由**（核心） |
| `src/shell/types.ts` | 99 | 全部 IPC/UI 类型（手写，无代码生成） |
| `src/shell/format.ts` | 189 | 风险分级 `classify`/`isCompoundOrRisky`、高亮、格式化（**安全相关**） |
| `src/shell/autoApprove.ts` | 105 | 自动放行决策 `decideApproval`（**安全相关**） |
| `src/shell/audit.ts` | — | 审计回写 |
| `src/shell/constants.ts` | 56 | 内置危险/只读规则、PHASE_TEXT |
| `src/shell/icons.tsx` | 86 | 内联 SVG 图标集（`Icon`） |
| `src/shell/Markdown.tsx` | 279 | 自研 Markdown 渲染（**不注入 HTML**） |
| `src/shell/ScrollFade.tsx` | 85 | 滚动渐变容器 |
| `src/shell/useScrollPinning.ts` | 58 | 滚动锚定/跳至最新 |
| `src/shell/components/*.tsx` | 24 个 | 常驻组件 + 页面级组件（见 §2） |
| `src/shell/hooks/*.ts` | 9 个 | 外观/窄窗/计时/矩形/键盘/持久化/事件/动作 |
| `src/styles/tokens.css` | 195 | **设计令牌（唯一信源）** |
| `src/styles/shell.css` | ~1620 | 全部组件样式（分区见 §5） |

---

## 2. 组件树与状态归属

```
App.tsx (AppErrorBoundary)
└─ Shell.tsx  ← ★ 状态中心（~25 useState，见下）
   ├─ <a.skip-link>（无障碍跳转，shell.css:75）
   ├─ StatusBar.tsx      顶栏：品牌/相位/计时/通道/信任与自动放行徽标/主题切换/急停
   └─ .workbench (flex row)
      ├─ LeftRail.tsx    左轨：导航(9 项=view) + 工作目录 + 会话/工具计数 + 运行/复制/主题
      ├─ <main class="main">
      │   ├─ view==="chat" → renderChat()
      │   │   ├─ ChatHeader.tsx   标题/计数/ModelSwitcher(通道切换)/新对话/自检/诊断
      │   │   ├─ MessageList.tsx  消息流（ScrollFade + 空态 + banner）
      │   │   │   ├─ Markdown.tsx（assistant 消息）
      │   │   │   ├─ ApprovalCard.tsx（toolReq 存在时内联）  ← props
      │   │   │   └─ ToolGroup.tsx（doneTools 汇总）           ← props
      │   │   ├─ DiagDrawer.tsx（diagOpen 时显示日志）
      │   │   └─ Composer.tsx    输入区（附件 dialog + 发送/裸回合）
      │   └─ 其它 view → <div class="page">{renderPage()}</div>
      │       ├─ SkillsPage / McpPage / KnowledgePage / WorkflowPage / PluginsPage
      │       └─ AuditPage / HistoryPage / SettingsPage
      ├─ .splitter          分栏拖拽（键盘 ←/→ 或指针拖拽，改写 CSS 变量 --w-browser）
      └─ .browser (section) 右侧浏览器面板（WebView2 原生视图覆盖 .browser-slot）
   └─ .toast-host（toasts 列表）
```

**常驻**：StatusBar、LeftRail、.browser、.toast-host。
**页面级（按 `view` 二选一渲染）**：renderChat() 或 renderPage()。

**Shell 持有的状态（`Shell.tsx:66-113` 起）**：
- 引擎相位：`phase`、`detail`、`logs`、`busy`、`channel`/`channelRaw`
- 会话/工具：`workdir`、`paths`、`msg`、`messages`、`toolReq`、`doneTools`、`expandedTools`、`toolsGroupOpen`、`approvalIdx`、`trustedRisk`(+ref)
- 外观/布局：`view`、`railCollapsed`、`browserCollapsed`、`diagOpen`、`enterSend`（`theme/density/motion/glass` 来自 `useAppearance`）
- 计数徽标：`skillCount`、`pluginCount`、`mcpCount`、`kbCount`
- 通道：`providers`
- 安全：`autoRules`、`security`
- 提示：`toasts`

**props 透传深度**：多为 1 层（Shell → 子组件）；`MessageList` 通过 `children` 承接审批卡/工具组，避免把 risk/options 灌进去（`MessageList.tsx:5-6` 注释）。**hooks 反向接收 setter**（如 `useEngineEvents` 接收 `setPhase/setDetail/...`，`Shell.tsx:298`）。

---

## 3. 数据流（与引擎的契约）

```
用户操作 ──invoke(command)──▶ Rust
Rust ──emit("engine://state"|"engine://tool")──▶ useEngineEvents ──setState──▶ Shell ──props──▶ 组件
```

- **两个事件主题**：
  - `engine://state` → `{seq,ts_ms,phase,detail}`：相位/进度/错误/崩溃（`useEngineEvents.ts:34-45`）。
  - `engine://tool` → `{...,phase:"request",detail:"<id>|<code>"}`：审批请求；前端按**首个 `|`** 切分（`useEngineEvents.ts:49-52`），再走 `decideApproval`（危险/复合 → 一律弹卡；否则可能自动放行）。
- **命令**：前端调用的命令全集以 `lib.rs` 的 `generate_handler!` 为准（本审计见 `lib.rs:498-555`）；前端侧类型在 `types.ts`。**重构 UI 时命令名与事件名是硬契约，不可改。**
- **审批出口唯一**：人工与自动放行都走同一个 `invoke("tool_decision",{id,allow})`（`useShellActions.ts:140-159`、`useEngineEvents.ts:65-67`）。
- **持久化**：`useSessionPersistence` 以 800ms 防抖把 `messages/doneTools/workdir` 存 `session.json`（`useSessionPersistence.ts:55-70`）。

---

## 4. 布局机制

- `.app`（`shell.css:92`）：flex 纵向；子项 = StatusBar(`--h-statusbar` 48px) + `.workbench`。
- `.workbench`（`shell.css:100`）：flex 横向，子项 = `.rail` + `.main(flex:1)` + `.splitter` + `.browser`。
- **尺寸来自令牌**：`--w-rail`(336)/`--w-rail-collapsed`(48)/`--w-browser`(480)/`--w-browser-collapsed`(40)/`--w-splitter-hit`(12)/`--w-chat-narrow`(780)/`--w-content-max`(1080)（`tokens.css:85-95`）。
- **折叠**：`.app[data-rail="collapsed"]`（`shell.css:310-325`）、`.app[data-browser="collapsed"]`（`938-990`、`1539-1554`）、`.app[data-narrow="true"]`（`1383-1397`）。
- **分栏拖拽**：`Shell.tsx:615-648` 用指针事件/键盘直接 `document.documentElement.style.setProperty("--w-browser", px)` 改全局变量（**JS 直接写 CSS 变量的唯一处**），再调 `pushRect()` 同步给原生 WebView 矩形（`useBrowserRect`）。
- **对话区宽度**：`.messages-inner`/`.composer-inner` 用 `max-width: var(--w-chat-narrow)` 居中（`shell.css:362`、`844`）；页面区 `.page-inner` 用 `--w-content-max`（`994`）。
- **响应式**：断点 `@media (max-width: 860px)`(807) / `(max-width: 780px)`(1151) / `(max-width: 900px)`(1213)；另有 **容器查询** `.main{container-type:inline-size;container-name:main}`(1598) 供 `.chat-header` 自适应；`@media (forced-colors: active)`(1407) 与 `(prefers-reduced-motion: reduce)`(1489)。

---

## 5. 样式架构与区域分区（`shell.css`）

**命名约定**：区域前缀 + 短缩写，风格接近 BEM 变体：
`sb-*`(状态条) / `rail-*`,`nav-*`,`session`,`workdir`(左轨) / `ch-*`(对话头) / `msg*`,`md-*`(消息与 Markdown) / `toolcard`,`tc-*` / `toolrow`,`tr-*` / `toolgroup`,`tg-*` / `appr*`(审批卡) / `composer*` / `browser*`,`bh-*` / `page*`,`section*`,`card*`,`field*`,`table`,`chip*`,`switch`,`skill-*`,`plugin-*`,`pc-*`,`catalog-*`(能力页) / `toast*`,`empty*`,`skeleton`,`thinking*`,`jump-latest`,`diag-drawer`,`log-line` / 原子类 `btn*`,`badge*`,`dot`,`input`,`kbd`,`sep`,`row/col/grow` 等。

**分区（行号区间）**：
| 区间 | 内容 |
|---|---|
| 1-100 | 重置 + 原子工具类（`.hidden/.row/.col/.grow/.muted/.mono/.ellipsis`…）+ `.skip-link` |
| 92-101 | **骨架** `.app`/`.workbench` |
| 103-169 | 原子组件 `.btn/.badge/.dot`（含 `@keyframes breathe`） |
| 172-214 | StatusBar |
| 217-325 | LeftRail（含折叠态） |
| 328-501 | main/chat/header/messages/banner/msg/**Markdown(`.md-*`)**/caret |
| 503-607 | **工具卡两套** `.toolcard+.tc-*`（503-555）与 `.toolrow+.tr-*`（560-607） |
| 610-674 | `.toolgroup/.tg-*` |
| 678-807 | 审批卡 `.appr*` + `@keyframes approval-in` |
| 812-835 | **另一套旧审批卡** `.approval/.ap-*` |
| 838-887 | Composer |
| 890-900 | DiagDrawer/log |
| 903-990 | splitter + browser 面板 |
| 993-1262 | **页面通用**（page/section/card/field/switch/chip/table/skill-*/plugin-*/pc-*）+ 断点 |
| 1265-1300 | empty / skeleton |
| 1303-1352 | thinking |
| 1352-1380 | toast |
| 1383-1426 | 窄窗降级 + forced-colors |
| 1428-1620 | **尾部"补丁层"**：动效、命中区、容器查询等后置覆盖 |

---

## 6. 可重构性观察（**给专家的切入点，均带证据**）

> 这些是"结构性弱点"，不是 bug；供评估重构范围与收益。

### 6.1 状态与组件
1. **巨型组件 / 无状态分层**：`Shell.tsx` 668 行持有 ~25 个 `useState`（`Shell.tsx:66-113`），IPC 与事件全部在顶层接线（`297-318`）。任意相位变化都会让整个工作台重渲染。**建议**：抽 `useChatStore`（`useReducer`）或轻量 store，把"引擎会话状态"从 UI 布局中剥离。
2. **props 透传 + setter 注入**：hook 以**参数**接收一堆 setter（`useEngineEvents` / `useShellActions` / `useSessionPersistence` 全部如此），组件 props 也偏多。**建议**：用 selector/细粒度订阅替代 setter 透传。
3. **消息渲染无虚拟化、Markdown 每次重解析**：`MessageList.tsx:91-122` `messages.map` 全量；`Markdown.tsx:223-303` 每次调用 `parseBlocks(text)` 且**无 `memo`**。**建议**：`Markdown` 加 `memo` + 解析结果缓存；列表按回合分段 memo（虚拟化视实测再上）。
4. **页面组件重复模式**：`HistoryPage/SkillsPage/PluginsPage/McpPage/KnowledgePage` 各自维护 `loading/err/列表` 三态与空态 UI（例 `HistoryPage.tsx:21-41`）。**建议**：抽 `<AsyncList>`/`<EmptyState>`/`<Skeleton>` 公共件（`shell.css` 已有 `.empty/.skeleton` 基座）。

### 6.2 样式
5. **全局类名（无作用域）**：两个全局 CSS，类名靠约定；仓库已有 `__tests__/cssClassCollision.test.ts` 专门防冲突 —— 说明这已是团队知道的痛点是**真问题**。**建议**：引入 CSS Modules 或至少按区域分包 + 命名前缀强制。
6. **重复声明 / 尾部补丁层**：`.section-title`(998 与 1168)、`.composer-box`(845 与 1471)、`.chat-header`(331 与 1536/1600)、`.main`(328 与 1598) **各定义两次**；`1428-1620` 是 appended 覆盖层。**建议**：合并同名规则、把"补丁层"回填到对应分区。
7. **疑似历史遗留样式**：`.approval/.ap-*`（812-835）与现役 `.appr*`（678-807）功能重叠；`.toolcard/.tc-*`（503-555）与现役 `.toolrow/.tr-*`（560-607）重叠。**建议**：核对组件实际用到的类（`ApprovalCard.tsx` 用 `.appr`；`ToolGroup.tsx` 用 `.toolrow/.toolgroup`），清理未被引用的那套。
8. **Markdown 样式混在 shell.css**（`.md-*` 322-489）：**建议**拆到 `markdown.css`，便于独立精修。
9. **`!important` 仅 1 处**（`.hidden`，`shell.css:56`）—— 很干净，重构时保持这一约束。

### 6.3 其它
10. **类型手写**：`types.ts` 全部手写、无生成（`types.ts:2-20`），Rust 结构与 TS 靠命名对齐。**建议**：`ts-rs`/`tauri-specta` 生成（与审计报告 P1-4 一致）。
11. **JS 直接改 CSS 变量**：splitter 拖拽（`Shell.tsx:620/632`）绕过 React 状态直接写 `--w-browser`。**建议**：评估是否纳入 `browserWidth` 状态（注意性能：高频 pointermove 下直接写变量反而更优）。

---

## 7. 重构红线（**不要动**）

1. **IPC 契约**：命令名（`lib.rs generate_handler!`）与事件名/负载格式（`engine://state`、`engine://tool` 的 `{id}|{code}`、首个 `|` 切分）是前后端硬约定 —— 改 UI 不得改这些（`useEngineEvents.ts:49-52`、`Shell.tsx:8-12` 顶部注释明确）。
2. **安全逻辑语义**：`format.ts` 的 `classify`/`isCompoundOrRisky` 与 `autoApprove.ts` 的 `decideApproval`（顺序：danger→ask / compound→ask / 信任 / 配置规则）**是安全边界**，重构只可"搬位置"，不可改判定。`config/approval.json` 分级与"危险类不给全部允许、默认停在拒绝"（`Shell.tsx:329-340`）同理。
3. **视觉令牌**：`tokens.css` 是唯一色彩/尺寸信源，重构须继续"只引变量、不写裸色值"。
4. **无障碍契约**：`role="alertdialog"/"log"/"listbox"`、`aria-live`、`.skip-link`、键盘操作（`useKeyboard`）与 forced-colors 支持（`shell.css:1407`）应保留。
5. **原生视图覆盖**：右侧面板是 WebView2 **原生视图**覆盖在 `.browser-slot` 上（React 只画占位与边框），必须继续用矩形同步（`useBrowserRect` → `set_browser_rect`）——**改布局时不要假定它是普通 DOM 元素**。

---

## 8. 未覆盖 / 需实测确认

- 本文按**结构**梳理，未逐像素核对视觉；`shell.css` 尾部补丁层（1428-1620）与 §6.2 列出的重复/疑似废弃规则**建议用浏览器实际渲染 + 覆盖率工具**确认后再删。
- `.approval/.ap-*`、`.toolcard/.tc-*` 是否完全未引用，**属推测**，需 grep JSX 再定。
- 各页面组件的完整 props 与内部 state 未逐行展开（仅抽关键项）；细节请配合 `components/*.tsx` 源码。
