// Waddle's Chrome extension: a native-messaging link to the desktop app.
// The app sends {id, cmd, args}; this answers {id, ok, result | error}.
// Nothing runs unless the app asks, and the app only asks when the user gave Waddle a task.
import { clickElement, locateElement, readPage, typeInto } from "./page.js";
const HOST = "dev.waddle.app";
const LOAD_TIMEOUT_MS = 15_000;
let port = null;
function connect() {
    if (port)
        return;
    try {
        port = chrome.runtime.connectNative(HOST);
    }
    catch {
        port = null;
        return;
    }
    port.onMessage.addListener((msg) => {
        handle(msg).then((result) => port?.postMessage({ id: msg.id, ok: true, result: result ?? null }), (e) => port?.postMessage({ id: msg.id, ok: false, error: e instanceof Error ? e.message : String(e) }));
    });
    port.onDisconnect.addListener(() => {
        // Usually: Waddle isn't running. The alarm tries again in a minute.
        void chrome.runtime.lastError;
        port = null;
    });
    port.postMessage({ type: "hello", version: chrome.runtime.getManifest().version });
}
chrome.runtime.onStartup.addListener(connect);
chrome.runtime.onInstalled.addListener(() => {
    void chrome.alarms.create("waddle-reconnect", { periodInMinutes: 1 });
    connect();
});
chrome.alarms.onAlarm.addListener((a) => {
    if (a.name === "waddle-reconnect")
        connect();
});
connect();
function webUrl(u) {
    const url = new URL(String(u ?? ""));
    if (url.protocol !== "http:" && url.protocol !== "https:")
        throw new Error("only http and https addresses");
    return url.href;
}
async function targetTab(tab) {
    if (typeof tab === "number")
        return chrome.tabs.get(tab);
    const [active] = await chrome.tabs.query({ active: true, lastFocusedWindow: true });
    if (!active)
        throw new Error("no Chrome tab is open");
    return active;
}
/** Brings a tab to the front, restoring a minimised window, so the user sees what Waddle does. */
async function show(t) {
    await chrome.tabs.update(t.id, { active: true });
    const w = await chrome.windows.get(t.windowId);
    await chrome.windows.update(t.windowId, w.state === "minimized" ? { state: "normal", focused: true } : { focused: true });
}
async function inPage(tabId, func, args) {
    const [frame] = await chrome.scripting.executeScript({ target: { tabId }, func, args });
    return frame?.result;
}
/** Waits until a tab has finished loading (or gives up after 15 s). */
function loaded(tabId) {
    return new Promise((resolve) => {
        const done = () => {
            chrome.tabs.onUpdated.removeListener(listener);
            clearTimeout(timer);
            void chrome.tabs.get(tabId).then(resolve);
        };
        const listener = (id, info) => {
            if (id === tabId && info.status === "complete")
                done();
        };
        const timer = setTimeout(done, LOAD_TIMEOUT_MS);
        chrome.tabs.onUpdated.addListener(listener);
    });
}
async function handle({ cmd, args }) {
    switch (cmd) {
        case "tabs": {
            const action = String(args.action ?? "list");
            if (action === "list") {
                const tabs = await chrome.tabs.query({});
                return tabs.map((t) => ({ id: t.id, title: t.title ?? "", url: t.url ?? "", active: t.active, windowId: t.windowId }));
            }
            if (action === "open") {
                const t = await chrome.tabs.create({ url: webUrl(args.url) });
                await show(t);
                const done = await loaded(t.id);
                return { id: done.id, title: done.title ?? "", url: done.url ?? "" };
            }
            const t = await targetTab(args.tab);
            if (action === "switch") {
                await show(t);
                return { id: t.id, title: t.title ?? "" };
            }
            if (action === "close") {
                await chrome.tabs.remove(t.id);
                return { id: t.id };
            }
            throw new Error(`unknown tabs action ${action}`);
        }
        case "read": {
            const t = await targetTab(args.tab);
            return inPage(t.id, readPage, [args.mode === "elements" ? "elements" : "text"]);
        }
        case "navigate": {
            const t = await targetTab(args.tab);
            await chrome.tabs.update(t.id, { url: webUrl(args.url) });
            await show(t);
            const done = await loaded(t.id);
            return { title: done.title ?? "", url: done.url ?? "" };
        }
        case "locate": {
            // The tab must be in front for a real click to land on it.
            const t = await targetTab(args.tab);
            await show(t);
            return inPage(t.id, locateElement, [String(args.element ?? "")]);
        }
        case "click": {
            const t = await targetTab(args.tab);
            return inPage(t.id, clickElement, [String(args.element ?? "")]);
        }
        case "type": {
            const t = await targetTab(args.tab);
            return inPage(t.id, typeInto, [String(args.element ?? ""), String(args.text ?? ""), args.submit === true]);
        }
        default:
            throw new Error(`unknown command ${cmd}`);
    }
}
