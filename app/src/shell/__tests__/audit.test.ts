/**
 * 审计留痕文案测试（可扩展性报告 v2 · S-04；对应"审批白名单"的**逐条审计**要求）
 *
 * 这些字符串会落进磁盘 `audit.log`，是**事后追责的唯一依据**。因此它们的
 * 关键性质必须被测试钉住：
 *   - 自动放行与人工确认**在文本上可区分**（否则审计等于没有）；
 *   - 自动放行的记录必须写明"非人工逐条确认"与"危险类永不进入"；
 *   - 信任集变化必须留痕（自动放行面的每一次扩大/收缩都有据可查）。
 */
import { describe, expect, it } from "vitest";
import { autoApprovedMessage, configAutoApprovedMessage, manualDecisionMessage, trustChangedMessage } from "../audit";

describe("审计文案：自动放行 vs 人工确认必须可区分", () => {
  it("自动放行文案含明确来源标注", () => {
    const m = autoApprovedMessage(7, "只读");
    expect(m).toContain("会话信任自动放行");
    expect(m).toContain("#7");
    expect(m).toContain("只读");
  });

  it("自动放行文案必须写明「非人工逐条确认」", () => {
    expect(autoApprovedMessage(1, "普通")).toContain("非人工逐条确认");
  });

  it("自动放行文案必须写明危险类边界", () => {
    const m = autoApprovedMessage(1, "只读");
    expect(m).toContain("危险类永不进入此路径");
  });

  it("人工文案含明确来源标注（允许 / 拒绝都对）", () => {
    expect(manualDecisionMessage(3, true, "普通")).toContain("人工允许");
    expect(manualDecisionMessage(3, false, "普通")).toContain("人工拒绝");
    expect(manualDecisionMessage(3, true, "普通")).toContain("#3");
  });

  it("★ 两者的来源关键词互不串味（审计可据此筛选）", () => {
    const auto = autoApprovedMessage(9, "只读");
    const manual = manualDecisionMessage(9, true, "只读");
    expect(auto).toContain("自动放行");
    expect(auto).not.toContain("人工允许");
    expect(manual).toContain("人工允许");
    expect(manual).not.toContain("自动放行");
    expect(auto).not.toBe(manual);
  });
});

describe("审计文案：配置规则自动放行（读法 A）", () => {
  it("标注为「配置规则自动放行」，与会话信任可区分", () => {
    const m = configAutoApprovedMessage(5, "^\\\\s*Get-ChildItem");
    expect(m).toContain("配置规则自动放行");
    expect(m).toContain("#5");
    expect(m).not.toContain("会话信任自动放行");
  });

  it("★ 必须记下命中的规则原文（事后要知道当时是哪条规则放行的）", () => {
    const pattern = "^\\\\s*Get-ChildItem";
    expect(configAutoApprovedMessage(5, pattern)).toContain(pattern);
  });

  it("★ 必须写明用户未逐条看到该命令", () => {
    expect(configAutoApprovedMessage(1, "x")).toContain("用户未逐条看到该命令");
  });

  it("★ 必须提示配置规则跨重启持续生效（区别于会话信任）", () => {
    expect(configAutoApprovedMessage(1, "x")).toContain("跨重启持续生效");
  });

  it("必须写明危险类边界", () => {
    expect(configAutoApprovedMessage(1, "x")).toContain("危险类永不进入此路径");
  });

  it("三种自动/人工来源两两可区分（审计可据此分类）", () => {
    const cfg = configAutoApprovedMessage(9, "p");
    const sess = autoApprovedMessage(9, "只读");
    const man = manualDecisionMessage(9, true, "只读");
    expect(new Set([cfg, sess, man]).size).toBe(3);
    expect(cfg).toContain("配置规则");
    expect(sess).toContain("会话信任");
    expect(man).toContain("人工允许");
  });
});

describe("审计文案：信任集变化留痕", () => {  it("开启信任写明类别与「不再逐条询问」", () => {
    const m = trustChangedMessage("grant", "普通");
    expect(m).toContain("信任集变更");
    expect(m).toContain("开启会话信任");
    expect(m).toContain("普通");
    expect(m).toContain("不再逐条询问");
  });

  it("撤销信任写明恢复逐条询问", () => {
    const m = trustChangedMessage("revoke", "只读");
    expect(m).toContain("撤销会话信任");
    expect(m).toContain("恢复逐条询问");
  });

  it("清空信任不依赖类别参数", () => {
    const m = trustChangedMessage("clear");
    expect(m).toContain("清空全部会话信任");
    expect(m).toContain("所有类别恢复逐条询问");
  });

  it("三种动作的文案两两不同（不会在日志里混淆）", () => {
    const g = trustChangedMessage("grant", "普通");
    const r = trustChangedMessage("revoke", "普通");
    const c = trustChangedMessage("clear", "普通");
    expect(new Set([g, r, c]).size).toBe(3);
  });
});

describe("审计文案：格式约束", () => {
  it("不含制表/回车等会破坏日志行结构的字符（换行除外）", () => {
    for (const m of [
      autoApprovedMessage(1, "只读"),
      manualDecisionMessage(1, true, "普通"),
      trustChangedMessage("grant", "普通"),
    ]) {
      expect(m).not.toContain("\r");
      expect(m).not.toContain("\t");
    }
  });

  it("自动放行文案为多行结构（首行为来源摘要，便于 grep）", () => {
    const lines = autoApprovedMessage(5, "普通").split("\n");
    expect(lines.length).toBeGreaterThan(1);
    expect(lines[0]).toContain("审批来源");
  });

  it("id 变化会反映到文案里（逐条可追溯）", () => {
    expect(autoApprovedMessage(1, "只读")).not.toBe(autoApprovedMessage(2, "只读"));
  });
});
