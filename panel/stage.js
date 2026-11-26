import { App, applyHostContext } from "./bridge.js";
import {
  AdaptiveBandwidthController, applyFramePayloadState, applyOptimisticTabClose, coalesceInputEvents,
  isNoTabsPayload, isSafeImageUrl, isTargetErrorText, REP, resolvePanelLang, resolveStageAspect, rttBand, setupRevealObservers,
} from "./net-policy.js";

const doc = document, b0 = globalThis.__CHROME_STREAM_BOOT__ || {}, fin = Number.isFinite, rnd = Math.round, isA = Array.isArray;
const on = (n, ev, fn, o) => n?.addEventListener(ev, fn, o), ck = (n, fn) => on(n, "click", fn), sa = (n, k, v) => n?.setAttribute(k, String(v));
const pd = (e) => e.preventDefault(), tx = (n, v) => { if (n.textContent !== v) n.textContent = v; }, hd = (n, v) => (n.hidden = !!v);
const tg = (n, c, v) => n.classList.toggle(c, !!v), pNow = () => performance.now(), nowMs = () => Date.now(), cl = (v, lo, hi) => Math.max(lo, Math.min(hi, v));
const sTm = (f, m) => globalThis.setTimeout(f, m), cTm = (i) => globalThis.clearTimeout(i), hid = () => doc.visibilityState === "hidden";
const root = doc.getElementById("app"), app = new App({ name: "RemoteCast", version: "0.4.0" }, {}), net = new AdaptiveBandwidthController("auto");
const KEYS = new Set([...REP, "Enter", "Escape", "Home", "End"]);
const QK = "auto low medium high ultra".split(" "), IK = "Enter Escape Tab Backspace Undo Copy Paste SelectAll".split(" ");
const DLG = "alert confirm prompt beforeunload".split(" "), PAGE_ACT = new Set("click mousedown mouseup mousemove scroll type key dialog navigate back forward reload close_tab".split(" "));
const BN = ["left", "middle", "right"], BM = [1, 4, 2];
const lsG = (k) => { try { return localStorage.getItem(k) || ""; } catch { return ""; } };
const lsS = (k, v) => { try { localStorage.setItem(k, v); } catch {} };

const D = {}, TAGS = { b: "button", s: "span", i: "input" }, ARIA = { e: "expanded", p: "pressed", c: "checked", h: "haspopup", l: "label", i: "hidden", v: "live" };
const h = (sel, at, ...kids) => {
  const hid0 = sel.endsWith("!"), sp = (hid0 ? sel.slice(0, -1) : sel).split("#"), tp = sp[0].split(".");
  let raw = tp.shift(); raw === "!" && (tp.unshift("b"), raw = "b");
  const tag = TAGS[raw] || raw || "div", n = doc.createElement(tag);
  if (tag === "button") n.type = "button";
  if (tp.length) n.className = tp.join(" ");
  if (sp[1]) D[n.id = sp[1]] = n;
  if (hid0) n.hidden = true;
  if (at) for (const kv of at.split(";")) {
    const j = kv.indexOf("=");
    if (j > 0) {
      const k = kv.slice(0, j), v = kv.slice(j + 1);
      k === "t" ? (n.textContent = v) : k[0] === "." ? (n.dataset[k.slice(1)] = v) : sa(n, k[0] === "@" ? `aria-${ARIA[k.slice(1)]}` : k, v);
    }
  }
  for (const c of kids) c != null && c !== false && n.append(isA(c) ? h(...c) : typeof c === "string" ? doc.createTextNode(c) : c);
  return n;
};
const IC = [
  "M2 3.5h12v9H2z;M2 6h12;M7 8l3 1.6L7 11.2z", "M5.5 3.5v9M10.5 3.5v9", "M5 3.5l7 4.5-7 4.5z",
  "M8 2.2v5.3;M4.6 4.4a5.2 5.2 0 1 0 6.8 0", "M10 3.5L5.5 8 10 12.5", "M6 3.5L10.5 8 6 12.5",
  "M13 8a5 5 0 1 1-1.3-3.4;M13 2.5v2.6h-2.6", "M2 4.5h12v7H2z;M4 7.5h1M7 7.5h2M11 7.5h1M5 9.8h6",
  "M3.5 8h.01M8 8h.01M12.5 8h.01", "M8 3.5v9M3.5 8h9",
];
const ico = (idx, sz = 14, pid) => {
  const ns = "http://www.w3.org/2000/svg", s = doc.createElementNS(ns, "svg"), a = { class: "ic", width: sz, height: sz, viewBox: "0 0 16 16", "aria-hidden": "true" };
  for (const k in a) sa(s, k, a[k]);
  for (const dp of IC[idx].split(";")) { const p = doc.createElementNS(ns, "path"); sa(p, "d", dp); pid && (D[pid] = p); s.appendChild(p); }
  return s;
};

