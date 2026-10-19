// 纯函数：风险分类 / 语法高亮 / 格式化 / localStorage / 摘要（拆分自 Shell.tsx）
import type { Risk } from "./types";
import { DANGER as BUILTIN_DANGER, READONLY as BUILTIN_READONLY } from "./constants";

/**
 * 生效中的风险规则。默认用内置常量（= 升级前行为，零变化）；
 * 启动时可由 `config/approval.json` 覆盖 —— 见 `setRiskRules`。
 */
let dangerRules: RegExp[] = BUILTIN_DANGER;
let readonlyRule: RegExp = BUILTIN_READONLY;

export type RiskRulesReport = {
  /** 采用的危险模式条数 */
  danger: number;
  /** 是否采用了自定义只读规则 */
  readonlyCustom: boolean;
  /** 被跳过的非法项数 */
  skipped: number;
  /** true = 自定义危险规则不可用，已回退内置 */
  dangerFallback: boolean;
};

/**
 * 用 `config/approval.json` 的内容覆盖风险规则。
 *
 * **安全兜底**：若自定义的危险列表为空或全部非法，则**回退内置列表**并把
 * `dangerFallback` 置真。理由：danger 分级一旦整体失效，「危险类不提供全部允许」
 * 这条保护、以及「危险类永不进入会话信任」这条不变式都会跟着失效 ——
 * 宁可回到内置规则，也不能静默地把危险分级关掉。
 *
 * 本函数**只影响分级，不影响放行**：审批门一道不减，所有工具照样需要用户确认。
 */
export function setRiskRules(raw: unknown): RiskRulesReport {
  const rep: RiskRulesReport = { danger: 0, readonlyCustom: false, skipped: 0, dangerFallback: false };
  const obj = (raw && typeof raw === "object" ? raw : {}) as { danger?: unknown; readonly?: unknown };

  const nextDanger: RegExp[] = [];
  if (Array.isArray(obj.danger)) {
    for (const p of obj.danger) {
      if (typeof p !== "string" || !p.trim()) {
        rep.skipped++;
        continue;
      }
      try {
        nextDanger.push(new RegExp(p, "i"));
      } catch {
        rep.skipped++;
      }
    }
  }

  if (nextDanger.length > 0) {
    dangerRules = nextDanger;
    rep.danger = nextDanger.length;
  } else {
    dangerRules = BUILTIN_DANGER;
    rep.danger = BUILTIN_DANGER.length;
    // 只有「配置里确实写了 danger 字段」才算一次回退告警；字段缺失属于正常情况
    rep.dangerFallback = Array.isArray(obj.danger);
  }

  if (typeof obj.readonly === "string" && obj.readonly.trim()) {
    try {
      readonlyRule = new RegExp(obj.readonly, "im");
      rep.readonlyCustom = true;
    } catch {
      rep.skipped++;
      readonlyRule = BUILTIN_READONLY;
    }
  } else {
    readonlyRule = BUILTIN_READONLY;
  }

  return rep;
}

/** 当前生效的危险模式条数（诊断用）。 */
export function dangerRuleCount(): number {
  return dangerRules.length;
}

/**
 * 引擎对"必须逐次确认"的工具（MCP / web_read）在审批文本前加这个前缀。
 * 归为危险类 → UI 不提供「本会话全部允许」，且自动放行规则一律不生效。
 */
export const FORCE_ASK_PREFIX = "[强制逐次确认]";

/** 单条命令里只要出现这些字符，就不允许进入只读/自动放行路径。 */
const RISKY_CHARS = new Set(["|", ";", ">", "<", "&", "$", "(", ")", "{", "}", "[", "]", "`"]);

/**
 * 保守的“白名单形状”解析：只允许**单条、非注释、不含管道/重定向/子表达式/脚本块/赋值**
 * 的命令进入 readonly/自动放行。命中返回 true（= 必须人工确认）。
 *
 * 这是自动放行链的核心：`Get-Content x | Set-Content config\agent.json` 这类
 * 以只读动词开头的复合命令必须在这里被拦下。
 * 引号里出现 `|`/`;` 也会被当成命中 —— 宁可多问一次。
 */
