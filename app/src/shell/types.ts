// 共享类型（拆分自 Shell.tsx · 可扩展性报告 v2 · S-01）
export type EngineEvent = { seq: number; ts_ms: number; phase: string; detail: string };
export type ToolReq = { id: number; code: string };
export type DoneTool = { id: number; code: string; allow: boolean; ts: string };
export type Msg = {
  id: number;
  role: "user" | "assistant";
  text: string;
  ts: string;
  /** 本条回复自己那一组运行步骤（问题里要"一个对话一个"） */
  steps?: Step[];
};
export type LogRow = { seq: number; t: string; phase: string; detail: string };
export type View =
  | "chat"
  | "history"
  | "skills"
  | "plugins"
  | "mcp"
  | "knowledge"
  | "workflows"
  | "audit"
  | "settings";
export type StepKind = "think" | "tool" | "phase";
export type Step = { id: number; kind: StepKind; text: string; ts: string };
export type PermissionMode = "readonly" | "workspace" | "smart" | "full";
export type MotionMode = "full" | "light" | "reduced";
export type ThemeMode = "system" | "dark" | "light";
export type Risk = "readonly" | "normal" | "danger";
export type Toast = { id: number; kind: "info" | "ok" | "warn" | "danger"; text: string };
/** 左栏导航项：[视图, 图标名, 文案, 徽标数（null = 不显示）] */
export type NavItem = [View, string, string, number | null];

/* ---------- Phase 2：对话通道 ---------- */
export type ProviderInfo = {
  id: string;
  label: string;
  kind: string;
  active: boolean;
  available: boolean;
  streaming: boolean;
  model: string | null;
  models: string[];
  error: string | null;
};
export type ProvidersPayload = { active: string; providers: ProviderInfo[] };
/** 安全配置快照状态（Rust `SecurityConfigStatus`）。 */
export type SecurityStatus = {
  loaded_hash: string;
  disk_hash: string;
  changed: boolean;
  loaded_at_ms: number;
};

export type AppPaths = {
  data_dir: string;
  cache_dir: string;
  webview_dir: string;
  default_workdir: string;
  path_probe: string;
};

/* ---------- Phase 3：会话 / 知识库 ---------- */
export type SessionRow = {
  id: string;
  provider: string;
  title: string;
  workdir: string;
  created_at: string;
  updated_at: string;
  message_count: number;
};
export type MessageRow = { id: number; role: string; content: string; ts: string };
export type KbDoc = { id: number; path: string; title: string; added_at: string; chunk_count: number };
export type KbHit = {
  doc_id: number;
  ord: number;
  title: string;
  path: string;
  text: string;
  score: number;
};

/* ---------- Phase 4：流程 ---------- */
export type WorkflowStep = { title: string; prompt: string };
export type Workflow = {
  id: string;
  name: string;
  description: string;
  steps: WorkflowStep[];
};
export type WorkflowEvent = {
  run_id: string;
  workflow_id: string;
  status: string;
  step: number;
  total: number;
  title: string;
  detail: string;
};
export type WorkflowRun = {
  id: string;
  workflow_id: string;
  status: string;
  current_step: number;
  total_steps: number;
  created_at: string;
  updated_at: string;
  error: string | null;
};