const GC = ["chrome://inspect/#remote-debugging", "127.0.0.1:9222", "--user-data-dir=<dir>"];
const I18N = {
  en: (
    "Not detected|Retry|Connect Personal Chrome|Launch Remote Chrome (No Allow)|Standby keeps Allow|Connecting…|Setup|Open Chrome.|Enable $0 or run isolated Chrome on $1 ($2).|Click *Connect Personal Chrome* & Allow, or Remote.|" +
    "Tabs|New tab|Close|Standby|Wake|Revoke & Disconnect|Revokes Allow (re-Allow locally)|Back|Forward|Reload|URL or search|Quality|Profile|Mode|Personal|Remote Profile|" +
    "Auto|Low|Med|High|Ultra|Frame|Viewport|Refresh|Cached 0 KB|Keyboard|Live view|No tabs|Loading…|Standby · Wake|Idle · Wake|Type or paste…|Insert|Send ↵|" +
    "Bksp|Undo|Copy|Paste|Select all|Scroll up|Scroll down|Standby|Idle|OK|Cancel|Reply…|Alert|Confirm|Prompt|Leave?|" +
    "✓ Sent|Disconnected|✓ Awake|↻ Refresh|✓ Closed|Switching…|✓ Switched|↗ New tab|✓ Copied|✓ Pasted|Right click|Tab changed|Menu|Dismiss|Ready · |Auto · | · q |Init: |Disconnecting…|Revoke failed, retry|Standby failed, retry|Remote|No tool access"
  ).split("|"),
  "zh-CN": (
    "未检测|重试|连接日常 Chrome|启动远程 Chrome（免 Allow）|休眠免重弹 Allow|连接中…|设置|打开 Chrome|开 $0 调试，或用 $2 在 $1 启独立配置|点 *连接日常 Chrome* 允许，或启远程模式|" +
    "标签|新建|关闭|休眠|唤醒|彻底断开 · 撤销授权|将撤销授权，远程无法点 Allow|后退|前进|刷新|网址或搜索|画质|档位|模式|日常配置|独立远程|" +
    "自动|弱网|均衡|高清|极速|单帧|视口|刷新|未变 0 KB|键盘|画面|无标签|加载中…|休眠中 · 唤醒|空闲 · 唤醒|输入或粘贴…|输入|发送 ↵|" +
    "退格|撤销|复制|粘贴|全选|上滚|下滚|休眠中|空闲省电|确定|取消|输入…|提示|确认|输入|离开？|" +
    "✓ 已发|已断开|✓ 唤醒|↻ 刷新|✓ 已关|切换中|✓ 已切|↗ 新标|✓ 复制|✓ 粘贴|右键|标签已切|更多|关闭|就绪 · |自动 · | · 待发 |初始化：|断开中…|断开失败，请重试|休眠失败，请重试|远程|无工具权限"
  ).split("|"),
};
const S = {
  on: !!b0.connected, cm: b0.connectionMode || "", cr: !!b0.crisp, fm: b0.format || "",
  tabs: isA(b0.tabs) ? b0.tabs : [], th: b0.tabsHash || "",
  aid: b0.activeTargetId || null, ep: Number(b0.epoch) || 1, tep: Number(b0.epoch) || 1, pe: Number(b0.pauseEpoch) || 0, pg: b0.page || {},
  seq: b0.seq || 0, fh: b0.frameHash || "", by: b0.imageBytes || 0, uc: false,
  nv: false, st: !b0.paused, pa: !!b0.paused, id: !!b0.screencastPausedIdle,
  dlg: b0.dialog || null, nt: !!(b0.connected && isNoTabsPayload(b0)),
};
let curLg = resolvePanelLang({ saved: lsG("chrome_stream_lang"), navLang: globalThis.navigator?.language || "", bootLocale: b0.locale || "" });
let T = I18N[curLg] || I18N.en, portF = b0.portFile || null, tabSig = "", qApp = b0.qualityApplied || "low", errMsg = null, cErr = 0;
let isCn = 0, isDisc = 0, isPa = 0, isSw = 0, qMode = "auto", lastMode = "personal", dlgKey = "", imeOn = 0, popOn = 0, moreOn = 0;
let qIn = [], inB = 0, inT = 0, pB = 0, pT = 0, oT = 0, oB = 0, urg = 0, act = nowMs(), tT = 0, dropTimers = () => {};
let pErr = 0, offErr = 0, offUrg = 0, lastFetchAt = b0.connected ? nowMs() : 0, lastForceAt = 0, stopReveal = null;
let pGen = 0, painted = 0, started = 0, synced = 0, ready = 0, tSig = "", lAr = 0, fTimes = [], fpsVal = 0, lastGuideLg = "";
const closing = new Set(), tMap = new Map();
let lNodes = [], gNodes = [], qBtns = [], lgBtns = [];

const kb = (b) => (!b ? "0 KB" : b < 1024 ? `${b} B` : `${(b / 1024).toFixed(b < 10240 ? 1 : 0)} KB`);
const vp = () => [Number(S.pg.viewportWidth) || 1280, Number(S.pg.viewportHeight) || 800];
const qName = (q) => T[26 + Math.max(0, QK.indexOf(q))];
const qLbl = () => { const e = qName(qApp || net.getEffectiveQuality()); return qMode === "auto" ? T[75] + e : e; };
const qArgs = () => ({ qualityPreset: net.getEffectiveQuality(), rttMs: net.ewmaRttMs || 0 });
const canCall = () => !!ready && !!app.getHostCapabilities()?.serverTools;
const liveOk = () => S.on && S.st && !S.pa && !isDisc;
const eMsg = (e, fb) => (typeof e === "string" ? e : e?.message || fb || String(e));
const rpcSc = (name, a) => app.callServerTool({ name: "chrome_stream_" + name, arguments: a }).then((r) => r?.structuredContent);
const sendIn = (events, targetId, targetEpoch) => rpcSc("input", fin(targetEpoch) && targetEpoch > 0 ? { targetId, targetEpoch, events } : { targetId, events });
const dropIn = () => (qIn = [], cTm(inT), inT = 0, dropTimers());
const evIn = (action, o, imm = 1, tid = S.aid, tep = S.tep) => pushIn({ action, ...o }, imm, tid, tep);
const tyIn = (text, pressEnter = false, imm = 1) => evIn("type", { text, pressEnter }, imm);
const kyIn = (key, modifiers, imm = 1) => evIn("key", modifiers ? { key, modifiers } : { key }, imm);
const clIn = (p, button, clickCount = 1, tid = S.aid, tep = S.tep) => evIn("click", { x: p.x, y: p.y, button, clickCount }, 1, tid, tep);
const fTab = (id) => S.tabs.find((t) => t.targetId === id), tErr = (e) => isTargetErrorText(e?.message), isRec = (d) => d?.recoverable || isTargetErrorText(d?.error);
const modsOf = (e) => (e.altKey ? 1 : 0) | (e.ctrlKey ? 2 : 0) | (e.metaKey ? 4 : 0) | (e.shiftKey ? 8 : 0);
const focusSt = () => (D.si || D.st).focus({ preventScroll: true });
const toast = (m) => m && (tx(D.ts, m), tg(D.ts, "on", 1), cTm(tT), tT = sTm(() => tg(D.ts, "on", 0), 1600));
const setPf = (d) => { if (!d) return; d.portFile && (portF = d.portFile); d.connectionMode && (S.cm = d.connectionMode); const e = Number(d.epoch); fin(e) && e > 0 && (S.tep = e); };
const ripple = (px, py) => { const d = h(".rp"); d.style.left = `${px}%`; d.style.top = `${py}%`; D.vw.appendChild(d); sTm(() => d.remove(), 160); };
const syncTabList = (d) => isA(d?.tabs) && (S.tabs = d.tabs, S.th = d.tabsHash || "", setMeta(fTab(S.aid)), uTabs());
const clsP = () => (popOn && setPop(0), moreOn && setMore(0));
const syncPe = (d, fb) => {
  const pe = Number(d?.pauseEpoch), has = typeof d?.paused === "boolean", g = fin(pe) && pe > S.pe;
  g && (S.pe = pe);
  return has && (g || fb) ? (S.st = !(S.pa = d.paused), g ? 1 : 0) : 0;
};
const clrTm = (w) => (w ? (cTm(pT), pT = 0) : (cTm(oT), oT = 0));
const hideCvs = () => (painted = 0, hd(D.vc, 1), D.vc.className = "c", hd(D.ph, 0), hd(D.pb, 1));
const clrTabs = (th = "") => (S.aid = S.dlg = null, S.tabs = [], S.th = th, S.pg = {}, sUrl(""), rstF(), syncBnr(), uTabs(), syncPh());

