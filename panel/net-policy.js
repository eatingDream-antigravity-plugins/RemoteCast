// Client-side adaptive bandwidth (EWMA + asymmetric hysteresis), input coalescing, locale & epoch helpers.
const TIERS = ["ultra", "low", "medium", "high"];
export const REP = new Set("Backspace Delete ArrowUp ArrowDown ArrowLeft ArrowRight PageUp PageDown Tab".split(" "));
export const isTargetErrorText = (m) => /no target with given id|target closed|session with given id not found|inspected target navigated or closed|target.*destroyed/i.test(String(m || ""));
export const rttBand = (ms) => { const n = +ms; return !nFin(n) || n <= 0 ? "ok" : n >= 200 ? "er" : n >= 80 ? "wn" : "ok"; };
export const parsePanelLocaleTag = (raw) => {
  const s = String(raw || "").trim().toLowerCase().replace(/_/g, "-");
  return !s || /^(c|posix|utf-8)$/.test(s) ? null : /^zh($|-)|-(hans|hant)/.test(s) ? "zh-CN" : "en";
};
export const resolvePanelLang = ({ saved = "", hostLocale = "", navLang = "", bootLocale = "" } = {}) =>
  [saved, hostLocale, navLang, bootLocale].map(parsePanelLocaleTag).find(Boolean) || "en";

const tierVal = (r, u, l, m, h) => (r >= 750 ? u : r >= 350 ? l : r >= 140 ? m : h);
const clamp = (v, lo, hi) => Math.max(lo, Math.min(hi, v)), nFin = Number.isFinite, eb = (c) => (c > 0 ? Math.min(4, 1 << Math.min(c, 2)) : 1);

export class AdaptiveBandwidthController {
  constructor(m = "auto") {
    const s = this; s.mode = m; s.effectiveTier = m === "auto" ? "low" : m;
    s.calibrated = false; s.ewmaRttMs = s.lastNetRttMs = s.lastKbps = s.fastStreak = s.slowStreak = s.lastUpgradeAt = 0;
  }
  setMode(m) {
    this.mode = m === "auto" || TIERS.includes(m) ? m : "auto"; this.fastStreak = this.slowStreak = 0;
    this.effectiveTier = this.mode !== "auto" ? this.mode : this.calibrated ? this._t(this.ewmaRttMs, 0, 0) : "low";
  }
  getEffectiveQuality() { return this.mode === "auto" ? this.effectiveTier : this.mode; }
  _t(r, b, n) { return r >= 750 || (b >= 14000 && n >= 850) ? "ultra" : r >= 350 || (b >= 14000 && n >= 420) ? "low" : r >= 140 ? "medium" : "high"; }
  recordSample({ totalMs: tm, serverMs: sm = 0, imageBytes: ib = 0, now: nw = Date.now() }) {
    const s = this, net = Math.max(1, Math.round((+tm || 0) - Math.max(0, +sm || 0)));
    s.lastNetRttMs = net;
    const r = !s.calibrated || s.ewmaRttMs <= 0 ? net : Math.round(s.ewmaRttMs * 0.75 + net * 0.25);
    s.ewmaRttMs = r;
    if (ib > 1000 && net > 0) s.lastKbps = Math.round((ib * 8) / net);
    const prev = s.effectiveTier;
    if (s.mode === "auto") {
      const want = s._t(r, ib, net);
      if (!s.calibrated) { s.calibrated = true; s.effectiveTier = want; s.fastStreak = s.slowStreak = 0; s.lastUpgradeAt = nw; }
      else {
        const ci = TIERS.indexOf(prev), di = TIERS.indexOf(want), ri = TIERS.indexOf(s._t(net, ib, net)), up = prev === "ultra" ? 520 : prev === "low" ? 240 : 100;
        if (di < ci && ri < ci) { s.fastStreak = 0; if (++s.slowStreak >= 2) { s.effectiveTier = TIERS[Math.max(di, ri)]; s.slowStreak = 0; } }
        else if (di > ci && ri > ci && r < up && net < up) {
          s.slowStreak = 0;
          if (++s.fastStreak >= 3 && nw - s.lastUpgradeAt >= 4000) { s.effectiveTier = TIERS[ci + 1]; s.fastStreak = 0; s.lastUpgradeAt = nw; }
        } else s.fastStreak = s.slowStreak = 0;
      }
    } else s.calibrated = true;
    const effectiveTier = s.getEffectiveQuality();
    return { netRttMs: net, ewmaRttMs: r, effectiveTier, tierChanged: prev !== effectiveTier };
  }
  getNetworkGrade() { return !this.calibrated ? "good" : this.ewmaRttMs >= 350 ? "weak" : this.ewmaRttMs >= 140 ? "mid" : "good"; }
  computePollTiming({ idleMs: im = 0, inputBusy: ib = false, screencastIdle: si = false, hidden: h = false, errCount: ec = 0, settled: st = false } = {}) {
    const r = this.ewmaRttMs, m = eb(ec), p = (waitMs, d) => ({ waitMs, nextDelayMs: d });
    if (h) return p(0, Math.min(30000, tierVal(r, 12000, 8000, 4000, 2000) * m));
    if (si && !ib) return p(0, Math.min(20000, 4000 * m));
    if (ib) return p(r > 350 ? 220 : 140, clamp(Math.round(r * 0.3), 75, 220));
    if (ec > 0) return p(0, Math.min(16000, (im < 3500 ? 1000 : im < 12000 ? 1500 : 2000) * m));
    const t3 = (a, b, c) => (r > 700 ? a : r > 350 ? b : c);
    return st && im >= 1000 ? p(1400, t3(600, 350, 180)) : im < 3500 ? p(r > 350 ? 280 : 180, t3(300, 160, 50)) : im < 12000 ? p(1350, t3(800, 450, 240)) : p(1400, t3(2400, 1600, 1100));
  }
  computeOfflinePollDelay({ hidden: h = false, errCount: ec = 0 } = {}) {
    const r = this.ewmaRttMs;
    return Math.min(30000, (h ? tierVal(r, 10000, 7500, 5000, 3000) : tierVal(r, 8000, 6000, 4500, 3000)) * eb(ec));
  }
  getWheelCoalesceMs() { return 75; }
  getTypeBatchMs() { return this.ewmaRttMs > 350 ? 60 : this.ewmaRttMs > 180 ? 30 : 0; }
}

