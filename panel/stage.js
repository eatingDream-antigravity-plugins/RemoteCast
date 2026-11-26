import { App, applyHostContext } from "./bridge.js";
import {
  AdaptiveBandwidthController, applyFramePayloadState, applyOptimisticTabClose, coalesceInputEvents,
  isNoTabsPayload, isSafeImageUrl, isTargetErrorText, resolvePanelLang, resolveStageAspect, rttBand, setupRevealObservers,
} from "./net-policy.js";

const doc = document, boot = globalThis.__CHROME_STREAM_BOOT__ || null, fin = Number.isFinite, rnd = Math.round, isA = Array.isArray;
const on = (n, ev, fn, o) => n?.addEventListener(ev, fn, o), ck = (n, fn) => on(n, "click", fn), sa = (n, k, v) => n?.setAttribute(k, String(v));
const pd = (e) => e.preventDefault(), tx = (n, v) => { if (n.textContent !== v) n.textContent = v; }, hd = (n, v) => { n.hidden = !!v; };
const tg = (n, c, v) => n.classList.toggle(c, !!v), pNow = () => performance.now(), nowMs = () => Date.now(), cl = (v, lo, hi) => Math.max(lo, Math.min(hi, v));
const sTm = (f, m) => globalThis.setTimeout(f, m), cTm = (i) => globalThis.clearTimeout(i), hid = () => doc.visibilityState === "hidden";
const root = doc.getElementById("app"), app = new App({ name: "Chrome Stream", version: "0.4.0" }, {}), net = new AdaptiveBandwidthController("auto");
const KEYS = new Set("Enter Backspace Delete Tab Escape ArrowUp ArrowDown ArrowLeft ArrowRight PageUp PageDown Home End".split(" "));
const QK = "auto low medium high ultra".split(" "), IK = "Enter Escape Tab Backspace Undo Copy Paste SelectAll".split(" ");
const DLG = "alert confirm prompt beforeunload".split(" "), PAGE_ACT = new Set("click mousedown mouseup mousemove scroll type key dialog navigate back forward reload".split(" "));
const BN = ["left", "middle", "right"], BM = [1, 4, 2];
const lsG = (k) => { try { return localStorage.getItem(k) || ""; } catch { return ""; } };
const lsS = (k, v) => { try { localStorage.setItem(k, v); } catch {} };

const D = {}, TAGS = { "!": "button", b: "button", s: "span", i: "input" }, ARIA = { e: "expanded", p: "pressed", c: "checked", h: "haspopup", l: "label", i: "hidden", v: "live" };
const h = (sel, at, ...kids) => {
  const hid0 = sel.endsWith("!"), s0 = hid0 ? sel.slice(0, -1) : sel;
  const sp = s0.split("#"), tp = sp[0].split("."), raw = tp.shift(), tag = TAGS[raw] || raw || "div";
  if (raw === "!") tp.unshift("b");
  const n = doc.createElement(tag);
  if (tag === "button") n.type = "button";
  if (tp.length) n.className = tp.join(" ");
  if (sp[1]) D[n.id = sp[1]] = n;
  if (hid0) n.hidden = true;
  if (at) for (const kv of at.split(";")) {
    const j = kv.indexOf("=");
    if (j < 1) continue;
    const k = kv.slice(0, j), v = kv.slice(j + 1);
    if (k === "t") n.textContent = v;
    else if (k[0] === ".") n.dataset[k.slice(1)] = v;
    else sa(n, k[0] === "@" ? `aria-${ARIA[k.slice(1)]}` : k, v);
  }
  for (const c of kids) if (c != null && c !== false) n.append(isA(c) ? h(...c) : typeof c === "string" ? doc.createTextNode(c) : c);
  return n;
};
const IC = [
  "M2 3.5h12v9H2z;M2 6h12;M7 8l3 1.6L7 11.2z", "M5.5 3.5v9M10.5 3.5v9", "M5 3.5l7 4.5-7 4.5z",
  "M8 2.2v5.3;M4.6 4.4a5.2 5.2 0 1 0 6.8 0", "M10 3.5L5.5 8 10 12.5", "M6 3.5L10.5 8 6 12.5",
  "M13 8a5 5 0 1 1-1.3-3.4;M13 2.5v2.6h-2.6", "M2 4.5h12v7H2z;M4 7.5h1M7 7.5h2M11 7.5h1M5 9.8h6",
  "M3.5 8h.01M8 8h.01M12.5 8h.01", "M8 3.5v9M3.5 8h9",
];
const ico = (idx, sz = 14, pid) => {
  const ns = "http://www.w3.org/2000/svg", s = doc.createElementNS(ns, "svg");
  sa(s, "class", "ic"); sa(s, "width", sz); sa(s, "height", sz); sa(s, "viewBox", "0 0 16 16"); sa(s, "aria-hidden", "true");
  for (const dp of IC[idx].split(";")) { const p = doc.createElementNS(ns, "path"); sa(p, "d", dp); if (pid) D[pid] = p; s.appendChild(p); }
  return s;
};

const GC = ["chrome://inspect/#remote-debugging", "--remote-debugging-port=9222 --user-data-dir=/tmp/chrome-debug"];
const I18N = {
  en: (
    "Not detected|Retry|Connect Personal Chrome|Launch Remote Chrome (No Allow)|Standby keeps Allow alive|Connecting…|Setup|Open Chrome.|Enable $0 or run $1.|Click *Connect Personal Chrome* & Allow, or launch Remote Chrome.|" +
    "Tabs|New tab|Close|Standby|Wake|Revoke & Disconnect|Revokes Chrome Allow (re-Allow locally)|Back|Forward|Reload|URL or search|Quality|Profile|Mode|Personal|Remote Profile|" +
    "Auto|Low|Med|High|Ultra|Frame|Viewport|Refresh|Cached 0 KB|Keyboard|Live view|No tabs|Loading…|Standby · Wake|Idle · Wake|Type or paste…|Insert|Send ↵|" +
    "Bksp|Undo|Copy|Paste|Select all|Scroll up|Scroll down|Standby|Idle|OK|Cancel|Reply…|Alert|Confirm|Prompt|Leave?|" +
    "✓ Sent|Disconnected|✓ Awake|↻ Refresh|✓ Closed|Switching…|✓ Switched|↗ New tab|✓ Copied|✓ Pasted|Right click|Tab changed|Menu|Dismiss|Ready · |Auto · | · q |Init: "
  ).split("|"),
  "zh-CN": (
    "未检测|重试|连接日常 Chrome|启动独立远程 Chrome（免 Allow）|休眠免重弹 Allow|连接中…|设置|打开 Chrome|在 $0 开调试或运行 $1|点 *连接日常 Chrome* 允许，或启远程模式|" +
    "标签|新建|关闭|休眠|唤醒|彻底断开 · 撤销授权|将撤销 Chrome 授权，远程无法点 Allow|后退|前进|刷新|网址或搜索|画质|档位|模式|日常配置|独立远程|" +
    "自动|弱网|均衡|高清|极速|单帧|视口|刷新|未变 0 KB|键盘|画面|无标签|加载中…|休眠·唤醒|空闲·唤醒|输入或粘贴…|输入|发送 ↵|" +
    "退格|撤销|复制|粘贴|全选|上滚|下滚|休眠保活|空闲省电|确定|取消|输入…|提示|确认|输入|离开？|" +
    "✓ 已发|已断开|✓ 唤醒|↻ 刷新|✓ 已关|切换中|✓ 已切|↗ 新标|✓ 复制|✓ 粘贴|右键|标签已切|更多|关闭|就绪 · |自动 · | · 待发 |初始化："
  ).split("|"),
};
const S = {
  on: !!boot?.connected, cm: boot?.connectionMode || "", cr: !!boot?.crisp, fm: boot?.format || "",
  tabs: isA(boot?.tabs) ? boot.tabs : [], th: boot?.tabsHash || "",
  aid: boot?.activeTargetId || null, ep: Number(boot?.epoch) || 1, tep: Number(boot?.epoch) || 1, pe: Number(boot?.pauseEpoch) || 0, pg: boot?.page || {},
  seq: boot?.seq || 0, fh: boot?.frameHash || "", by: boot?.imageBytes || 0, uc: false,
  nv: false, st: !boot?.paused, pa: !!boot?.paused, id: !!boot?.screencastPausedIdle,
  dlg: boot?.dialog || null, nt: !!(boot?.connected && isNoTabsPayload(boot)),
};
let curLg = resolvePanelLang({ saved: lsG("chrome_stream_lang"), navLang: globalThis.navigator?.language || "", bootLocale: boot?.locale || "" });
let portF = boot?.portFile || null, tabSig = "", qApp = boot?.qualityApplied || "low", errMsg = null;
let isCn = false, isSw = false, qMode = "auto", dlgKey = "", imeOn = false, popOn = false, moreOn = false;
const L = () => I18N[curLg] || I18N.en;