const bumpFps = (hit) => {
  const t = nowMs(); hit && fTimes.push(t);
  while (fTimes.length && t - fTimes[0] > 1500) fTimes.shift();
  fpsVal = fTimes.length ? Math.min(60, rnd(fTimes.length / 1.5)) : 0;
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
    const v = D.vc; fitStage(nw, nh);
    if (v.width !== nw || v.height !== nh) { v.width = nw; v.height = nh; }
    const cx = v.getContext("2d", { alpha: false });
    cx && (cx.imageSmoothingEnabled = true, cx.imageSmoothingQuality = "high", cx.drawImage(src, 0, 0, nw, nh));
    src.close?.(); painted = 1; bumpFps(1); hd(v, 0); hd(D.ph, 1); setSw(0); uSt();
  };
  const go = () => {
    if (gen !== pGen) return;
    const nw = im.naturalWidth || dm[0], nh = im.naturalHeight || dm[1];
    typeof createImageBitmap === "function" ? createImageBitmap(im).then((b) => blit(b, b.width || nw, b.height || nh), () => blit(im, nw, nh)) : blit(im, nw, nh);
  };
  im.src = url;
  im.decode ? im.decode().then(go, () => im.complete && im.naturalWidth && go()) : (im.onload = go);
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
  const hasErr = !!(S.on && errMsg), g = S.on && !hasErr && S.dlg && typeof S.dlg === "object" ? S.dlg : null;
  const show = (m) => "dp di dk do er dx bn".split(" ").forEach((k, i) => hd(D[k], !(m & (1 << i))));
  if (hasErr) { tg(D.bn, "wn", 0); tx(D.dm, errMsg); show(112); dlgKey = ""; }
  else if (g) {
    const tp = g.type || "alert", key = `${g.targetId || ""}:${tp}:${g.message || ""}`;
    tg(D.bn, "wn", 1); tx(D.dp, T[56 + Math.max(0, DLG.indexOf(tp))]); tx(D.dm, g.message || g.url || "");
    show(73 | (tp === "prompt" ? 2 : 0) | (tp !== "alert" ? 4 : 0));
    if (key !== dlgKey) { dlgKey = key; if (tp === "prompt") { D.di.value = g.defaultPrompt || ""; sTm(() => D.di.focus(), 0); } }
  } else { hd(D.bn, 1); dlgKey = ""; }
  tx(D.om, errMsg || ""); hd(D.oe, !errMsg);
};
const syncDots = () => {
  D.ld.className = "ed " + (!S.on ? "" : errMsg ? "er" : S.pa || S.id ? "wn" : S.nt ? "" : "ok");
  D.od.className = D.o0.className = "ed " + (errMsg ? "er" : isCn ? "wn" : portF?.valid ? "ok" : "");
};
const setErr = (m, st) => (errMsg = m || null, cErr = !!(m && st), syncBnr(), syncDots());
const clrErr = () => !cErr && setErr();
const setSw = (v) => (isSw = v, tg(D.vc, "sw", v), hd(D.lb, !(S.nv || isSw)));
const sUrl = (u) => doc.activeElement !== D.ui && (D.ui.value = u || "");
const setMeta = (m) => m && (S.pg = { ...S.pg, title: m.title || "", url: m.url || "" }, sUrl(m.url));
const rstF = (th) => (S.seq = 0, S.fh = "", S.cr = false, th && (S.th = ""));
const syncPh = () => {
  if (S.nt) return hideCvs(), tx(D.pm, T[37]), hd(D.n2, 0);
  hd(D.n2, 1);
  if (painted) { const dim = !!(S.pa || S.id); hd(D.ph, 1); hd(D.vc, 0); tg(D.vc, "pa", dim); tx(D.pb, T[S.pa ? 39 : 40]); hd(D.pb, !dim); }
  else { tx(D.pm, T[S.pa ? 51 : 38]); hd(D.ph, 0); hd(D.pb, 1); }
};
const syncPauseUi = () => {
  const lbl = T[S.pa ? 14 : 13], dis = !canCall(), b = D.bp;
  tg(b, "wp", S.pa); sa(b, "aria-pressed", S.pa); sa(b, "aria-label", lbl); b.title = lbl;
  b.disabled = isPa || isDisc || dis; D.dc.disabled = isDisc || dis;
  sa(D.pi, "d", IC[S.pa ? 2 : 1]); syncDots(); syncPh(); uSt();
};
const schedOffP = (ms = net.computeOfflinePollDelay({ hidden: hid(), errCount: offErr })) => { clrTm(0); if (!S.on && !isCn && canCall()) oT = sTm(() => { oT = 0; probeOff(); }, ms); };
const probeOff = async () => {
  if (S.on || isCn || oB || !canCall()) return;
  oB = 1; const t0 = pNow();
  try {
    const d = await reqFrame(0, 0, 0);
    lastFetchAt = nowMs(); offErr = d?.error ? Math.min(4, offErr + 1) : 0;
    !d?.screencastPausedIdle && net.recordSample({ totalMs: rnd(pNow() - t0), serverMs: d?.serverMs || 0, imageBytes: 0 });
    if (isCn) return;
    d?.portFile && (setPf(d), !S.on && uOff());
    if (d?.connected) { act = nowMs(); onPoll(d, 0, S.ep); liveOk() && sched(nextMs(0), 0); return; }
  } catch { offErr = Math.min(4, offErr + 1); } finally {
    oB = 0; !S.on && !isCn && schedOffP(offUrg ? 20 : undefined); offUrg = 0;
  }
};
const goOff = (err, pf) => {
  S.on = S.pa = S.id = S.nt = S.cr = false; isDisc = isPa = cErr = 0; S.dlg = null; S.st = true; S.th = S.cm = S.fm = ""; errMsg = err || null;
  fTimes = []; fpsVal = 0; pf && (portF = pf);
  dropIn(); clrTm(1); sView(); schedOffP();
};
const uOff = () => {
  const ok = !!portF?.valid;
  tg(D.op, "ok", ok); tx(D.op, ok ? T[74] + (Number(portF.port) || 9222) : T[0]);
  tx(D.om, errMsg || ""); hd(D.oe, !errMsg); D.ob.disabled = D.ol.disabled = !!isCn || !canCall();
  tx(D.ob, T[isCn ? 5 : 2]); tx(D.ol, T[isCn ? 5 : 3]);
  syncDots(); !ok && !D.dt.dataset.u && (D.dt.open = true);
};
const sView = () => {
  hd(D.vo, S.on); hd(D.vn, !S.on);
  S.on ? (clrTm(0), syncBnr(), syncPauseUi(), fitStage()) : (clsP(), hideCvs(), syncBnr(), uOff());
};
const renderGuide = () => {
  if (lastGuideLg === curLg) return;
  lastGuideLg = curLg;
  gNodes.forEach((g, idx) => {
    const frag = doc.createDocumentFragment();
    for (const part of String(T[7 + idx] || "").split(/(\$[0-2]|\*[^*]+\*)/g))
      part && frag.append(part[0] === "$" ? h("code", `t=${GC[+part[1]]}`) : part[0] === "*" ? h("b", `t=${part.slice(1, -1)}`) : doc.createTextNode(part));
    g.replaceChildren(frag);
  });
};
const applyLangUi = () => {
  T = I18N[curLg] || I18N.en; doc.documentElement.lang = curLg;
  lNodes.forEach((n) => {
    const ds = n.dataset, vi = T[+ds.i], va = T[+ds.a], vp0 = T[+ds.p];
    vi && tx(n, vi); va && (sa(n, "aria-label", va), n.tagName === "BUTTON" && (n.title = va)); vp0 && (n.placeholder = vp0);
  });
  lgBtns.forEach((b) => { const onL = b.dataset.lg === curLg; sa(b, "aria-pressed", onL); tg(b, "on", onL); });
  tMap.forEach((n) => sa(n.lastChild, "aria-label", T[12]));
  renderGuide(); uOff(); syncBnr(); syncPauseUi();
};
const mkEmblem = (dotId, big) => h(big ? "s.e.el" : "s.e", 0, ico(0, big ? 18 : 13), [`s.ed#${dotId}`]);
const mkLang = () => {
  const setL = (next) => { curLg !== next && (curLg = next, lsS("chrome_stream_lang", next), applyLangUi()); setMore(0); };
  const bs = [["EN", "en"], ["中文", "zh-CN"]].map(([lbl, lg]) => { const b = h("b", `t=${lbl};.lg=${lg};@p=${curLg === lg}`); ck(b, () => setL(lg)); lgBtns.push(b); return b; });
  return h(".l", "role=group;@l=Language", ...bs);
};

