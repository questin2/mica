// 设置页 —— 拆分自 Shell.tsx（可扩展性报告 v2 · S-01）
//
// 纯展示 + 回调：所有开关状态由容器持有并持久化（localStorage），组件不自行读写。
import { Icon } from "../icons";
import { KEYS } from "../constants";
import { MemoryPanel } from "./MemoryPanel";
import { UpdatePanel } from "./UpdatePanel";
import type { AppPaths, MotionMode, SecurityStatus, ThemeMode } from "../types";

type Props = {
  theme: ThemeMode;
  setTheme: (t: ThemeMode) => void;
  /** 跳到其它页面（知识库 / 流程 / 审计 已从主导航移到这里） */
  onSetView?: (v: "knowledge" | "workflows" | "audit") => void;
  glass: string;
  setGlass: (v: string) => void;
  density: string;
  setDensity: (v: string) => void;
  motion: string;
  setMotion: (v: MotionMode) => void;
  enterSend: boolean;
  setEnterSend: (fn: (v: boolean) => boolean) => void;
  channel: { text: string; ok: boolean | null };
  paths: AppPaths | null;
  security: SecurityStatus | null;
  onReloadSecurity: () => void;
  onSelfcheck: () => void;
};

const GLASS_LABELS = ["实色", "轻玻璃", "中玻璃", "强玻璃"];

