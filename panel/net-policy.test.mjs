import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { describe, it } from "node:test";
import {
  AdaptiveBandwidthController,
  applyOptimisticTabClose,
  coalesceInputEvents,
  isTargetErrorText,
  parsePanelLocaleTag,
  resolvePanelLang,
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

  it("drops stale frames from previous epoch or mismatched targetId while accepting recovery", () => {
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
  });

  it("handles optimistic tab close for active, non-active, and last tab", () => {
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
  });

  it("ships dark-first, zero-flicker, CSP-clean, jargon-free panel bundle under 44 KiB", () => {
    const html = fs.readFileSync(path.join(__dirname, "dist", "panel.html"), "utf8");
    const bytes = Buffer.byteLength(html, "utf8");
    assert.ok(bytes < 44 * 1024, `panel.html is ${bytes} B (>= 44 KiB)`);
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

  it("resolves bilingual panel locale and backs off polling when screencast is idle", () => {
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
  });
});