const mount = () => {
  const gLi = (num, idx) => { const s = h("s"); gNodes[idx] = s; return h("li", 0, ["s.n", `t=${num}`], s); };
  const row = (k, id) => [".pr", 0, ["s", `.i=${k}`], [`s.nm#${id}`, "t=-"]];
  const ib = (id, a, ic, ex = "", sz = 14, pid) => [`!.i#${id}`, `.a=${a}${ex}`, ico(ic, sz, pid)];
  const hdr = (did, mid, ...rt) => ["header.h", 0, [".m", 0, mkEmblem(did, 0), ["s.w", "t=RemoteCast"], did === "ld" && ["s.rm#rm!", ".i=81"]], mid, [".ha", 0, ...rt]];
  const okI = !!portF?.valid, qRail = h(".sg#qs", "role=radiogroup", [".qt#qt"]);
  qBtns = QK.map((v, i) => { const b = h(v === qMode ? "b.on" : "b", `role=radio;.q=${v};.i=${26 + i};@c=${v === qMode}`); ck(b, () => setQ(v)); qRail.append(b); return b; });

  root.replaceChildren(h(".a", 0,
    [".v#vo", 0,
      hdr("o0", [".t"], mkLang()),
      ["section.f", 0,
        [".oh", 0, mkEmblem("od", 1), [okI ? "s.bd.ok#op" : "s.bd#op"]],
        [".r#oe!", 0, ["s.mt#om"], ["!#or", ".i=1"]],
        ["!.bp#ob"], ["s.nt", ".i=4"], ["!.bs#ol"],
        ["details.d#dt", okI ? "" : "open=", ["summary", ".i=6"], ["ol.ol", 0, ...[0, 1, 2].map((i) => gLi(`0${i + 1}`, i))]]
      ]
    ],
    [".v#vn", 0,
      hdr("ld", [".tw", 0, [".t#tb", "role=tablist;.a=10"], ["!.nb#nt", ".a=11", ico(9, 13)]],
        mkLang(), ib("bp", 13, 1, ";@p=false", 14, "pi"), ib("mo", 72, 8, ";@h=menu;@e=false")
      ),
      [".nr", 0,
        [".pg", 0, ib("bk", 17, 4), ib("fw", 18, 5), ib("rl", 19, 6)],
        ["form.u#uf", 0, ["i.in#ui", ".a=20;.p=20"], ["s.uk", "t=⌘L;@i=true"]],
        ["b.q#bq", ".a=21;@h=dialog;@e=false", ["s.rd#sd"], ["s#ql", 0, ["s#qm"], ["s.fp#qf"], ["s.hd#qh!", "t= · HD"]]],
        ib("bi", 35, 7, ";@e=false")
      ],
      [".s#st", "tabindex=0;.a=36",
        ["textarea.si#si", "autocomplete=off;spellcheck=false;.a=36"], [".lb#lb!"],
        [".r#bn!", 0, ["s.dtg#dp!"], ["s.mt#dm"], ["i.in#di!", ".p=55"], [".da", 0, ["!#dk!", ".i=54"], ["!.bs#do!", ".i=53"], ["!.bs#er!", ".i=1"], ["!.i#dx!", ".a=73;t=×"]]],
        [".ph#ph", 0, ["s#pm"], ["!.bs#n2!", ".i=11"]], ["b.g#pb!"],
        [".vw#vw", 0, ["canvas.c#vc!", ".a=32"]], [".z#ts", "role=status;@v=polite"],
        [".y#im!", 0, ["i.in#ti", ".p=41"], ["b.ch#ty", ".i=42"], ["b.ch.bs#te", ".i=43"], ["s.dv"],
          ...IK.map((k, i) => ["b.ch", `.k=${k};` + (i < 3 ? `t=${k === "Escape" ? "Esc" : k}` : `.i=${41 + i}`)]),
          ["b.ch", "t=↑;.sc=-360;.a=49"], ["b.ch", "t=↓;.sc=360;.a=50"]
        ]
      ],
      [".o#pp!", "role=dialog;.a=21", [".pl", ".i=21"], qRail, row(31, "sr"), row(32, "sv"), row(22, "sp"), row(23, "sm"), ["!.bs#sn", ".i=33"]],
      [".o#mp!", "role=menu", mkLang(), [".mw", ".i=16"], ["!.dg#dc", ".i=15", ico(3)]]
    ]
  ));

  const qAll = (n, s) => [...n.querySelectorAll(s)];
  lNodes = qAll(root, "[data-i],[data-a],[data-p]");
  applyLangUi();

  const wake = () => { S.id = false; act = nowMs(); };
  const sendTx = (pe) => { const v = D.ti.value; if (!v && !pe) return; D.ti.value = ""; toast(T[60]); tyIn(v, pe); };
  const openNT = () => { S.nt = S.id = false; S.ep++; rstF(1); setSw(1); evIn("new_tab", { url: "about:blank" }); };
  const doDisc = async () => {
    if (isDisc || !S.on || !canCall()) return;
    clsP(); isDisc = 1; S.ep++; dropIn(); clrTm(1); syncPauseUi(); toast(T[78]);
    try {
      const d = await rpcSc("disconnect", { mode: "revoke" });
      if (d?.connected === false && d?.ok !== false && !d?.unconfirmed) { S.tabs = []; S.aid = null; rstF(1); goOff(null, d.portFile); return toast(T[61]); }
      throw d?.error || T[79];
    } catch (e) { setErr(eMsg(e, T[79]), 1); }
    isDisc = 0; syncPauseUi(); askUrg(1);
  };
  const togglePa = async () => {
    if (!S.on || isPa || isDisc || !canCall()) return;
    const want = !S.pa; isPa = 1; syncPauseUi(); want && clrTm(1);
    try {
      const d = await sendIn([{ action: want ? "pause" : "resume" }]);
      if (d?.connected === false && !isRec(d)) return goOff(d.error, d.portFile);
      if (!d || d.ok === false) throw d?.error || T[80];
      const pe = Number(d.pauseEpoch); fin(pe) && pe >= S.pe && (S.pe = pe);
      S.pa = typeof d.paused === "boolean" ? d.paused : want; S.st = !S.pa; S.ep++; wake(); setErr();
      toast(T[S.pa ? 51 : 62]);
    } catch (e) { setErr(eMsg(e, T[80]), 1); }
    isPa = 0; syncPauseUi(); S.pa ? clrTm(1) : askUrg(1);
  };
  const ansDlg = (accept) => {
    const g = S.dlg; if (!g) return;
    S.dlg = null; syncBnr(); evIn("dialog", { accept, promptText: g.type === "prompt" ? D.di.value : undefined }); askUrg(1); focusSt();
  };

  const cbs = {
    ob: () => conn("personal"), or: () => (ready ? conn(lastMode, 1) : initBridge()), ol: () => conn("launch_remote", 1),
    nt: openNT, n2: openNT, er: () => (ready ? (setErr(), askUrg(1)) : initBridge()), dx: () => setErr(),
    sn: () => { S.fh = ""; S.cr = false; wake(); setPop(0); toast(T[63]); poll(1); },
    dc: doDisc, bp: togglePa, pb: () => S.on && (S.pa ? togglePa() : (wake(), syncPauseUi(), poll(1))),
    dk: () => ansDlg(false), do: () => ansDlg(true), bi: () => setIme(!imeOn), ty: () => sendTx(false), te: () => sendTx(true),
    bq: (e) => (e.stopPropagation(), setPop(!popOn)), mo: (e) => (e.stopPropagation(), setMore(!moreOn)),
  };
  for (const k in cbs) ck(D[k], cbs[k]);
  for (const s of "bk,back,← ,17|fw,forward,→ ,18|rl,reload,↻ ,19".split("|")) {
    const [id, a, p, i] = s.split(",");
    ck(D[id], () => { S.nv = true; S.id = false; hd(D.lb, 0); toast(p + T[+i]); evIn(a); });
  }

  const onK = (n, ent, esc) => on(n, "keydown", (e) => { ent && e.key === "Enter" && !e.isComposing ? (pd(e), ent()) : esc && e.key === "Escape" && (pd(e), esc()); });
  on(D.dt, "toggle", () => (D.dt.dataset.u = "1"));
  onK(D.di, () => ansDlg(true), () => ansDlg(S.dlg?.type === "alert"));
  on(D.ui, "focus", () => D.ui.select());
  onK(D.ui, 0, () => { D.ui.value = S.pg.url || ""; focusSt(); });
  on(D.uf, "submit", (e) => { pd(e); const u = D.ui.value.trim(); if (!u) return; S.nv = true; S.id = false; S.pg = { ...S.pg, url: u }; hd(D.lb, 0); evIn("navigate", { url: u }); focusSt(); });

  const setQ = (v) => {
    qMode = v; net.setMode(v); qApp = net.getEffectiveQuality(); S.fh = ""; S.cr = false; wake();
    qBtns.forEach((b) => { const onQ = b.dataset.q === v; sa(b, "aria-checked", onQ); tg(b, "on", onQ); });
    D.qt.style.transform = `translateX(${QK.indexOf(v) * 100}%)`;
    uSt(); toast(`✓ ${qLbl()}`); askUrg(1);
  };
  on(D.qs, "keydown", (e) => { const dir = lrDir(e); if (!dir) return; pd(e); const nx = QK[(QK.indexOf(qMode) + dir + QK.length) % QK.length]; setQ(nx); qBtns[QK.indexOf(nx)].focus(); });
  onK(D.ti, () => sendTx(true), () => { setIme(0); focusSt(); });
  qAll(D.im, "[data-k]").forEach((ch) => ck(ch, async () => {
    const k = ch.dataset.k, cb = navigator.clipboard;
    if (k === "Paste" && cb?.readText) try { const txt = await cb.readText(); if (txt) return toast(T[69]), tyIn(txt); } catch {}
    toast(`✓ ${ch.textContent}`); kyIn(k);
  }));
  qAll(D.im, "[data-sc]").forEach((ch) => ck(ch, () => { const dm = vp(); evIn("scroll", { x: rnd(dm[0] / 2), y: rnd(dm[1] / 2), deltaY: Number(ch.dataset.sc) || 360 }); }));

  const ptOf = (e) => {
    const r = D.vc.getBoundingClientRect(); if (!r.width || !r.height) return null;
    const rx = cl((e.clientX - r.left) / r.width, 0, 1), ry = cl((e.clientY - r.top) / r.height, 0, 1), dm = vp();
    const vw = S.pg.viewportWidth || D.vc.width || dm[0], vh = S.pg.viewportHeight || D.vc.height || dm[1];
    return { x: rnd(rx * vw), y: rnd(ry * vh), px: +(rx * 100).toFixed(2), py: +(ry * 100).toFixed(2), sx: vw / r.width, sy: vh / r.height };
  };
  let mM = 0, mN = "left", mvT = 0, mv = null, lRD = 0, supM = 0;
  const flMv = () => { cTm(mvT); mvT = 0; if (mv) { const v = mv; mv = null; evIn("mousemove", v.p, 0, v.t, v.e); } };
  const ptrEv = (act, e, p, button, extra) => evIn(act, { x: p.x, y: p.y, button, clickCount: cl(e.detail || 1, 1, 3), modifiers: modsOf(e) || undefined, ...extra });
  const sup = () => nowMs() < supM, hyp = Math.hypot, onV = (ev, fn, o) => on(D.vc, ev, fn, o), bt = (e) => BN[e.button] || "left";

  onV("mousedown", (e) => {
    if (sup() || e.button > 2) return;
    e.button === 1 && pd(e); clsP(); const p = ptOf(e); if (!p) return;
    focusSt(); mM = BM[e.button] || 1; mN = bt(e); e.button === 2 && (lRD = nowMs());
    ripple(p.px, p.py); ptrEv("mousedown", e, p, mN, { buttons: mM });
  });
  const onMv = (e) => {
    if (sup() || (!mM && (S.id || S.pa || !S.on))) return;
    const p = ptOf(e); if (!p) return;
    mv = { p: { x: p.x, y: p.y, button: mM ? mN : "none", buttons: mM, modifiers: modsOf(e) || undefined }, t: S.aid, e: S.tep };
    mvT || (mvT = sTm(flMv, mM ? 32 : net.ewmaRttMs > 350 ? 150 : 75));
  };
  onV("mousemove", onMv);
  on(window, "mousemove", (e) => mM && e.target !== D.vc && onMv(e));
  on(window, "mouseup", (e) => mM && !sup() && (flMv(), mM = 0, ptrEv("mouseup", e, ptOf(e) || { x: 0, y: 0 }, bt(e), { buttons: 0 })));
  on(window, "resize", () => popOn && posPop());
  onV("auxclick", (e) => e.button === 1 && pd(e));
  onV("contextmenu", (e) => { pd(e); if (sup() || nowMs() - lRD < 450) return; const p = ptOf(e); p && (ripple(p.px, p.py), ptrEv("click", e, p, "right", { clickCount: 1 })); });

  let tch = null, lTp = 0, lTx = 0, lTy = 0;
  onV("touchstart", (e) => {
    if (e.touches.length !== 1) return tch?.lp && cTm(tch.lp), tch = null;
    clsP(); const t0 = e.touches[0], p = ptOf(t0); if (!p) return;
    const cx = t0.clientX, cy = t0.clientY, c = { x0: cx, y0: cy, lx: cx, ly: cy, pt: p, t: S.aid, e: S.tep, mv: 0, lg: 0, lp: 0 };
    c.lp = sTm(() => !c.mv && tch === c && (c.lg = 1, ripple(p.px, p.py), toast(T[70]), clIn(p, "right", 1, c.t, c.e)), 500);
    tch = c;
  }, { passive: true });
  onV("touchmove", (e) => {
    if (!tch || e.touches.length !== 1) return;
    const t0 = e.touches[0], cx = t0.clientX, cy = t0.clientY;
    !tch.mv && hyp(cx - tch.x0, cy - tch.y0) > 8 && (tch.mv = 1, cTm(tch.lp));
    tch.mv && !tch.lg && (pd(e), qScr((tch.lx - cx) * tch.pt.sx, (tch.ly - cy) * tch.pt.sy, tch.pt), tch.lx = cx, tch.ly = cy);
  }, { passive: false });
  const endT = (can) => {
    if (!tch) return;
    const c = tch; tch = null; cTm(c.lp); supM = nowMs() + 500;
    if (can || c.mv || c.lg) return;
    const ts = nowMs(), dbl = ts - lTp < 320 && hyp(c.x0 - lTx, c.y0 - lTy) < 24;
    lTp = dbl ? 0 : ts; lTx = c.x0; lTy = c.y0;
    focusSt(); ripple(c.pt.px, c.pt.py); clIn(c.pt, "left", dbl ? 2 : 1, c.t, c.e);
  };
  onV("touchend", () => endT(0)); onV("touchcancel", () => endT(1));

  let wh = { x: 0, y: 0, p: null, t: null, e: 0 }, wt = 0;
  const qScr = (dx, dy, p) => {
    if (!wt || wh.t !== S.aid || wh.e !== S.tep) { cTm(wt); wt = 0; wh = { x: 0, y: 0, p: null, t: S.aid, e: S.tep }; }
    wh.x += dx; wh.y += dy; wh.p = p || wh.p;
    wt || (wt = sTm(() => {
      const sx = cl(rnd(wh.x), -1400, 1400), sy = cl(rnd(wh.y), -1400, 1400), pt = wh.p || { x: 500, y: 350 };
      wh.x = wh.y = wt = 0;
      (Math.abs(sx) >= 2 || Math.abs(sy) >= 2) && evIn("scroll", { x: pt.x, y: pt.y, deltaX: sx, deltaY: sy }, 1, wh.t, wh.e);
    }, net.getWheelCoalesceMs()));
  };
  onV("wheel", (e) => { pd(e); const sc = e.deltaMode === 1 ? 40 : e.deltaMode === 2 ? vp()[1] * 0.85 : 1; qScr(e.deltaX * sc, e.deltaY * sc, ptOf(e)); }, { passive: false });

  let comp = 0, pArm = 0;
  dropTimers = () => { cTm(wt); cTm(mvT); cTm(pArm); tch?.lp && cTm(tch.lp); wt = wh.x = wh.y = mvT = pArm = 0; mv = tch = wh.p = null; };
  on(D.st, "focus", () => doc.activeElement === D.st && focusSt());
  on(D.si, "compositionstart", () => (comp = 1));
  on(D.si, "compositionend", (e) => { comp = 0; const v = e.data || D.si.value; D.si.value = ""; v && tyIn(v); });
  on(D.si, "beforeinput", (e) => {
    if (comp || e.isComposing) return;
    e.inputType === "deleteContentBackward" ? (pd(e), kyIn("Backspace", 0, 0)) : e.inputType === "insertLineBreak" && (pd(e), kyIn("Enter"));
  });
  on(D.si, "input", () => { if (comp) return; const v = D.si.value; v && (D.si.value = "", tyIn(v, false, 0)); });
  on(D.st, "keydown", (e) => {
    if ((e.target !== D.st && e.target !== D.si) || comp || e.isComposing || e.key === "Process" || e.keyCode === 229) return;
    const mods = modsOf(e), low = e.key.toLowerCase();
    if ((e.metaKey || e.ctrlKey) && !e.altKey) {
      if (low === "l") return;
      if (low === "v") { const a = S.aid, e0 = S.tep; return cTm(pArm), pArm = sTm(() => (pArm = 0, S.aid === a && S.tep === e0 && kyIn("Paste")), 45); }
      const cmd = { a: "SelectAll", c: "Copy", x: "Cut", z: e.shiftKey ? "Redo" : "Undo", y: "Redo" }[low];
      return cmd && (pd(e), low === "c" && toast(T[68]), kyIn(cmd));
    }
    const k = e.key === " " ? "Space" : e.key;
    if (k === "Space" || KEYS.has(k)) return pd(e), kyIn(k, mods, k === "Enter" || k === "Escape");
    if (e.key.length === 1 && !e.metaKey && (!e.ctrlKey || e.altKey)) {
      pd(e); const c = e.key.charCodeAt(0);
      !e.altKey && c >= 33 && c <= 126 ? kyIn(e.key, mods, 0) : tyIn(e.key, false, 0);
    }
  });
  on(doc, "paste", (e) => {
    if (!S.on || [D.ui, D.ti, D.di].includes(e.target)) return;
    pArm && (cTm(pArm), pArm = 0); pd(e); const txt = e.clipboardData?.getData("text/plain") || ""; D.si.value = "";
    txt ? (toast(T[69]), tyIn(txt)) : kyIn("Paste");
  });
};

