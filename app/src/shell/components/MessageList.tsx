// 消息流：滚动区 + 空态 + 消息渲染 + 执行中指示 + 「跳至最新」—— 拆分自 Shell.tsx（S-01）
//
// 用 **children** 承接「审批卡 / 已决策命令」区域：那两块由容器装配，
// 避免把 risk、approvalOptions、doneTools 等一堆状态灌进本组件。
import { Fragment, useEffect, useRef, useState, type ReactNode } from "react";
import { Icon } from "../icons";
import { ScrollFade } from "../ScrollFade";
import { useScrollPinning } from "../useScrollPinning";
import { PHASE_TEXT } from "../constants";
import { PHASE_QUIPS, stepQuip } from "../quips";
import Markdown from "../Markdown";
import type { Msg, Step } from "../types";

type Props = {
  messages: Msg[];
  busy: boolean;
  phase: string;
  /** 内联步骤时间线 */
  steps: Step[];
  /** 本轮思考过程（空串则不渲染思考区） */
  thinking: string;
  detail: string;
  channel: { text: string; ok: boolean | null };
  pin: ReturnType<typeof useScrollPinning>;
  onCopy: (text: string) => void;
  /** 删除单条消息 */
  onDeleteMsg: (id: number) => void;
  onSelfcheck: () => void;
  onOpenAudit: () => void;
  onRunSession: () => void;
  /** 是否显示空态（容器算好：无消息且无待审批工具） */
  showEmpty: boolean;
  /** 审批卡 / 已决策命令等由容器注入 */
  children?: ReactNode;
};

export function MessageList({
  // eslint-disable-next-line @typescript-eslint/no-unused-vars
  messages,
  busy,
  phase,
  detail,
  channel,
  pin,
  onCopy,
  onDeleteMsg,
  onSelfcheck,
  onOpenAudit,
  onRunSession,
  showEmpty,
  children,
}: Props) {
  // 语音条：只在消息流下方那一行出现（顶栏不重复）。
  // 每 5s 换一句、不连出同一句；减弱动效时固定第一句不轮换。
  const [quip, setQuip] = useState("");
  const prevQuip = useRef<string | null>(null);
  useEffect(() => {
    const list = PHASE_QUIPS[phase] ?? PHASE_QUIPS.idle;
    const tick = () => {
      if (document.documentElement.getAttribute("data-motion") === "reduced") {
        setQuip(list[0] ?? "");
        return;
      }
      const q = stepQuip(list, prevQuip.current);
      prevQuip.current = q;
      setQuip(q);
    };
    tick();
    const t = window.setInterval(tick, 5000);
    return () => window.clearInterval(t);
  }, [phase]);

  // 运行步骤：抽成变量，插到"最后一条回复"的上方 —— 时间顺序是先想/先动手，再给答案。
  // 用原生 <details>：可折叠、无需 JS、键盘可达；默认展开（用户想看过程）。
  const renderTimeline = (list: Step[]) =>
    list.length > 0 ? (
      <details className="step-block" open>
        <summary>运行步骤 · {list.length} 步</summary>
        <ol className="stepline" aria-label="运行步骤">
          {list.map((st) => (
            <li key={st.id} className={`step step-${st.kind}`}>
              <span className="step-dot" aria-hidden="true" />
              <span className="step-kind">
                {st.kind === "think" ? "思考" : st.kind === "tool" ? "工具" : "阶段"}
              </span>
              <span className="step-text" title={st.text}>
                {st.text.length > 90 ? `${st.text.slice(0, 90)}…` : st.text}
              </span>
              <span className="step-ts">{st.ts}</span>
            </li>
          ))}
        </ol>
      </details>
    ) : null;

  return (
    <>
      <ScrollFade
        className="messages"
        containerRef={pin.containerRef}
        onScroll={pin.handleScroll}
        onWheel={pin.markUserInput}
        onTouchMove={pin.markUserInput}
        role="log"
        aria-live="polite"
        aria-relevant="additions text"
      >
        <div className="messages-inner">
          {channel.ok === false && (
            <div className="banner danger" role="alert">
              <Icon name="shield-alert" size={16} />
              <div>通道自检发现 0 命中选择器：DeepSeek 页面结构可能已改版，回合与工具可能失效。</div>
              <div className="banner-actions">
                <button className="btn sm primary" onClick={onSelfcheck}>
                  重新自检
                </button>
                <button className="btn sm" onClick={onOpenAudit}>
                  查看报告
                </button>
              </div>
            </div>
          )}

          {messages.length === 0 && showEmpty && (
            <div className="empty">
              <div className="empty-art">
                <Icon name="bot" size={48} />
              </div>
              <div className="empty-title">还没有对话</div>
              <div className="empty-desc">
                左栏填好工作目录 → 注入提示词 → 发送指令。工具执行会在这里弹出审批卡；
                DeepSeek 网页是观察窗（Ctrl+J 开合），消息统一从下方输入框发送。
              </div>
              <button className="btn primary" disabled={busy} onClick={onRunSession}>
                <Icon name="zap" size={16} />注入提示词
              </button>
            </div>
          )}

          {messages.map((m) => (
            <Fragment key={m.id}>
              {/* 步骤放在最后一条回复的上方：先有过程与思考，再有答案 */}
              {m.role === "assistant" && m.steps && m.steps.length > 0 && renderTimeline(m.steps)}
            <article className={`msg ${m.role}`} aria-label={m.role === "user" ? "用户消息" : "AI 回复"}>
              <div className="msg-avatar">
                <Icon name={m.role === "user" ? "user" : "bot"} size={16} />
              </div>
              <div className="msg-body">
                {/* 助手回复按 Markdown 渲染（Claude 式排版：标题/列表/表格/代码块）；
                    用户消息保持原文（预格式），避免用户输入里的符号被误解析。 */}
                <div className="msg-text">
                  {m.role === "assistant" ? <Markdown text={m.text} /> : m.text}
                </div>
                <div className="msg-meta">{m.ts}</div>
                <div className="msg-actions">
                  {m.role === "assistant" && (
                    <button className="btn ghost sm" onClick={() => onCopy(m.text)}>
                      <Icon name="copy" size={14} />
                      复制
                    </button>
                  )}
                  <button
                    className="btn ghost sm"
                    title="删除这条消息"
                    aria-label="删除这条消息"
                    onClick={() => onDeleteMsg(m.id)}
                  >
                    <Icon name="x" size={14} />
                    删除
                  </button>
                </div>
              </div>
            </article>
            </Fragment>
          ))}



          {busy && messages.length > 0 && (
            <div className="thinking" aria-label="执行中" aria-live="polite">
              <div className="thinking-dots" aria-hidden="true">
                <span />
                <span />
                <span />
              </div>
              <div className="thinking-text">
                <span className="phase-tag">{PHASE_TEXT[phase] ?? "处理中"}</span>
                {detail ? ` · ${detail.length > 60 ? `${detail.slice(0, 60)}…` : detail}` : ""}
              </div>
              {/* 语音条只在这一行（顶栏不再重复） */}
              {quip && <span className="sb-quip">{quip}</span>}
            </div>
          )}

          {children}
        </div>
      </ScrollFade>

      {pin.showJumpControl && (
        <button className="jump-latest" onClick={() => pin.scrollToBottom(true)} title="跳至最新消息">
          <Icon name="chevron-down" size={14} />
          跳至最新
        </button>
      )}
    </>
  );
}
