// 输入区：文本域 + 字符计数 + 测试回合 / 发送 —— 拆分自 Shell.tsx（S-01）
//
// 输入框自动增高（min 46 / max 200px，采编自 open-claude-ui RichComposer）也归本组件：
// 它才是 textarea 的所有者，容器不必持有一个只用于此处的高度调节 ref。
import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import { Icon } from "../icons";
import { SEND_LIMIT } from "../constants";
import { fmt } from "../format";

type AttachedFile = {
  name: string;
  path: string;
  bytes: number;
  /** 文本内容；二进制为 null */
  text: string | null;
  truncated: boolean;
};

type Props = {
  msg: string;
  setMsg: (v: string) => void;
  busy: boolean;
  enterSend: boolean;
  onBareRound: () => void;
  onSend: () => void;
};

export function Composer({ msg, setMsg, busy, enterSend, onBareRound, onSend }: Props) {
  const textareaRef = useRef<HTMLTextAreaElement>(null);
  // 附件：只把能当文本读的内容并入消息；二进制只报名字与大小
  const [files, setFiles] = useState<AttachedFile[]>([]);
  const [attaching, setAttaching] = useState(false);

  const pickFiles = async () => {
    setAttaching(true);
    try {
      const picked = await open({ multiple: true, title: "选择附件" });
      if (!picked) return;
      const paths = Array.isArray(picked) ? picked : [picked];
      const got = await invoke<AttachedFile[]>("read_attachments", { paths });
      // 同路径覆盖，避免重复添加
      setFiles((prev) => [...prev.filter((p) => !paths.includes(p.path)), ...got]);
    } catch (e) {
      console.error("附件读取失败", e);
    } finally {
      setAttaching(false);
    }
  };

  const removeFile = (path: string) => setFiles((prev) => prev.filter((f) => f.path !== path));

  /** 把附件拼成消息前缀：文本整段并入，二进制只留元信息 */
  const attachmentBlock = () =>
    files.length === 0
      ? ""
      : files
          .map((f) =>
            f.text === null
              ? `[附件] ${f.name}（${f.bytes} 字节，二进制，未读取内容；如需处理请用工具读取该路径：${f.path}）`
              : `[附件] ${f.name}\n\`\`\`text\n${f.text}${f.truncated ? "\n…（内容已截断）" : ""}\n\`\`\``,
          )
          .join("\n\n") + "\n\n";

  /** 发送前把附件并入消息。onSend 读的是父级 state，故等一帧再发。 */
  const sendWithAttachments = () => {
    if (files.length === 0) {
      onSend();
      return;
    }
    setMsg(attachmentBlock() + msg);
    setFiles([]);
    requestAnimationFrame(() => onSend());
  };

  // 随内容增高，超过 200px 交给内部滚动
  useEffect(() => {
    const el = textareaRef.current;
    if (!el) return;
    el.style.height = "auto";
    el.style.height = `${Math.min(Math.max(el.scrollHeight, 46), 200)}px`;
  }, [msg]);

  return (
    <div className="composer">
      <div className="composer-inner">
        <div className="composer-box">
          <textarea
            id="composer-input"
            ref={textareaRef}
            rows={1}
            value={msg}
            spellCheck={false}
            placeholder="给 Agent 的指令，例如：看看我桌面报表文件夹里有什么"
            aria-label="消息输入"
            onChange={(e) => setMsg(e.target.value)}
          />
          {files.length > 0 && (
            <div className="attach-chips">
              {files.map((f) => (
                <span className="attach-chip" key={f.path} title={`${f.path}${f.text ? "" : "（二进制，仅报元信息）"}`}>
                  <Icon name="attach" size={12} />
                  {f.name}
                  <span className="muted">
                    {f.bytes < 1024 ? `${f.bytes}B` : `${Math.round(f.bytes / 1024)}KB`}
                    {f.text ? "" : " · 非文本"}
                    {f.truncated ? " · 已截断" : ""}
                  </span>
                  <button className="chip-x" aria-label={`移除 ${f.name}`} onClick={() => removeFile(f.path)}>
                    ×
                  </button>
                </span>
              ))}
            </div>
          )}
          <div className="composer-actions">
            <button
              className="btn ghost sm"
              disabled={busy || attaching}
              title="添加附件（可多选；文本会并入消息，二进制只报名字与大小）"
              onClick={() => void pickFiles()}
            >
              <Icon name="attach" size={16} />
              {attaching ? "读取中…" : "附件"}
            </button>
            <span className="spacer" />
            <span className={`char-count tabular ${msg.length > SEND_LIMIT * 0.9 ? "warn" : ""}`}>
              {fmt(msg.length)} 字符
            </span>
            <button className="btn" disabled={busy} onClick={onBareRound} title="不经 Agent 循环的裸回合（诊断用）">
              测试回合
            </button>
            <button className="btn primary" disabled={busy} onClick={sendWithAttachments}>
              <Icon name={busy ? "stop" : "send"} size={16} />
              {busy ? "执行中…" : "发送指令"}
            </button>
          </div>
        </div>
        <div className="composer-hint">
          <span className="kbd">{enterSend ? "Enter 发送" : "Ctrl+Enter 发送"}</span>
          <span className="kbd">Shift+Enter 换行</span>
          <span className="kbd">Esc 拒绝</span>
          <span className="kbd">Ctrl+. 急停</span>
          <span className="hint-source" title="DeepSeek 网页仅作观察窗；消息统一从 App 发送">
            <Icon name="monitor" size={14} />
            DeepSeek 网页是观察窗（Ctrl+J 开合）；消息从 App 发送
          </span>
        </div>
      </div>
    </div>
  );
}