const chgTab = (tid) => { dropIn(); S.aid = tid; S.id = false; S.dlg = null; S.ep++; rstF(); setMeta(fTab(tid)); syncBnr(); setSw(1); };
const swTab = (tid) => { if (!tid || tid === S.aid) return; chgTab(tid); uTabs(); uSt(); evIn("switch_tab", { targetId: tid }, 1, tid, S.tep); };
const clTab = (tid) => {
  if (!tid || closing.has(tid)) return;
  closing.add(tid);
  const tep = S.tep, o = applyOptimisticTabClose(S.tabs, S.aid, tid);
  S.tabs = o.tabs; S.th = ""; S.id = false;
  o.wasActive && chgTab(o.activeTargetId);
  uTabs(); uSt(); toast(T[64]); evIn("close_tab", { targetId: tid }, 1, tid, tep);
};
const lrDir = (e) => e.key === "ArrowRight" ? 1 : e.key === "ArrowLeft" ? -1 : 0;
const mkTab = (tid) => {
  const x = h("b.x", `t=×;@l=${T[12]}`), node = h(".p", `tabindex=0;role=tab;.id=${tid}`, ["s.tt"], x);
  ck(node, (e) => e.target !== x && swTab(tid));
  on(node, "auxclick", (e) => e.button === 1 && (pd(e), clTab(tid)));
  on(node, "keydown", (e) => {
    if (e.key === "Enter" || e.key === " ") return pd(e), swTab(tid);
    const dir = lrDir(e), i = dir && S.tabs.length > 1 ? S.tabs.findIndex((t) => t.targetId === tid) : -1;
    if (i < 0) return;
    pd(e); const nx = S.tabs[(i + dir + S.tabs.length) % S.tabs.length].targetId;
    swTab(nx); tMap.get(nx)?.focus();
  });
  ck(x, (e) => { e.stopPropagation(); clTab(tid); });
  return node;
};
const uTabs = () => {
  const sig = S.aid + S.tabs.map((t) => `|${t.targetId}:${t.title}:${t.url}`).join("");
  if (sig === tabSig) return;
  tabSig = sig;
  const live = new Set(S.tabs.map((t) => t.targetId));
  tMap.forEach((n, id) => !live.has(id) && (n.remove(), tMap.delete(id)));
  let cur = D.tb.firstChild;
  for (const t of S.tabs) {
    const id = t.targetId, a = id === S.aid, u = t.url || "";
    let node = tMap.get(id);
    node || tMap.set(id, node = mkTab(id));
    tg(node, "on", a); sa(node, "aria-selected", a);
    node.title !== u && (node.title = u);
    tx(node.firstChild, t.title || u || "Tab");
    cur !== node && D.tb.insertBefore(node, cur);
    cur = node.nextSibling;
  }
};
const uSt = () => {
  bumpFps(0);
  const rtt = net.lastNetRttMs || net.ewmaRttMs, pend = inB + qIn.length, dm = vp(), mode = T[S.cm === "remote" ? 25 : 24];
  const tag = S.cr ? " · HD WebP" : S.fm ? ` · ${S.fm.toUpperCase()}` : "", idle = S.pa || S.id || S.nt;
  D.sd.className = `rd ${S.pa || S.id ? "wn" : S.nt ? "id" : rttBand(rtt)}`;
  tx(D.qm, idle ? T[S.pa ? 51 : S.id ? 52 : 37] : `${rtt ? `${rtt}ms` : "--ms"} · ${qName(qMode)}`);
  tx(D.qf, idle ? "" : ` · ${fpsVal}fps`);
  hd(D.qh, idle || !S.cr || S.fm !== "webp");
  hd(D.rm, S.cm !== "remote"); D.rm.title = T[25];
  D.ld.parentElement && (D.ld.parentElement.title = mode);
  tx(D.sr, `${S.uc ? T[34] : kb(S.by)} · #${S.seq}${pend ? T[76] + pend : ""}`);
  tx(D.sv, `${dm[0]}×${dm[1]}`); tx(D.sp, qLbl() + tag); tx(D.sm, mode);
  hd(D.lb, !(S.nv || isSw));
};
const recover = (m) => (S.aid = S.dlg = null, rstF(1), m && toast(m), askUrg(1));
const onPoll = (d, totalMs, reqEpoch = S.ep) => {
  if (!d) return;
  totalMs > 0 && (synced = 1);
  if (d.connected === false) return setPf(d), isRec(d) ? recover(T[65]) : goOff(d.error, d.portFile);
  const wasPa = S.pa, oA = S.aid, oE = S.tep, paintIt = applyFramePayloadState(S, d, reqEpoch, totalMs);
  if (paintIt === null) return liveOk() && askUrg(0);
  setPf(d);
  const sw = d.popupSwitched || d.recovered;
  (sw || S.aid !== oA || S.tep !== oE) && dropTimers();
  if (totalMs > 0 && !d.screencastPausedIdle) {
    const s = net.recordSample({ totalMs, serverMs: d.serverMs || 0, imageBytes: d.changed ? d.imageBytes || 0 : 0 });
    qApp = d.qualityApplied || (s.tierChanged ? s.effectiveTier : qApp);
  } else d.qualityApplied && (qApp = d.qualityApplied);
  sView();
  wasPa !== S.pa && (syncPauseUi(), S.pa && clrTm(1));
  S.nt ? sUrl("") : d.page && sUrl(d.page.url);
  sw ? (clrErr(), toast(T[d.popupSwitched ? 67 : 66])) : (d.error || !cErr) && setErr(d.error);
  uTabs(); syncBnr(); syncDots();
  paintIt ? paint(d.imageDataUrl) : (fitStage(), isSw && painted && S.fh && setSw(0), syncPh());
  uSt();
};
const pushIn = (ev, imm = 1, tid = S.aid, tep = S.tep) => {
  const wasIdle = S.id; act = nowMs(); S.cr = false;
  ev.action !== "pause" && ev.action !== "disconnect" && (S.id = false);
  qIn.push([ev, tid, tep]);
  wasIdle && !S.id ? syncPauseUi() : uSt();
  if (inB) return;
  const ms = imm ? 0 : net.getTypeBatchMs();
  ms <= 0 ? (cTm(inT), inT = 0, flIn()) : !inT && (inT = sTm(() => (inT = 0, flIn()), ms));
};
const flIn = async () => {
  if (inB || !qIn.length || !canCall()) return;
  inB = 1;
  const [f0, bTid, bTep] = qIn[0], isTab = /_tab$/.test(f0.action);
  let k = 1;
  while (k < qIn.length && /_tab$/.test(qIn[k][0].action) === isTab && qIn[k][1] === bTid && qIn[k][2] === bTep) k++;
  const batch = coalesceInputEvents(qIn.splice(0, k).map((x) => x[0]));
  const closed = batch.filter((e) => e.action === "close_tab").map((e) => e.targetId);
  const needEp = batch.some((e) => PAGE_ACT.has(e.action)), ep = S.ep;
  const sendTid = isTab && batch.length === 1 && batch[0].targetId ? batch[0].targetId : bTid;
  uSt();
  try {
    const d = await sendIn(batch, sendTid || undefined, needEp ? bTep : undefined);
    synced = 1;
    if (d?.connected === false && !isRec(d)) return goOff(d.error, d.portFile);
    setPf(d);
    if (d?.staleTarget) {
      dropIn(); clrErr(); toast(T[71]);
      d.activeTargetId !== undefined && (S.aid = d.activeTargetId || null);
      syncTabList(d); "dialog" in d && (S.dlg = d.dialog || null, syncBnr());
      rstF(); !S.pa && askUrg(1); return;
    }
    d?.recovered ? (clrErr(), toast(T[66])) : d?.error && !isTargetErrorText(d.error) && setErr(d.error);
    syncPe(d) && syncPauseUi();
    if (ep === S.ep) {
      "dialog" in (d || {}) && (S.dlg = d.dialog || null, syncBnr());
      (S.nt = isNoTabsPayload(d)) ? clrTabs(d?.tabsHash || "") : (d?.activeTargetId && d.activeTargetId !== S.aid && (S.aid = d.activeTargetId, rstF()), syncTabList(d));
    }
    !S.nt && !S.pa && askUrg(0);
  } catch (err) {
    tErr(err) ? recover() : setErr(eMsg(err));
  } finally {
    closed.forEach((id) => closing.delete(id));
    inB = 0; uSt(); qIn.length && flIn();
  }
};
const sched = (ms, f = 0) => { clrTm(1); if (S.on && !isDisc && (!S.pa || f)) pT = sTm(() => { pT = 0; poll(f); }, ms); };
const askUrg = (f = 0) => { if (!S.on || isDisc || (S.pa && !f)) return; if (pB) urg = { f: !!(f || urg?.f) }; else sched(20, f); };
const timing = (idleMs, inputBusy = 0) => net.computePollTiming({ idleMs, inputBusy: !!inputBusy, screencastIdle: S.id, hidden: hid(), errCount: pErr, settled: S.cr });
const nextMs = (idleMs, busy = 0) => !hid() && S.nt ? 1200 : timing(idleMs, busy).nextDelayMs;
const reqFrame = (f, w, keep = 1) => rpcSc("frame", { targetId: S.aid || undefined, sinceSeq: !f && keep ? S.seq : 0, lastFrameHash: S.fh || undefined, knownTabsHash: S.th || undefined, ...qArgs(), waitMs: f ? 0 : w, forceRefresh: !!f });
const poll = async (f = 0) => {
  if (pB || isDisc || !canCall() || (!f && S.pa)) { if (f && pB && !isDisc) urg = { f: 1 }; return; }
  pB = 1; const ep = S.ep, t0 = pNow();
  try {
    const d = await reqFrame(f, hid() ? 0 : S.nt ? 1000 : timing(nowMs() - act).waitMs);
    lastFetchAt = nowMs(); pErr = d?.error && !d?.recovered ? Math.min(4, pErr + 1) : 0;
    onPoll(d, rnd(pNow() - t0), ep);
  } catch (err) {
    pErr = Math.min(4, pErr + 1);
    tErr(err) ? (S.aid = null, rstF(1), urg = { f: 1 }) : setErr(eMsg(err));
  } finally {
    pB = 0;
    if (S.on && !S.pa && !isDisc) urg ? (sched(20, urg.f), urg = 0) : S.st && sched(nextMs(nowMs() - act, inB || qIn.length), 0);
  }
};
const conn = async (mode = "personal", f = 0) => {
  if (isCn || !canCall()) return;
  lastMode = mode; isCn = 1; errMsg = null; cErr = 0; clrTm(0); !S.on && uOff();
  const ep = S.ep, t0 = pNow(), rem = mode === "launch_remote";
  try {
    const c = await rpcSc(rem ? mode : "connect", !rem && mode !== "auto" ? { ...qArgs(), mode } : qArgs());
    if (!c?.connected) return isCn = 0, goOff(c?.error || null, c?.portFile);
    act = nowMs(); S.on = S.st = true; S.pa = S.id = false; S.pe = Number(c.pauseEpoch) || 0;
    setPf(c); c.activeTargetId && (S.aid = c.activeTargetId); syncTabList(c);
    S.dlg = c.dialog || null; S.nt = isNoTabsPayload(c);
    const d = await reqFrame(f || 1, 0, 0);
    isCn = 0; lastFetchAt = nowMs(); pErr = offErr = 0; onPoll(d, rnd(pNow() - t0), ep);
    liveOk() && sched(nextMs(0), 0);
  } catch (err) { isCn = 0; S.on && tErr(err) ? recover() : goOff(eMsg(err)); }
};
const forceNow = () => {
  if (!ready) return;
  const t = nowMs(); if (t - lastForceAt < 250) return;
  lastForceAt = t; pErr = offErr = 0;
  if (S.on) { if (S.pa) return; act = t; S.id && (S.id = false, syncPauseUi()); clrTm(1); poll(1); }
  else if (!isCn) { clrTm(0); oB ? (offUrg = 1) : probeOff(); }
};
const onHide = () => {
  if (!ready) return;
  S.on && !S.pa && !pB && pT ? sched(nextMs(nowMs() - act, inB || qIn.length), 0) : !S.on && !isCn && !oB && oT && schedOffP();
};

