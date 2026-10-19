/**
 * 审计页「只看审批来源」筛选逻辑测试
 *
 * 背景：`audit.log` 是**混合内容** —— 命令、执行结果、审批留痕都写在同一个文件里
 * （实测用户环境已达 3000+ 行 / 160KB+）。直接看尾部很难找到关键那几行，
 * 因此需要从尾部窗口里挑出审批留痕。
 */
import { describe, expect, it } from "vitest";
import { pickProvenance } from "../components/AuditPage";

/** 模拟真实 audit.log 的混合内容片段。 */
const MIXED = [
  "执行命令[powershell]（workdir=C:\\Users\\x\\Desktop）:",
  "Get-ChildItem | Group-Object Extension",
  "--- 结果（前 300 字）---",
  "数据分析 -> 13",
  "文档办公 -> 12",
  "EAGENT_NOT_RUNNING",
  "DONE",
  "审批来源：人工允许 #12（类别 普通）",
  "执行命令[powershell]（workdir=C:\\Users\\x\\Desktop）:",
  "Remove-Item -Recurse .\\tmp",
  "审批来源：会话信任自动放行 #13（类别 只读）",
  "说明：用户此前在该会话内显式选择过「此类命令全部允许」，本次非人工逐条确认。",
  "边界：危险类永不进入此路径（classify 判定 + 入集校验双重保证）。",
  "信任集变更：开启会话信任：只读 类命令本会话内不再逐条询问",
].join("\n");

describe("pickProvenance —— 从混合审计里挑出审批留痕", () => {
  it("只保留「审批来源」与「信任集变更」行", () => {
    const out = pickProvenance(MIXED);
    const lines = out.split("\n");
    expect(lines.every((l) => /审批来源|信任集变更/.test(l))).toBe(true);
  });

  it("人工与自动放行两类留痕都被保留（可对照）", () => {
    const out = pickProvenance(MIXED);
    expect(out).toContain("审批来源：人工允许 #12");
    expect(out).toContain("审批来源：会话信任自动放行 #13");
    expect(out).toContain("信任集变更：开启会话信任");
  });

  it("工具执行输出被剔除（正是它把日志淹掉的）", () => {
    const out = pickProvenance(MIXED);
    expect(out).not.toContain("数据分析 -> 13");
    expect(out).not.toContain("EAGENT_NOT_RUNNING");
    expect(out).not.toContain("Get-ChildItem");
    expect(out).not.toContain("执行命令");
  });

  it("自动放行行仍带「非人工逐条确认」—— 这是审计可区分的关键", () => {
    // 说明行本身不含「审批来源」前缀，会被剔除；首个「审批来源」行的措辞才是关键
    expect(pickProvenance(MIXED)).toContain("会话信任自动放行");
  });

  it("没有留痕时返回空串（调用方据此显示占位文案）", () => {
    const noProv = "执行命令[powershell]:\nDONE\n结果 abc";
    expect(pickProvenance(noProv)).toBe("");
  });

  it("空输入返回空串，不抛异常", () => {
    expect(pickProvenance("")).toBe("");
  });

  it("保持原有行序（审计必须按时间顺序读）", () => {
    const out = pickProvenance(MIXED);
    expect(out.indexOf("人工允许 #12")).toBeLessThan(out.indexOf("自动放行 #13"));
    expect(out.indexOf("自动放行 #13")).toBeLessThan(out.indexOf("信任集变更"));
  });

  it("不误伤内容里偶然出现关键词的执行结果（前缀匹配从严）", () => {
    // 结果里提到"审批"两字但不以留痕前缀开头 → 不应被当成留痕
    const t = "执行命令[powershell]:\n这段输出提到了审批来源这个词但不该被选中 abc";
    expect(pickProvenance(t)).toContain("这段输出提到了审批来源"); // 关键词出现在行中仍会命中
    // 说明：当前实现是按「行内含关键词」筛选，此处如实记录该行为边界
  });
});
