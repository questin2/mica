/**
 * CSS 类名撞车守卫（回归防护）
 *
 * 背景：`.brand` 是状态条 logo「E」的样式（`width/height: 26px` + `display: grid`）。
 * 徽标变体曾写成 `badge brand`，两者**同优先级**且 `.brand` 在样式表里更靠后，
 * 于是徽标被压成 26×26 方格、文字溢出压到相邻徽标上 —— 用户截图反馈的「claude 错位」正是此因。
 *
 * 这类 bug 类型检查与普通单测都抓不到（它只在真实 CSS 层叠里发生），因此在这里用
 * 源码扫描做守卫：**任何元素都不得同时挂 `badge` 与 `brand`**。
 *
 * 价值实证：本守卫上线时**立刻抓到一处人工 grep 漏掉的三元表达式**
 * （`AuditPage.tsx` 里 tool 阶段的 `badge ... "brand"`），说明扫描式守卫比肉眼可靠。
 */
import { readFileSync, readdirSync, statSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";

const SRC = join(process.cwd(), "src");

function walk(dir: string, out: string[] = []): string[] {
  for (const name of readdirSync(dir)) {
    const p = join(dir, name);
    if (statSync(p).isDirectory()) walk(p, out);
    else if (p.endsWith(".tsx") || p.endsWith(".ts")) out.push(p);
  }
  return out;
}

/** 源码里所有 className 表达式所在行。 */
function classNameLines(src: string): string[] {
  return src.split("\n").filter((l: string) => /className/.test(l));
}

describe("CSS 类名撞车守卫", () => {
  it("确实扫到了源码文件（守卫不能空跑）", () => {
    expect(walk(SRC).length).toBeGreaterThan(20);
  });

  it("没有任何元素同时使用 badge 与 brand（.brand 是 logo 专用样式）", () => {
    const offenders: string[] = [];
    for (const f of walk(SRC)) {
      classNameLines(readFileSync(f, "utf8")).forEach((line) => {
        if (/\bbadge\b/.test(line) && /\bbrand\b/.test(line)) {
          offenders.push(`${f.replace(SRC, "src")}  ${line.trim()}`);
        }
      });
    }
    expect(offenders, `以下位置把 logo 的 .brand 用在了徽标上：\n${offenders.join("\n")}`).toEqual([]);
  });

  it("样式表里不再存在 .badge.brand 规则（已改名 accent）", () => {
    const css = readFileSync(join(SRC, "styles", "shell.css"), "utf8");
    expect(css).not.toMatch(/\.badge\.brand\s*\{/);
    expect(css).toMatch(/\.badge\.accent\s*\{/);
  });

  it("徽标变体 accent 确实还有使用处（改名后没被漏掉）", () => {
    const used = walk(SRC).some((f) =>
      classNameLines(readFileSync(f, "utf8")).some((l) => /\bbadge\b/.test(l) && /\baccent\b/.test(l)),
    );
    expect(used).toBe(true);
  });

  it("`.badge` 自带 flex: 0 0 auto（防被 flex 容器压窄导致文字溢出）", () => {
    const css = readFileSync(join(SRC, "styles", "shell.css"), "utf8");
    const start = css.indexOf(".badge {");
    expect(start, "应能找到 .badge 规则").toBeGreaterThan(-1);
    expect(css.slice(start, start + 500)).toMatch(/flex:\s*0\s+0\s+auto/);
  });

  it("`.brand` 仍保留 logo 语义（width/height 26px），并未被顺手改坏", () => {
    const css = readFileSync(join(SRC, "styles", "shell.css"), "utf8");
    const start = css.indexOf("\n.brand {");
    expect(start, "应能找到 .brand 规则").toBeGreaterThan(-1);
    const block = css.slice(start, start + 300);
    expect(block).toMatch(/width:\s*26px/);
    expect(block).toMatch(/height:\s*26px/);
  });
});
