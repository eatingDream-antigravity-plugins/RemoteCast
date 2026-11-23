// Client-side adaptive bandwidth, input coalescing, and tab epoch helpers.
const TIERS = ["ultra", "low", "medium", "high"];
const REP = new Set("Backspace Delete ArrowUp ArrowDown ArrowLeft ArrowRight PageUp PageDown Tab".split(" "));
const TERR = /no target with given id|target closed|session with given id not found|inspected target navigated or closed|target.*destroyed/i;

export const isTargetErrorText = (m) => TERR.test(String(m || ""));

export class AdaptiveBandwidthController {
  constructor(m = "auto") {
    this.mode = m; this.effectiveTier = m === "auto" ? "low" : m;
    this.calibrated = false; this.ewmaRttMs = this.lastNetRttMs = this.lastKbps = this.fastStreak = this.lastUpgradeAt = 0;
  }
  setMode(m) {
    this.mode = m === "auto" || m === "low" || m === "medium" || m === "high" ? m : "auto";
    this.fastStreak = 0;
    this.effectiveTier = this.mode !== "auto" ? this.mode : this.calibrated ? this._t(this.ewmaRttMs, 0, 0) : "low";
  }
  getEffectiveQuality() { return this.mode === "auto" ? this.effectiveTier : this.mode; }
  _t(r, b, n) {
    return r >= 750 || (b >= 14000 && n >= 850) ? "ultra" : r >= 350 || (b >= 14000 && n >= 420) ? "low" : r >= 140 ? "medium" : "high";
  }
  recordSample({ totalMs, serverMs = 0, imageBytes = 0, now = Date.now() }) {
    const net = Math.max(1, Math.round((Number(totalMs) || 0) - Math.max(0, Number(serverMs) || 0)));
    this.lastNetRttMs = net;
    let r = this.ewmaRttMs;
    if (!this.calibrated || r <= 0) r = net;
    else { const a = net > r ? 0.5 : 0.25; r = Math.round(r * (1 - a) + net * a); }
    this.ewmaRttMs = r;
    if (imageBytes > 1000 && net > 0) this.lastKbps = Math.round((imageBytes * 8) / net);
    const prev = this.effectiveTier;
    if (this.mode === "auto") {
      const want = this._t(r, imageBytes, net);
      if (!this.calibrated) { this.calibrated = true; this.effectiveTier = want; this.fastStreak = 0; this.lastUpgradeAt = now; }
      else {
        const ci = TIERS.indexOf(prev), di = TIERS.indexOf(want);
        if (di < ci) { this.effectiveTier = want; this.fastStreak = 0; }
        else if (di > ci && ((prev === "ultra" && r < 520) || (prev === "low" && r < 240) || (prev === "medium" && r < 100))) {
          if (++this.fastStreak >= 4 && now - this.lastUpgradeAt >= 6000) { this.effectiveTier = TIERS[ci + 1]; this.fastStreak = 0; this.lastUpgradeAt = now; }
        } else this.fastStreak = 0;
      }
    } else this.calibrated = true;
    const effectiveTier = this.getEffectiveQuality();
    return { netRttMs: net, ewmaRttMs: r, effectiveTier, tierChanged: prev !== effectiveTier };
  }
  getNetworkGrade() { return !this.calibrated ? "good" : this.ewmaRttMs >= 350 ? "weak" : this.ewmaRttMs >= 140 ? "mid" : "good"; }
  computePollTiming({ idleMs = 0, inputBusy = false } = {}) {
    const r = this.ewmaRttMs;
    if (inputBusy) return { waitMs: r > 350 ? 220 : 140, nextDelayMs: Math.min(240, Math.max(90, Math.round(r * 0.35))) };
    if (idleMs < 3500) return { waitMs: r > 350 ? 280 : 180, nextDelayMs: r > 700 ? 320 : r > 350 ? 180 : 60 };
    if (idleMs < 15000) return { waitMs: r > 350 ? 750 : 520, nextDelayMs: r > 700 ? 900 : r > 350 ? 500 : 280 };
    return { waitMs: 1000, nextDelayMs: r > 700 ? 2200 : r > 350 ? 1400 : 750 };
  }
  getWheelCoalesceMs() { return this.ewmaRttMs > 350 ? 140 : 80; }
  getTypeBatchMs() { return this.ewmaRttMs > 350 ? 60 : this.ewmaRttMs > 180 ? 30 : 0; }
}

export function coalesceInputEvents(raw) {
  const out = [], seen = new Set(), cl = (v) => Math.max(-1600, Math.min(1600, v));
  for (const e of raw || []) {
    if (!e?.action) continue;
    const p = out[out.length - 1], a = e.action;
    if (a === "close_tab") {
      const id = e.targetId || "";
      if (id && seen.has(id)) continue;
      if (id) seen.add(id);
      out.push({ ...e }); continue;
    }
    if (p?.action === "switch_tab" && a === "switch_tab") { p.targetId = e.targetId; continue; }
    if (p?.action === "mousemove" && a === "mousemove") { p.x = e.x; p.y = e.y; continue; }
    if (p?.action === "type" && !p.pressEnter && a === "type" && !e.pressEnter) { p.text = (p.text || "") + (e.text || ""); continue; }
    if (p?.action === "scroll" && a === "scroll") {
      p.deltaX = cl((p.deltaX || 0) + (e.deltaX || 0)); p.deltaY = cl((p.deltaY || 0) + (e.deltaY || 0));
      p.x = e.x; p.y = e.y; continue;
    }
    if (p?.action === "key" && a === "key" && p.key === e.key && REP.has(e.key)) {
      const n = (Number(p.repeat) || 1) + (Number(e.repeat) || 1);
      if (n <= 20) { p.repeat = n; continue; }
    }
    out.push({ ...e });
  }
  return out;
}

export function shouldApplyFrameResponse({ reqEpoch, currentEpoch, currentTargetId, payload: d }) {
  if (!d || typeof d !== "object") return { apply: false, reason: "empty" };
  if (reqEpoch !== currentEpoch) return { apply: false, reason: "stale_epoch" };
  if (d.activeTargetId && currentTargetId && d.activeTargetId !== currentTargetId) {
    if (!d.recovered && !(Array.isArray(d.tabs) && !d.tabs.some((t) => t.targetId === currentTargetId))) {
      return { apply: false, reason: "stale_target" };
    }
  }
  return { apply: true, reason: "ok" };
}

export function applyOptimisticTabClose(tabs, aid, cid) {
  const ls = Array.isArray(tabs) ? tabs : [], i = ls.findIndex((t) => t.targetId === cid);
  const nt = ls.filter((t) => t.targetId !== cid), wa = aid === cid;
  const na = wa || !nt.some((t) => t.targetId === aid) ? (nt.length ? nt[i >= 0 ? Math.min(i, nt.length - 1) : 0].targetId : null) : aid;
  return { tabs: nt, activeTargetId: na, wasActive: wa };
}
