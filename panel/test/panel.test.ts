import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { describe, it } from "node:test";
import vm from "node:vm";
import { assertPanelInvariants, checkEnNoCjk, checkI18nParity, HARD_MAX_BYTES, WARN_BYTES } from "../kit/guard.ts";
import { I18N } from "../i18n.ts";
import { mapBridgeErrorKey } from "../net-policy.ts";
import type { FramePayload } from "../net-policy.ts";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const panelDir = path.resolve(__dirname, "..");
const repoDir = path.resolve(panelDir, "..");

interface MockElement {
  tagName: string;
  style: {
    setProperty(): void;
    colorScheme: string;
    backgroundColor: string;
    transform: string;
    left: string;
    top: string;
  };
  dataset: Record<string, string>;
  hidden: boolean;
  disabled: boolean;
  open: boolean;
  textContent: string;
  value: string;
  title: string;
  className: string;
  classList: {
    toggle(c: string, force?: boolean): boolean;
    add(c: string): void;
    remove(c: string): void;
    contains(c: string): boolean;
  };
  id: string;
  readonly firstChild: MockElement | null;
  readonly lastChild: MockElement | null;
  readonly nextSibling: MockElement | null;
  setAttribute(k: string, v: string): void;
  getAttribute(k: string): string | null;
  addEventListener(ev: string, fn: (arg?: any) => void): void;
  removeEventListener(ev: string, fn: (arg?: any) => void): void;
  querySelector(): MockElement;
  querySelectorAll(sel: string): MockElement[];
  children: MockElement[];
  append(...items: Array<MockElement | null>): void;
  appendChild(it: MockElement | null): MockElement | null;
  replaceChildren(...items: Array<MockElement | null>): void;
  insertBefore(it: MockElement | null): MockElement | null;
  remove(): void;
  closest(): null;
  focus(): void;
  select(): void;
  getContext(): { drawImage(): void };
  getBoundingClientRect(): { left: number; right: number; top: number; bottom: number; width: number; height: number };
  clientWidth: number;
  clientHeight: number;
  listeners: Map<string, Array<(arg?: any) => void>>;
}

interface PostedMessage {
  id?: number;
  method?: string;
  params?: {
    name?: string;
    arguments?: Record<string, any>;
  };
}

