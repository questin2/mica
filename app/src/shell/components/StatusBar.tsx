// 顶部状态条 —— 拆分自 Shell.tsx（可扩展性报告 v2 · S-01）
//
// 纯展示：引擎状态、通道自检、计时与三个快捷开关全部由容器传入。
import { Icon } from "../icons";
import { DOT_STATE, PHASE_TEXT } from "../constants";
import { baseName, mmss } from "../format";
import type { PermissionMode, Risk, ThemeMode, ToolReq } from "../types";
import { PERMISSION_LABEL } from "../autoApprove";

type Props = {
  workdir: string;
  phase: string;
  detail: string;
  toolReq: ToolReq | null;
  channel: { text: string; ok: boolean | null };
  busy: boolean;
  elapsed: number;
  browserCollapsed: boolean;
  theme: ThemeMode;
  /** 当前会话信任的风险类别（这些类别的命令**不再逐条询问**） */
  trusted: Set<Risk>;
  /** 已启用的**持久**自动放行规则原文（来自 agent.json；非空表示功能已开启） */
  autoRules: string[];
  onToggleRail: () => void;
  onToggleBrowser: () => void;
  onCycleTheme: () => void;
  onEstop: () => void;
  onClearTrust: () => void;
  /** 当前权限模式（自动放行的粒度） */
  permissionMode: PermissionMode;
  onPermissionMode: (m: PermissionMode) => void;
};

const RISK_LABEL: Record<Risk, string> = { readonly: "只读", normal: "普通", danger: "危险" };

/**
 * 把引擎发来的 detail 变成**面向使用者**的一句话。
 *
 * 引擎的 detail 是给诊断用的，含实现术语（「（网络层）」「（空间启发式）」「SSE」等）；
 * 直接贴到顶栏会显得像日志。这里只做**显示层**归一 —— 不改后端、不改事件负载：
 *   1. 去掉括号里的实现术语
 *   2. 去掉与阶段名重复的前缀（如「完成，443 字符」里的「完成」）
 *   3. 截断到 46 字符，超出加省略号
 * 细节仍完整保留在诊断抽屉里（logs 面板），不丢信息。
 */
function narrate(raw: string): string {
  let t = raw.trim();
  if (!t) return "";
  // 含有实现细节（协议/端点/渲染层术语）的整句直接不显示 ——
  // 它们只在诊断抽屉里出现。顶栏要的是"人话"，不是日志。
  if (/(SSE|webview|WebView|DOM|spatial|heuristic|api\/v0|\/chat\/completion|HttpClient|Invoke-WebRequest)/i.test(t)) {
    return "";
  }
  // 去掉实现术语括号（中英文括号都处理）
  t = t.replace(/[（(]\s*(网络层|DOM|SSE|空间启发式|兜底|原生|via\s*\w+)\s*[)）]/gi, "");
  // 行内残留的同类术语
  t = t.replace(/\b(网络层|空间启发式)\b/g, "");
  // 去掉重复的阶段词前缀，如「完成，443 字符」→「443 字符」
  t = t.replace(/^(完成|空闲|生成中|等待审批)\s*[，,、·:：]\s*/, "");
  t = t.replace(/\s{2,}/g, " ").trim();
  if (!t) return "";
  return t.length > 46 ? `${t.slice(0, 46)}…` : t;
}

export function StatusBar({
  workdir,
  phase,
  detail,
  toolReq,
  channel,
  busy,
  elapsed,
  browserCollapsed,
  theme,
  trusted,
  autoRules,
  onToggleRail,
  onToggleBrowser,
  onCycleTheme,
  onEstop,
  onClearTrust,
  permissionMode,
  onPermissionMode,
}: Props) {
  return (
    <header className="statusbar">
      <button
        className="btn ghost sm"
        title="折叠/展开左栏（Ctrl+B）"
        aria-label="折叠或展开左栏"
        onClick={onToggleRail}
      >
        <Icon name="panel" size={16} />
      </button>
      <span className="brand" aria-hidden="true">
        M
      </span>
      <div className="sb-left">
        <span className="sb-title ellipsis" title={workdir}>
          会话 · {baseName(workdir)}
        </span>
        <span className="dot" data-state={DOT_STATE[phase] ?? "offline"} role="img" aria-label="引擎状态" />
        <span className="sb-phase">
          {PHASE_TEXT[phase] ?? phase}
          {narrate(detail) ? ` · ${narrate(detail)}` : ""}
        </span>
      </div>
      <div className="sb-mid">
        {/* 权限模式：一眼可见"现在会不会被问"，避免一堆逐条同意 */}
        <div
          role="radiogroup"
          aria-label="权限模式"
          style={{ display: "flex", gap: "var(--s-1)", alignItems: "center" }}
        >
          {(["readonly", "workspace", "smart", "full"] as const).map((m) => (
            <button
              key={m}
              className="btn ghost sm"
              role="radio"
              aria-checked={permissionMode === m}
              aria-pressed={permissionMode === m}
              title={m === "full" ? "全权限：危险命令也不再询问" : `权限模式：${PERMISSION_LABEL[m]}`}
              onClick={() => onPermissionMode(m)}
              style={
                permissionMode === m
                  ? {
                      borderColor: m === "full" ? "var(--c-danger)" : "var(--c-brand)",
                      color: m === "full" ? "var(--c-danger)" : "var(--c-brand)",
                    }
                  : undefined
              }
            >
              {PERMISSION_LABEL[m]}
            </button>
          ))}
        </div>
        {/* ⚠ 持久自动放行已启用：这是最需要被看见的状态 —— 意味着有些命令不再征求确认 */}
        {autoRules.length > 0 && (
          <span
            className="badge danger autoapprove-badge"
            title={`以下命令将不再征求你的确认、你也不会看到它们：\n\n${autoRules.join("\n")}\n\n来自 config/agent.json → autoApprovePatterns；每次自动放行都写入 app\\data\\audit.log`}
          >
            <Icon name="shield-alert" size={12} />
            ⚠ 持久自动放行 {autoRules.length} 条（你看不到这些命令）
          </span>
        )}
        {toolReq && <span className="badge warn">有工具待审批</span>}
        {/* 自动放行面必须**可见**且**可撤销** —— 原先只能开不能关、界面上也看不见 */}
        {trusted.size > 0 && (
          <button
            className="badge danger trust-badge"
            onClick={onClearTrust}
            title="这些类别的命令本会话内不再逐条询问。点击清空，恢复逐条询问。"
          >
            <Icon name="shield-alert" size={12} />
            自动放行：{[...trusted].map((r) => RISK_LABEL[r]).join("、")}（点此撤销）
          </button>
        )}
        <span className={`badge ${channel.ok === false ? "warn" : channel.ok ? "ok" : ""}`}>通道：{channel.text}</span>
        {busy && <span className="timer">{mmss(elapsed)}</span>}
      </div>
      <div className="sb-right">
        <button
          className="btn ghost sm"
          title="展开/折叠 DeepSeek 网页观察窗（Ctrl+J）"
          aria-label="展开或折叠 DeepSeek 网页观察窗"
          aria-pressed={!browserCollapsed}
          onClick={onToggleBrowser}
        >
          <Icon name="monitor" size={16} />
        </button>
        <button className="btn ghost sm" title="切换主题" aria-label="切换主题" onClick={onCycleTheme}>
          <Icon name={theme === "light" ? "sun" : theme === "system" ? "monitor" : "moon"} size={16} />
        </button>
        <button className="btn danger sm" title="急停（Ctrl+.）" onClick={onEstop}>
          <Icon name="stop" size={16} />
          急停
        </button>
      </div>
    </header>
  );
}