let qIn = [], inB = 0, inT = 0, pB = 0, pT = 0, oT = 0, oB = 0, urg = 0, act = nowMs(), tT = 0;
let pErr = 0, offErr = 0, offUrg = 0, lastFetchAt = boot ? nowMs() : 0, lastForceAt = 0, stopReveal = null;
let pGen = 0, painted = 0, started = 0, synced = 0, ready = 0, tSig = "", lAr = 0, fTimes = [], fpsVal = 0, lastGuideLg = "";
const closing = new Set(), tMap = new Map();
let lNodes = [], gNodes = [], qBtns = [], lgBtns = [];

const kb = (b) => (!b ? "0 KB" : b < 1024 ? `${b} B` : `${(b / 1024).toFixed(b < 10240 ? 1 : 0)} KB`);
const vp = () => [Number(S.pg.viewportWidth) || 1280, Number(S.pg.viewportHeight) || 800];
const qName = (q) => L()[26 + Math.max(0, QK.indexOf(q))];
const qLbl = () => { const e = qName(qApp || net.getEffectiveQuality()); return qMode === "auto" ? L()[75] + e : e; };
const qArgs = () => ({ qualityPreset: net.getEffectiveQuality(), rttMs: net.ewmaRttMs || 0 });
const canCall = () => app.getHostCapabilities()?.serverTools !== false;
const rpcSc = (name, a) => app.callServerTool({ name, arguments: a }).then((r) => r?.structuredContent);
const sendIn = (events, targetId, targetEpoch) => rpcSc("chrome_stream_input", fin(targetEpoch) && targetEpoch > 0 ? { targetId, targetEpoch, events } : { targetId, events });
const evIn = (action, o, imm = true) => pushIn({ action, ...o }, imm);
const tyIn = (text, pressEnter = false, imm = true) => evIn("type", { text, pressEnter }, imm);
const kyIn = (key, modifiers, imm = true) => evIn("key", modifiers ? { key, modifiers } : { key }, imm);
const clIn = (p, button, clickCount = 1) => evIn("click", { x: p.x, y: p.y, button, clickCount });
const fTab = (id) => S.tabs.find((t) => t.targetId === id), tErr = (e) => isTargetErrorText(e?.message), isRec = (d) => d?.recoverable || isTargetErrorText(d?.error);
const modsOf = (e) => (e.altKey ? 1 : 0) | (e.ctrlKey ? 2 : 0) | (e.metaKey ? 4 : 0) | (e.shiftKey ? 8 : 0);
const focusSt = () => (D.si || D.st).focus({ preventScroll: true });
const toast = (m) => { if (!m) return; tx(D.ts, m); tg(D.ts, "on", 1); cTm(tT); tT = sTm(() => tg(D.ts, "on", 0), 1600); };
const setPf = (d) => { if (d?.portFile) portF = d.portFile; if (d?.connectionMode) S.cm = d.connectionMode; const e = Number(d?.epoch); if (fin(e) && e > 0) S.tep = e; };
const ripple = (px, py) => { const d = h(".rp"); d.style.left = `${px}%`; d.style.top = `${py}%`; D.vw.appendChild(d); sTm(() => d.remove(), 160); };
const syncTabList = (d) => { if (isA(d?.tabs)) { S.tabs = d.tabs; S.th = d.tabsHash || ""; setMeta(fTab(S.aid)); uTabs(); } };
const clsP = () => { popOn && setPop(0); moreOn && setMore(0); };
const syncPe = (d, fb) => { const pe = Number(d?.pauseEpoch); if (fin(pe) && pe > S.pe) { S.pe = pe; if (typeof d.paused === "boolean") { S.pa = d.paused; S.st = !d.paused; return 1; } } else if (fb && typeof d?.paused === "boolean") { S.pa = d.paused; S.st = !d.paused; } return 0; };
const clrTm = (w) => { if (w) { cTm(pT); pT = 0; } else { cTm(oT); oT = 0; } };