function bootPanel(boot: FramePayload = {}, { navLang = "en-US", hostLocale = "" } = {}) {
  const html = fs.readFileSync(path.join(panelDir, "dist", "panel.html"), "utf8");
  const match = html.match(/<script>\/\*__CHROME_STREAM_BOOT__\*\/([\s\S]*?)<\/script>/);
  assert.ok(match, "built panel.html must contain single inline script");

  const els = new Map<string, MockElement>();
  const mkEl = (tag = "div"): MockElement => {
    const attrs = new Map<string, string>();
    const listeners = new Map<string, Array<(arg?: any) => void>>();
    const kids: MockElement[] = [];
    const classes = new Set<string>();
    let _id = "";
    const node: MockElement = {
      tagName: tag.toUpperCase(),
      style: { setProperty() {}, colorScheme: "", backgroundColor: "", transform: "", left: "", top: "" },
      dataset: {},
      hidden: false,
      disabled: false,
      open: false,
      textContent: "",
      value: "",
      title: "",
      get className() { return [...classes].join(" "); },
      set className(v: string) { classes.clear(); String(v || "").split(/\s+/).filter(Boolean).forEach((c) => classes.add(c)); },
      classList: {
        toggle(c: string, force?: boolean) { const on = force !== undefined ? !!force : !classes.has(c); on ? classes.add(c) : classes.delete(c); return on; },
        add(c: string) { classes.add(c); },
        remove(c: string) { classes.delete(c); },
        contains(c: string) { return classes.has(c); },
      },
      get id() { return _id; },
      set id(v: string) { _id = String(v || ""); if (_id) els.set(_id, node); },
      get firstChild() { return kids[0] || null; },
      get lastChild() { return kids[kids.length - 1] || null; },
      get nextSibling() { return null; },
      setAttribute(k: string, v: string) { attrs.set(k, String(v)); },
      getAttribute(k: string) { return attrs.get(k) ?? null; },
      addEventListener(ev: string, fn: (arg?: any) => void) { if (!listeners.has(ev)) listeners.set(ev, []); listeners.get(ev)!.push(fn); },
      removeEventListener(ev: string, fn: (arg?: any) => void) { listeners.set(ev, (listeners.get(ev) || []).filter((f) => f !== fn)); },
      querySelector() { return mkEl(); },
      querySelectorAll(sel: string) {
        const out: MockElement[] = [];
        const walk = (n: MockElement) => {
          for (const c of n?.children || []) {
            if (sel === "[data-i],[data-a],[data-p],[data-t]" && (c.dataset?.i || c.dataset?.a || c.dataset?.p || c.dataset?.t)) out.push(c);
            else if (sel === "[data-k],[data-sc]" && (c.dataset?.k || c.dataset?.sc)) out.push(c);
            walk(c);
          }
        };
        walk(node);
        return out;
      },
      children: kids,
      append(...items: Array<MockElement | null>) { for (const it of items) it && kids.push(it); },
      appendChild(it: MockElement | null) { if (it) kids.push(it); return it; },
      replaceChildren(...items: Array<MockElement | null>) { kids.length = 0; node.append(...items); },
      insertBefore(it: MockElement | null) { if (it) kids.push(it); return it; },
      remove() {},
      closest() { return null; },
      focus() {},
      select() {},
      getContext() { return { drawImage() {} }; },
      getBoundingClientRect() { return { left: 0, right: 400, top: 0, bottom: 300, width: 400, height: 300 }; },
      clientWidth: 400,
      clientHeight: 300,
      listeners,
    };
    return node;
  };

  const rootEl = mkEl("html");
  const doc = {
    ...mkEl("document"),
    documentElement: rootEl,
    body: mkEl("body"),
    head: mkEl("head"),
    visibilityState: "visible",
    getElementById(id: string) { if (!els.has(id)) { const n = mkEl(id); n.id = id; } return els.get(id)!; },
    createElement(t: string) { return mkEl(t); },
    createElementNS(_ns: string, t: string) { return mkEl(t); },
    createTextNode(t: string) { return { nodeType: 3, textContent: String(t) }; },
    createDocumentFragment() { return mkEl("frag"); },
  };
  const posted: PostedMessage[] = [];
  const winListeners = new Map<string, Array<(ev: unknown) => void>>();
  const win = {
    parent: { postMessage(msg: PostedMessage) { posted.push(msg); } },
    addEventListener(ev: string, fn: (ev: unknown) => void) { if (!winListeners.has(ev)) winListeners.set(ev, []); winListeners.get(ev)!.push(fn); },
    removeEventListener(ev: string, fn: (ev: unknown) => void) { winListeners.set(ev, (winListeners.get(ev) || []).filter((f) => f !== fn)); },
    innerWidth: 480,
    innerHeight: 720,
  };
  let disposed = false;
  const activeTimers = new Set<ReturnType<typeof setTimeout>>();
  const scheduleTimer = (fn: (...args: unknown[]) => void, ms?: number, ...args: unknown[]) => {
    if (disposed) return 0 as unknown as ReturnType<typeof setTimeout>;
    const id = setTimeout(() => { activeTimers.delete(id); if (!disposed) fn(...args); }, ms);
    id.unref?.();
    activeTimers.add(id);
    return id;
  };
  const cancelTimer = (id: ReturnType<typeof setTimeout>) => { activeTimers.delete(id); clearTimeout(id); };
  const ctx = vm.createContext({
    window: win,
    document: doc,
    navigator: { language: navLang, platform: "MacIntel", clipboard: { writeText: async () => {} } },
    performance: { now: () => 10_000 },
    console,
    requestAnimationFrame: (fn: () => void) => scheduleTimer(fn, 0),
    cancelAnimationFrame: cancelTimer,
    ResizeObserver: class { observe() {} disconnect() {} },
    IntersectionObserver: class { observe() {} disconnect() {} },
    Image: class {},
    Map, Set, Array, Object, Math, JSON, RegExp, Promise, Error, Number, String, Boolean,
    Date: { now: () => 10_000 },
    getComputedStyle: () => ({ getPropertyValue: () => "" }),
    __CHROME_STREAM_BOOT__: boot,
    setTimeout: scheduleTimer,
    clearTimeout: cancelTimer,
  });
  vm.runInContext(match[1], ctx);

  const initReq = posted.find((m) => m.method === "ui/initialize");
  const msgHandler = winListeners.get("message")?.[0]!;
  msgHandler({
    source: win.parent,
    data: {
      jsonrpc: "2.0",
      id: initReq!.id,
      result: {
        protocolVersion: "2026-01-26",
        hostContext: { theme: "dark", ...(hostLocale ? { locale: hostLocale } : {}) },
        hostCapabilities: { serverTools: {} },
      },
    },
  });

  const cleanup = () => {
    disposed = true;
    for (const id of activeTimers) clearTimeout(id);
    activeTimers.clear();
  };
  return { els, posted, msgHandler, win, cleanup };
}

