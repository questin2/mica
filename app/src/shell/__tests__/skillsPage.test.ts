/**
 * 技能页展示逻辑测试（拆分自 Shell.tsx 的 SkillsPage）
 *
 * 钉住的是「来源徽标」这条**用户反馈过**的展示规则：
 * 裸渲染 `source`（如 `claude`）会看不出是来源还是分类，且与相邻的「许可：未知」
 * 都是暖色块、视觉上糊成一片 —— 因此必须带「来源：」标注（插件来源自带前缀则不重复加）。
 */
import { describe, expect, it } from "vitest";
import { sourceBadgeLabel } from "../components/SkillsPage";

describe("sourceBadgeLabel —— 来源徽标文案", () => {
  it("普通来源补「来源：」前缀", () => {
    expect(sourceBadgeLabel("claude")).toBe("来源：claude");
    expect(sourceBadgeLabel("我的导入")).toBe("来源：我的导入");
  });

  it("插件来源自带前缀，不重复添加", () => {
    expect(sourceBadgeLabel("插件：demo-toolkit")).toBe("插件：demo-toolkit");
    expect(sourceBadgeLabel("插件：x")).not.toContain("来源：来源");
  });

  it("前后空白被去除", () => {
    expect(sourceBadgeLabel("  claude  ")).toBe("来源：claude");
  });

  it("空来源返回空串（调用方据此不渲染该徽标）", () => {
    expect(sourceBadgeLabel("")).toBe("");
    expect(sourceBadgeLabel("   ")).toBe("");
  });

  it("文案只加一次前缀（不出现「来源：来源：」）", () => {
    for (const s of ["a", "claude", "abc-def", "  x  "]) {
      expect(sourceBadgeLabel(s).match(/来源：/g)?.length ?? 0).toBeLessThanOrEqual(1);
    }
  });

  it("对既有真实数据形态成立（138 条 source=claude 的实际取值）", () => {
    expect(sourceBadgeLabel("claude")).toBe("来源：claude");
  });
});
