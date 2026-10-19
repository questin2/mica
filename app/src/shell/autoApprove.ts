// 持久自动放行规则（`config/agent.json` → `autoApprovePatterns`）。
//
// ⚠ 这是「读法 A」的执行点：命中规则的命令会**不经用户确认直接执行** ——
// 也就是说用户**看不到这些命令**，只在审计页留下痕迹。因此这里刻意把边界写死：
//
//   1. **危险类永不自动放行**：顺序固定在 `decideApproval` 里，危险判定永远最先；
//   2. **空规则 = 完全不生效**，行为与不配时逐字一致。
//   3. **非法正则跳过并计数**，不会让整组失效，也不会把整组降级成"全部放行"。
//   4. **每次命中都必须写审计**（见 `audit.ts` 的 `auditConfigAutoApproved`），
//      记录命中的规则原文 —— 否则事后无从追责。

import { classify, isCompoundOrRisky } from "./format";
import type { PermissionMode, Risk } from "./types";

export type AutoApproveReport = {
  /** 生效的规则条数（0 = 功能未启用） */
  active: number;
  /** 被跳过的非法/空规则数 */
  skipped: number;
};

let compiled: { source: string; re: RegExp }[] = [];

/**
 * 用配置里的规则覆盖当前自动放行规则。
 *
 * 传空数组或不传即表示**关闭**该功能（这也是默认状态）。
 */
export function setAutoApproveRules(patterns: unknown): AutoApproveReport {
  const rep: AutoApproveReport = { active: 0, skipped: 0 };
  const next: { source: string; re: RegExp }[] = [];

  if (Array.isArray(patterns)) {
    for (const p of patterns) {
      if (typeof p !== "string" || !p.trim()) {
        rep.skipped++;
        continue;
      }
      const src = p.trim();
      try {
        // 与风险规则保持一致：忽略大小写（PowerShell 关键字大小写不敏感）
        next.push({ source: src, re: new RegExp(src, "i") });
      } catch {
        rep.skipped++;
      }
    }
  }

  compiled = next;
  rep.active = next.length;
  return rep;
}

/** 当前生效的规则原文（界面告警要用它把"放行了什么"讲清楚）。 */
export function activeAutoApproveRules(): string[] {
  return compiled.map((c) => c.source);
}

/** 自动放行是否处于启用状态。 */
export function autoApproveEnabled(): boolean {
  return compiled.length > 0;
}

/**
 * 命中则返回**命中的规则原文**（供审计记录），未命中返回 `null`。
 *
 * ⚠ 调用方必须先做过危险判定；本函数对危险命令没有任何概念。
 * 正常情况下不要直接调用它做放行 —— 请用 `decideApproval`（它把顺序固定死）。
 */
export function matchAutoApprove(code: string): string | null {
  for (const c of compiled) {
    if (c.re.test(code)) return c.source;
  }
  return null;
}

/** 一次审批请求的处置方式。 */
export type ApprovalDecision =
  /** 问用户（弹审批卡） */
  | { kind: "ask" }
  /** 按会话信任放行（用户在本会话内点过「全部允许」） */
  | { kind: "auto-session"; clsLabel: string }
  /** 按配置规则放行（持久，跨重启生效） */
  | { kind: "auto-config"; pattern: string };

/**
 * **放行决策（纯函数）** —— 把"顺序"这一安全属性固定在一处，便于测试与审计。
 *
 * 顺序**不可颠倒**：
 *   1. 危险类 → 一律 `ask`（永不自动放行，无论会话信任还是配置规则怎么写）；
 *   2. 会话信任命中 → `auto-session`；
 *   3. 配置规则命中 → `auto-config`（返回命中的规则原文，供审计留痕）；
 *   4. 否则 → `ask`。
 */
/** 权限模式 → 自动放行的风险类别。**唯一**决定"问不问"的地方。 */
export function trustedForMode(mode: PermissionMode): Set<Risk> {
  switch (mode) {
    // 仅查看：只放行只读命令
    case "readonly":
      return new Set<Risk>(["readonly"]);
    // 工作区内修改：只读 + 普通（写文件、装包等）；危险类仍逐条问
    case "workspace":
      return new Set<Risk>(["readonly", "normal"]);
    // 智能审核：只读自动放行；普通类仍问一次，危险类永远问
    case "smart":
      return new Set<Risk>(["readonly"]);
    // 全权限：连危险类也不再询问 —— 只在用户明确选择时生效
    case "full":
      return new Set<Risk>(["readonly", "normal", "danger"]);
  }
}

/**
 * **按权限模式**决定是否询问。与 `decideApproval` 的分工：
 * - `decideApproval` 保持原契约（危险类永远问），既有测试锁着它；
 * - 本函数是模式入口：只有 `full` 会显式跳过危险类的询问，
 *   其它模式完全委托给 `decideApproval`，行为零变化。
 */
export function decideApprovalForMode(code: string, mode: PermissionMode): ApprovalDecision {
  if (mode === "full") {
    // 全权限：连危险类也不问。这是用户显式选择的模式，UI 上必须持续可见。
    return { kind: "auto-session", clsLabel: "全权限" };
  }
  return decideApproval(code, trustedForMode(mode));
}

export const PERMISSION_LABEL: Record<PermissionMode, string> = {
  readonly: "仅查看",
  workspace: "工作区内改",
  smart: "智能审核",
  full: "全权限",
};

export function decideApproval(code: string, trusted: Set<Risk>): ApprovalDecision {
  const cls = classify(code);
  // ★ 危险类**永远询问** —— 这条安全契约不因任何模式而改变（既有测试锁着它）。
  //   「全权限」模式走下面独立的 decideApprovalForMode，语义显式分离。
  if (cls === "danger") return { kind: "ask" };
  // 防御纵深：即使自定义 readonly 正则误判，复合命令也不得走自动放行。
  if (isCompoundOrRisky(code)) return { kind: "ask" };
  const clsLabel = cls === "readonly" ? "只读" : "普通";
  if (trusted.has(cls)) return { kind: "auto-session", clsLabel };
  const pattern = matchAutoApprove(code);
  if (pattern !== null) return { kind: "auto-config", pattern };
  return { kind: "ask" };
}