const bumpFps = (hit) => {
  const t = nowMs(); if (hit) fTimes.push(t);
  while (fTimes.length && t - fTimes[0] > 1500) fTimes.shift();
  fpsVal = fTimes.length ? Math.min(60, rnd((fTimes.length * 1000) / 1500)) : 0;
};
const fitStage = (nw = painted ? D.vc.width : 0, nh = painted ? D.vc.height : 0) => {
  const a = resolveStageAspect(S.pg.viewportWidth, S.pg.viewportHeight, nw, nh);
  if (a && a !== lAr) D.vw.style.setProperty("--ar", lAr = a);
};
const paint = (url) => {
  if (!isSafeImageUrl(url)) return;
  const gen = ++pGen, im = new Image(), dm = vp();
  im.decoding = "async";
  const blit = (src, nw, nh) => {
    if (gen !== pGen) return src.close?.();
    fitStage(nw, nh);
    if (D.vc.width !== nw || D.vc.height !== nh) { D.vc.width = nw; D.vc.height = nh; }
    const cx = D.vc.getContext("2d", { alpha: false });
    if (cx) { cx.imageSmoothingEnabled = true; cx.imageSmoothingQuality = "high"; cx.drawImage(src, 0, 0, nw, nh); }
    src.close?.(); painted = 1; bumpFps(1); hd(D.vc, 0); hd(D.ph, 1); setSw(0); uSt();
  };
  const go = () => {
    if (gen !== pGen) return;
    const nw = im.naturalWidth || dm[0], nh = im.naturalHeight || dm[1];
    typeof createImageBitmap === "function" ? createImageBitmap(im).then((b) => blit(b, b.width || nw, b.height || nh), () => blit(im, nw, nh)) : blit(im, nw, nh);
  };
  im.src = url;
  im.decode ? im.decode().then(go, () => { im.complete && im.naturalWidth && go(); }) : (im.onload = go);
};
const posPop = () => {
  if (!popOn) return;
  const r = D.bq.getBoundingClientRect(), vw = window.innerWidth || 400, vh = window.innerHeight || 600, pw = Math.min(304, Math.max(180, vw - 16));
  D.pp.style.left = `${cl(rnd(vw <= 420 ? r.left : r.right - pw), 8, Math.max(8, vw - pw - 8))}px`;
  D.pp.style.top = `${cl(rnd(r.bottom + 6), 8, Math.max(8, vh - 230))}px`;
};
const setOv = (id, bt, v) => { hd(D[id], !v); sa(D[bt], "aria-expanded", v); };
const setPop = (o) => { popOn = !!o; popOn && setMore(0); setOv("pp", "bq", popOn); popOn && posPop(); };
const setMore = (o) => { moreOn = !!o; moreOn && setPop(0); setOv("mp", "mo", moreOn); };
const setIme = (o) => { imeOn = !!o; setOv("im", "bi", imeOn); tg(D.st, "im", imeOn); tg(D.bi, "on", imeOn); imeOn && D.ti.focus(); };
const syncBnr = () => {
  const t = L(), hasErr = !!(S.on && errMsg), g = S.on && !hasErr && S.dlg && typeof S.dlg === "object" ? S.dlg : null;
  const show = (m) => ["dp", "di", "dk", "do", "er", "dx", "bn"].forEach((k, i) => hd(D[k], !m[i]));
  if (hasErr) { tg(D.bn, "wn", 0); tx(D.dm, errMsg); show([0, 0, 0, 0, 1, 1, 1]); dlgKey = ""; }
  else if (g) {
    const tp = g.type || "alert", key = `${g.targetId || ""}:${tp}:${g.message || ""}`;
    tg(D.bn, "wn", 1); tx(D.dp, t[56 + Math.max(0, DLG.indexOf(tp))]); tx(D.dm, g.message || g.url || "");
    show([1, tp === "prompt", tp !== "alert", 1, 0, 0, 1]);
    if (key !== dlgKey) { dlgKey = key; if (tp === "prompt") { D.di.value = g.defaultPrompt || ""; sTm(() => D.di.focus(), 0); } }
  } else { hd(D.bn, 1); dlgKey = ""; }
  tx(D.om, errMsg || ""); hd(D.oe, !errMsg);
};
const syncDots = () => {
  const sl = !S.on ? "" : errMsg ? "er" : (S.pa || S.id) ? "wn" : S.nt ? "" : "ok", so = errMsg ? "er" : isCn ? "wn" : portF?.valid ? "ok" : "";
  D.ld.className = `ed ${sl}`; D.od.className = D.o0.className = `ed ${so}`;
};
const setErr = (m) => { errMsg = m || null; syncBnr(); syncDots(); };
const setSw = (v) => { isSw = v; tg(D.vc, "sw", v); hd(D.lb, !(S.nv || isSw)); };
const sUrl = (u) => { if (doc.activeElement !== D.ui) D.ui.value = u || ""; };
const setMeta = (m) => { if (m) { S.pg = { ...S.pg, title: m.title || "", url: m.url || "" }; sUrl(m.url); } };
const rstF = (th) => { S.seq = 0; S.fh = ""; S.cr = false; if (th) S.th = ""; };
const syncPh = () => {
  const t = L();
  if (S.nt) { painted = 0; hd(D.vc, 1); D.vc.className = "c"; tx(D.pm, t[37]); hd(D.n2, 0); hd(D.ph, 0); hd(D.pb, 1); return; }
  hd(D.n2, 1);
  if (painted) { const dim = !!(S.pa || S.id); hd(D.ph, 1); hd(D.vc, 0); tg(D.vc, "pa", dim); tx(D.pb, t[S.pa ? 39 : 40]); hd(D.pb, !dim); }
  else { tx(D.pm, t[S.pa ? 51 : 38]); hd(D.ph, 0); hd(D.pb, 1); }
};
const syncPauseUi = () => {
  const lbl = L()[S.pa ? 14 : 13];
  tg(D.bp, "wp", S.pa); sa(D.bp, "aria-pressed", S.pa); sa(D.bp, "aria-label", lbl); D.bp.title = lbl;
  sa(D.pi, "d", IC[S.pa ? 2 : 1]); syncDots(); syncPh(); uSt();
};
const schedOffP = (ms = net.computeOfflinePollDelay({ hidden: hid(), errCount: offErr })) => { clrTm(0); if (!S.on && !isCn && ready) oT = sTm(() => { oT = 0; probeOff(); }, ms); };
const probeOff = async () => {
  if (S.on || isCn || oB || !ready || !canCall()) return;
  oB = 1; const t0 = pNow();
  try {
    const d = await reqFrame(false, 0, false);
    lastFetchAt = nowMs(); offErr = d?.error ? Math.min(4, offErr + 1) : 0;
    if (!d?.screencastPausedIdle) net.recordSample({ totalMs: rnd(pNow() - t0), serverMs: d?.serverMs || 0, imageBytes: 0 });
    if (isCn) return;
    if (d?.portFile) { setPf(d); !S.on && uOff(); }
    if (d?.connected) { act = nowMs(); onPoll(d, 0, S.ep); S.on && S.st && !S.pa && sched(nextMs(0), false); return; }
  } catch { offErr = Math.min(4, offErr + 1); } finally {
    oB = 0;
    if (!S.on && !isCn) { if (offUrg) { offUrg = 0; schedOffP(20); } else schedOffP(); } else offUrg = 0;
  }
};
const goOff = (err, pf) => {
  S.on = S.pa = S.id = S.nt = S.cr = false; S.dlg = null; S.st = true; S.th = S.cm = S.fm = ""; errMsg = err || null;
  fTimes = []; fpsVal = 0; if (pf) portF = pf;
  clrTm(1); sView(); schedOffP();
};
const uOff = () => {
  const t = L(), ok = !!portF?.valid, port = Number(portF?.port) || 9222;
  tg(D.op, "ok", ok); tx(D.op, ok ? t[74] + port : t[0]);
  tx(D.om, errMsg || ""); hd(D.oe, !errMsg); D.ob.disabled = D.ol.disabled = !!isCn;
  tx(D.ob, t[isCn ? 5 : 2]); tx(D.ol, t[isCn ? 5 : 3]);
  syncDots(); if (!ok && !D.dt.dataset.u) D.dt.open = true;
};
const sView = () => {
  hd(D.vo, S.on); hd(D.vn, !S.on);
  if (!S.on) { clsP(); painted = 0; syncBnr(); hd(D.vc, 1); D.vc.className = "c"; hd(D.ph, 0); hd(D.pb, 1); uOff(); }
  else { clrTm(0); syncBnr(); syncPauseUi(); fitStage(); }
};
const renderGuide = () => {
  if (lastGuideLg === curLg) return;
  lastGuideLg = curLg;
  const t = L();
  for (let idx = 0; idx < 3; idx++) {
    const frag = doc.createDocumentFragment();
    for (const part of String(t[7 + idx] || "").split(/(\$[01]|\*[^*]+\*)/g)) {
      if (!part) continue;
      frag.append(part[0] === "$" ? h("code", `t=${GC[+part[1]]}`) : part[0] === "*" ? h("b", `t=${part.slice(1, -1)}`) : doc.createTextNode(part));
    }
    gNodes[idx].replaceChildren(frag);
  }
};
const applyLangUi = () => {
  const t = L(); doc.documentElement.lang = curLg;
  for (const n of lNodes) {
    const ds = n.dataset, vi = t[+ds.i], va = t[+ds.a], vp0 = t[+ds.p];
    vi && tx(n, vi);
    if (va) { sa(n, "aria-label", va); if (n.tagName === "BUTTON") n.title = va; }
    if (vp0) n.placeholder = vp0;
  }
  for (const b of lgBtns) { const onL = b.dataset.lg === curLg; sa(b, "aria-pressed", onL); tg(b, "on", onL); }
  for (const n of tMap.values()) sa(n.lastChild, "aria-label", t[12]);
  renderGuide(); uOff(); syncBnr(); syncPauseUi();
};
const mkEmblem = (dotId, big) => h(big ? "s.e.el" : "s.e", 0, ico(0, big ? 18 : 14), [`s.ed#${dotId}`]);
const mkLang = () => {
  const setL = (next) => { if (curLg !== next) { curLg = next; lsS("chrome_stream_lang", next); applyLangUi(); } setMore(0); };
  const bE = h("b", `t=EN;.lg=en;@p=${curLg === "en"}`), bZ = h("b", `t=中文;.lg=zh-CN;@p=${curLg === "zh-CN"}`);
  ck(bE, () => setL("en")); ck(bZ, () => setL("zh-CN")); lgBtns.push(bE, bZ);
  return h(".l", "role=group;@l=Language", bE, bZ);
};

