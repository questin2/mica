// 对话通道切换（策划案 2.0 · Phase 2.3）
//
// 一个下拉同时承担"通道 + Ollama 模型"选择：值形如 `ollama::qwen2.5:7b`。
// 组件只做展示与回调，切换确认/清空会话由容器负责。
import type { ProvidersPayload } from "../types";

type Props = {
  providers: ProvidersPayload | null;
  busy: boolean;
  onChange: (id: string, model?: string) => void;
};

export function ModelSwitcher({ providers, busy, onChange }: Props) {
  if (!providers) {
    return <span className="badge">通道读取中…</span>;
  }
  const ollama = providers.providers.find((p) => p.id === "ollama");
  const value =
    providers.active === "ollama" && ollama?.model
      ? `ollama::${ollama.model}`
      : "deepseek-web";
  const models = ollama?.models ?? [];
  return (
    <span className="model-switcher" title={ollama?.error ?? "切换对话通道（会开启新会话）"}>
      <select
        className="provider-select"
        value={value}
        disabled={busy}
        aria-label="对话通道"
        onChange={(e) => {
          const v = e.target.value;
          if (v.startsWith("ollama::")) onChange("ollama", v.slice("ollama::".length));
          else onChange("deepseek-web");
        }}
      >
        <option value="deepseek-web">DeepSeek 网页（免费）</option>
        {models.map((m) => (
          <option key={m} value={`ollama::${m}`}>
            Ollama: {m}
            {ollama?.available ? "" : "（不可用）"}
          </option>
        ))}
        {models.length === 0 && <option value="ollama::qwen2.5:7b">Ollama（不可用）</option>}
      </select>
      {ollama && !ollama.available && (
        <span className="badge warn" title={ollama.error ?? ""}>
          Ollama 不可用
        </span>
      )}
    </span>
  );
}
