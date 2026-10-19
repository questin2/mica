// 审计留痕：把**审批来源**写进磁盘 `audit.log`。
//
// 为什么需要它：会话信任的自动放行发生在前端（引擎只看到"被批准了"）。
// 若前端不回报，磁盘审计就**分不出「人工点击允许」与「会话信任自动放行」**——
// 而这恰恰是审计链最该留痕的一处。
//
// 设计：
// - **文案构造与写盘分离**（`*Message` 为纯函数，可单测；`audit*` 负责写盘）；
// - **尽力而为**：审计写失败绝不影响审批与执行主流程（失败静默，只在控制台提示）。
import { invoke } from "@tauri-apps/api/core";

export type TrustAction = "grant" | "revoke" | "clear";

export function logAudit(text: string): void {
  void invoke("log_audit", { text }).catch((e) => {
    console.warn("audit 写入失败（不影响主流程）", e);
  });
}

/** 会话信任自动放行的留痕文案。必须让人一眼看出「这不是人工确认的」。 */
export function autoApprovedMessage(id: number, clsLabel: string): string {
  return (
    `审批来源：会话信任自动放行 #${id}（类别 ${clsLabel}）\n` +
    `说明：用户此前在该会话内显式选择过「此类命令全部允许」，本次非人工逐条确认。\n` +
    `边界：危险类永不进入此路径（classify 判定 + 入集校验双重保证）。`
  );
}

/**
 * **配置规则自动放行**的留痕文案（`config/agent.json` → `autoApprovePatterns`）。
 *
 * 与会话信任分开标注：会话信任随会话消失，配置规则**会跨重启持续生效**，
 * 追责时需要知道当时究竟命中了哪一条规则，因此把规则原文一并落盘。
 */
export function configAutoApprovedMessage(id: number, pattern: string): string {
  return (
    `审批来源：配置规则自动放行 #${id}\n` +
    `命中规则（config/agent.json → autoApprovePatterns）：${pattern}\n` +
    `说明：本次非人工确认，用户未逐条看到该命令；配置规则跨重启持续生效。\n` +
    `边界：危险类永不进入此路径（classify 判定 + 内置危险规则回退保证）。`
  );
}

/** 人工决策（点击审批卡 / 键盘确认 / 急停按拒绝处理）的留痕文案。 */
export function manualDecisionMessage(id: number, allow: boolean, clsLabel: string): string {
  return `审批来源：人工${allow ? "允许" : "拒绝"} #${id}（类别 ${clsLabel}）`;
}

/** 会话信任集合变化（开启 / 撤销 / 清空）的留痕文案。 */
export function trustChangedMessage(action: TrustAction, clsLabel?: string): string {
  const what =
    action === "grant"
      ? `开启会话信任：${clsLabel} 类命令本会话内不再逐条询问`
      : action === "revoke"
        ? `撤销会话信任：${clsLabel} 类恢复逐条询问`
        : "清空全部会话信任：所有类别恢复逐条询问";
  return `信任集变更：${what}`;
}

export function auditAutoApproved(id: number, clsLabel: string): void {
  logAudit(autoApprovedMessage(id, clsLabel));
}

/** 配置规则自动放行 —— 记录命中的规则原文，便于事后追责。 */
export function auditConfigAutoApproved(id: number, pattern: string): void {
  logAudit(configAutoApprovedMessage(id, pattern));
}

export function auditManualDecision(id: number, allow: boolean, clsLabel: string): void {
  logAudit(manualDecisionMessage(id, allow, clsLabel));
}

export function auditTrustChanged(action: TrustAction, clsLabel?: string): void {
  logAudit(trustChangedMessage(action, clsLabel));
}
