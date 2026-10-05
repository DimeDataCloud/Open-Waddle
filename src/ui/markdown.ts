// A small Markdown renderer for replies. It builds DOM nodes and sets only
// textContent, never innerHTML, so nothing in a reply can become markup or
// script. Links keep their address in data-href; the drawer opens web links
// through the backend, and nothing else is clickable.

/** Only these open; anything else in a link stays plain text. */
export function webLink(url: string): string | null {
  try {
    const u = new URL(url.trim());
    return u.protocol === "https:" || u.protocol === "http:" ? u.href : null;
  } catch {
    return null;
  }
}

// **bold**, *italic*, `code`, [text](url), and bare web addresses. Underscores
// are left alone: they're more often in names (snake_case, __init__) than emphasis.
const INLINE = /(`+)([^`]+?)\1|\*\*(.+?)\*\*|\*([^*\s][^*]*?)\*|\[([^\]]+)\]\(([^)\s]+)\)|(https?:\/\/[^\s<>()]+[^\s<>().,;:!?'"])/g;

function inline(doc: Document, text: string, into: Node): void {
  let at = 0;
  for (const m of text.matchAll(INLINE)) {
    const start = m.index ?? 0;
    if (start > at) into.appendChild(doc.createTextNode(text.slice(at, start)));
    at = start + m[0].length;
    if (m[2] !== undefined) {
      const code = doc.createElement("code");
      code.textContent = m[2];
      into.appendChild(code);
    } else if (m[3] !== undefined) {
      const b = doc.createElement("strong");
      inline(doc, m[3], b);
      into.appendChild(b);
    } else if (m[4] !== undefined) {
      const i = doc.createElement("em");
      inline(doc, m[4], i);
      into.appendChild(i);
    } else if (m[5] !== undefined) {
      into.appendChild(link(doc, m[5], m[6]));
    } else if (m[7] !== undefined) {
      into.appendChild(link(doc, m[7], m[7]));
    }
  }
  if (at < text.length) into.appendChild(doc.createTextNode(text.slice(at)));
}

function link(doc: Document, label: string, url: string): Node {
  const href = webLink(url);
  if (!href) return doc.createTextNode(label);
  const a = doc.createElement("a");
  a.textContent = label;
  a.dataset.href = href;
  a.title = href;
  a.tabIndex = 0;
  a.setAttribute("role", "link");
  return a;
}

const BULLET = /^\s*[-*+]\s+(.*)$/;
const NUMBER = /^\s*\d{1,3}[.)]\s+(.*)$/;
const HEADING = /^#{1,6}\s+(.*)$/;
const FENCE = /^\s*```/;
const QUOTE = /^\s*>\s?(.*)$/;

/** Renders Markdown into a fragment of safe nodes. */
export function renderMarkdown(text: string, doc: Document = document): DocumentFragment {
  const out = doc.createDocumentFragment();
  const lines = text.replace(/\r\n?/g, "\n").split("\n");
  let para: string[] = [];
  let list: HTMLElement | null = null;

  const flushPara = () => {
    if (!para.length) return;
    const p = doc.createElement("p");
    para.forEach((l, i) => {
      if (i) p.appendChild(doc.createElement("br"));
      inline(doc, l, p);
    });
    out.appendChild(p);
    para = [];
  };
  const endList = () => {
    list = null;
  };

  for (let i = 0; i < lines.length; i++) {
    const line = lines[i];
    if (FENCE.test(line)) {
      flushPara();
      endList();
      const body: string[] = [];
      while (++i < lines.length && !FENCE.test(lines[i])) body.push(lines[i]);
      const pre = doc.createElement("pre");
      const code = doc.createElement("code");
      code.textContent = body.join("\n");
      pre.appendChild(code);
      out.appendChild(pre);
      continue;
    }
    if (!line.trim()) {
      flushPara();
      endList();
      continue;
    }
    const bullet = BULLET.exec(line);
    const number = bullet ? null : NUMBER.exec(line);
    if (bullet || number) {
      flushPara();
      const tag = bullet ? "UL" : "OL";
      if (!list || list.tagName !== tag) {
        list = doc.createElement(tag.toLowerCase());
        out.appendChild(list);
      }
      const li = doc.createElement("li");
      inline(doc, (bullet ?? number)![1], li);
      list.appendChild(li);
      continue;
    }
    const heading = HEADING.exec(line);
    if (heading) {
      flushPara();
      endList();
      const h = doc.createElement("p");
      h.className = "heading";
      const b = doc.createElement("strong");
      inline(doc, heading[1], b);
      h.appendChild(b);
      out.appendChild(h);
      continue;
    }
    const quote = QUOTE.exec(line);
    if (quote) {
      flushPara();
      endList();
      const q = doc.createElement("blockquote");
      inline(doc, quote[1], q);
      out.appendChild(q);
      continue;
    }
    // A wrapped list item continues the item above it.
    if (list && /^\s{2,}\S/.test(line)) {
      const last = list.lastElementChild;
      if (last) {
        last.appendChild(doc.createTextNode(" "));
        inline(doc, line.trim(), last);
        continue;
      }
    }
    endList();
    para.push(line);
  }
  flushPara();
  return out;
}