export function isCompoundOrRisky(code: string): boolean {
  const lines = code
    .split(/\r?\n/)
    .map((l) => l.trim())
    .filter((l) => l.length > 0 && !l.startsWith("#"));
  if (lines.length !== 1) return true;
  const line = lines[0];
  if ([...line].some((ch) => RISKY_CHARS.has(ch))) return true;
  if (line.includes("::")) return true;
  // 变量赋值（仅拦语句级；参数里的 name=value 不拦）
  if (/^\s*\$[A-Za-z_][\w:]*\s*=/.test(line)) return true;
  // 编码/别名/下载执行
  if (/(^|\s)-enc(odedcommand)?\b/i.test(line)) return true;
  if (/\b(iex|invoke-expression|start-process|invoke-item|cmd\s*\/c|powershell|pwsh|wsl|bash|sh\s+-c)\b/i.test(line)) return true;
  if (/\b(invoke-webrequest|invoke-restmethod|iwr|curl|wget|certutil)\b/i.test(line)) return true;
  // 写入类 cmdlet
  if (/\b(set-content|add-content|out-file|new-item|remove-item|copy-item|move-item|rename-item|clear-content|set-item|set-itemproperty|new-itemproperty|reg\s+add|tee-object)\b/i.test(line)) return true;
  // 写入/执行类别名（按 token 边界，避免误伤路径）
  if (/(^|\s)(sc|ac|clc|ni|ri|cp|mv|ren|epcsv|tee|rm|del|erase|rd|rmdir|mkdir|md|copy|move|xcopy|robocopy|start)(\s|$)/i.test(line)) return true;
  return false;
}

export function classify(code: string): Risk {
  if (code.startsWith(FORCE_ASK_PREFIX)) return "danger";
  if (dangerRules.some((r) => r.test(code))) return "danger";
  // 复合/危险形状一律 danger：审批卡默认停在拒绝，且不提供“本会话全部允许”。
  if (isCompoundOrRisky(code)) return "danger";
  if (readonlyRule.test(code)) return "readonly";
  return "normal";
}

export function esc(s: string): string {
  return s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;").replace(/"/g, "&quot;");
}
export const HL = /(#[^\n]*)|('(?:[^']|'')*'|"(?:[^"]|"")*")|(\s-{1,2}[A-Za-z][\w-]*)|\b(\d+(?:\.\d+)?)\b/g;
export function highlight(code: string): string {
  return esc(code).replace(HL, (m, cmt?: string, str?: string, param?: string, num?: string) => {
    if (cmt) return `<span class="c">${cmt}</span>`;
    if (str) return `<span class="s">${str}</span>`;
    if (param) return ` <span class="k">${param.slice(1)}</span>`;
    if (num) return `<span class="n">${num}</span>`;
    return m;
  });
}
export function hm(): string {
  const d = new Date();
  return `${String(d.getHours()).padStart(2, "0")}:${String(d.getMinutes()).padStart(2, "0")}`;
}
export function mmss(ms: number): string {
  const s = Math.max(0, Math.floor(ms / 1000));
  return `${String(Math.floor(s / 60)).padStart(2, "0")}:${String(s % 60).padStart(2, "0")}`;
}
export function fmt(n: number): string {
  return String(n).replace(/\B(?=(\d{3})+(?!\d))/g, ",");
}
export function lsGet(k: string, d: string): string {
  try {
    return localStorage.getItem(`eagent.${k}`) ?? d;
  } catch {
    return d;
  }
}
export function lsSet(k: string, v: string): void {
  try {
    localStorage.setItem(`eagent.${k}`, v);
  } catch {
    /* 隐私模式等场景静默忽略 */
  }
}
export function baseName(p: string): string {
  const parts = p.split(/[\\/]/).filter(Boolean);
  return parts.length ? parts[parts.length - 1] : p;
}

/**
 * 从 PowerShell 命令里提取一行摘要，用于「已决策工具行」的标题。
 * 优先取命令头部的 `# 操作：…` 注释（引擎提示词要求模型这么写），
 * 其次 `# 描述：…`，再次第一行真实代码。
 */
export function summarizeCode(code: string): string {
  const lines = code
    .split("\n")
    .map((l) => l.trim())
    .filter((l) => l && !l.startsWith(FORCE_ASK_PREFIX));
  const op = lines.find((l) => /^#\s*操作\s*[:：]/.test(l));
  if (op) return op.replace(/^#\s*操作\s*[:：]\s*/, "").trim();
  const desc = lines.find((l) => /^#\s*描述\s*[:：]/.test(l));
  if (desc) return desc.replace(/^#\s*描述\s*[:：]\s*/, "").trim();
  return lines.find((l) => !l.startsWith("#")) ?? lines[0] ?? "(空命令)";
}