export const coalesceInputEvents = (raw) => {
  const out = [], seen = new Set();
  for (const e of raw || []) {
    if (!e?.action) continue;
    const p = out[out.length - 1], a = e.action;
    if (a === "close_tab") { const k = `${e.targetId || ""}:${e.targetEpoch || 0}`; if (e.targetId && seen.has(k)) continue; e.targetId && seen.add(k); out.push({ ...e }); continue; }
    if (p?.action === "switch_tab" && a === "switch_tab") { p.targetId = e.targetId; continue; }
    const pa = p && p.action === a && (p.targetId || "") === (e.targetId || "") && (p.targetEpoch || 0) === (e.targetEpoch || 0) ? a : "";
    if (pa === "mousemove" && (p.buttons || 0) === (e.buttons || 0)) {
      p.x = e.x; p.y = e.y; e.modifiers !== undefined ? (p.modifiers = e.modifiers) : delete p.modifiers; continue;
    }
    if (pa === "type" && !p.pressEnter && !e.pressEnter) { p.text = (p.text || "") + (e.text || ""); continue; }
    if (pa === "scroll") {
      p.deltaX = clamp((p.deltaX || 0) + (e.deltaX || 0), -1600, 1600);
      p.deltaY = clamp((p.deltaY || 0) + (e.deltaY || 0), -1600, 1600);
      p.x = e.x; p.y = e.y; continue;
    }
    if (pa === "key" && p.key === e.key && (p.modifiers || 0) === (e.modifiers || 0) && REP.has(e.key)) {
      const n = (+p.repeat || 1) + (+e.repeat || 1);
      if (n <= 20) { p.repeat = n; continue; }
    }
    out.push({ ...e });
  }
  return out;
};

export const shouldApplyFrameResponse = ({ reqEpoch: re, currentEpoch: ce, currentTargetId: ct, payload: d }) => {
  const r = (apply, reason) => ({ apply, reason });
  return !d || typeof d !== "object" ? r(false, "empty")
    : re !== ce ? r(false, "stale_epoch")
    : d.activeTargetId && ct && d.activeTargetId !== ct && !d.recovered && !d.popupSwitched && !(Array.isArray(d.tabs) && !d.tabs.some((t) => t.targetId === ct)) ? r(false, "stale_target")
    : r(true, "ok");
};

export const applyOptimisticTabClose = (tabs, aid, cid) => {
  const ls = Array.isArray(tabs) ? tabs : [], i = ls.findIndex((t) => t.targetId === cid), nt = ls.filter((t) => t.targetId !== cid), wa = aid === cid;
  return { tabs: nt, activeTargetId: wa || !nt.some((t) => t.targetId === aid) ? (nt[i >= 0 ? Math.min(i, nt.length - 1) : 0]?.targetId ?? null) : aid, wasActive: wa };
};

export const isNoTabsPayload = (d) => !!(d?.noTabs || (Array.isArray(d?.tabs) && !d.tabs.length && !d?.activeTargetId));
export const isSafeImageUrl = (u) => typeof u === "string" && u.startsWith("data:image/");

