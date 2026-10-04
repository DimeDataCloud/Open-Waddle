// Runs inside web pages (injected with chrome.scripting.executeScript). Each
// exported function must stand alone: Chrome copies only its own source into
// the page, so it can't call anything else in this file.

export interface PageElement {
  id: string;
  role: string;
  name: string;
  value?: string;
  offscreen?: boolean;
}

export interface PageRead {
  title: string;
  url: string;
  text?: string;
  truncated?: boolean;
  elements?: PageElement[];
}

export interface Located {
  x: number;
  y: number;
  w: number;
  h: number;
  screenX: number;
  screenY: number;
  outerWidth: number;
  outerHeight: number;
  innerWidth: number;
  innerHeight: number;
  dpr: number;
}

/** The page's title, address, and either its readable text (8 KB at most) or its controls, each tagged with an id. */
export function readPage(mode: "text" | "elements"): PageRead {
  const MAX_TEXT = 8000;
  const MAX_ELEMENTS = 200;
  const clean = (s: string | null | undefined) => (s ?? "").replace(/\s+/g, " ").trim();
  const out: PageRead = { title: document.title, url: location.href };
  if (mode === "text") {
    const root = document.querySelector("main, article, [role=main]") ?? document.body;
    const raw = (root as HTMLElement | null)?.innerText ?? root?.textContent ?? "";
    // Keep paragraph breaks, squash the rest.
    const text = raw
      .split(/\n+/)
      .map((l) => l.replace(/[ \t ]+/g, " ").trim())
      .filter(Boolean)
      .join("\n");
    out.text = text.slice(0, MAX_TEXT);
    out.truncated = text.length > MAX_TEXT;
    return out;
  }
  document.querySelectorAll("[data-waddle-id]").forEach((el) => el.removeAttribute("data-waddle-id"));
  const selector =
    "a[href], button, input:not([type=hidden]), textarea, select, summary, [role=button], [role=link], [role=checkbox], [role=tab], [role=menuitem], [role=textbox], [contenteditable=true], [contenteditable='']";
  const elements: PageElement[] = [];
  for (const el of Array.from(document.querySelectorAll<HTMLElement>(selector))) {
    if (elements.length >= MAX_ELEMENTS) break;
    const style = getComputedStyle(el);
    const r = el.getBoundingClientRect();
    if (style.display === "none" || style.visibility === "hidden" || r.width === 0 || r.height === 0) continue;
    const tag = el.tagName.toLowerCase();
    const input = el as HTMLInputElement;
    const type = (input.type ?? "").toLowerCase();
    const role =
      el.getAttribute("role") ??
      (tag === "a"
        ? "link"
        : tag === "button" || tag === "summary" || ["button", "submit", "reset"].includes(type)
          ? "button"
          : tag === "select"
            ? "select"
            : ["checkbox", "radio"].includes(type)
              ? type
              : tag === "input" || tag === "textarea" || el.isContentEditable
                ? "textbox"
                : tag);
    const labelled = el.id ? document.querySelector(`label[for="${CSS.escape(el.id)}"]`) : null;
    const name =
      clean(el.getAttribute("aria-label")) ||
      clean(labelled?.textContent) ||
      clean(el.innerText ?? el.textContent) ||
      clean(input.placeholder) ||
      clean(el.getAttribute("title")) ||
      clean(el.getAttribute("alt")) ||
      clean(input.name) ||
      (tag === "input" && ["button", "submit"].includes(type) ? clean(input.value) : "");
    const id = `e${elements.length + 1}`;
    el.setAttribute("data-waddle-id", id);
    const item: PageElement = { id, role, name: name.slice(0, 80) };
    if (role === "textbox" && tag !== "div" && type !== "password" && input.value) item.value = input.value.slice(0, 80);
    if (["checkbox", "radio"].includes(role)) item.value = input.checked ? "checked" : "unchecked";
    if (r.bottom < 0 || r.top > innerHeight || r.right < 0 || r.left > innerWidth) item.offscreen = true;
    elements.push(item);
  }
  out.elements = elements;
  return out;
}

/** Scrolls an element into view and says where it is, with what's needed to turn that into a screen position. */
export function locateElement(id: string): Located | null {
  const el = document.querySelector<HTMLElement>(`[data-waddle-id="${CSS.escape(id)}"]`);
  if (!el) return null;
  el.scrollIntoView({ block: "center", inline: "center", behavior: "instant" as ScrollBehavior });
  const r = el.getBoundingClientRect();
  if (r.width === 0 || r.height === 0) return null;
  return {
    x: r.left,
    y: r.top,
    w: r.width,
    h: r.height,
    screenX: window.screenX,
    screenY: window.screenY,
    outerWidth: window.outerWidth,
    outerHeight: window.outerHeight,
    innerWidth: window.innerWidth,
    innerHeight: window.innerHeight,
    dpr: window.devicePixelRatio,
  };
}

/** The fallback when a real click can't be made: the page's own click. */
export function clickElement(id: string): boolean {
  const el = document.querySelector<HTMLElement>(`[data-waddle-id="${CSS.escape(id)}"]`);
  if (!el) return false;
  el.scrollIntoView({ block: "center", behavior: "instant" as ScrollBehavior });
  el.focus();
  el.click();
  return true;
}

/** Replaces a field's text the way typing would (so frameworks see it), then optionally submits. */
export function typeInto(id: string, text: string, submit: boolean): boolean {
  const el = document.querySelector<HTMLElement>(`[data-waddle-id="${CSS.escape(id)}"]`);
  if (!el) return false;
  el.scrollIntoView({ block: "center", behavior: "instant" as ScrollBehavior });
  el.focus();
  if (el.isContentEditable) {
    el.textContent = text;
    el.dispatchEvent(new InputEvent("input", { bubbles: true, data: text, inputType: "insertText" }));
  } else {
    const field = el as HTMLInputElement | HTMLTextAreaElement;
    // React and friends watch the native value setter, not the property.
    const proto = field instanceof HTMLTextAreaElement ? HTMLTextAreaElement.prototype : HTMLInputElement.prototype;
    const setter = Object.getOwnPropertyDescriptor(proto, "value")?.set;
    if (setter) setter.call(field, text);
    else field.value = text;
    field.dispatchEvent(new Event("input", { bubbles: true }));
    field.dispatchEvent(new Event("change", { bubbles: true }));
  }
  if (submit) {
    const enter = { key: "Enter", code: "Enter", keyCode: 13, which: 13, bubbles: true, cancelable: true };
    const go = el.dispatchEvent(new KeyboardEvent("keydown", enter));
    el.dispatchEvent(new KeyboardEvent("keyup", enter));
    const form = (el as HTMLInputElement).form;
    if (go && form) form.requestSubmit();
  }
  return true;
}
