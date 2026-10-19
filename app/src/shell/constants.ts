// 常量与静态数据（拆分自 Shell.tsx · 可扩展性报告 v2 · S-01）
export const PHASE_TEXT: Record<string, string> = {
  idle: "空闲",
  "chain-test": "链路自测中",
  injecting: "注入中",
  generating: "生成中",
  finalizing: "收尾中",
  done: "已完成",
  timeout: "超时",
  error: "错误",
  tool: "工具执行",
  think: "思考中",
};
export const DOT_STATE: Record<string, string> = {
  idle: "connected",
  "chain-test": "generating",
  injecting: "generating",
  generating: "generating",
  finalizing: "generating",
  tool: "waiting",
  think: "generating",
  done: "connected",
  timeout: "waiting",
  error: "error",
};
export const BUSY = ["chain-test", "injecting", "generating", "finalizing", "tool", "think"];
export const SEND_LIMIT = 4000;

/* 危险红名单（策划案 §5.1 R6 初版，需人工复核后固化到配置） */
export const DANGER: RegExp[] = [
  /remove-item[^\n]*-recurse/i,
  /remove-item[^\n]*-force/i,
  /\brmdir\b[^\n]*\/s/i,
  /\brd\b[^\n]*\/s/i,
  /\bdel\b[^\n]*\/[fsq]/i,
  /format-volume|\bformat\s+[a-z]:/i,
  /diskpart|\bcipher\s+\/w\b/i,
  /reg\s+(add|delete|import)/i,
  /set-itemproperty[^\n]*hklm/i,
  /new-itemproperty[^\n]*hklm/i,
  /(invoke-webrequest|invoke-restmethod|\biwr\b|curl)[^\n]*\|[^\n]*\b(iex|invoke-expression)\b/i,
  /set-executionpolicy/i,
  /takeown|\bicacls\b[^\n]*\/grant/i,
  /schtasks[^\n]*\/create/i,
  /stop-computer|restart-computer|\bshutdown\b/i,
  /start-process[^\n]*-verb\s+runas/i,
];
// 只读白名单：刻意不含 out-（Out-File 会写文件）、foreach-/where-/sort-/format-
// （可执行脚本块）。任何复合命令都会在 classify 里被 isCompoundOrRisky 拦成 danger。
export const READONLY = /^\s*(get-|select-|test-|measure-|resolve-|compare-|write-output|dir\b|ls\b|gci\b|cat\b|type\b)/im;

export const KEYS: [string, string][] = [
  ["Enter", "发送指令"],
  ["Ctrl+Enter", "发送（回车发送关闭时）"],
  ["Esc", "拒绝当前审批"],
  ["Ctrl+.", "急停"],
  ["Ctrl+B / Ctrl+J", "折叠左栏 / 右栏"],
  ["Ctrl+Shift+T", "通道自检"],
  ["Ctrl+N", "清空输入（新会话待 M2）"],
];