const mount = () => {
  const gLi = (num, idx) => { const s = h("s"); gNodes[idx] = s; return h("li", 0, ["s.n", `t=${num}`], s); };
  const row = (k, id) => [".pr", 0, ["s", `.i=${k}`], [`s.nm#${id}`, "t=-"]];
  const ib = (id, a, ic, ex = "", sz = 14, pid) => [`!.i#${id}`, `.a=${a}${ex}`, ico(ic, sz, pid)];
  const hdr = (did, mid, ...rt) => ["header.h", 0, [".m", 0, mkEmblem(did, 0), ["s.w", "t=Chrome Stream"]], mid, [".ha", 0, ...rt]];
  const okI = !!portF?.valid, qRail = h(".sg#qs", "role=radiogroup", [".qt#qt"]);
  qBtns = QK.map((v, i) => { const b = h(v === qMode ? "b.on" : "b", `role=radio;.q=${v};.i=${26 + i};@c=${v === qMode}`); ck(b, () => setQ(v)); qRail.append(b); return b; });

  root.replaceChildren(h(".a", 0,
    [".v#vo", 0,
      hdr("o0", [".t"], mkLang()),
      ["section.f", 0,
        [".oh", 0, mkEmblem("od", 1), [okI ? "s.bd.ok#op" : "s.bd#op"]],
        [".r#oe!", 0, ["s.mt#om"], ["!#or", ".i=1"]],
        ["!.bp#ob"], ["s.nt", ".i=4"], ["!.bs#ol"],
        ["details.d#dt", okI ? "" : "open=", ["summary", ".i=6"], ["ol.ol", 0, gLi("01", 0), gLi("02", 1), gLi("03", 2)]]
      ]
    ],
    [".v#vn", 0,
      hdr("ld", [".t#tb", "role=tablist;.a=10", ["!.nb#nt", ".a=11", ico(9, 13)]],
        mkLang(), ib("bp", 13, 1, ";@p=false", 14, "pi"), ib("mo", 72, 8, ";@h=menu;@e=false")
      ),
      [".nr", 0,
        [".pg", 0, ib("bk", 17, 4), ib("fw", 18, 5), ib("rl", 19, 6)],
        ["form.u#uf", 0, ["i.in#ui", ".a=20;.p=20"], ["s.uk", "t=⌘L;@i=true"]],
        ["b.q#bq", ".a=21;@h=dialog;@e=false", ["s.rd#sd"], ["s#ql"]],
        ib("bi", 35, 7, ";@e=false")
      ],
      [".s#st", "tabindex=0;.a=36",
        ["textarea.si#si", "autocomplete=off;spellcheck=false;.a=36"], [".lb#lb!"],
        [".r#bn!", 0, ["s.dtg#dp!"], ["s.mt#dm"], ["i.in#di!", ".p=55"], [".da", 0, ["!#dk!", ".i=54"], ["!.bs#do!", ".i=53"], ["!.bs#er!", ".i=1"], ["!.i#dx!", ".a=73;t=×"]]],
        [".ph#ph", 0, ["s#pm"], ["!.bs#n2!", ".i=11"]], ["b.g#pb!"],
        [".vw#vw", 0, ["canvas.c#vc!", ".a=32"]], [".z#ts", "role=status;@v=polite"],
        [".y#im!", 0, ["i.in#ti", ".p=41"], ["b.ch#ty", ".i=42"], ["b.ch.bs#te", ".i=43"], ["s.dv"],
          ...IK.map((k, i) => ["b.ch", i < 3 ? `.k=${k};t=${k === "Escape" ? "Esc" : k}` : `.k=${k};.i=${41 + i}`]),
          ["b.ch", "t=↑;.sc=-360;.a=49"], ["b.ch", "t=↓;.sc=360;.a=50"]
        ]
      ],
      [".o#pp!", "role=dialog;.a=21", [".pl", ".i=21"], qRail, row(31, "sr"), row(32, "sv"), row(22, "sp"), row(23, "sm"), ["!.bs#sn", ".i=33"]],
      [".o#mp!", "role=menu", mkLang(), [".wn", ".i=16"], ["!.dg#dc", ".i=15", ico(3)]]
    ]
  ));

  const qAll = (n, s) => [...n.querySelectorAll(s)];
  lNodes = qAll(root, "[data-i],[data-a],[data-p]");
  D.qt.style.transform = "translateX(0)";
  applyLangUi();

  const wake = () => { S.id = false; act = nowMs(); };
  const sendTx = (pe) => { const v = D.ti.value; if (!v && !pe) return; D.ti.value = ""; toast(L()[60]); tyIn(v, pe); };
  const openNT = () => { S.nt = S.id = false; S.ep++; rstF(1); setSw(1); evIn("new_tab", { url: "about:blank" }); };
  const doDisc = () => {
    clsP(); S.ep++; qIn = []; S.tabs = []; S.aid = null; rstF(1); goOff(null); toast(L()[61]);
    canCall() && rpcSc("chrome_stream_disconnect", { mode: "revoke" }).then((d) => { if (d?.portFile) { setPf(d); uOff(); } }, () => {});
  };
  const togglePa = () => {
    if (!S.on) return;
    S.pa = !S.pa; S.st = !S.pa; S.ep++; S.pe++; wake(); syncPauseUi();
    if (S.pa) { clrTm(1); toast(L()[51]); canCall() && sendIn([{ action: "pause" }]).then((d) => syncPe(d), () => {}); }
    else { toast(L()[62]); canCall() && sendIn([{ action: "resume" }]).then((d) => syncPe(d), () => {}).finally(() => { S.on && !S.pa && askUrg(true); }); }
  };
  const ansDlg = (accept) => {
    const g = S.dlg; if (!g) return;
    S.dlg = null; syncBnr(); evIn("dialog", { accept, promptText: g.type === "prompt" ? D.di.value : undefined }); askUrg(true); focusSt();
  };

  const cbs = {
    ob: () => conn("auto"), or: () => conn("auto", true), ol: () => conn("launch_remote", true),
    nt: openNT, n2: openNT, er: () => { setErr(null); askUrg(true); }, dx: () => setErr(null),
    sn: () => { S.fh = ""; S.cr = false; wake(); setPop(0); toast(L()[63]); poll(true); },
    dc: doDisc, bp: togglePa, pb: () => { if (!S.on) return; if (S.pa) return togglePa(); wake(); syncPauseUi(); poll(true); },
    dk: () => ansDlg(false), do: () => ansDlg(true), bi: () => setIme(!imeOn), ty: () => sendTx(false), te: () => sendTx(true),
    bq: (e) => { e.stopPropagation(); setPop(!popOn); }, mo: (e) => { e.stopPropagation(); setMore(!moreOn); },
  };
  for (const k in cbs) ck(D[k], cbs[k]);
  for (const nav of [["bk", "back", "← ", 17], ["fw", "forward", "→ ", 18], ["rl", "reload", "↻ ", 19]]) {
    ck(D[nav[0]], () => { S.nv = true; S.id = false; hd(D.lb, 0); toast(nav[2] + L()[nav[3]]); evIn(nav[1]); });
  }

  const onK = (n, ent, esc) => on(n, "keydown", (e) => { if (ent && e.key === "Enter" && !e.isComposing) { pd(e); ent(); } else if (esc && e.key === "Escape") { pd(e); esc(); } });
  on(D.dt, "toggle", () => { D.dt.dataset.u = "1"; });
  onK(D.di, () => ansDlg(true), () => ansDlg(S.dlg?.type === "alert"));
  on(D.ui, "focus", () => D.ui.select());
  onK(D.ui, 0, () => { D.ui.value = S.pg.url || ""; focusSt(); });
  on(D.uf, "submit", (e) => { pd(e); const u = D.ui.value.trim(); if (!u) return; S.nv = true; S.id = false; S.pg = { ...S.pg, url: u }; hd(D.lb, 0); evIn("navigate", { url: u }); focusSt(); });

  const setQ = (v) => {
    qMode = v; net.setMode(v); qApp = net.getEffectiveQuality(); S.fh = ""; S.cr = false; wake();
    for (const b of qBtns) { const onQ = b.dataset.q === v; sa(b, "aria-checked", onQ); tg(b, "on", onQ); }
    D.qt.style.transform = `translateX(${Math.max(0, QK.indexOf(v)) * 100}%)`;
    uSt(); toast(`✓ ${qLbl()}`); askUrg(true);
  };
  on(D.qs, "keydown", (e) => {
    const dir = lrDir(e); if (!dir) return;
    pd(e); const nx = QK[(QK.indexOf(qMode) + dir + QK.length) % QK.length];
    setQ(nx); qBtns[QK.indexOf(nx)].focus();
  });
  onK(D.ti, () => sendTx(true), () => { setIme(0); focusSt(); });
  for (const ch of qAll(D.im, "[data-k]")) ck(ch, async () => {
    const k = ch.dataset.k;
    if (k === "Paste" && navigator.clipboard?.readText) { try { const txt = await navigator.clipboard.readText(); if (txt) { toast(L()[69]); tyIn(txt); return; } } catch {} }
    toast(`✓ ${ch.textContent}`); kyIn(k);
  });
  for (const ch of qAll(D.im, "[data-sc]")) ck(ch, () => { const dm = vp(); evIn("scroll", { x: rnd(dm[0] / 2), y: rnd(dm[1] / 2), deltaY: Number(ch.dataset.sc) || 360 }); });

  const ptOf = (e) => {
    const r = D.vc.getBoundingClientRect(); if (!r.width || !r.height) return null;
    const rx = cl((e.clientX - r.left) / r.width, 0, 1), ry = cl((e.clientY - r.top) / r.height, 0, 1), dm = vp();
    const vw = S.pg.viewportWidth || D.vc.width || dm[0], vh = S.pg.viewportHeight || D.vc.height || dm[1];
    return { x: rnd(rx * vw), y: rnd(ry * vh), px: +(rx * 100).toFixed(2), py: +(ry * 100).toFixed(2), sx: vw / r.width, sy: vh / r.height };
  };
  let mM = 0, mN = "left", mvT = 0, mvP = null, lRD = 0, supM = 0;
  const flMv = () => { if (mvT) { cTm(mvT); mvT = 0; } if (mvP) { const p = mvP; mvP = null; evIn("mousemove", p, false); } };
  const ptrEv = (act, e, p, button, extra) => evIn(act, { x: p.x, y: p.y, button, clickCount: cl(e.detail || 1, 1, 3), modifiers: modsOf(e) || undefined, ...extra });
  const sup = () => nowMs() < supM, hyp = Math.hypot, onV = (ev, fn, o) => on(D.vc, ev, fn, o), bt = (e) => BN[e.button] || "left";

  onV("mousedown", (e) => {
    if (sup() || e.button > 2) return;
    e.button === 1 && pd(e); clsP(); const p = ptOf(e); if (!p) return;
    focusSt(); mM = BM[e.button] || 1; mN = bt(e); if (e.button === 2) lRD = nowMs();
    ripple(p.px, p.py); ptrEv("mousedown", e, p, mN, { buttons: mM });
  });
  const onMv = (e) => {
    if (sup() || (!mM && (S.id || S.pa || !S.on))) return;
    const p = ptOf(e); if (!p) return;
    mvP = { x: p.x, y: p.y, button: mM ? mN : "none", buttons: mM, modifiers: modsOf(e) || undefined };
    if (!mvT) mvT = sTm(flMv, mM ? 32 : net.ewmaRttMs > 350 ? 150 : 75);
  };
  onV("mousemove", onMv);
  on(window, "mousemove", (e) => { mM && e.target !== D.vc && onMv(e); });
  on(window, "mouseup", (e) => { if (!mM || sup()) return; flMv(); mM = 0; ptrEv("mouseup", e, ptOf(e) || { x: 0, y: 0 }, bt(e), { buttons: 0 }); });
  on(window, "resize", () => { popOn && posPop(); });
  onV("auxclick", (e) => { e.button === 1 && pd(e); });
  onV("contextmenu", (e) => { pd(e); if (sup() || nowMs() - lRD < 450) return; const p = ptOf(e); if (p) { ripple(p.px, p.py); ptrEv("click", e, p, "right", { clickCount: 1 }); } });

  let tch = null, lTp = 0, lTx = 0, lTy = 0;
  onV("touchstart", (e) => {
    if (e.touches.length !== 1) { tch?.lp && cTm(tch.lp); tch = null; return; }
    clsP(); const t0 = e.touches[0], p = ptOf(t0); if (!p) return;
    const cx = t0.clientX, cy = t0.clientY, c = { x0: cx, y0: cy, lx: cx, ly: cy, pt: p, mv: 0, lg: 0, lp: 0 };
    c.lp = sTm(() => { if (!c.mv && tch === c) { c.lg = 1; ripple(p.px, p.py); toast(L()[70]); clIn(p, "right"); } }, 500);
    tch = c;
  }, { passive: true });
  onV("touchmove", (e) => {
    if (!tch || e.touches.length !== 1) return;
    const t0 = e.touches[0], cx = t0.clientX, cy = t0.clientY;
    if (!tch.mv && hyp(cx - tch.x0, cy - tch.y0) > 8) { tch.mv = 1; cTm(tch.lp); }
    if (tch.mv && !tch.lg) { pd(e); qScr((tch.lx - cx) * tch.pt.sx, (tch.ly - cy) * tch.pt.sy, tch.pt); tch.lx = cx; tch.ly = cy; }
  }, { passive: false });
  const endT = (can) => {
    if (!tch) return;
    const c = tch; tch = null; cTm(c.lp); supM = nowMs() + 500;
    if (can || c.mv || c.lg) return;
    const ts = nowMs(), dbl = ts - lTp < 320 && hyp(c.x0 - lTx, c.y0 - lTy) < 24;
    lTp = dbl ? 0 : ts; lTx = c.x0; lTy = c.y0;
    focusSt(); ripple(c.pt.px, c.pt.py); clIn(c.pt, "left", dbl ? 2 : 1);
  };
  onV("touchend", () => endT(false)); onV("touchcancel", () => endT(true));

  let wx = 0, wy = 0, wp = null, wt = 0;
  const qScr = (dx, dy, p) => {
    wx += dx; wy += dy; wp = p || wp;
    if (!wt) wt = sTm(() => {
      const sx = cl(rnd(wx), -1400, 1400), sy = cl(rnd(wy), -1400, 1400), pt = wp || { x: 500, y: 350 };
      wx = wy = wt = 0;
      (Math.abs(sx) >= 2 || Math.abs(sy) >= 2) && evIn("scroll", { x: pt.x, y: pt.y, deltaX: sx, deltaY: sy });
    }, net.getWheelCoalesceMs());
  };
  onV("wheel", (e) => { pd(e); const sc = e.deltaMode === 1 ? 40 : e.deltaMode === 2 ? vp()[1] * 0.85 : 1; qScr(e.deltaX * sc, e.deltaY * sc, ptOf(e)); }, { passive: false });

  let comp = 0, pArm = 0;
  on(D.st, "focus", () => { doc.activeElement === D.st && focusSt(); });
  on(D.si, "compositionstart", () => { comp = 1; });
  on(D.si, "compositionend", (e) => { comp = 0; const v = e.data || D.si.value; D.si.value = ""; v && tyIn(v); });
  on(D.si, "beforeinput", (e) => {
    if (comp || e.isComposing) return;
    if (e.inputType === "deleteContentBackward") { pd(e); kyIn("Backspace", 0, false); }
    else if (e.inputType === "insertLineBreak") { pd(e); kyIn("Enter"); }
  });
  on(D.si, "input", () => { if (comp) return; const v = D.si.value; if (v) { D.si.value = ""; tyIn(v, false, false); } });
  on(D.st, "keydown", (e) => {
    if ((e.target !== D.st && e.target !== D.si) || comp || e.isComposing || e.key === "Process" || e.keyCode === 229) return;
    const mods = modsOf(e), low = e.key.toLowerCase();
    if ((e.metaKey || e.ctrlKey) && !e.altKey) {
      if (low === "l") return;
      if (low === "v") { cTm(pArm); pArm = sTm(() => { pArm = 0; kyIn("Paste"); }, 45); return; }
      const cmd = { a: ["SelectAll"], c: ["Copy", L()[68]], x: ["Cut"], z: [e.shiftKey ? "Redo" : "Undo"], y: ["Redo"] }[low];
      if (cmd) { pd(e); cmd[1] && toast(cmd[1]); kyIn(cmd[0]); }
      return;
    }
    const k = e.key === " " ? "Space" : e.key;
    if (k === "Space" || KEYS.has(k)) { pd(e); kyIn(k, mods, k === "Enter" || k === "Escape"); return; }
    if (e.key.length === 1 && !e.metaKey && (!e.ctrlKey || e.altKey)) {
      pd(e); const c = e.key.charCodeAt(0);
      !e.altKey && c >= 33 && c <= 126 ? kyIn(e.key, mods, false) : tyIn(e.key, false, false);
    }
  });
  on(doc, "paste", (e) => {
    if (!S.on || [D.ui, D.ti, D.di].includes(e.target)) return;
    if (pArm) { cTm(pArm); pArm = 0; }
    pd(e); const txt = e.clipboardData?.getData("text/plain") || "";
    D.si.value = "";
    if (txt) { toast(L()[69]); tyIn(txt); } else kyIn("Paste");
  });
};

