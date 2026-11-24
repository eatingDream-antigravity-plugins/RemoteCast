// Client-side adaptive bandwidth, input coalescing, locale resolution, and tab epoch helpers.
const TIERS = ["ultra", "low", "medium", "high"];
const REP = new Set("Backspace Delete ArrowUp ArrowDown ArrowLeft ArrowRight PageUp PageDown Tab".split(" "));
const TERR = /no target with given id|target closed|session with given id not found|inspected target navigated or closed|target.*destroyed/i;

export const isTargetErrorText = (m) => TERR.test(String(m || ""));

export function parsePanelLocaleTag(raw) {
  const s = String(raw || "").trim().toLowerCase().replace(/_/g, "-");
  if (!s || s === "c" || s === "posix" || s === "utf-8") return null;
  return s === "zh" || s.startsWith("zh-") || s.includes("-hans") || s.includes("-hant") ? "zh-CN" : "en";
}

export function resolvePanelLang({ saved = "", hostLocale = "", navLang = "", bootLocale = "" } = {}) {
  for (const c of [saved, hostLocale, navLang, bootLocale]) {
    const r = parsePanelLocaleTag(c);
    if (r) return r;
  }
  return "en";
}

export class AdaptiveBandwidthController {
  constructor(m = "auto") {
    this.mode = m; this.effectiveTier = m === "auto" ? "low" : m;
    this.calibrated = false; this.ewmaRttMs = this.lastNetRttMs = this.lastKbps = this.fastStreak = this.lastUpgradeAt = 0;
  }
  setMode(m) {
    this.mode = m === "auto" || TIERS.includes(m) ? m : "auto";
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
  computePollTiming({ idleMs = 0, inputBusy = false, screencastIdle = false, hidden = false, errCount = 0 } = {}) {
    const r = this.ewmaRttMs;
    const mult = errCount > 0 ? Math.min(4, 1 << Math.min(errCount, 2)) : 1;
    if (hidden) {
      const base = r >= 750 ? 12000 : r >= 350 ? 8000 : r >= 140 ? 4000 : 2000;
      return { waitMs: 0, nextDelayMs: Math.min(30000, base * mult) };
    }
    if (screencastIdle && !inputBusy) return { waitMs: 0, nextDelayMs: Math.min(20000, 4000 * mult) };
    if (inputBusy) return { waitMs: r > 350 ? 220 : 140, nextDelayMs: Math.min(240, Math.max(90, Math.round(r * 0.35))) };
    if (errCount > 0) {
      const base = idleMs < 3500 ? 1000 : idleMs < 12000 ? 1500 : 2000;
      return { waitMs: 0, nextDelayMs: Math.min(16000, base * mult) };
    }
    if (idleMs < 3500) return { waitMs: r > 350 ? 280 : 180, nextDelayMs: r > 700 ? 320 : r > 350 ? 180 : 60 };
    if (idleMs < 12000) return { waitMs: r > 350 ? 750 : 520, nextDelayMs: r > 700 ? 900 : r > 350 ? 500 : 280 };
    return { waitMs: 400, nextDelayMs: r > 700 ? 2800 : r > 350 ? 2000 : 1400 };
  }
  computeOfflinePollDelay({ hidden = false, errCount = 0 } = {}) {
    const r = this.ewmaRttMs;
    const base = hidden
      ? (r >= 750 ? 10000 : r >= 350 ? 7500 : r >= 140 ? 5000 : 3000)
      : (r >= 750 ? 8000 : r >= 350 ? 6000 : r >= 140 ? 4500 : 3000);
    const mult = errCount > 0 ? Math.min(4, 1 << Math.min(errCount, 2)) : 1;
    return Math.min(30000, base * mult);
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
    if (p?.action === "mousemove" && a === "mousemove" && (p.buttons || 0) === (e.buttons || 0)) {
      p.x = e.x; p.y = e.y;
      if (e.modifiers !== undefined) p.modifiers = e.modifiers; else delete p.modifiers;
      continue;
    }
    if (p?.action === "type" && !p.pressEnter && a === "type" && !e.pressEnter) { p.text = (p.text || "") + (e.text || ""); continue; }
    if (p?.action === "scroll" && a === "scroll") {
      p.deltaX = cl((p.deltaX || 0) + (e.deltaX || 0)); p.deltaY = cl((p.deltaY || 0) + (e.deltaY || 0));
      p.x = e.x; p.y = e.y; continue;
    }
    if (p?.action === "key" && a === "key" && p.key === e.key && (p.modifiers || 0) === (e.modifiers || 0) && REP.has(e.key)) {
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
    if (!d.recovered && !d.popupSwitched && !(Array.isArray(d.tabs) && !d.tabs.some((t) => t.targetId === currentTargetId))) {
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

export const isNoTabsPayload = (d) => !!(d?.noTabs || (Array.isArray(d?.tabs) && d.tabs.length === 0 && !d?.activeTargetId));
export const isSafeImageUrl = (u) => typeof u === "string" && u.startsWith("data:image/");

export function applyFramePayloadState(S, d, reqEpoch = S.ep, totalMs = 0) {
  if (!shouldApplyFrameResponse({ reqEpoch, currentEpoch: S.ep, currentTargetId: S.aid, payload: d }).apply) return null;
  S.on = true;
  if (typeof d.paused === "boolean" && !totalMs) { S.pa = d.paused; S.st = !d.paused; }
  S.id = !S.pa && !!d.screencastPausedIdle;
  S.dlg = d.dialog || null;
  const nt = isNoTabsPayload(d);
  S.nt = nt;
  if (nt) { S.aid = null; S.tabs = []; S.pg = {}; S.dlg = null; S.seq = 0; S.fh = ""; }
  if (d.popupSwitched) { S.seq = 0; S.fh = ""; }
  S.nv = false;
  if (d.tabsHash) S.th = d.tabsHash;
  if (!nt) {
    if (Array.isArray(d.tabs)) S.tabs = d.tabs;
    if (d.activeTargetId) S.aid = d.activeTargetId;
    if (d.page) S.pg = d.page;
  }
  const sp = !nt && !!d.changed && isSafeImageUrl(d.imageDataUrl);
  if (sp) { S.seq = d.seq || S.seq + 1; S.fh = d.frameHash || S.fh; S.by = d.imageBytes || S.by; S.uc = false; }
  else {
    S.uc = !!(d.unchanged || !d.changed);
    if (d.seq) S.seq = Math.max(S.seq, d.seq);
    if (d.frameHash) S.fh = d.frameHash;
  }
  return sp;
}

export function setupRevealObservers(o = {}) {
  const g = globalThis;
  const dc = o.rootDoc ?? g.document;
  const wn = o.rootWin ?? g.window;
  const ap = o.bridgeApp;
  const getLF = o.getLastFetchAt || (() => 0);
  const nw = o.clock || (() => Date.now());
  const sm = o.staleMs ?? 900;
  const tm = o.throttleMs ?? 250;
  const RO = o.ResizeObs ?? g.ResizeObserver;
  const IO = o.IntersectionObs ?? g.IntersectionObserver;
  const offs = [];
  let lt = -1e15;
  const trig = () => { const t = nw(); if (t - lt < tm) return false; lt = t; o.onReveal?.(); return true; };
  const rt = dc?.documentElement;
  const add = (tg, ev, fn, op) => { if (tg?.addEventListener) { tg.addEventListener(ev, fn, op); offs.push(() => tg.removeEventListener?.(ev, fn, op)); } };
  if (RO && rt) {
    let wc = !(rt.clientWidth > 0 && rt.clientHeight > 0);
    const ro = new RO((es) => {
      const r = es?.[0]?.contentRect, v = (r ? r.width : rt.clientWidth || 0) > 0 && (r ? r.height : rt.clientHeight || 0) > 0;
      if (wc && v) trig();
      wc = !v;
    });
    ro.observe(rt); offs.push(() => ro.disconnect?.());
  }
  if (IO && rt) {
    let wv = false;
    const io = new IO((es) => { for (const e of es || []) { const v = !!e?.isIntersecting; if (!wv && v) trig(); wv = v; } }, { threshold: 0 });
    io.observe(rt); offs.push(() => io.disconnect?.());
  }
  add(dc, "visibilitychange", () => { const vs = dc?.visibilityState; if (vs === "hidden") o.onHide?.(); else if (vs === "visible") trig(); });
  add(wn, "focus", trig);
  add(wn, "pageshow", trig);
  add(rt, "pointerenter", () => { if (nw() - getLF() > sm) trig(); }, { passive: true });
  if (ap) {
    const pv = ap.onhostcontextchanged || null;
    ap.onhostcontextchanged = (c) => { pv?.(c); if (c?.containerDimensions || c?.displayMode) trig(); };
    offs.push(() => { ap.onhostcontextchanged = pv; });
  }
  return () => { while (offs.length) try { offs.pop()(); } catch {} };
}
