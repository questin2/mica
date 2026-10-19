// 会话头部：通道切换 + 回合统计 + 诊断/自检/审计入口 —— 拆分自 Shell.tsx（S-01）
import { Icon } from "../icons";
import { ModelSwitcher } from "./ModelSwitcher";
import type { ProvidersPayload } from "../types";

type Props = {
  turns: number;
  messageCount: number;
  diagOpen: boolean;
  onToggleDiag: () => void;
  onSelfcheck: () => void;
  onOpenAudit: () => void;
  /** 开新对话：清空本地消息与存档 */
  onNewChat: () => void;
  providers: ProvidersPayload | null;
  busy: boolean;
  onProviderChange: (id: string, model?: string) => void;
  /** 当前会话的工作目录（每个会话独立，DSH 风格放在对话区内） */
  workdir: string;
  onPickWorkdir: () => void;
  onCopyWorkdir: () => void;
};

export function ChatHeader({
  turns,
  messageCount,
  diagOpen,
  onToggleDiag,
  onSelfcheck,
  onOpenAudit,
  onNewChat,
  providers,
  busy,
  onProviderChange,
  workdir,
  onPickWorkdir,
  onCopyWorkdir,
}: Props) {
  return (
    <div className="chat-header">
      {/* 内层容器与消息列表 / 输入框同宽居中（--w-chat-narrow），
          保证 rail 收起或浏览器栏变宽时，顶栏内容与下方窄栏始终对齐。 */}
      <div className="chat-header-inner">
        <span className="ch-title">对话</span>
        <ModelSwitcher providers={providers} busy={busy} onChange={onProviderChange} />
        <span className="ch-meta">
          {turns} 个用户回合 · {messageCount} 条消息
        </span>
        <div className="ch-actions">
          {/* 工作目录属于本会话：放在对话区内，切换会话会跟着变 */}
          <button
            className="btn ghost sm"
            title={`工作目录（本会话）：${workdir || "未设置"}　点击选择文件夹`}
            onClick={onPickWorkdir}
          >
            <Icon name="folder" size={14} />
            <span className="ellipsis" style={{ maxWidth: 160 }}>
              {workdir ? workdir.split(/[\\/]/).filter(Boolean).pop() : "选择目录"}
            </span>
          </button>
          <button className="btn ghost sm" title="复制完整工作目录路径" onClick={onCopyWorkdir} aria-label="复制工作目录">
            <Icon name="copy" size={14} />
          </button>
          <button className="btn ghost sm" onClick={onNewChat} title="开新对话：清空本地消息与存档">
            <Icon name="plus" size={14} />
            新对话
          </button>
          <button className="btn ghost sm" aria-pressed={diagOpen} onClick={onToggleDiag}>
            <Icon name="list" size={14} />
            诊断
          </button>
          <button className="btn ghost sm" onClick={onSelfcheck}>
            <Icon name="refresh" size={14} />
            自检
          </button>
          <button className="btn ghost sm" onClick={onOpenAudit}>
            <Icon name="shield" size={14} />
            审计
          </button>
        </div>
      </div>
    </div>
  );
}