const swTab = (tid) => {
  if (!tid || tid === S.aid) return;
  S.aid = tid; S.id = false; S.dlg = null; S.ep++; rstF(); setMeta(fTab(tid));
  syncBnr(); setSw(1); uTabs(); uSt(); evIn("switch_tab", { targetId: tid });
};
const clTab = (tid) => {
  if (!tid || closing.has(tid)) return;
  closing.add(tid);
  const o = applyOptimisticTabClose(S.tabs, S.aid, tid);
  S.tabs = o.tabs; S.th = ""; S.id = false;
  if (o.wasActive) { S.aid = o.activeTargetId; S.dlg = null; S.ep++; rstF(); syncBnr(); setSw(1); setMeta(fTab(S.aid)); }
  uTabs(); uSt(); toast(L()[64]); evIn("close_tab", { targetId: tid });
};
const lrDir = (e) => e.key === "ArrowRight" ? 1 : e.key === "ArrowLeft" ? -1 : 0;
const mkTab = (tid) => {
  const x = h("b.x", `t=×;@l=${L()[12]}`), node = h(".p", `tabindex=0;role=tab;.id=${tid}`, ["s.tt"], x);
  ck(node, (e) => { e.target !== x && swTab(tid); });
  on(node, "auxclick", (e) => { if (e.button === 1) { pd(e); clTab(tid); } });
  on(node, "keydown", (e) => {
    if (e.key === "Enter" || e.key === " ") { pd(e); swTab(tid); }
    else {
      const dir = lrDir(e); if (!dir || S.tabs.length < 2) return;
      pd(e); const i = S.tabs.findIndex((t) => t.targetId === tid); if (i < 0) return;
      const nx = S.tabs[(i + dir + S.tabs.length) % S.tabs.length].targetId;
      swTab(nx); tMap.get(nx)?.focus();
    }
  });
  ck(x, (e) => { e.stopPropagation(); clTab(tid); });
  return node;
};
const uTabs = () => {
  const sig = S.aid + S.tabs.map((t) => `|${t.targetId}:${t.title}:${t.url}`).join("");
  if (sig === tabSig) return;
  tabSig = sig;
  const live = new Set(S.tabs.map((t) => t.targetId));
  tMap.forEach((n, id) => { if (!live.has(id)) { n.remove(); tMap.delete(id); } });
  let cur = D.tb.firstChild;
  for (const t of S.tabs) {
    let node = tMap.get(t.targetId);
    if (!node) tMap.set(t.targetId, node = mkTab(t.targetId));
    const a = t.targetId === S.aid;
    tg(node, "on", a); sa(node, "aria-selected", a);
    if (node.title !== (t.url || "")) node.title = t.url || "";
    tx(node.firstChild, t.title || t.url || "Tab");
    if (cur !== node) D.tb.insertBefore(node, cur || D.nt);
    cur = node.nextSibling;
  }
};
const uSt = () => {
  bumpFps(0);
  const t = L(), rtt = net.lastNetRttMs || net.ewmaRttMs, pend = (inB ? 1 : 0) + qIn.length, dm = vp();
  const tag = S.cr ? " · HD WebP" : S.fm ? ` · ${S.fm.toUpperCase()}` : "";
  D.sd.className = `rd ${S.pa || S.id ? "wn" : S.nt ? "id" : rttBand(rtt)}`;
  tx(D.ql, S.pa ? t[51] : S.id ? t[52] : S.nt ? t[37] : `${rtt ? `${rtt}ms` : "--ms"} · ${fpsVal}fps · ${qName(qMode)}`);
  tx(D.sr, `${S.uc ? t[34] : kb(S.by)} · #${S.seq}${pend ? t[76] + pend : ""}`);
  tx(D.sv, `${dm[0]}×${dm[1]}`); tx(D.sp, qLbl() + tag); tx(D.sm, t[S.cm === "remote" ? 25 : 24]);
  hd(D.lb, !(S.nv || isSw));
};
const recover = (m) => { S.aid = null; S.dlg = null; rstF(1); m && toast(m); askUrg(true); };
const onPoll = (d, totalMs, reqEpoch = S.ep) => {
  if (!d) return;
  if (totalMs > 0) synced = 1;
  setPf(d);
  if (d.connected === false) return isRec(d) ? recover(L()[65]) : goOff(d.error, d.portFile);
  const wasPa = S.pa, paintIt = applyFramePayloadState(S, d, reqEpoch, totalMs);
  if (paintIt === null) return S.on && S.st && !S.pa && askUrg(false);
  if (totalMs > 0 && !d.screencastPausedIdle) {
    const s = net.recordSample({ totalMs, serverMs: d.serverMs || 0, imageBytes: d.changed ? d.imageBytes || 0 : 0 });
    qApp = d.qualityApplied || (s.tierChanged ? s.effectiveTier : qApp);
  } else if (d.qualityApplied) qApp = d.qualityApplied;
  sView();
  if (wasPa !== S.pa) { syncPauseUi(); S.pa && clrTm(1); }
  if (S.nt) sUrl(""); else if (d.page) sUrl(d.page.url);
  if (d.popupSwitched || d.recovered) { setErr(null); toast(L()[d.popupSwitched ? 67 : 66]); } else setErr(d.error || null);
  uTabs(); syncBnr(); syncDots();
  if (paintIt) paint(d.imageDataUrl); else { fitStage(); isSw && painted && S.fh && setSw(0); syncPh(); }
  uSt();
};
const pushIn = (ev, imm = true) => {
  const wasIdle = S.id; act = nowMs(); S.cr = false;
  if (ev.action !== "pause" && ev.action !== "disconnect") S.id = false;
  qIn.push(ev);
  wasIdle && !S.id ? syncPauseUi() : uSt();
  if (inB) return;
  const ms = imm ? 0 : net.getTypeBatchMs();
  if (ms <= 0) { cTm(inT); inT = 0; flIn(); } else if (!inT) inT = sTm(() => { inT = 0; flIn(); }, ms);
};
const flIn = async () => {
  if (inB || !qIn.length || !canCall()) return;
  inB = 1;
  const batch = coalesceInputEvents(qIn); qIn = [];
  const closed = batch.filter((e) => e.action === "close_tab").map((e) => e.targetId), hadDisc = batch.some((e) => e.action === "disconnect");
  const hasPageEv = batch.some((e) => PAGE_ACT.has(e.action)) && !batch.some((e) => /_tab$/.test(e.action)), ep = S.ep;
  uSt();
  try {
    const d = await sendIn(batch, S.aid || undefined, hasPageEv ? S.tep : undefined);
    synced = 1;
    if (hadDisc || (d?.connected === false && !isRec(d))) return goOff(d?.error, d?.portFile);
    setPf(d);
    if (d?.staleTarget) {
      setErr(null); toast(L()[71]);
      if (d.activeTargetId !== undefined) S.aid = d.activeTargetId || null;
      syncTabList(d); if ("dialog" in d) { S.dlg = d.dialog || null; syncBnr(); }
      rstF(); !S.pa && askUrg(true); return;
    }
    if (d?.error && !d?.recovered && !isTargetErrorText(d.error)) setErr(d.error);
    else if (d?.recovered) { setErr(null); toast(L()[66]); }
    syncPe(d) && syncPauseUi();
    if (ep === S.ep) {
      if ("dialog" in (d || {})) { S.dlg = d.dialog || null; syncBnr(); }
      if (S.nt = isNoTabsPayload(d)) { S.aid = null; S.tabs = []; S.th = d?.tabsHash || ""; S.pg = {}; S.dlg = null; sUrl(""); rstF(); syncBnr(); uTabs(); syncPh(); }
      else { if (d?.activeTargetId && d.activeTargetId !== S.aid) { S.aid = d.activeTargetId; rstF(); } syncTabList(d); }
    }
    !S.nt && !S.pa && askUrg(false);
  } catch (err) {
    tErr(err) ? recover() : setErr(err?.message || String(err));
  } finally {
    for (const id of closed) closing.delete(id);
    inB = 0; uSt(); qIn.length && flIn();
  }
};
const sched = (ms, f = false) => { clrTm(1); if (S.on && (!S.pa || f)) pT = sTm(() => { pT = 0; poll(f); }, ms); };
const askUrg = (f = false) => { if (!S.on || (S.pa && !f)) return; if (pB) urg = { f: !!(f || urg?.f) }; else sched(20, f); };
const timing = (idleMs, inputBusy = false) => net.computePollTiming({ idleMs, inputBusy, screencastIdle: S.id, hidden: hid(), errCount: pErr, settled: S.cr });
const nextMs = (idleMs, busy = false) => !hid() && S.nt ? 1200 : timing(idleMs, busy).nextDelayMs;
const reqFrame = (f, w, keep = true) => rpcSc("chrome_stream_frame", { targetId: S.aid || undefined, sinceSeq: !f && keep ? S.seq : 0, lastFrameHash: S.fh || undefined, knownTabsHash: S.th || undefined, ...qArgs(), waitMs: f ? 0 : w, forceRefresh: f });
const poll = async (f = false) => {
  if (pB || !canCall() || (!f && S.pa)) { if (f && pB) urg = { f: true }; return; }
  pB = 1; const ep = S.ep, t0 = pNow();
  try {
    const d = await reqFrame(f, hid() ? 0 : S.nt ? 1000 : timing(nowMs() - act).waitMs);
    lastFetchAt = nowMs(); pErr = d?.error && !d?.recovered ? Math.min(4, pErr + 1) : 0;
    onPoll(d, rnd(pNow() - t0), ep);
  } catch (err) {
    pErr = Math.min(4, pErr + 1);
    if (tErr(err)) { S.aid = null; rstF(1); urg = { f: true }; } else setErr(err?.message || String(err));
  } finally {
    pB = 0;
    if (S.on && !S.pa) { if (urg) { const u = urg; urg = 0; sched(20, u.f); } else if (S.st) sched(nextMs(nowMs() - act, !!(inB || qIn.length)), false); }
  }
};
const conn = async (mode = "auto", f = false) => {
  if (isCn || !canCall()) return;
  isCn = true; errMsg = null; clrTm(0); !S.on && uOff();
  const ep = S.ep, t0 = pNow(), tool = mode === "launch_remote" ? "chrome_stream_launch_remote" : "chrome_stream_connect", args = qArgs();
  if (mode !== "launch_remote" && mode !== "auto") args.mode = mode;
  try {
    const c = await rpcSc(tool, args);
    if (!c?.connected) { isCn = false; return goOff(c?.error || null, c?.portFile); }
    act = nowMs(); S.on = true; S.pa = S.id = false; S.st = true; S.pe = Number(c.pauseEpoch) || 0;
    setPf(c); if (c.activeTargetId) S.aid = c.activeTargetId; syncTabList(c);
    S.dlg = c.dialog || null; S.nt = isNoTabsPayload(c);
    const d = await reqFrame(f || true, 0, false);
    isCn = false; lastFetchAt = nowMs(); pErr = offErr = 0; onPoll(d, rnd(pNow() - t0), ep);
    S.on && S.st && !S.pa && sched(nextMs(0), false);
  } catch (err) {
    isCn = false;
    S.on && tErr(err) ? recover() : goOff(err?.message || String(err));
  }
};
const forceNow = () => {
  if (!ready) return;
  const t = nowMs(); if (t - lastForceAt < 250) return;
  lastForceAt = t; pErr = offErr = 0;
  if (S.on) { if (S.pa) return; act = t; if (S.id) { S.id = false; syncPauseUi(); } clrTm(1); poll(true); }
  else if (!isCn) { clrTm(0); oB ? (offUrg = 1) : probeOff(); }
};
const onHide = () => {
  if (!ready) return;
  if (S.on && !S.pa && !pB && pT) sched(nextMs(nowMs() - act, !!(inB || qIn.length)), false);
  else if (!S.on && !isCn && !oB && oT) schedOffP();
};

