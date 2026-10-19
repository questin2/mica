/* ==========================================================================
   Mica Shell（正式工程 UI）—— 实现策划案 docs/07-ui-design-plan.md
   §5.2 三栏工作台 / §5.3 工具三级策略 / §5.4 状态矩阵 / §6 动效 / §7 可访问性

   与引擎的接线（全部沿用既有契约，未新增/改名任何命令或事件）：
     命令 invoke：set_browser_rect{x,y,width,height} | start_agent_session{workdir}
                 agent_turn{workdir,message} | test_roundtrip{message}
                 tool_decision{id,allow} | selfcheck
     事件 listen：engine://state {seq,ts_ms,phase,detail}
                 engine://tool  {seq,ts_ms,phase:"request",detail:"<id>|<code>"}
   安全约束（与引擎一致，UI 不得放宽）：工具一律等待用户点击；危险类不给「同类全允」；
   审批卡默认焦点在「拒绝」；急停在存在待审批时直接按拒绝处理（fail-closed）。
   ========================================================================== */
import { useCallback, useEffect, useRef, useState } from "react";
import { lsGet, lsSet } from "./format";
import type { PermissionMode, Step } from "./types";
import { invoke } from "@tauri-apps/api/core";
import { fetchQuote } from "./quips";
import { open } from "@tauri-apps/plugin-dialog";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { Icon } from "./icons";
import { useScrollPinning } from "./useScrollPinning";
import { useKeyboard } from "./hooks/useKeyboard";
import { useSessionPersistence } from "./hooks/useSessionPersistence";
import { useBrowserRect } from "./hooks/useBrowserRect";
import { useElapsed } from "./hooks/useElapsed";
import { useEngineEvents } from "./hooks/useEngineEvents";
import { useAppearance } from "./hooks/useAppearance";
import { useNarrow } from "./hooks/useNarrow";
import { useShellActions } from "./hooks/useShellActions";
import "../styles/tokens.css";
import "../styles/shell.css";

import type {
  ToolReq,
  DoneTool,
  Msg,
  LogRow,
  View,
  Risk,
  Toast,
  NavItem,
  ProvidersPayload,
  AppPaths,
  SecurityStatus,
} from "./types";
import { BUSY } from "./constants";
import { ApprovalCard } from "./components/ApprovalCard";
import { ToolGroup } from "./components/ToolGroup";
import { SkillsPage } from "./components/SkillsPage";
import { AuditPage } from "./components/AuditPage";
import { HistoryPage } from "./components/HistoryPage";
import { SettingsPage } from "./components/SettingsPage";
import { StatusBar } from "./components/StatusBar";
import { LeftRail } from "./components/LeftRail";
import { ChatHeader } from "./components/ChatHeader";
import { MessageList } from "./components/MessageList";
import { DiagDrawer } from "./components/DiagDrawer";
import { Composer } from "./components/Composer";
import { classify, setRiskRules } from "./format";
import { auditTrustChanged } from "./audit";
import { activeAutoApproveRules, setAutoApproveRules } from "./autoApprove";
import { PluginsPage } from "./components/PluginsPage";
import { McpPage } from "./components/McpPage";
import { KnowledgePage } from "./components/KnowledgePage";
import { WorkflowPage } from "./components/WorkflowPage";