describe("Remote Cast panel kit invariants & Wave 2 regressions", () => {
  it("passes Panel Kit invariants, i18n EN/ZH parity, EN no-CJK, and Lucid Indigo logo check", () => {
    const html = fs.readFileSync(path.join(panelDir, "dist", "panel.html"), "utf8");
    const report = assertPanelInvariants(html, {
      maxBytes: HARD_MAX_BYTES,
      hardMaxBytes: HARD_MAX_BYTES,
      warnBytes: WARN_BYTES,
      bootPlaceholder: "/*__CHROME_STREAM_BOOT__*/",
    });
    assert.equal(report.ok, true);
    assert.ok(report.bytes <= HARD_MAX_BYTES);

    const parity = checkI18nParity(I18N.en, I18N["zh-CN"]);
    assert.equal(parity.ok, true, `i18n parity issues: ${JSON.stringify(parity)}`);
    const enCjk = checkEnNoCjk(I18N.en);
    assert.equal(enCjk.ok, true, `EN strings contain CJK: ${JSON.stringify(enCjk.errors)}`);

    const logo = fs.readFileSync(path.join(repoDir, "assets", "logo.svg"), "utf8");
    assert.ok(logo.includes("#0B0D12") && logo.includes("#6895FF"), "logo.svg must use Lucid Indigo palette");

    const plugin = JSON.parse(fs.readFileSync(path.join(repoDir, "plugin.json"), "utf8")) as { version: string };
    assert.ok(!html.includes(plugin.version), "built panel.html must not bake plugin.json version");
  });

  it("reads runtime appVersion / relayVersion from boot payload for ui/initialize", () => {
    const h1 = bootPanel({ connected: false, appVersion: "9.8.7", relayVersion: "9.8.6" });
    try {
      const init1 = h1.posted.find((m) => m.method === "ui/initialize") as { params?: { appInfo?: { version?: string } } };
      assert.equal(init1?.params?.appInfo?.version, "9.8.7");
    } finally {
      h1.cleanup();
    }
    const h2 = bootPanel({ connected: false, relayVersion: "9.8.6" });
    try {
      const init2 = h2.posted.find((m) => m.method === "ui/initialize") as { params?: { appInfo?: { version?: string } } };
      assert.equal(init2?.params?.appInfo?.version, "9.8.6");
    } finally {
      h2.cleanup();
    }
  });

  it("clears loading bar on 0-tab/closed-window transition and shows actionable New Tab + Disconnect", async () => {
    const h = bootPanel({
      connected: true,
      connectionMode: "personal",
      tabs: [{ targetId: "t1", title: "Docs", url: "https://example.com" }],
      activeTargetId: "t1",
      epoch: 1,
    });
    try {
      await new Promise((r) => setTimeout(r, 15));
      const frameReq = h.posted.filter((m) => m.params?.name === "chrome_stream_frame").pop();
      assert.ok(frameReq);

      // Simulate user closing the last Chrome window/tab while connected
      h.msgHandler({
        source: h.win.parent,
        data: {
          jsonrpc: "2.0",
          id: frameReq.id,
          result: {
            structuredContent: {
              connected: true,
              noTabs: true,
              tabs: [],
              activeTargetId: null,
              changed: false,
            },
          },
        },
      });
      await new Promise((r) => setTimeout(r, 15));

      assert.equal(h.els.get("lb")!.hidden, true, "loading bar must clear when all tabs/windows close");
      assert.equal(h.els.get("ph")!.hidden, false, "empty state overlay must be visible");
      assert.equal(h.els.get("pm")!.textContent, "Chrome window closed");
      assert.equal(h.els.get("pa2")!.hidden, false, "New Tab + Disconnect action row must be visible");

      // Clicking New Tab (#n2) sends chrome://newtab
      h.els.get("n2")!.listeners.get("click")![0]();
      await new Promise((r) => setTimeout(r, 10));
      const newTabCall = h.posted.filter((m) => m.params?.name === "chrome_stream_input").pop();
      assert.equal(newTabCall?.params?.arguments?.events?.[0]?.action, "new_tab");
      assert.equal(newTabCall?.params?.arguments?.events?.[0]?.url, "chrome://newtab");
    } finally {
      h.cleanup();
    }
  });

  it("orders quality presets Auto -> Saver -> Low -> Balanced -> High and hides re-auth note in remote mode", async () => {
    const h = bootPanel(
      { connected: true, connectionMode: "remote", tabs: [{ targetId: "t1", title: "Home", url: "https://a.test" }], activeTargetId: "t1" },
      { navLang: "zh-CN" }
    );
    try {
      await new Promise((r) => setTimeout(r, 10));
      const qs = h.els.get("qs")!.children;
      assert.deepEqual(qs.map((b) => b.dataset.q), ["auto", "ultra", "low", "medium", "high"]);
      assert.deepEqual(qs.map((b) => b.textContent), ["自动", "省流", "流畅", "均衡", "高清"]);
      assert.equal(h.els.get("mn")!.hidden, true, "re-auth note must stay hidden in separate Chrome mode");
      assert.equal(h.els.get("rm")!.textContent, "独立");
    } finally {
      h.cleanup();
    }
  });

  it("localizes all 13 backend errorCodes and 5 bridge errors in EN & ZH with Details toggle", async () => {
    const allCodes = [
      "not_detected", "port_busy", "launch_failed", "chrome_missing", "allow_timeout", "allow_denied",
      "conn_lost", "timeout", "tab_changed", "tab_close_failed", "relay_busy", "relay_down", "unconfirmed",
    ];
    for (const c of allCodes) {
      assert.ok(I18N.en[`e_${c}`], `missing EN copy for e_${c}`);
      assert.ok(I18N["zh-CN"][`e_${c}`], `missing ZH copy for e_${c}`);
      assert.ok(!I18N.en[`e_${c}`].includes("CHROME_STREAM_"), `EN e_${c} must not expose env var`);
      assert.ok(!I18N["zh-CN"][`e_${c}`].includes("CHROME_STREAM_"), `ZH e_${c} must not expose env var`);
    }
    assert.equal(mapBridgeErrorKey("Closed"), "b_closed");
    assert.equal(mapBridgeErrorKey("Timeout:tools/call"), "b_timeout");
    assert.equal(mapBridgeErrorKey("Bad init"), "b_init");
    assert.equal(mapBridgeErrorKey("No serverTools"), "b_notools");
    assert.equal(mapBridgeErrorKey("unexpected boom"), "b_generic");

    const h = bootPanel({ connected: false }, { navLang: "zh-CN" });
    try {
      await new Promise((r) => setTimeout(r, 10));
      h.els.get("ob")!.listeners.get("click")![0]();
      await new Promise((r) => setTimeout(r, 10));
      const connCall = h.posted.filter((m) => m.params?.name === "chrome_stream_connect").pop();
      h.msgHandler({
        source: h.win.parent,
        data: {
          jsonrpc: "2.0",
          id: connCall!.id,
          result: { structuredContent: { connected: false, errorCode: "port_busy", error: "raw port detail" } },
        },
      });
      await new Promise((r) => setTimeout(r, 10));
      assert.equal(h.els.get("om")!.textContent, I18N["zh-CN"].e_port_busy);
      assert.equal(h.els.get("ob2")!.hidden, false, "Details toggle must appear when raw error differs");
      assert.equal(h.els.get("od2")!.hidden, true, "raw error details start collapsed");
      h.els.get("ob2")!.listeners.get("click")![0]();
      assert.equal(h.els.get("od2")!.hidden, false, "clicking Details expands raw error");
    } finally {
      h.cleanup();
    }
  });

  it("saves unified settings via chrome_stream_config and hot-reloads on settingsGen bump", async () => {
    const h = bootPanel({
      connected: true,
      connectionMode: "personal",
      tabs: [{ targetId: "t1", title: "Home", url: "https://a.test" }],
      activeTargetId: "t1",
      settings: { lang: "en", defaultQuality: "auto", defaultConnectMode: "auto", bgPollIntervalSec: 0, idlePauseSec: 12 },
      settingsGen: 1,
      settingsPath: "~/.gemini/antigravity/plugin_data/chrome-stream-mcp/settings.json",
    });
    try {
      await new Promise((r) => setTimeout(r, 10));
      assert.equal(h.els.get("mn")!.hidden, false, "personal mode shows neutral re-auth note in popover");

      // Open Settings drawer & change idlePauseSec -> 30
      h.els.get("bg")!.listeners.get("click")![0]({ stopPropagation() {} });
      assert.equal(h.els.get("sw")!.hidden, false, "Settings drawer must open");
      h.els.get("si2")!.value = "30";
      h.els.get("si2")!.listeners.get("change")![0]();
      await new Promise((r) => setTimeout(r, 10));

      const cfgCall = h.posted.filter((m) => m.params?.name === "chrome_stream_config").pop();
      assert.deepEqual(JSON.parse(JSON.stringify(cfgCall?.params?.arguments)), { action: "update", patch: { idlePauseSec: 30 } });

      h.msgHandler({
        source: h.win.parent,
        data: {
          jsonrpc: "2.0",
          id: cfgCall!.id,
          result: { structuredContent: { ok: true, settingsGen: 2, settings: { idlePauseSec: 30 } } },
        },
      });
      await new Promise((r) => setTimeout(r, 10));
      assert.equal(h.els.get("ss")!.textContent, "✓ Saved");
    } finally {
      h.cleanup();
    }
  });

  it("resumes on stage mousedown while paused without forwarding a blind click", async () => {
    const h = bootPanel({
      connected: true,
      paused: true,
      pauseEpoch: 2,
      tabs: [{ targetId: "t1", title: "Home", url: "https://a.test" }],
      activeTargetId: "t1",
    });
    try {
      await new Promise((r) => setTimeout(r, 10));
      h.els.get("vc")!.listeners.get("mousedown")![0]({ button: 0, clientX: 100, clientY: 100, preventDefault() {} });
      await new Promise((r) => setTimeout(r, 10));
      const inCall = h.posted.filter((m) => m.params?.name === "chrome_stream_input").pop();
      assert.deepEqual(JSON.parse(JSON.stringify(inCall?.params?.arguments?.events)), [{ action: "resume" }]);
    } finally {
      h.cleanup();
    }
  });

  it("drops .ok on #op when a standby error is active and uses direct-child .f > .b in CSS", async () => {
    const css = fs.readFileSync(path.join(panelDir, "styles.css"), "utf8");
    assert.ok(css.includes(".f > .b"), "styles.css must scope .b sizing with .f > .b");
    assert.ok(!/\.f\s+\.b\b/.test(css), "styles.css must not use descendant .f .b selector");

    const h = bootPanel({ connected: false, portFile: { valid: true, port: 9222 } }, { navLang: "zh-CN" });
    try {
      await new Promise((r) => setTimeout(r, 10));
      const op = h.els.get("op")!;
      assert.equal(op.classList.contains("ok"), true, "#op shows .ok when portFile is valid and no error");
      assert.equal(op.classList.contains("er"), false);

      h.msgHandler({
        source: h.win.parent,
        data: {
          jsonrpc: "2.0",
          method: "ui/notifications/tool-result",
          params: {
            structuredContent: {
              connected: false,
              portFile: { valid: true, port: 9222 },
              errorCode: "port_busy",
              error: "Debug port 9222 is already in use",
            },
          },
        },
      });
      await new Promise((r) => setTimeout(r, 10));
      assert.equal(op.classList.contains("ok"), false, "#op must drop .ok when standby error is active");
      assert.equal(op.classList.contains("er"), true, "#op should show .er when standby error is active");
      assert.equal(h.els.get("od")!.classList.contains("er"), true);

      // Dismissing/clearing the error restores .ok on #op when portFile is still valid
      h.msgHandler({
        source: h.win.parent,
        data: {
          jsonrpc: "2.0",
          method: "ui/notifications/tool-result",
          params: {
            structuredContent: {
              connected: false,
              portFile: { valid: true, port: 9222 },
            },
          },
        },
      });
      await new Promise((r) => setTimeout(r, 10));
      assert.equal(op.classList.contains("ok"), true, "#op must restore .ok once standby error clears");
      assert.equal(op.classList.contains("er"), false);
    } finally {
      h.cleanup();
    }
  });
});
