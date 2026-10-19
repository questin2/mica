/**
 * Markdown 渲染器测试（可扩展性报告 v2 · S-04）
 *
 * 两条重点：
 * 1. **块级解析正确性** —— `parseBlocks` 是纯函数，直接断言其数据结构；
 * 2. **不可信内容安全** —— 模型输出属于不可信数据，渲染器承诺「不注入 HTML、
 *    全量产出 React 元素」。这里用 renderToStaticMarkup 端到端验证：
 *    任何脚本/标签都必须以转义文本形式出现，绝不可成为可执行标签。
 */
import { describe, expect, it } from "vitest";
import { renderToStaticMarkup } from "react-dom/server";
import Markdown, { parseBlocks } from "../Markdown";

describe("parseBlocks —— 块级解析", () => {
  it("空输入产出空块列表", () => {
    expect(parseBlocks("")).toEqual([]);
    expect(parseBlocks("   \n\n  ")).toEqual([]);
  });

  it("CRLF 与孤立 CR 都被规整为 LF", () => {
    expect(parseBlocks("a\r\n\r\nb")).toEqual(parseBlocks("a\n\nb"));
    // 孤立 \r 同样规整为换行（而不是留在文本里）
    expect(parseBlocks("a\rb")).toEqual(parseBlocks("a\nb"));
  });

  it.each([
    ["# 一级", 1],
    ["## 二级", 2],
    ["###### 六级", 6],
  ])("识别标题：%s", (src, level) => {
    expect(parseBlocks(src)).toEqual([{ t: "h", level, text: src.replace(/^#+\s*/, "") }]);
  });

  it("识别段落：多个连续行合并为一个段落（软换行合并为空格，标准 Markdown 行为）", () => {
    expect(parseBlocks("第一行\n第二行")).toEqual([{ t: "p", text: "第一行 第二行" }]);
  });

  it("识别围栏代码块并保留语言与原文", () => {
    const out = parseBlocks("```powershell\nGet-Date\n```");
    expect(out).toEqual([{ t: "code", lang: "powershell", code: "Get-Date" }]);
  });

  it("围栏未闭合时不丢内容（按剩余全部收进代码块）", () => {
    const out = parseBlocks("```\nabc\ndef");
    expect(out).toHaveLength(1);
    expect(out[0]).toMatchObject({ t: "code" });
    expect((out[0] as { code: string }).code).toContain("abc");
  });

  it("识别无序列表（-、*、+ 三种记号）", () => {
    expect(parseBlocks("- a\n- b")).toEqual([{ t: "ul", items: ["a", "b"] }]);
    expect(parseBlocks("* a\n* b")).toEqual([{ t: "ul", items: ["a", "b"] }]);
    expect(parseBlocks("+ a")).toEqual([{ t: "ul", items: ["a"] }]);
  });

  it("识别有序列表（. 与 ) 两种记号）", () => {
    expect(parseBlocks("1. a\n2. b")).toEqual([{ t: "ol", items: ["a", "b"] }]);
    expect(parseBlocks("1) a\n2) b")).toEqual([{ t: "ol", items: ["a", "b"] }]);
  });

  it("识别引用块", () => {
    expect(parseBlocks("> 引用一\n> 引用二")).toEqual([{ t: "quote", lines: ["引用一", "引用二"] }]);
  });

  it.each([["---"], ["***"], ["___"]])("识别分割线：%s", (src) => {
    expect(parseBlocks(src)).toEqual([{ t: "hr" }]);
  });

  it("识别表格（表头 + 分隔行 + 数据行）", () => {
    const out = parseBlocks("| A | B |\n|---|---|\n| 1 | 2 |");
    expect(out).toEqual([{ t: "table", head: ["A", "B"], rows: [["1", "2"]] }]);
  });

  it("表格首尾竖线可省略", () => {
    const out = parseBlocks("A | B\n---|---\n1 | 2");
    expect(out).toEqual([{ t: "table", head: ["A", "B"], rows: [["1", "2"]] }]);
  });

  it("多个块按顺序解析", () => {
    const out = parseBlocks("# 标题\n\n正文\n\n- 项");
    expect(out.map((b) => b.t)).toEqual(["h", "p", "ul"]);
  });

  it("块之间的空行不产生空块", () => {
    const out = parseBlocks("a\n\n\n\nb");
    expect(out).toEqual([
      { t: "p", text: "a" },
      { t: "p", text: "b" },
    ]);
  });
});

describe("Markdown 渲染 —— 安全属性（不可信内容）", () => {
  it("脚本标签被转义为文本，绝不成为可执行标签", () => {
    const html = renderToStaticMarkup(Markdown({ text: "<script>alert(1)</script>" }));
    expect(html).not.toContain("<script>");
    expect(html).toContain("&lt;script&gt;");
  });

  it("img onerror 之类的事件属性不会被注入", () => {
    const html = renderToStaticMarkup(Markdown({ text: '<img src=x onerror="alert(1)">' }));
    // 关键：不产出 <img> 标签。文本里当然还能读到 "onerror=" 这几个字 —— 它是惰性文本，不是属性。
    expect(html).not.toContain("<img");
    expect(html).toContain("&lt;img");
  });

  it("行内 HTML 一律按纯文本呈现", () => {
    const html = renderToStaticMarkup(Markdown({ text: "**粗体** 与 <b>标签</b>" }));
    expect(html).toContain("<strong>");
    expect(html).toContain("&lt;b&gt;");
    expect(html).not.toContain("<b>标签</b>");
  });

  it("渲染标题、列表、表格、行内代码的结构正确", () => {
    const html = renderToStaticMarkup(
      Markdown({ text: "## 标题\n\n- 甲\n- 乙\n\n| A | B |\n|---|---|\n| 1 | 2 |\n\n`code`" }),
    );
    // 标题**刻意下移一级**（## → <h3>），把 <h1> 留给页面自身标题
    expect(html).toMatch(/<h3[ >]/);
    expect(html).toContain("md-h2");
    expect(html).toMatch(/<ul[ >]/);
    expect(html).toMatch(/<li[ >][^<]*甲/);
    expect(html).toMatch(/<table[ >]/);
    expect(html).toContain(">code</code>");
  });

  it("单列表格也能识别（分隔行 |---| 不应被降级成段落）", () => {
    const out = parseBlocks("| A |\n|---|\n| 1 |");
    expect(out).toEqual([{ t: "table", head: ["A"], rows: [["1"]] }]);
    const html = renderToStaticMarkup(Markdown({ text: "| A |\n|---|\n| 1 |" }));
    expect(html).toMatch(/<table[ >]/);
  });

  it("分割线不会被表格判断抢走", () => {
    expect(parseBlocks("---")).toEqual([{ t: "hr" }]);
    expect(parseBlocks("a\n\n---\n\nb").map((b) => b.t)).toEqual(["p", "hr", "p"]);
  });

  it("代码块内容被转义且保留原文", () => {
    const html = renderToStaticMarkup(Markdown({ text: "```\nif (a < b) {}\n```" }));
    expect(html).toMatch(/<pre[ >]/);
    expect(html).toContain("a &lt; b");
  });

  it("http(s) 链接正常产出 <a>", () => {
    const html = renderToStaticMarkup(Markdown({ text: "[官网](https://example.com/x)" }));
    expect(html).toContain('href="https://example.com/x"');
    expect(html).toContain("官网");
  });

  it("非 http(s) 协议不产出链接，只留纯文本（纵深防御，不依赖 React 拦截）", () => {
    const html = renderToStaticMarkup(Markdown({ text: "[点我](javascript:alert(1))" }));
    expect(html).not.toContain("<a ");
    expect(html).not.toContain("href=");
    // 原样文本仍在（不吞内容），但不可点击
    expect(html).toContain("javascript:alert(1)");
  });

  it("data: 协议同样不产出链接", () => {
    const html = renderToStaticMarkup(Markdown({ text: "[x](data:text/html,<script>alert(1)</script>)" }));
    expect(html).not.toContain("<a ");
  });

  it("空文本渲染不抛异常", () => {
    expect(() => renderToStaticMarkup(Markdown({ text: "" }))).not.toThrow();
  });
});
