import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { describe, it } from "node:test";
import vm from "node:vm";
import {
  AdaptiveBandwidthController,
  applyFramePayloadState,
  applyOptimisticTabClose,
  coalesceInputEvents,
  isTargetErrorText,
  parsePanelLocaleTag,
  resolvePanelLang,
  setupRevealObservers,
  shouldApplyFrameResponse,
} from "./net-policy.js";
import { applyHostStyleVariables, lum } from "./bridge.js";

const __dirname = path.dirname(fileURLToPath(import.meta.url));

describe("Chrome Stream MCP panel net-policy.js", () => {
  it("adapts quality immediately on high RTT and upgrades conservatively with hysteresis", () => {
    const net = new AdaptiveBandwidthController("auto");
    assert.equal(net.getEffectiveQuality(), "low");

    let s = net.recordSample({ totalMs: 260, serverMs: 200, imageBytes: 15_000, now: 1000 });
    assert.equal(s.netRttMs, 60);
    assert.equal(s.effectiveTier, "high");

    s = net.recordSample({ totalMs: 900, serverMs: 100, imageBytes: 18_000, now: 2000 });
    assert.equal(s.effectiveTier, "low");

    s = net.recordSample({ totalMs: 1400, serverMs: 100, imageBytes: 18_000, now: 3000 });
    assert.equal(s.effectiveTier, "ultra");
    assert.equal(net.getNetworkGrade(), "weak");
    assert.equal(net.getWheelCoalesceMs(), 140);
    assert.equal(net.getTypeBatchMs(), 60);

    s = net.recordSample({ totalMs: 120, serverMs: 50, imageBytes: 10_000, now: 4000 });
    assert.equal(s.effectiveTier, "ultra");

    for (let i = 0; i < 6; i++) {
      s = net.recordSample({ totalMs: 90, serverMs: 40, imageBytes: 10_000, now: 12_000 + i * 200 });
    }
    assert.equal(s.effectiveTier, "low");

    net.setMode("high");
    assert.equal(net.getEffectiveQuality(), "high");
  });

  it("coalesces type, scroll, mousemove, repeatable key, switch_tab, and duplicate close_tab", () => {
    const out = coalesceInputEvents([
      { action: "type", text: "h", pressEnter: false },
      { action: "type", text: "i", pressEnter: false },
      { action: "scroll", x: 10, y: 20, deltaX: 0, deltaY: 120 },
      { action: "scroll", x: 15, y: 25, deltaX: 0, deltaY: 180 },
      { action: "mousemove", x: 1, y: 2 },
      { action: "mousemove", x: 50, y: 60 },
      { action: "key", key: "Backspace" },
      { action: "key", key: "Backspace" },
      { action: "key", key: "Backspace" },
      { action: "switch_tab", targetId: "t1" },
      { action: "switch_tab", targetId: "t2" },
      { action: "close_tab", targetId: "t3" },
      { action: "close_tab", targetId: "t3" },
    ]);
    assert.deepEqual(out, [
      { action: "type", text: "hi", pressEnter: false },
      { action: "scroll", x: 15, y: 25, deltaX: 0, deltaY: 300 },
      { action: "mousemove", x: 50, y: 60 },
      { action: "key", key: "Backspace", repeat: 3 },
      { action: "switch_tab", targetId: "t2" },
      { action: "close_tab", targetId: "t3" },
    ]);
  });

  it("enforces stale-frame filtering, optimistic tab close, and background payload state", () => {
    // Stale epoch & mismatched target filtering vs external close recovery
    assert.equal(
      shouldApplyFrameResponse({
        reqEpoch: 2,
        currentEpoch: 3,
        currentTargetId: "t2",
        payload: { activeTargetId: "t1", changed: true },
      }).apply,
      false
    );
    assert.equal(
      shouldApplyFrameResponse({
        reqEpoch: 3,
        currentEpoch: 3,
        currentTargetId: "t2",
        payload: { activeTargetId: "t1", tabs: [{ targetId: "t1" }, { targetId: "t2" }] },
      }).apply,
      false
    );
    assert.equal(
      shouldApplyFrameResponse({
        reqEpoch: 3,
        currentEpoch: 3,
        currentTargetId: "t2",
        payload: { activeTargetId: "t1", tabs: [{ targetId: "t1" }] },
      }).apply,
      true
    );

    // Optimistic tab close (active, non-active, last) & target-gone classification
    const tabs = [{ targetId: "t1" }, { targetId: "t2" }, { targetId: "t3" }];
    const a = applyOptimisticTabClose(tabs, "t2", "t2");
    assert.equal(a.wasActive, true);
    assert.equal(a.activeTargetId, "t3");

    const na = applyOptimisticTabClose(tabs, "t2", "t3");
    assert.equal(na.wasActive, false);
    assert.equal(na.activeTargetId, "t2");

    const last = applyOptimisticTabClose([{ targetId: "t1" }], "t1", "t1");
    assert.equal(last.wasActive, true);
    assert.equal(last.activeTargetId, null);

    assert.equal(isTargetErrorText("No target with given id found"), true);
    assert.equal(isTargetErrorText("Chrome CDP connection closed"), false);

    // Background state & frame payload updates preserve userPaused and unchanged 0-churn behavior
    const state = {
      on: true, ep: 1, aid: "t1", tabs: [{ targetId: "t1", title: "One" }], th: "th1",
      pg: { url: "https://a.test" }, seq: 5, fh: "fh5", by: 1200, uc: false,
      pa: false, st: true, id: true, dlg: null, nt: false, nv: true,
    };

    const paintUnchanged = applyFramePayloadState(state, {
      connected: true,
      activeTargetId: "t1",
      tabs: [{ targetId: "t1", title: "One" }, { targetId: "t2", title: "Two" }],
      tabsHash: "th2",
      changed: false,
      unchanged: true,
      screencastPausedIdle: true,
    }, 1, 45);
    assert.equal(paintUnchanged, false);
    assert.equal(state.uc, true);
    assert.equal(state.fh, "fh5");
    assert.equal(state.by, 1200);
    assert.equal(state.tabs.length, 2);
    assert.equal(state.th, "th2");
    assert.equal(state.id, true);

    const paintChanged = applyFramePayloadState(state, {
      connected: true,
      activeTargetId: "t2",
      tabs: [{ targetId: "t1", title: "One" }, { targetId: "t2", title: "Two" }],
      tabsHash: "th2",
      page: { title: "Two", url: "https://b.test" },
      seq: 8,
      frameHash: "fh8",
      imageBytes: 2400,
      imageDataUrl: "data:image/jpeg;base64,AAAA",
      changed: true,
      unchanged: false,
      screencastPausedIdle: false,
      popupSwitched: true,
    }, 1, 55);
    assert.equal(paintChanged, true);
    assert.equal(state.uc, false);
    assert.equal(state.seq, 8);
    assert.equal(state.fh, "fh8");
    assert.equal(state.by, 2400);
    assert.equal(state.aid, "t2");
    assert.equal(state.id, false);
    assert.equal(state.pg.url, "https://b.test");

    state.pa = true;
    state.st = false;
    const paintWhilePaused = applyFramePayloadState(state, {
      connected: true,
      activeTargetId: "t2",
      paused: false,
      screencastPausedIdle: false,
      changed: false,
      unchanged: true,
    }, 1, 30);
    assert.equal(paintWhilePaused, false);
    assert.equal(state.pa, true);
    assert.equal(state.id, false);
  });

  it("ships dark-first, zero-flicker, CSP-clean, jargon-free panel bundle under 45 KiB", () => {
    const html = fs.readFileSync(path.join(__dirname, "dist", "panel.html"), "utf8");
    const bytes = Buffer.byteLength(html, "utf8");
    assert.ok(bytes < 45 * 1024, `panel.html is ${bytes} B (>= 45 KiB)`);
    for (const jargon of ["SHA-1", "0600 Unix Socket", "原生 Rust", "Local Loopback", "CDP"]) {
      assert.ok(!html.includes(jargon), `panel.html must not expose internal jargon: ${jargon}`);
    }
    assert.ok(html.includes('<meta name="color-scheme" content="dark">'));
    assert.ok(html.includes('style="background:#141311;color-scheme:dark"'));
    assert.ok(html.includes("background-color:var(--color-background-primary,#141311)"));
    assert.ok(html.includes("background-color:var(--color-background-primary,#FAF8F5)"));
    assert.ok(!html.includes("prefers-color-scheme:light") && !html.includes("prefers-color-scheme: light"));
    assert.ok(html.includes("/*__CHROME_STREAM_BOOT__*/"));
    assert.ok(html.includes("createImageBitmap"));
    assert.ok(html.includes("IntersectionObserver"));
    assert.ok(html.includes("pointerenter"));
    assert.ok(html.includes("pageshow"));
    assert.ok(html.includes("displayMode"));
    assert.ok(html.includes('bH("bp"') && html.includes('bH("dc"'), "always-visible pause and disconnect controls must exist");
    assert.ok(!html.includes("S.on||S.pf?.valid") && !html.includes("S.on || S.pf?.valid"), "must never auto-dial Chrome when disconnected");
    assert.ok(!html.includes("cs-enter"));
    assert.ok(html.includes("prefers-reduced-motion"));
    assert.ok(html.includes("containerDimensions"));
    assert.ok(!/https?:\/\/|wss?:\/\//i.test(html), "panel.html must have zero external URLs");
  });

  it("ignores transparent or low-alpha --color-background-primary and parses alpha in lum()", () => {
    const props = new Map();
    const fakeEl = {
      style: {
        setProperty(k, v) {
          props.set(k, v);
        },
      },
    };

    for (const bad of [
      "",
      "   ",
      "transparent",
      "Transparent",
      "rgba(0, 0, 0, 0)",
      "rgba(255, 255, 255, 0)",
      "rgba(20, 19, 17, 0.5)",
      "rgba(20, 19, 17, 0.89)",
      "rgb(0 0 0 / 0)",
      "rgb(255 255 255 / 50%)",
      "#00000000",
      "#14131180",
      "#0000",
      "hsla(0, 0%, 100%, 0)",
    ]) {
      props.clear();
      applyHostStyleVariables(
        {
          "--color-background-primary": bad,
          "--color-text-primary": "#F4F1EA",
        },
        fakeEl
      );
      assert.equal(
        props.has("--color-background-primary"),
        false,
        `expected --color-background-primary=${JSON.stringify(bad)} to be ignored`
      );
      assert.equal(props.get("--color-text-primary"), "#F4F1EA");
    }

    for (const ok of ["#141311", "#FAF8F5", "#141311ff", "rgb(20, 19, 17)", "rgba(20, 19, 17, 0.95)", "rgb(250 248 245 / 95%)"]) {
      props.clear();
      applyHostStyleVariables({ "--color-background-primary": ok }, fakeEl);
      assert.equal(props.get("--color-background-primary"), ok, `expected opaque ${ok} to be applied`);
    }

    assert.equal(lum("transparent"), null);
    assert.equal(lum("rgba(255, 255, 255, 0)"), null);
    assert.equal(lum("rgba(255, 255, 255, 0.49)"), null);
    assert.equal(lum("rgb(255 255 255 / 40%)"), null);
    assert.equal(lum("#ffffff00"), null);
    assert.equal(lum("#ffffff40"), null);
    assert.ok(lum("rgba(255, 255, 255, 0.5)") > 0.9);
    assert.ok(lum("#141311ff") < 0.1);
    assert.ok(lum("#FAF8F5") > 0.9);
  });

  it("resolves bilingual locale and computes RTT-adaptive idle/hidden poll timings", () => {
    assert.equal(parsePanelLocaleTag("C"), null);
    assert.equal(parsePanelLocaleTag("POSIX"), null);
    assert.equal(parsePanelLocaleTag("zh_TW"), "zh-CN");
    assert.equal(parsePanelLocaleTag("ja-JP"), "en");
    assert.equal(resolvePanelLang({ saved: "en", hostLocale: "zh-CN", navLang: "zh-CN" }), "en");
    assert.equal(resolvePanelLang({ saved: "", hostLocale: "zh-Hans-CN", navLang: "en-US" }), "zh-CN");
    assert.equal(resolvePanelLang({ saved: "", hostLocale: "", navLang: "fr-FR", bootLocale: "zh-CN" }), "en");
    assert.equal(resolvePanelLang({}), "en");

    const net = new AdaptiveBandwidthController("auto");
    assert.deepEqual(net.computePollTiming({ idleMs: 15000, inputBusy: false, screencastIdle: true }), {
      waitMs: 0,
      nextDelayMs: 4000,
    });

    // Uncalibrated / fast RTT (< 140ms)
    assert.deepEqual(net.computePollTiming({ hidden: true, idleMs: 0 }), { waitMs: 0, nextDelayMs: 2000 });
    assert.equal(net.computeOfflinePollDelay({ hidden: true }), 3000);
    assert.equal(net.computeOfflinePollDelay({ hidden: false }), 3000);

    // Fast calibrated (60ms) + error backoff
    net.recordSample({ totalMs: 160, serverMs: 100, imageBytes: 5000, now: 1000 });
    assert.deepEqual(net.computePollTiming({ hidden: true }), { waitMs: 0, nextDelayMs: 2000 });
    assert.deepEqual(net.computePollTiming({ hidden: true, errCount: 1 }), { waitMs: 0, nextDelayMs: 4000 });
    assert.deepEqual(net.computePollTiming({ hidden: true, errCount: 2 }), { waitMs: 0, nextDelayMs: 8000 });

    // Medium RTT (200ms)
    const netMed = new AdaptiveBandwidthController("auto");
    netMed.recordSample({ totalMs: 250, serverMs: 50, imageBytes: 5000, now: 1000 });
    assert.deepEqual(netMed.computePollTiming({ hidden: true }), { waitMs: 0, nextDelayMs: 4000 });
    assert.equal(netMed.computeOfflinePollDelay({ hidden: true }), 5000);

    // Slow RTT (500ms)
    const netSlow = new AdaptiveBandwidthController("auto");
    netSlow.recordSample({ totalMs: 550, serverMs: 50, imageBytes: 5000, now: 1000 });
    assert.deepEqual(netSlow.computePollTiming({ hidden: true }), { waitMs: 0, nextDelayMs: 8000 });
    assert.equal(netSlow.computeOfflinePollDelay({ hidden: true }), 7500);

    // Ultra-slow RTT (900ms) + error backoff cap
    const netUltra = new AdaptiveBandwidthController("auto");
    netUltra.recordSample({ totalMs: 950, serverMs: 50, imageBytes: 5000, now: 1000 });
    assert.deepEqual(netUltra.computePollTiming({ hidden: true }), { waitMs: 0, nextDelayMs: 12000 });
    assert.deepEqual(netUltra.computePollTiming({ hidden: true, errCount: 3 }), { waitMs: 0, nextDelayMs: 30000 });
    assert.equal(netUltra.computeOfflinePollDelay({ hidden: true }), 10000);
    assert.equal(netUltra.computeOfflinePollDelay({ hidden: true, errCount: 2 }), 30000);
  });

  it("wires reveal observers (ResizeObserver, IntersectionObserver, hostContext, pointerenter), throttles, and tears down cleanly", () => {
    const makeEmitter = () => {
      const listeners = new Map();
      return {
        listeners,
        addEventListener(ev, fn, opts) {
          if (!listeners.has(ev)) listeners.set(ev, []);
          listeners.get(ev).push({ fn, opts });
        },
        removeEventListener(ev, fn) {
          const arr = listeners.get(ev);
          if (!arr) return;
          listeners.set(ev, arr.filter((x) => x.fn !== fn));
        },
        emit(ev, arg) {
          for (const item of [...(listeners.get(ev) || [])]) item.fn(arg);
        },
      };
    };

    const rootEl = { ...makeEmitter(), clientWidth: 0, clientHeight: 0 };
    const fakeDoc = { ...makeEmitter(), documentElement: rootEl, visibilityState: "hidden" };
    const fakeWin = makeEmitter();
    let baseHostCtxCalls = 0;
    const fakeApp = {
      onhostcontextchanged: () => {
        baseHostCtxCalls += 1;
      },
    };

    let roCb = null, roDisconnected = false, roTarget = null;
    class FakeRO {
      constructor(cb) { roCb = cb; }
      observe(el) { roTarget = el; }
      disconnect() { roDisconnected = true; }
    }

    let ioCb = null, ioOpts = null, ioDisconnected = false, ioTarget = null;
    class FakeIO {
      constructor(cb, opts) { ioCb = cb; ioOpts = opts; }
      observe(el) { ioTarget = el; }
      disconnect() { ioDisconnected = true; }
    }

    let tNow = 1000, lastFetchAt = 1000, reveals = 0, hides = 0;
    const stop = setupRevealObservers({
      rootDoc: fakeDoc,
      rootWin: fakeWin,
      bridgeApp: fakeApp,
      onReveal: () => { reveals += 1; },
      onHide: () => { hides += 1; },
      getLastFetchAt: () => lastFetchAt,
      clock: () => tNow,
      ResizeObs: FakeRO,
      IntersectionObs: FakeIO,
    });

    assert.equal(roTarget, rootEl);
    assert.equal(ioTarget, rootEl);
    assert.deepEqual(ioOpts, { threshold: 0 });

    // ResizeObserver: collapsed -> visible triggers reveal; already visible resize does not; rapid edge throttled within 250ms
    roCb([{ contentRect: { width: 0, height: 0 } }]);
    assert.equal(reveals, 0);
    roCb([{ contentRect: { width: 420, height: 600 } }]);
    assert.equal(reveals, 1);
    tNow += 300;
    roCb([{ contentRect: { width: 520, height: 640 } }]);
    assert.equal(reveals, 1);

    // IntersectionObserver: !wasVis -> isIntersecting triggers reveal; throttle suppresses immediate duplicate
    ioCb([{ isIntersecting: false }]);
    assert.equal(reveals, 1);
    ioCb([{ isIntersecting: true }]);
    assert.equal(reveals, 2);
    tNow += 100;
    ioCb([{ isIntersecting: false }, { isIntersecting: true }]);
    assert.equal(reveals, 2);
    tNow += 300;
    ioCb([{ isIntersecting: false }]);
    ioCb([{ isIntersecting: true }]);
    assert.equal(reveals, 3);

    // hostContextChanged triggers only for containerDimensions / displayMode
    fakeApp.onhostcontextchanged({ theme: "dark" });
    assert.equal(baseHostCtxCalls, 1);
    assert.equal(reveals, 3);
    tNow += 300;
    fakeApp.onhostcontextchanged({ containerDimensions: { width: 480 } });
    assert.equal(baseHostCtxCalls, 2);
    assert.equal(reveals, 4);
    tNow += 300;
    fakeApp.onhostcontextchanged({ displayMode: "fullscreen" });
    assert.equal(reveals, 5);

    // pointerenter: passive:true, only fires when stale > 900ms
    const ptrEntries = rootEl.listeners.get("pointerenter");
    assert.equal(ptrEntries.length, 1);
    assert.deepEqual(ptrEntries[0].opts, { passive: true });
    lastFetchAt = tNow - 500;
    tNow += 300;
    rootEl.emit("pointerenter");
    assert.equal(reveals, 5);
    lastFetchAt = tNow - 950;
    rootEl.emit("pointerenter");
    assert.equal(reveals, 6);

    // visibilitychange, focus, pageshow
    fakeDoc.visibilityState = "hidden";
    fakeDoc.emit("visibilitychange");
    assert.equal(hides, 1);
    tNow += 300;
    fakeDoc.visibilityState = "visible";
    fakeDoc.emit("visibilitychange");
    assert.equal(reveals, 7);
    tNow += 300;
    fakeWin.emit("focus");
    assert.equal(reveals, 8);
    tNow += 300;
    fakeWin.emit("pageshow");
    assert.equal(reveals, 9);

    // Teardown cleanly disconnects observers and removes listeners
    stop();
    assert.equal(roDisconnected, true);
    assert.equal(ioDisconnected, true);
    assert.equal(fakeDoc.listeners.get("visibilitychange").length, 0);
    assert.equal(fakeWin.listeners.get("focus").length, 0);
    assert.equal(fakeWin.listeners.get("pageshow").length, 0);
    assert.equal(rootEl.listeners.get("pointerenter").length, 0);
    fakeApp.onhostcontextchanged({ displayMode: "inline" });
    assert.equal(reveals, 9);
    assert.equal(baseHostCtxCalls, 4);
  });

  it("boots built panel.html cleanly in sandboxed vm (throwing localStorage + strict WebIDL timers)", async () => {
    const html = fs.readFileSync(path.join(__dirname, "dist", "panel.html"), "utf8");
    const match = html.match(/<script>\/\*__CHROME_STREAM_BOOT__\*\/<\/script><script>([\s\S]*?)<\/script>/);
    assert.ok(match, "built panel.html must contain inline bundle script");
    const js = match[1];

    const mkEl = (tag = "div") => {
      const attrs = new Map();
      const listeners = new Map();
      return {
        tagName: tag.toUpperCase(),
        style: { setProperty() {}, colorScheme: "", backgroundColor: "" },
        dataset: {},
        hidden: false,
        textContent: "",
        value: "",
        title: "",
        classList: { toggle() {}, remove() {}, add() {} },
        setAttribute(k, v) { attrs.set(k, String(v)); },
        getAttribute(k) { return attrs.get(k) || null; },
        addEventListener(ev, fn) {
          if (!listeners.has(ev)) listeners.set(ev, []);
          listeners.get(ev).push(fn);
        },
        removeEventListener(ev, fn) {
          listeners.set(ev, (listeners.get(ev) || []).filter((f) => f !== fn));
        },
        querySelector() { return mkEl(); },
        querySelectorAll() { return []; },
        append() {},
        appendChild() {},
        insertBefore() {},
        focus() {},
        select() {},
        getContext() { return { drawImage() {} }; },
        getBoundingClientRect() { return { width: 400, height: 600 }; },
        clientWidth: 0,
        clientHeight: 0,
        listeners,
      };
    };

    const els = new Map();
    const rootEl = mkEl("html");
    const doc = {
      ...mkEl("document"),
      documentElement: rootEl,
      body: mkEl("body"),
      head: mkEl("head"),
      visibilityState: "hidden",
      getElementById(id) {
        if (!els.has(id)) els.set(id, mkEl(id));
        return els.get(id);
      },
      createElement(t) { return mkEl(t); },
    };

    const posted = [];
    const winListeners = new Map();
    const win = {
      parent: { postMessage(msg) { posted.push(msg); } },
      addEventListener(ev, fn) {
        if (!winListeners.has(ev)) winListeners.set(ev, []);
        winListeners.get(ev).push(fn);
      },
      removeEventListener(ev, fn) {
        winListeners.set(ev, (winListeners.get(ev) || []).filter((f) => f !== fn));
      },
      innerWidth: 400,
    };

    const activeTimers = new Set();
    let roCb = null, roDisc = false, ioDisc = false;
    let fakeNowMs = 10_000;

    const ctx = vm.createContext({
      window: win,
      document: doc,
      navigator: { language: "en-US" },
      performance: { now: () => fakeNowMs },
      console,
      requestAnimationFrame: (fn) => setTimeout(fn, 0),
      cancelAnimationFrame: clearTimeout,
      ResizeObserver: class {
        constructor(cb) { roCb = cb; }
        observe() {}
        disconnect() { roDisc = true; }
      },
      IntersectionObserver: class {
        observe() {}
        disconnect() { ioDisc = true; }
      },
      Image: class {},
      Map, Set, Array, Object, Math, JSON, RegExp, Promise, Error, Number, String, Boolean,
      Date: { now: () => fakeNowMs },
      getComputedStyle: () => ({ getPropertyValue: () => "" }),
      __CHROME_STREAM_BOOT__: {
        connected: true,
        paused: false,
        tabs: [{ targetId: "t1", title: "Home", url: "https://a.test" }],
        activeTargetId: "t1",
        seq: 1,
        frameHash: "h1",
      },
    });
    const vmGlobal = vm.runInContext("globalThis", ctx);

    Object.defineProperty(ctx, "localStorage", {
      configurable: true,
      get() {
        const err = new Error("Blocked localStorage in sandboxed iframe");
        err.name = "SecurityError";
        throw err;
      },
    });
    ctx.setTimeout = function strictSetTimeout(fn, ms, ...args) {
      if (this !== vmGlobal) throw new TypeError("Illegal invocation");
      const id = setTimeout(() => {
        activeTimers.delete(id);
        fn(...args);
      }, ms);
      activeTimers.add(id);
      return id;
    };
    ctx.clearTimeout = function strictClearTimeout(id) {
      if (this !== vmGlobal) throw new TypeError("Illegal invocation");
      activeTimers.delete(id);
      clearTimeout(id);
    };

    try {
      vm.runInContext(js, ctx);

      const initReq = posted.find((m) => m.method === "ui/initialize");
      assert.ok(initReq, "panel must send ui/initialize without crashing on blocked localStorage");
      const msgHandler = winListeners.get("message")?.[0];
      assert.equal(typeof msgHandler, "function");

      msgHandler({
        source: win.parent,
        data: {
          jsonrpc: "2.0",
          id: initReq.id,
          result: { hostContext: { theme: "dark" }, hostCapabilities: { serverTools: true } },
        },
      });
      await new Promise((r) => setTimeout(r, 15));

      const frameCalls = posted.filter((m) => m.method === "tools/call" && m.params?.name === "chrome_stream_frame");
      assert.ok(frameCalls.length >= 1, "panel must start background polling when hidden");
      assert.equal(frameCalls[0].params.arguments.waitMs, 0);

      msgHandler({
        source: win.parent,
        data: {
          jsonrpc: "2.0",
          id: frameCalls[0].id,
          result: {
            structuredContent: {
              connected: true,
              activeTargetId: "t1",
              changed: false,
              unchanged: true,
              screencastPausedIdle: true,
            },
          },
        },
      });
      await new Promise((r) => setTimeout(r, 10));

      fakeNowMs += 300;
      roCb([{ contentRect: { width: 420, height: 600 } }]);
      await new Promise((r) => setTimeout(r, 10));

      const callsAfterReveal = posted.filter((m) => m.method === "tools/call" && m.params?.name === "chrome_stream_frame");
      assert.ok(callsAfterReveal.length >= 2, "ResizeObserver reveal must trigger immediate refresh");
      assert.equal(callsAfterReveal[callsAfterReveal.length - 1].params.arguments.forceRefresh, true);

      msgHandler({
        source: win.parent,
        data: { jsonrpc: "2.0", id: 999, method: "ui/resource-teardown", params: {} },
      });
      assert.equal(roDisc, true);
      assert.equal(ioDisc, true);
    } finally {
      for (const id of activeTimers) clearTimeout(id);
      activeTimers.clear();
    }
  });
});
