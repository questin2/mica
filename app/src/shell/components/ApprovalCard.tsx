// 审批卡（DSH/Claude 式「内联编号选项」）—— 拆分自 Shell.tsx（可扩展性报告 v2 · S-01）
// 纯展示组件：不持有状态；键盘导航仍由容器（Shell）的全局键盘处理器驱动。
import { Icon } from "../icons";
import { fmt, highlight } from "../format";
import type { Risk, ToolReq } from "../types";

/** 一个审批选项（由容器按风险等级构造）。 */
export type ApprovalOption = {
  key: "allow" | "allowAll" | "deny";
  label: string;
  hint?: string;
  tone: string;
};

type Props = {
  toolReq: ToolReq;
  risk: Risk;
  options: ApprovalOption[];
  activeIdx: number;
  onHover: (index: number) => void;
  onChoose: (key: ApprovalOption["key"]) => void;
};

export function ApprovalCard({ toolReq, risk, options, activeIdx, onHover, onChoose }: Props) {
  return (
    <div className={`appr risk-${risk}`} role="alertdialog" aria-label="命令执行确认">
      <div className="appr-head">
        <Icon name={risk === "danger" ? "shield-alert" : "terminal"} size={16} />
        <span className="appr-title">PowerShell 命令</span>
        <span className={`badge risk-${risk}`}>
          <Icon name={risk === "danger" ? "shield-alert" : risk === "readonly" ? "shield" : "zap"} size={12} />
          {risk === "danger" ? "危险" : risk === "readonly" ? "只读" : "普通"}
        </span>
        <span className="badge">#{toolReq.id}</span>
        <span className="appr-meta">{fmt(toolReq.code.length)} / 4,000 字符</span>
      </div>

      <div className="appr-code">
        <pre className="code-body" dangerouslySetInnerHTML={{ __html: highlight(toolReq.code) }} />
      </div>

      {risk === "danger" && (
        <div className="appr-warn">
          <Icon name="shield-alert" size={14} />
          <span>命中危险红名单（批量删除 / 格式化 / 注册表写 / 下载即执行 / 提权链类），可能不可逆。</span>
        </div>
      )}

      <div className="appr-ask">
        <span className="appr-q">是否执行这条命令？</span>
        <span className="appr-scope">
          {risk === "danger" ? "危险类不提供「全部允许」" : "执行结果会回注对话；决策写入审计"}
        </span>
      </div>

      <ul className="appr-opts" role="listbox" aria-label="审批选项">
        {options.map((o, i) => (
          <li key={o.key} role="option" aria-selected={i === activeIdx}>
            <button
              className={`appr-opt tone-${o.tone}${i === activeIdx ? " sel" : ""}`}
              onMouseEnter={() => onHover(i)}
              onClick={() => onChoose(o.key)}
            >
              <span className="appr-num">{i + 1}</span>
              <span className="appr-label">{o.label}</span>
              {o.hint && <span className="appr-hint">{o.hint}</span>}
            </button>
          </li>
        ))}
      </ul>

      <div className="appr-keys">
        <span className="kbd">↑↓</span> 选择
        <span className="kbd">Enter</span> 确认
        <span className="kbd">1-{options.length}</span> 直选
        <span className="kbd">Esc</span> 拒绝
      </div>
    </div>
  );
}