ck(doc, (e) => { popOn && !e.target.closest?.("#pp,#bq") && setPop(0); moreOn && !e.target.closest?.("#mp,#mo") && setMore(0); });
on(doc, "keydown", (e) => {
  if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "l" && S.on) { pd(e); D.ui.focus(); D.ui.select(); }
  else if (e.key === "Escape" && (popOn || moreOn)) { pd(e); popOn ? (setPop(0), D.bq.focus()) : (setMore(0), D.mo.focus()); }
});
const syncHostCtx = (ctx) => {
  applyHostContext(ctx);
  const nextLg = resolvePanelLang({ saved: lsG("chrome_stream_lang"), hostLocale: ctx?.locale || ctx?.language || "", navLang: globalThis.navigator?.language || "", bootLocale: b0.locale || "" });
  nextLg !== curLg && (curLg = nextLg, applyLangUi());
};
const onResult = (r) => {
  const d = r?.structuredContent; if (!d) return;
  const sig = [!!d.connected, !!d.portFile?.valid, +d.portFile?.port || 0, d.activeTargetId, d.tabsHash, d.error].join("|");
  if (sig === tSig) return d.connected && !started && ready && (started = 1, lastForceAt = nowMs(), !S.pa && poll(0));
  tSig = sig; setPf(d); syncPe(d, !synced);
  d.activeTargetId && !synced && (S.aid = d.activeTargetId);
  "dialog" in d && (S.dlg = d.dialog || null);
  if (d.connected) { S.on = true; act = lastFetchAt = nowMs(); sView(); uTabs(); uSt(); ready && poll(0); return; }
  (synced ? S.on : !isCn) && goOff(d.error, d.portFile);
};

