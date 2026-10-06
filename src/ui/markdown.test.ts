// @vitest-environment jsdom
import { describe, expect, it } from "vitest";
import { renderMarkdown, webLink } from "./markdown";

function html(md: string): string {
  const div = document.createElement("div");
  div.appendChild(renderMarkdown(md));
  return div.innerHTML;
}

describe("renderMarkdown", () => {
  it("renders the common pieces", () => {
    expect(html("Hello **there** and *you*, run `ls -la`.")).toBe(
      "<p>Hello <strong>there</strong> and <em>you</em>, run <code>ls -la</code>.</p>",
    );
    expect(html("- one\n- two\n\n1. first\n2. second")).toBe("<ul><li>one</li><li>two</li></ul><ol><li>first</li><li>second</li></ol>");
    expect(html("## Cost\nCheaper.\nStill.")).toBe('<p class="heading"><strong>Cost</strong></p><p>Cheaper.<br>Still.</p>');
    expect(html("```\nlet x = 1;\n<b>\n```")).toBe("<pre><code>let x = 1;\n&lt;b&gt;</code></pre>");
    expect(html("> quoted")).toBe("<blockquote>quoted</blockquote>");
    expect(html("- a long item\n  that wraps")).toBe("<ul><li>a long item that wraps</li></ul>");
  });

  it("never turns text into markup", () => {
    const evil = '<img src=x onerror="alert(1)"> <script>alert(2)</script> **<b>bold</b>**';
    const div = document.createElement("div");
    div.appendChild(renderMarkdown(evil));
    expect(div.querySelector("img, script, b")).toBeNull();
    expect(div.textContent).toContain("<img src=x");
    expect(div.querySelector("strong")!.textContent).toBe("<b>bold</b>");
  });

  it("keeps only web links, without an href to follow", () => {
    const div = document.createElement("div");
    div.appendChild(renderMarkdown("[site](https://example.com/a) [bad](javascript:alert) see https://b.example/x."));
    const links = [...div.querySelectorAll("a")];
    expect(links.map((a) => a.dataset.href)).toEqual(["https://example.com/a", "https://b.example/x"]);
    expect(links.every((a) => !a.hasAttribute("href"))).toBe(true);
    // The bad link is just its label, and the full stop isn't part of the address.
    expect(div.textContent).toBe("site bad see https://b.example/x.");
  });

  it("leaves underscores in names alone", () => {
    expect(html("call my_long_name or __init__")).toBe("<p>call my_long_name or __init__</p>");
  });
});

describe("webLink", () => {
  it("allows http and https only", () => {
    expect(webLink("https://a.example")).toBe("https://a.example/");
    expect(webLink(" HTTP://A.example/x ")).toBe("http://a.example/x");
    expect(webLink("javascript:alert(1)")).toBeNull();
    expect(webLink("file:///C:/Windows")).toBeNull();
    expect(webLink("not a url")).toBeNull();
  });
});