export const applyFramePayloadState = (S, d, reqEpoch = S.ep, totalMs = 0) => {
  if (!shouldApplyFrameResponse({ reqEpoch, currentEpoch: S.ep, currentTargetId: S.aid, payload: d }).apply) return null;
  S.on = true;
  const srvEp = +d.epoch;
  nFin(srvEp) && srvEp > 0 && (S.tep = srvEp);
  d.connectionMode && (S.cm = d.connectionMode);
  if (typeof d.paused === "boolean") {
    const pe = +d.pauseEpoch, ok = nFin(pe), cur = +S.pe || 0;
    (ok ? pe > cur || (!totalMs && pe >= cur) : !totalMs) && (ok && (S.pe = pe), S.pa = d.paused, S.st = !d.paused);
  }
  S.id = !S.pa && !!d.screencastPausedIdle; S.dlg = d.dialog || null; S.nv = false;
  const nt = S.nt = isNoTabsPayload(d);
  nt ? (S.aid = S.dlg = null, S.tabs = [], S.pg = {}, S.seq = 0, S.fh = "", S.cr = false)
    : (Array.isArray(d.tabs) && (S.tabs = d.tabs), d.activeTargetId && (S.aid = d.activeTargetId), d.page && (S.pg = d.page));
  d.popupSwitched && (S.seq = 0, S.fh = "", S.cr = false);
  d.tabsHash && (S.th = d.tabsHash);
  typeof d.crisp === "boolean" && (S.cr = d.crisp);
  d.format && (S.fm = d.format);
  const sp = !nt && !!d.changed && isSafeImageUrl(d.imageDataUrl);
  sp ? (S.seq = d.seq || S.seq + 1, S.fh = d.frameHash || S.fh, S.by = d.imageBytes || S.by, S.uc = false)
    : (S.uc = !!(d.unchanged || !d.changed), d.seq && (S.seq = Math.max(S.seq, d.seq)), d.frameHash && (S.fh = d.frameHash));
  return sp;
};

export const resolveStageAspect = (viewW, viewH, imgW, imgH) => {
  const va = +viewW > 0 && +viewH > 0 ? viewW / viewH : 0, ia = +imgW > 0 && +imgH > 0 ? imgW / imgH : 0;
  const a = va ? (ia && Math.abs(ia - va) / va > 0.04 ? ia : va) : ia;
  return a > 0 ? +a.toFixed(4) : 0;
};

export const setupRevealObservers = (o = {}) => {
  const g = globalThis, dc = o.rootDoc ?? g.document, wn = o.rootWin ?? g.window, ap = o.bridgeApp;
  const getLF = o.getLastFetchAt || (() => 0), nw = o.clock || (() => Date.now()), sm = o.staleMs ?? 900, tm = o.throttleMs ?? 250;
  const RO = o.ResizeObs ?? g.ResizeObserver, IO = o.IntersectionObs ?? g.IntersectionObserver, offs = [], rt = dc?.documentElement;
  let lt = -1e15;
  const trig = () => { const t = nw(); if (t - lt < tm) return false; lt = t; o.onReveal?.(); return true; };
  const add = (tg, ev, fn, op) => { tg?.addEventListener && (tg.addEventListener(ev, fn, op), offs.push(() => tg.removeEventListener?.(ev, fn, op))); };
  const obs = (ob) => { ob.observe(rt); offs.push(() => ob.disconnect?.()); };
  if (RO && rt) {
    let wc = !(rt.clientWidth > 0 && rt.clientHeight > 0);
    obs(new RO((es) => { const r = es?.[0]?.contentRect, v = (r ? r.width : rt.clientWidth || 0) > 0 && (r ? r.height : rt.clientHeight || 0) > 0; wc && v && trig(); wc = !v; }));
  }
  if (IO && rt) {
    let wv = false;
    obs(new IO((es) => { for (const e of es || []) { const v = !!e?.isIntersecting; !wv && v && trig(); wv = v; } }, { threshold: 0 }));
  }
  add(dc, "visibilitychange", () => { const vs = dc?.visibilityState; vs === "hidden" ? o.onHide?.() : vs === "visible" && trig(); });
  add(wn, "focus", trig); add(wn, "pageshow", trig);
  add(rt, "pointerenter", () => nw() - getLF() > sm && trig(), { passive: true });
  if (ap) {
    const pv = ap.onhostcontextchanged || null;
    ap.onhostcontextchanged = (c) => { pv?.(c); (c?.containerDimensions || c?.displayMode) && trig(); };
    offs.push(() => { ap.onhostcontextchanged = pv; });
  }
  return () => { while (offs.length) try { offs.pop()(); } catch {} };
};