app.ontoolresult = onResult;
app.onhostcontextchanged = syncHostCtx;
app.onteardown = () => { clrTm(1); clrTm(0); stopReveal?.(); stopReveal = null; return {}; };

mount(); sView();
if (b0.connected) {
  if (Number(b0.rttMs) > 0) net.lastNetRttMs = net.ewmaRttMs = Number(b0.rttMs);
  S.nt ? sUrl("") : b0.page && sUrl(b0.page.url);
  uTabs(); uSt();
  !S.nt && b0.changed && isSafeImageUrl(b0.imageDataUrl) && paint(b0.imageDataUrl);
}
const initBridge = () => {
  setErr();
  return app.connect().then(() => {
    ready = 1; syncHostCtx(app.getHostContext());
    if (!canCall()) setErr(T[82]);
    uOff(); syncPauseUi();
    stopReveal?.();
    stopReveal = setupRevealObservers({ onReveal: forceNow, onHide, getLastFetchAt: () => lastFetchAt, bridgeApp: app });
    if (!started && canCall()) { started = 1; lastForceAt = nowMs(); S.on ? !S.pa && poll(0) : schedOffP(); }
  }).catch((err) => { ready = 0; uOff(); syncPauseUi(); setErr(T[77] + eMsg(err)); });
};
initBridge();