export function SettingsPage({
  theme,
  setTheme,
  glass,
  setGlass,
  density,
  setDensity,
  motion,
  setMotion,
  enterSend,
  setEnterSend,
  channel,
  paths,
  security,
  onReloadSecurity,
  onSelfcheck,
  onSetView,
}: Props) {
  return (
    <div className="page-inner">
      <h1 className="page-title">设置</h1>
      <p className="page-sub">调整外观与行为偏好</p>

      {/* 知识库 / 流程 / 审计 已从主导航移到此处 —— 主导航只留高频入口 */}
      {onSetView && (
        <div className="section">
          <h2 className="section-title">更多</h2>
          <div className="card" style={{ display: "flex", gap: "var(--s-2)", flexWrap: "wrap" }}>
            <button className="btn sm" onClick={() => onSetView("knowledge")}>
              <Icon name="book" size={14} />
              知识库
            </button>
            <button className="btn sm" onClick={() => onSetView("workflows")}>
              <Icon name="flow" size={14} />
              流程
            </button>
            <button className="btn sm" onClick={() => onSetView("audit")}>
              <Icon name="shield" size={14} />
              审计
            </button>
          </div>
        </div>
      )}

      <div className="section">
        <h2 className="section-title">外观</h2>
        <div className="card">
          <div className="field">
            <div className="field-label">
              <b>主题</b>
              
            </div>
            <div className="field-control">
              <select value={theme} onChange={(e) => setTheme(e.target.value as ThemeMode)} aria-label="主题">
                <option value="system">跟随系统</option>
                <option value="dark">强制暗色</option>
                <option value="light">强制浅色</option>
              </select>
            </div>
          </div>
          <div className="field">
            <div className="field-label">
              <b>面板玻璃强度</b>
              <span>只影响顶栏和面板标题</span>
            </div>
            <div className="field-control">
              <input
                type="range"
                min={0}
                max={3}
                step={1}
                value={Number(glass)}
                onChange={(e) => setGlass(e.target.value)}
                aria-label="面板玻璃强度"
              />
              <span className="badge tabular">{GLASS_LABELS[Number(glass)]}</span>
            </div>
          </div>
          <div className="field">
            <div className="field-label">
              <b>密度</b>
              <span>舒适 / 紧凑</span>
            </div>
            <div className="field-control">
              <select value={density} onChange={(e) => setDensity(e.target.value)} aria-label="界面密度">
                <option value="cozy">舒适</option>
                <option value="compact">紧凑</option>
              </select>
            </div>
          </div>
          <div className="field">
            <div className="field-label">
              <b>动效强度</b>
              <span>「关」等同减弱动效；同时遵循系统的减少动态效果设置</span>
            </div>
            <div className="field-control">
              <div role="radiogroup" aria-label="动效强度" style={{ display: "flex", gap: "var(--s-1)" }}>
                {([
                  ["full", "完整"],
                  ["light", "轻"],
                  ["reduced", "关"],
                ] as const).map(([v, label]) => (
                  <button
                    key={v}
                    className="btn sm"
                    role="radio"
                    aria-checked={motion === v}
                    aria-pressed={motion === v}
                    onClick={() => setMotion(v)}
                    style={motion === v ? { borderColor: "var(--c-brand)", color: "var(--c-brand)" } : undefined}
                  >
                    {label}
                  </button>
                ))}
              </div>
            </div>
          </div>
        </div>
      </div>

      <div className="section">
        <h2 className="section-title">行为</h2>
        <div className="card">
          <div className="field">
            <div className="field-label">
              <b>回车发送</b>
              <span>关闭后用 Ctrl+Enter 发送</span>
            </div>
            <div className="field-control">
              <button
                className="switch"
                role="switch"
                aria-checked={enterSend}
                aria-label="回车发送"
                onClick={() => setEnterSend((v) => !v)}
              />
            </div>
          </div>
          <div className="field">
            <div className="field-label">
              <b>允许自动执行</b>
              <span>引擎侧无自动批准旁路；只读/普通类可在会话内「全部允许」，危险类永不自动放行</span>
            </div>
            <div className="field-control">
              <span className="badge">会话内按类信任（见审批卡）</span>
            </div>
          </div>
          <div className="field">
            <div className="field-label">
              <b>隐藏命令窗口</b>
              <span>命令执行时不弹出黑窗</span>
            </div>
            <div className="field-control">
              <span className="badge ok">已生效（引擎层）</span>
            </div>
          </div>
        </div>
      </div>

      <div className="section">
        <h2 className="section-title">引擎</h2>
        <div className="card">
          <div className="field">
            <div className="field-label">
              <b>完成判定</b>
              <span>
                主信号 = 回复流（SSE）loadingFinished；读回 = Network.getResponseBody 解析 SSE；
                兜底 = DOM 文本稳定 + 回声守卫
              </span>
            </div>
            <div className="field-control">
              <span className="badge ok">网络层</span>
            </div>
          </div>
          <div className="field">
            <div className="field-label">
              <b>通道自检</b>
              <span>只读统计选择器命中；user 组依赖混淆类名（高危）—— 现仅作兜底</span>
            </div>
            <div className="field-control">
              <span className={`badge ${channel.ok === false ? "warn" : channel.ok ? "ok" : ""}`}>{channel.text}</span>
              <button className="btn sm" onClick={onSelfcheck}>
                <Icon name="refresh" size={14} />
                运行自检
              </button>
            </div>
          </div>
          <div className="field">
            <div className="field-label">
              <b>本地端口</b>
              <span>进程内 CallDevToolsProtocolMethod 直调，无任何监听端口</span>
            </div>
            <div className="field-control">
              <span className="badge ok">0 个</span>
            </div>
          </div>
          <div className="field">
            <div className="field-label">
              <b>数据目录</b>
              <span>DB / 日志 / 知识库 / WebView2 profile；按可用性自动回退</span>
            </div>
            <div className="field-control">
              <span className="mono muted ellipsis" title={paths?.data_dir ?? ""}>
                {paths?.data_dir ?? "读取中…"}
              </span>
            </div>
          </div>
          <div className="field">
            <div className="field-label">
              <b>缓存目录</b>
              <span>WebView2 profile / npm 缓存；可重建，不写 Roaming</span>
            </div>
            <div className="field-control">
              <span className="mono muted ellipsis" title={paths?.cache_dir ?? ""}>
                {paths?.cache_dir ?? "读取中…"}
              </span>
            </div>
          </div>
          <div className="field">
            <div className="field-label">
              <b>WebView2 profile</b>
              <span>登录态持久保存，重启免登录</span>
            </div>
            <div className="field-control">
              <span className="mono muted ellipsis" title={paths?.webview_dir ?? ""}>
                {paths?.webview_dir ?? "读取中…"}
              </span>
            </div>
          </div>
          <div className="field">
            <div className="field-label">
              <b>路径可写性探测</b>
              <span>启动时逐级写探针；不可写会回退并在启动日志留痕</span>
            </div>
            <div className="field-control">
              <span className="mono muted ellipsis" title={paths?.path_probe ?? ""}>
                {paths?.path_probe || "读取中…"}
              </span>
            </div>
          </div>
        </div>
      </div>

      <div className="section">
        <h2 className="section-title">安全配置</h2>
        <div className="card">
          <div className="field">
            <div className="field-label">
              <b>配置快照</b>
              <span>
                agent.json / approval.json / mcp.json / provider.json 的安全字段在启动时冻结；
                磁盘上的外部改动需在这里显式确认后生效，未确认时使用安全默认
              </span>
            </div>
            <div className="field-control">
              <span className={`badge ${security?.changed ? "warn" : security ? "ok" : ""}`}>
                {security
                  ? security.changed
                    ? "检测到未确认变更（已回退安全默认）"
                    : "已确认"
                  : "读取中…"}
              </span>
              <button className="btn sm" onClick={onReloadSecurity} disabled={!security}>
                <Icon name="refresh" size={14} />
                重新加载安全配置
              </button>
            </div>
          </div>
          <div className="field">
            <div className="field-label">
              <b>快照指纹</b>
              <span>已确认 / 磁盘当前；不一致说明有未确认改动</span>
            </div>
            <div className="field-control">
              <span
                className="mono muted ellipsis"
                title={`${security?.loaded_hash ?? ""} / ${security?.disk_hash ?? ""}`}
              >
                {security
                  ? `${security.loaded_hash.slice(0, 12)}… / ${security.disk_hash.slice(0, 12)}…`
                  : "—"}
              </span>
              <span className="muted">
                {security ? new Date(security.loaded_at_ms).toLocaleString() : ""}
              </span>
            </div>
          </div>
        </div>
      </div>

      <UpdatePanel />

      <MemoryPanel />

      <div className="section">
        <h2 className="section-title">关于</h2>
        <div className="card">
          <p className="card-title">Mica</p>
          <p className="card-desc">
            Mica（本客户端）｜ 参考实现：Mica Legacy v2.2.0.0（易语言）｜ 作用：能力对照与防漂移校验
          </p>
          <div className="sep" />
          <div className="row gap-2 wrap">
            <span className="badge">v0.2.0</span>
            <span className="badge">Tauri 2.12.4 · WebView2</span>
            <span className="badge">GPL-3.0</span>
            <span className="badge">UI 依据 docs/07 v1.0</span>
            <span className="badge">命名 ADR-0003</span>
          </div>
          <div className="sep" />
          <p className="card-desc">快捷键：{KEYS.map((k) => `${k[0]} ${k[1]}`).join(" · ")}</p>
        </div>
      </div>
    </div>
  );
}