ck(doc, (e) => { popOn && !e.target.closest?.("#pp,#bq") && setPop(0); moreOn && !e.target.closest?.("#mp,#mo") && setMore(0); });
on(doc, "keydown", (e) => {
  if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "l" && S.on) { pd(e); D.ui.focus(); D.ui.select(); }
  else if (e.key === "Escape" && popOn) { pd(e); setPop(0); D.bq.focus(); }
  else if (e.key === "Escape" && moreOn) { pd(e); setMore(0); D.mo.focus(); }
});
const syncHostCtx = (ctx) => {
  applyHostContext(ctx);
  const nextLg = resolvePanelLang({ saved: lsG("chrome_stream_lang"), hostLocale: ctx?.locale || ctx?.language || "", navLang: globalThis.navigator?.language || "", bootLocale: boot?.locale || "" });
  if (nextLg !== curLg) { curLg = nextLg; applyLangUi(); }
};
const onResult = (r) => {
  const d = r?.structuredContent; if (!d) return;
  const sig = [!!d.connected, !!d.portFile?.valid, +d.portFile?.port || 0, d.activeTargetId, d.tabsHash, d.error].join("|");
  if (sig === tSig) { if (d.connected && !started && ready) { started = 1; lastForceAt = nowMs(); !S.pa && poll(false); } return; }
  tSig = sig; setPf(d); syncPe(d, !synced);
  if (d.activeTargetId && !synced) S.aid = d.activeTargetId;
  if ("dialog" in d) S.dlg = d.dialog || null;
  if (d.connected) { S.on = true; act = nowMs(); lastFetchAt = nowMs(); sView(); uTabs(); uSt(); ready && poll(false); return; }
  if (synced && S.on && !d.connected) return goOff(d.error, d.portFile);
  !synced && !isCn && goOff(d.error, d.portFile);
};

app.ontoolresult = onResult;
app.onhostcontextchanged = syncHostCtx;
app.onteardown = () => { clrTm(1); clrTm(0); stopReveal?.(); stopReveal = null; return {}; };

mount(); sView();
if (boot?.connected) {
  if (S.nt) sUrl(""); else if (boot.page) sUrl(boot.page.url);
  uTabs(); uSt();
  !S.nt && boot.changed && isSafeImageUrl(boot.imageDataUrl) && paint(boot.imageDataUrl);
}
app.connect().then(() => {
  ready = 1; syncHostCtx(app.getHostContext());
  stopReveal = setupRevealObservers({ onReveal: forceNow, onHide, getLastFetchAt: () => lastFetchAt, bridgeApp: app });
  if (!started) { started = 1; lastForceAt = nowMs(); if (S.on && !S.pa) poll(false); else if (!S.on) schedOffP(); }
}).catch((err) => { setErr(L()[77] + (err?.message || String(err))); });