export default function Shell() {
  const slotRef = useRef<HTMLDivElement>(null);

  /* ---------- 引擎状态（唯一信源） ---------- */
  const [phase, setPhase] = useState("idle");
  const [detail, setDetail] = useState("等待引擎事件");
  const [logs, setLogs] = useState<LogRow[]>([]);
  const [busy, setBusy] = useState(false);
  const [channel, setChannel] = useState<{ text: string; ok: boolean | null }>({ text: "未自检", ok: null });
  const [channelRaw, setChannelRaw] = useState("");

  /* ---------- 会话 / 工具 ---------- */
  // 初始为空：由 app_paths 提供当前用户的桌面/主目录，避免发布环境硬编码开发机路径。
  const [workdir, setWorkdir] = useState("");
  const [paths, setPaths] = useState<AppPaths | null>(null);
  const [msg, setMsg] = useState("");
  const [messages, setMessages] = useState<Msg[]>([]);
  const [toolReq, setToolReq] = useState<ToolReq | null>(null);
  const [doneTools, setDoneTools] = useState<DoneTool[]>([]);
  /* 已决策工具的展开集合：默认全部收起（一行摘要），点开才显示命令全文。
     这样连续执行多条长命令时，对话流不会被一堆完整命令卡撑爆。 */
  const [expandedTools, setExpandedTools] = useState<Set<number>>(() => new Set());
  /* 已决策命令「汇总行」是否展开：默认收起 → 无论执行多少条，对话流里只占一行。 */
  const [toolsGroupOpen, setToolsGroupOpen] = useState(false);
  /* 审批选项的当前高亮下标（DSH/Claude 式：↑↓ 选择、Enter 确认、数字键直选）。 */
  const [approvalIdx, setApprovalIdx] = useState(0);
  /* 本会话信任的风险类（仅 readonly / normal；危险类永不进入）——
     用户在审批里显式选「本会话内此类全部允许」后生效。 */
  const [trustedRisk, setTrustedRisk] = useState<Set<Risk>>(() => new Set());
  // 权限模式：仅查看 / 工作区内改 / 智能审核 / 全权限 —— 决定哪些风险类别不再逐条询问
  const [thinking, setThinking] = useState("");
  // 内联步骤时间线（思考 / 工具 / 阶段），按发生顺序
  const [steps, setSteps] = useState<Step[]>([]);
  // 最近会话（左栏切换入口）—— 后端 open_session 已存在
  const [sessions, setSessions] = useState<{ id: string; title: string; updated_at: number }[]>([]);
  // 用 ref 传进动作层：useCallback 闭包会捕获旧的 steps（数组引用），
  // 直接传值会导致追加回复时永远拿到空数组 → 步骤丢失。
  const stepsRef = useRef<Step[]>([]);
  stepsRef.current = steps;
  const [permissionMode, setPermissionMode] = useState<PermissionMode>(
    () => (lsGet("permissionMode", "smart") as PermissionMode) || "smart",
  );
  const trustedRiskRef = useRef<Set<Risk>>(new Set());

  /* ---------- 外观 / 布局 ---------- */
  const [view, setView] = useState<View>("chat");
  // 外观（主题/密度/动效/玻璃）自带 localStorage 持久化与 DOM 应用
  const { theme, setTheme, density, setDensity, motion, setMotion, glass, setGlass } = useAppearance();
  const [railCollapsed, setRailCollapsed] = useState(false);
  const [browserCollapsed, setBrowserCollapsed] = useState(true); /* Claude 原生形态：观察窗默认收起（Ctrl+J / 状态条开关） */
  const [diagOpen, setDiagOpen] = useState(false);
  const [enterSend, setEnterSend] = useState(true);
  // 技能数（供左栏导航徽标）：技能库已数据化，从后端取一次即可
  const [skillCount, setSkillCount] = useState(0);
  // 插件数（左栏徽标）；插件页加载/增删后回报
  const [pluginCount, setPluginCount] = useState(0);
  const [mcpCount, setMcpCount] = useState(0);
  // MCP server 数（左栏徽标）
  const [, setKbCount] = useState(0);
  // 对话通道（Phase 2.3）
  const [providers, setProviders] = useState<ProvidersPayload | null>(null);
  const [t] = useState(() => ({ seq: Date.now() }));
  const [toasts, setToasts] = useState<Toast[]>([]);
  const toastId = useRef(0);

  const pushToast = useCallback((kind: Toast["kind"], text: string) => {
    toastId.current += 1;
    const id = toastId.current;
    setToasts((list) => [...list, { id, kind, text }]);
    window.setTimeout(() => setToasts((list) => list.filter((x) => x.id !== id)), 5000);
  }, []);

  /* 审批出现时重置高亮项：普通/只读类停在「允许」，危险类停在「拒绝」（fail-closed）。 */
  useEffect(() => {
    if (!toolReq) return;
    // 审批卡只渲染在对话页：流程/知识库等页面运行时若弹出审批，自动切回对话，
    // 否则用户看不到审批卡，只能等 120s 超时按拒绝处理。
    setView("chat");
    setApprovalIdx(classify(toolReq.code) === "danger" ? 1 : 0);
  }, [toolReq]);

  /* 会话信任集合同步到 ref（事件监听器依赖为空，需读最新值）。 */
  useEffect(() => {
    trustedRiskRef.current = trustedRisk;
  }, [trustedRisk]);

  /* 技能数 / 插件数：供左栏导航徽标显示（两者都已数据化，插件会贡献技能） */
  const refreshSkillCount = useCallback(async () => {
    try {
      setSkillCount((await invoke<unknown[]>("list_skills")).length);
    } catch {
      /* 读不到就保持原值 */
    }
  }, []);

  useEffect(() => {
    void refreshSkillCount();
    invoke<unknown[]>("list_plugins")
      .then((p) => setPluginCount(p.length))
      .catch(() => {});
    invoke<ProvidersPayload>("list_providers")
      .then(setProviders)
      .catch(() => {});
    invoke<AppPaths>("app_paths")
      .then((p) => {
        setPaths(p);
        // 只有当前还没有工作目录（未被存档恢复）时才用默认值。
        setWorkdir((w) => w || p.default_workdir);
      })
      .catch(() => {});
  }, [refreshSkillCount]);

  /* 切换对话通道：确认 → 后端切换 → 清空本地会话视图。 */
  const changeProvider = useCallback(
    async (id: string, model?: string) => {
      if (busy) {
        pushToast("warn", "回合进行中，暂不能切换通道");
        return;
      }
      const current = providers?.providers.find((p) => p.active);
      const sameModel = id !== "ollama" || current?.model === model;
      if (current?.id === id && sameModel) return;
      if (
        messages.length > 0 &&
        !window.confirm("切换通道会开启新会话；当前对话已保存在历史记录中。继续？")
      ) {
        return;
      }
      try {
        const text = await invoke<string>("set_provider", { id, model: model ?? null });
        setMessages([]);
        setDoneTools([]);
        setToolReq(null);
        await invoke("clear_session").catch(() => {});
        setProviders(await invoke<ProvidersPayload>("list_providers"));
        pushToast("ok", text);
      } catch (e) {
        pushToast("danger", `切换通道失败：${String(e)}`);
      }
    },
    [busy, providers, messages.length, pushToast],
  );

  /* ---------- 窄窗降级：见 hooks/useNarrow.ts ---------- */
  const narrow = useNarrow(setBrowserCollapsed, pushToast);

  /* 持久自动放行规则（agent.json → autoApprovePatterns）。
     默认空 = 不生效。**启用后必须让用户看得见** —— 因为这意味着有些命令将不再征求确认。 */
  const [autoRules, setAutoRules] = useState<string[]>([]);

  /** 从后端重新读取并应用审批/自动放行规则（启动与「重载安全配置」共用）。 */
  const applyApprovalRules = useCallback(async () => {
    try {
      const rawStr = await invoke<string>("approval_rules");
      const rep = setRiskRules(JSON.parse(rawStr));
      if (rep.dangerFallback || rep.skipped > 0) {
        pushToast(
          "warn",
          `审批规则有 ${rep.skipped} 项非法${rep.dangerFallback ? "，危险列表已回退内置" : ""}（已采用 ${rep.danger} 条危险规则）`,
        );
      }
    } catch {
      /* 读不到就用内置规则 */
    }
    try {
      const patterns = await invoke<string[]>("auto_approve_rules");
      const rep = setAutoApproveRules(patterns);
      const active = activeAutoApproveRules();
      setAutoRules(active);
      if (rep.active > 0) {
        pushToast(
          "danger",
          `⚠ 已启用 ${rep.active} 条持久自动放行规则 —— 命中的命令将不再征求确认，你也看不到它们（审计页可查）`,
        );
      }
      if (rep.skipped > 0) pushToast("warn", `自动放行规则有 ${rep.skipped} 项非法，已跳过`);
    } catch {
      /* 读不到 = 不启用（安全默认） */
    }
  }, [pushToast]);

  useEffect(() => {
    void applyApprovalRules();
  }, [applyApprovalRules]);

  /* 安全配置快照状态：磁盘改动未确认时后端已回退安全默认；前端只提示，不自动采用。 */
  const [security, setSecurity] = useState<SecurityStatus | null>(null);
  const securityNotified = useRef(false);
  const refreshSecurityStatus = useCallback(async () => {
    try {
      const st = await invoke<SecurityStatus>("security_config_status");
      setSecurity(st);
      if (st.changed && !securityNotified.current) {
        securityNotified.current = true;
        pushToast("warn", "检测到安全配置未确认变更，已回退到安全默认；到设置页确认后启用");
      }
      if (!st.changed) securityNotified.current = false;
    } catch {
      /* 状态读不到不影响主流程 */
    }
  }, [pushToast]);

  useEffect(() => {
    void refreshSecurityStatus();
    const onFocus = () => void refreshSecurityStatus();
    window.addEventListener("focus", onFocus);
    const timer = window.setInterval(() => void refreshSecurityStatus(), 30_000);
    return () => {
      window.removeEventListener("focus", onFocus);
      window.clearInterval(timer);
    };
  }, [refreshSecurityStatus]);

  const reloadSecurity = useCallback(async () => {
    try {
      const st = await invoke<SecurityStatus>("reload_security_config");
      setSecurity(st);
      securityNotified.current = false;
      await applyApprovalRules();
      pushToast("ok", "安全配置已重新加载");
    } catch (e) {
      pushToast("danger", `重新加载安全配置失败：${String(e)}`);
    }
  }, [applyApprovalRules, pushToast]);

  /* ---------- 会话持久化（S-02）：见 hooks/useSessionPersistence.ts ---------- */  useSessionPersistence({
    workdir,
    messages,
    doneTools,
    setWorkdir,
    setMessages,
    setDoneTools,
    onRestored: (n) => pushToast("ok", `已恢复上次会话（${n} 条消息）`),
  });

  /* ---------- 消息流滚动锚定（采编自 open-claude-ui：图钉 + 跳至最新） ---------- */
  const pin = useScrollPinning([messages.length, busy, doneTools.length, toolReq]);

  /* ---------- 右栏矩形同步：见 hooks/useBrowserRect.ts ---------- */
  const pushRect = useBrowserRect({
    slotRef,
    deps: [browserCollapsed, railCollapsed, narrow, view],
  });

  /* ---------- 计时：见 hooks/useElapsed.ts ---------- */
  const { elapsed, begin: beginTimer, end: endTimer } = useElapsed(phase);

  /* 读取最近会话列表（挂载时 + 回合数变化时刷新） */
  // 列表实时性：回合结束（done）或回到空闲（idle）时重新拉一次。
  // 之前只在挂载时加载 → 新对话 / 新回合后不刷新（用户反馈"不会实时更新"）。
  useEffect(() => {
    if (phase !== "done" && phase !== "idle") return;
    invoke<{ id: string; title: string; updated_at: number }[]>("list_sessions")
      .then(setSessions)
      .catch(() => {});
  }, [phase]);

  /* 切换到某个历史会话 */
  const openSession = useCallback(
    async (id: string) => {
      try {
        const d = await invoke<{ session: { workdir: string }; messages: { role: string; content: string; ts: number }[] }>(
          "open_session",
          { id },
        );
        // ★ DB 行是 {role, content, ts(数字)}，Msg 是 {id, role, text, ts(显示串)}。
        //   直接塞原始行 → text 为 undefined → 下游 .replace() 崩溃（已实测）。
        // ★ 工作目录是**每个会话各自的**：切换会话时一并恢复它（DSH / 豆包的行为）。
        const wd = d?.session?.workdir;
        if (typeof wd === "string" && wd) setWorkdir(wd);
        const rows = Array.isArray(d?.messages) ? d.messages : [];
        setMessages(
          rows
            .filter((m) => m.role === "user" || m.role === "assistant")
            .map((m) => ({
              id: t.seq++,
              role: m.role as "user" | "assistant",
              text: m.content ?? "",
              ts: "",
            })),
        );
        setView("chat");
        pushToast("ok", `已切换到该会话（${d.messages.length} 条消息）`);
      } catch (e) {
        pushToast("danger", `切换会话失败：${String(e)}`);
      }
    },
    [pushToast],
  );

  /** 删除历史会话并刷新左栏列表（当前会话被删则自动重置指针，后端已处理） */
  const deleteSession = useCallback(
    async (id: string) => {
      try {
        await invoke("delete_session", { id });
        setSessions(await invoke<{ id: string; title: string; updated_at: number }[]>("list_sessions"));
        pushToast("ok", "会话已删除");
      } catch (e) {
        pushToast("danger", `删除失败：${String(e)}`);
      }
    },
    [pushToast],
  );

  /* 原生标题栏：Mica · <随机句子>
     · 句子来自一言 API（CSP 已放行），失败回落本地库
     · 打字机式逐字显示 —— 原生标题栏无法淡入，"啪"一下换掉太生硬
     · 减弱动效：整句直出，不逐字、不轮播 */
  useEffect(() => {
    const win = getCurrentWindow();
    const reduced = document.documentElement.getAttribute("data-motion") === "reduced";
    let cancelled = false;
    let typer = 0;
    let cycler = 0;

    const show = (text: string) => {
      window.clearInterval(typer);
      const full = `Mica · ${text}`;
      if (reduced) {
        void win.setTitle(full).catch(() => {});
        return;
      }
      let i = "Mica · ".length;
      void win.setTitle("Mica · ").catch(() => {});
      typer = window.setInterval(() => {
        i += 1;
        void win.setTitle(full.slice(0, i)).catch(() => {});
        if (i >= full.length) window.clearInterval(typer);
      }, 45);
    };

    const tick = () => {
      void fetchQuote().then((q) => {
        if (!cancelled) show(q);
      });
    };

    tick();
    if (!reduced) cycler = window.setInterval(tick, 22000);

    return () => {
      cancelled = true;
      window.clearInterval(typer);
      window.clearInterval(cycler);
      void win.setTitle("Mica").catch(() => {});
    };
  }, []);

  /* ---------- 引擎事件：见 hooks/useEngineEvents.ts ---------- */


  useEngineEvents({ setPhase, setDetail, setLogs, setToolReq, setDoneTools, permissionMode, setThinking, setSteps, pushToast });

  /* ---------- 动作：见 hooks/useShellActions.ts ---------- */
  const { runSession, runTurn, runBareRound, runSelfcheck, decideTool, estop, copy, toggleTool } = useShellActions({
    workdir,
    msg,
    setMsg,
    setBusy,
    setMessages,
    setLogs,
    setChannelRaw,
    setChannel,
    toolReq,
    setToolReq,
    setDoneTools,
    stepsRef,
    setSteps,
    setExpandedTools,
    seq: t,
    beginTimer,
    endTimer,
    pushToast,
  });

  /* ---------- 键盘（策划案 §7.4；急停用 Ctrl+. ，不用系统保留的 Ctrl+Shift+Esc） ---------- */
  /* ---------- 派生值 ---------- */
  const isBusyPhase = BUSY.includes(phase);
  const turns = messages.filter((m) => m.role === "user").length;
  const risk: Risk = toolReq ? classify(toolReq.code) : "normal";

  /* 审批选项（DSH/Claude 式编号列表）：
       - 危险类：仅「允许 / 拒绝」，且默认高亮停在「拒绝」（fail-closed）；
       - 只读/普通类：额外提供「本会话内此类全部允许」（对应 docs/07 的「同类本会话全允」）。 */
  const approvalOptions = ((): { key: "allow" | "allowAll" | "deny"; label: string; hint?: string; tone: string }[] => {
    if (!toolReq) return [];
    const r = classify(toolReq.code);
    const base: { key: "allow" | "allowAll" | "deny"; label: string; hint?: string; tone: string }[] = [
      { key: "allow", label: "允许执行", tone: "primary" },
    ];
    if (r !== "danger") {
      base.push({ key: "allowAll", label: "本会话内此类命令全部允许", hint: r === "readonly" ? "只读类" : "普通类", tone: "plain" });
    }
    base.push({ key: "deny", label: "拒绝", hint: "Esc", tone: "danger" });
    return base;
  })();

  /* 执行一个审批选择（供鼠标点击与键盘共用）。 */
  const chooseApproval = useCallback(
    (key: "allow" | "allowAll" | "deny") => {
      if (!toolReq) return;
      const r = classify(toolReq.code);
      if (key === "allowAll" && r !== "danger") {
        setTrustedRisk((prev) => new Set(prev).add(r));
        // 【审计】自动放行面的每一次变化都留痕
        auditTrustChanged("grant", r === "readonly" ? "只读" : "普通");
        pushToast("warn", `已开启会话信任：${r === "readonly" ? "只读" : "普通"}类命令不再逐条询问（危险类仍然必须确认）`);
      }
      void decideTool(key !== "deny");
    },
    [decideTool, pushToast, toolReq],
  );

  /* 清空会话信任 —— 让"自动放行面"随时可撤销（原先只能开、不能关）。 */
  const clearTrust = useCallback(() => {
    setTrustedRisk((prev) => {
      if (prev.size === 0) return prev;
      auditTrustChanged("clear");
      return new Set<Risk>();
    });
    pushToast("ok", "已清空会话信任，所有类别恢复逐条询问");
  }, [pushToast]);
  const navItems: NavItem[] = [
    ["chat", "bot", "对话", turns || null],
    ["history", "history", "历史", null],
    ["skills", "sparkle", "技能", skillCount || null],
    ["plugins", "plug", "插件", pluginCount || null],
    ["mcp", "zap", "MCP", mcpCount || null],
    ["settings", "gear", "设置", null],
  ];


  /* ---------- 全局键盘（见 hooks/useKeyboard.ts） ---------- */
  useKeyboard({
    toolReq,
    approvalOptions,
    approvalIdx,
    setApprovalIdx,
    onDecide: (allow) => void decideTool(allow),
    onChooseApproval: chooseApproval,
    onEstop: estop,
    onSelfcheck: () => void runSelfcheck(),
    onSend: () => void runTurn(),
    onClearInput: () => setMsg(""),
    toggleRail: () => setRailCollapsed((v) => !v),
    toggleBrowser: () => setBrowserCollapsed((v) => !v),
    enterSend,
  });

  /* ---------- 渲染：内容页 ---------- */
  const renderPage = () => {
    if (view === "skills") {
      // 技能页已抽为独立组件，并改为「数据驱动、可增删」（见 components/SkillsPage.tsx）
      return <SkillsPage onCount={setSkillCount} />;
    }

    if (view === "mcp") {
      // MCP 工具市场（策划案 P1.6）：server / 工具 / 连接状态
      return <McpPage onCount={setMcpCount} />;
    }

    if (view === "knowledge") {
      return <KnowledgePage onCount={setKbCount} />;
    }

    if (view === "workflows") {
      return <WorkflowPage />;
    }

    if (view === "plugins") {
      // 插件管理页（见 components/PluginsPage.tsx）：列表 / 新建 / 删除 / 打开目录
      return (
        <PluginsPage
          onChanged={(n) => {
            setPluginCount(n);
            // 插件会贡献技能 → 技能徽标也要跟着刷新
            void refreshSkillCount();
          }}
        />
      );
    }

    if (view === "audit") {
      // 筛选状态与派生行都在组件内部（见 components/AuditPage.tsx）
      return <AuditPage logs={logs} channelRaw={channelRaw} onCopy={copy} />;
    }

    if (view === "history") {
      return (
        <HistoryPage
          workdir={workdir}
          turns={turns}
          busy={isBusyPhase}
          onLoad={(msgs, wd) => {
            setMessages(msgs);
            if (wd) setWorkdir(wd);
            setView("chat");
            pushToast("ok", `已载入 ${msgs.length} 条历史消息（仅显示；模型上下文未迁移）`);
          }}
        />
      );
    }

    if (view === "settings") {
      // 见 components/SettingsPage.tsx
      return (
        <SettingsPage
        onSetView={setView}
          theme={theme}
          setTheme={setTheme}
          glass={glass}
          setGlass={setGlass}
          density={density}
          setDensity={setDensity}
          motion={motion}
          setMotion={setMotion}
          enterSend={enterSend}
          setEnterSend={setEnterSend}
          channel={channel}
          paths={paths}
          security={security}
          onReloadSecurity={() => void reloadSecurity()}
          onSelfcheck={() => void runSelfcheck()}
        />
      );
    }

    return null;
  };

  /* ---------- 渲染：会话流 ---------- */
  const renderChat = () => (
    <div className="chat">
      <ChatHeader
        turns={turns}
        messageCount={messages.length}
        diagOpen={diagOpen}
        onToggleDiag={() => setDiagOpen((v) => !v)}
        onSelfcheck={() => void runSelfcheck()}
        onOpenAudit={() => setView("audit")}
        providers={providers}
        busy={busy}
        onProviderChange={(id, model) => void changeProvider(id, model)}
        workdir={workdir}
        onPickWorkdir={async () => {
          try {
            const picked = await open({ directory: true, multiple: false, title: "选择工作目录" });
            if (typeof picked === "string" && picked) setWorkdir(picked);
          } catch (e) {
            pushToast("danger", `选择目录失败：${String(e)}`);
          }
        }}
        onCopyWorkdir={() => copy(workdir)}
        onNewChat={() => {
          // 开新对话：清空本地消息与已决策命令，并清掉磁盘存档。
          // 引擎侧的"新对话"由下次注入提示词时自动完成（与既有设计一致）。
          setMessages([]);
          setDoneTools([]);
          void invoke("new_chat")
            .then(() => {
              pushToast("ok", "已开始新对话（本地消息、引擎会话与存档已复位）");
              void invoke<ProvidersPayload>("list_providers").then(setProviders).catch(() => {});
            })
            .catch((e) => pushToast("warn", `已清空本地视图，但引擎会话未复位：${String(e)}`));
        }}
      />

      <MessageList
        messages={messages}
        busy={isBusyPhase}
        phase={phase}
        thinking={thinking}
        steps={steps}
        detail={detail}
        channel={channel}
        pin={pin}
        onCopy={copy}
        onDeleteMsg={(id) => setMessages((prev) => prev.filter((m) => m.id !== id))}
        onSelfcheck={() => void runSelfcheck()}
        onOpenAudit={() => setView("audit")}
        onRunSession={() => void runSession()}
        showEmpty={!toolReq}
      >
        {/* 审批（DSH/Claude 式内联编号选项）与已决策命令由容器装配 */}
        {toolReq && (
          <ApprovalCard
            toolReq={toolReq}
            risk={risk}
            options={approvalOptions}
            activeIdx={approvalIdx}
            onHover={setApprovalIdx}
            onChoose={chooseApproval}
          />
        )}
        <ToolGroup
          tools={doneTools}
          expanded={expandedTools}
          groupOpen={toolsGroupOpen}
          onToggleGroup={() => setToolsGroupOpen((v) => !v)}
          onToggleTool={toggleTool}
          onClear={() => {
            setDoneTools([]);
            setExpandedTools(new Set());
            setToolsGroupOpen(false);
          }}
          onCopy={copy}
        />
      </MessageList>

      {diagOpen && <DiagDrawer logs={logs} />}

      <Composer
        msg={msg}
        setMsg={setMsg}
        busy={busy}
        enterSend={enterSend}
        onBareRound={runBareRound}
        onSend={() => void runTurn()}
      />
    </div>
  );

  return (
    <div
      className="app"
      data-rail={railCollapsed ? "collapsed" : "expanded"}
      data-browser={browserCollapsed ? "collapsed" : "expanded"}
      data-narrow={narrow ? "true" : "false"}
          data-busy={busy ? "true" : "false"}
    >
      <a className="skip-link" href="#composer-input">
        跳到输入框
      </a>

      <StatusBar
        workdir={workdir}
        phase={phase}
        detail={detail}
        toolReq={toolReq}
        channel={channel}
        busy={isBusyPhase}
        elapsed={elapsed}
        permissionMode={permissionMode}
        onPermissionMode={(m: PermissionMode) => {
          setPermissionMode(m);
          lsSet("permissionMode", m);
          pushToast(
            m === "full" ? "warn" : "ok",
            m === "full"
              ? "已切到全权限：危险命令也不再询问，请务必盯着"
              : m === "readonly"
                ? "仅查看：只读命令自动放行，其余仍需确认"
                : m === "workspace"
                  ? "工作区内改：只读与普通命令自动放行，危险类仍会询问"
                  : "智能审核：只读自动放行，其余逐条确认",
          );
        }}
        browserCollapsed={browserCollapsed}
        theme={theme}
        trusted={trustedRisk}
        autoRules={autoRules}
        onToggleRail={() => setRailCollapsed((v) => !v)}
        onToggleBrowser={() => setBrowserCollapsed((v) => !v)}
        onCycleTheme={() => setTheme(theme === "system" ? "dark" : theme === "dark" ? "light" : "system")}
        onEstop={estop}
        onClearTrust={clearTrust}
      />

      <div className="workbench" id="workbench">
        <LeftRail
          busy={busy}
          view={view}
          navItems={navItems}
          workdir={workdir}
          turns={turns}
          logCount={logs.length}
          onRunSession={() => void runSession()}
          onSetView={setView}
          onSetWorkdir={setWorkdir}
          onCopy={copy}
          onCycleTheme={() => setTheme(theme === "system" ? "dark" : theme === "dark" ? "light" : "system")}
          sessions={sessions}
          onOpenSession={(id) => void openSession(id)}
          onDeleteSession={(id) => void deleteSession(id)}
        />

        <main className="main" aria-label="主工作区">
          {view === "chat" ? renderChat() : <div className="page">{renderPage()}</div>}
        </main>

        <div
          className="splitter"
          role="separator"
          aria-orientation="vertical"
          aria-label="调整浏览器面板宽度"
          tabIndex={0}
          title="拖拽，或 ←/→ 调整浏览器面板宽度"
          onKeyDown={(e) => {
            if (e.key !== "ArrowLeft" && e.key !== "ArrowRight") return;
            e.preventDefault();
            const cur = Number(getComputedStyle(document.documentElement).getPropertyValue("--w-browser").replace("px", "")) || 480;
            const next = Math.min(Math.max(420, cur + (e.key === "ArrowLeft" ? 16 : -16)), Math.round(window.innerWidth * 0.6));
            document.documentElement.style.setProperty("--w-browser", `${next}px`);
            window.setTimeout(pushRect, 0);
          }}
          onPointerDown={(e) => {
            const el = e.currentTarget;
            el.setAttribute("data-dragging", "true");
            el.setPointerCapture(e.pointerId);
            const move = (ev: PointerEvent) => {
              const wb = document.getElementById("workbench");
              if (!wb) return;
              const r = wb.getBoundingClientRect();
              const w = Math.min(Math.max(420, r.right - ev.clientX), Math.round(r.width - 360));
              document.documentElement.style.setProperty("--w-browser", `${w}px`);
              window.requestAnimationFrame(pushRect);
            };
            const up = (ev: PointerEvent) => {
              el.removeAttribute("data-dragging");
              try {
                el.releasePointerCapture(ev.pointerId);
              } catch {
                /* 指针已释放 */
              }
              el.removeEventListener("pointermove", move);
              el.removeEventListener("pointerup", up);
              pushRect();
            };
            el.addEventListener("pointermove", move);
            el.addEventListener("pointerup", up);
          }}
        />

        <section className="browser" aria-label="DeepSeek 浏览器面板">
          <div className="browser-header">
            {!browserCollapsed && (
              <span className="bh-item">
                <span className="dot" data-state={channel.ok ? "connected" : channel.ok === false ? "error" : "offline"} />
                通道：{channel.text}
              </span>
            )}
            <div className="bh-right">
              {!browserCollapsed && (
                <>
                  <button className="btn ghost sm" title="重新加载内嵌页面（引擎侧新对话在注入时自动执行）" onClick={() => pushToast("info", "重连由引擎在注入时处理；如需强制刷新请重启应用")}>
                    重连
                  </button>
                  <button className="btn ghost sm" onClick={() => void runSelfcheck()}>
                    自检
                  </button>
                </>
              )}
              <button
                className="btn ghost sm"
                title="折叠/展开浏览器面板（Ctrl+J）"
                aria-label="折叠或展开浏览器面板"
                onClick={() => setBrowserCollapsed((v) => !v)}
              >
                <Icon name={browserCollapsed ? "chevron-left" : "chevron-right"} size={16} />
              </button>
            </div>
          </div>

          <div className="browser-slot" id="browser-slot" ref={slotRef}>
            <div className="browser-placeholder">
              <div className="big">
                <Icon name="panel" size={24} />
              </div>
              <div>
                这里是内嵌的 <b>DeepSeek 网页</b>（WebView2 原生视图，覆盖此区域）
              </div>
              <div className="muted">登录态持久保存（重启免登录）；消息请从左侧输入框发送。</div>
              <div className="muted mono">{paths?.webview_dir ?? "读取中…"}</div>
            </div>
          </div>

          <div className="browser-rail">
            <span className="dot" data-state={channel.ok ? "connected" : "offline"} />
            <button
              className="btn ghost sm"
              title="展开浏览器面板（Ctrl+J）"
              aria-label="展开浏览器面板"
              onClick={() => setBrowserCollapsed(false)}
            >
              <Icon name="chevron-left" size={16} />
            </button>
          </div>
        </section>
      </div>

      <div className="toast-host" aria-live="polite">
        {toasts.map((x) => (
          <div className={`toast ${x.kind}`} key={x.id} role="status">
            <Icon
              name={x.kind === "ok" ? "check" : x.kind === "warn" ? "alert-triangle" : x.kind === "danger" ? "shield-alert" : "info"}
              size={16}
            />
            <div>{x.text}</div>
          </div>
        ))}
      </div>
    </div>
  );
}
