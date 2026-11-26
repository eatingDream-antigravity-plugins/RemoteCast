import { App, applyHostContext } from "./bridge.js";
import {
  AdaptiveBandwidthController,
  applyFramePayloadState,
  applyOptimisticTabClose,
  coalesceInputEvents,
  isNoTabsPayload,
  isSafeImageUrl,
  isTargetErrorText,
  resolvePanelLang,
  resolveStageAspect,
  setupRevealObservers,
} from "./net-policy.js";

const doc = document, boot = globalThis.__CHROME_STREAM_BOOT__ || null;
const $ = (i) => doc.getElementById(i), on = (el, ev, fn, o) => el?.addEventListener(ev, fn, o), ck = (el, fn) => on(el, "click", fn);
const sa = (el, k, v) => el?.setAttribute(k, String(v)), qa = (p, s) => p.querySelectorAll(s), pd = (e) => e.preventDefault();
const tx = (el, v) => { if (el && el.textContent !== v) el.textContent = v; };
const hd = (el, v) => { if (el) el.hidden = !!v; }, tg = (el, c, v) => el?.classList.toggle(c, !!v);
const mk = (t, c) => { const e = doc.createElement(t); if (c) e.className = c; return e; };
const rnd = Math.round, pNow = () => performance.now(), nowMs = () => Date.now(), cl = (v, lo, hi) => Math.max(lo, Math.min(hi, v)), isA = Array.isArray;
const sTm = (f, m) => globalThis.setTimeout(f, m), cTm = (i) => globalThis.clearTimeout(i);
const root = $("app"), app = new App({ name: "Chrome Stream", version: "0.1.0" }, {}), net = new AdaptiveBandwidthController("auto");
const KEYS = new Set("Enter Backspace Delete Tab Escape ArrowUp ArrowDown ArrowLeft ArrowRight PageUp PageDown Home End".split(" "));
const QK = "auto ultra low medium high".split(" "), IK = "Enter Escape Tab Backspace Undo Copy Paste SelectAll".split(" ");
const lsG = (k) => { try { return localStorage.getItem(k) || ""; } catch { return ""; } };
const lsS = (k, v) => { try { localStorage.setItem(k, v); } catch {} };

const DBG = "<span class=m>chrome://inspect/#remote-debugging</span>", FLG = "<span class=m>--remote-debugging-port=9222 --user-data-dir=/tmp/chrome-debug</span>";
const TK = "lg of ry co wt gd g1 g2 g3 tb nt cl pa rs dc bk fw rl up qa q_auto q_ultra q_low q_medium q_high tr vp sn ru kb sa no ld pb ib ip ty se k_Backspace k_Undo k_Copy k_Paste k_SelectAll su sd ps is ok cn pp d_alert d_confirm d_prompt d_beforeunload s1 s2 s3 s4 s5 s6 s7 s8 s9 s0 sr".split(" ");
const mkDict = (s, fn) => { const a = s.split("|"), o = { k_Enter: "Enter", k_Escape: "Esc", k_Tab: "Tab", ...fn }; TK.forEach((k, i) => { o[k] = a[i]; }); return o; };
const I18N = {
  en: mkDict(
    `中文|Not detected|Retry|Connect to Chrome|Click Allow in Chrome…|Setup|Open Chrome.|Enable ${DBG} or run ${FLG}.|Click Connect, then Allow.|` +
    "Tabs|New tab|Close|⏸ Pause|▶ Resume|Disconnect|Back|Forward|Reload|" +
    "URL or search (⌘L)|Quality|Auto|Ultra|Low|Med|High|Frame|Viewport|Refresh|Cached 0 KB|" +
    "Keyboard|Live view|No tabs|Loading…|Paused · Resume|Idle · Wake|" +
    "Type or paste, Enter…|Insert|Send ↵|Bksp|Undo|Copy|Paste|Select all|Scroll up|Scroll down|Paused|Idle|" +
    "OK|Cancel|Response…|Alert|Confirm|Prompt|Leave page?|" +
    "✓ Sent|Disconnected|✓ Resumed|↻ Refreshing|✓ Closed|Switching…|✓ Switched|↗ New tab|✓ Copied|✓ Pasted|Right click",
    { rdy: (p) => `Ready · ${p}`, aDot: (e) => `Auto · ${e}`, qu: (n) => ` · ${n} queued`, iEr: (m) => `Init failed: ${m}` }
  ),
  "zh-CN": mkDict(
    `EN|未检测到|重试|连接 Chrome|请在 Chrome 点允许…|设置|打开本机 Chrome。|在 ${DBG} 开启调试，或用 ${FLG} 启动。|点「连接 Chrome」并点允许。|` +
    "标签页|新建标签页|关闭|⏸ 暂停|▶ 继续|断开|后退|前进|刷新|" +
    "网址或搜索（⌘L）|画质|自动|极速|弱网|均衡|高清|单帧体积|视口|刷新画面|未变 0 KB|" +
    "键盘栏|实时画面|无标签页|加载中…|已暂停 · 继续|空闲省电 · 唤醒|" +
    "输入或粘贴，回车…|输入|发送 ↵|退格|撤销|复制|粘贴|全选|上滚|下滚|已暂停|空闲省电|" +
    "确定|取消|输入内容…|提示|确认|输入|离开页面？|" +
    "✓ 已发送|已断开|✓ 已恢复|↻ 刷新中|✓ 已关闭|切换中…|✓ 已切换|↗ 新标签页|✓ 已复制|✓ 已粘贴|右键",
    { rdy: (p) => `已就绪 · ${p}`, aDot: (e) => `自动·${e}`, qu: (n) => ` · 待发 ${n}`, iEr: (m) => `初始化失败：${m}` }
  ),
};

const S = {
  lg: resolvePanelLang({ saved: lsG("chrome_stream_lang"), navLang: globalThis.navigator?.language || "", bootLocale: boot?.locale || "" }),
  on: !!boot?.connected, pf: boot?.portFile || null,
  tabs: isA(boot?.tabs) ? boot.tabs : [], ts: "", th: boot?.tabsHash || "",
  aid: boot?.activeTargetId || null, ep: Number(boot?.epoch) || 1, pe: Number(boot?.pauseEpoch) || 0, pg: boot?.page || {},
  seq: boot?.seq || 0, fh: boot?.frameHash || "", by: boot?.imageBytes || 0,
  uc: false, qa: boot?.qualityApplied || "low", er: null,
  cn: false, sw: false, nv: false, qm: "auto", st: !boot?.paused, pa: !!boot?.paused,
  id: !!boot?.screencastPausedIdle, dlg: boot?.dialog || null, dk: "",
  nt: !!(boot?.connected && isNoTabsPayload(boot)),
  im: false, po: false,
};
const L = () => I18N[S.lg] || I18N.en;

let qIn = [], inB = 0, inT = 0, pB = 0, pT = 0, oT = 0, oB = 0, urg = 0, act = nowMs(), tT = 0;
let pErr = 0, offErr = 0, offUrg = 0, lastFetchAt = boot ? nowMs() : 0, lastForceAt = 0, stopReveal = null;
let pGen = 0, painted = 0, started = 0, synced = 0, ready = 0, tSig = "", lAr = 0;
const closing = new Set(), tMap = new Map(), D = {};

const kb = (b) => (!b ? "0 KB" : b < 1024 ? `${b} B` : `${(b / 1024).toFixed(b < 10240 ? 1 : 0)} KB`);
const vp = () => [Number(S.pg.viewportWidth) || 1280, Number(S.pg.viewportHeight) || 800];
const qLbl = () => { const t = L(), e = t[`q_${S.qa || net.getEffectiveQuality()}`] || t.q_medium; return S.qm === "auto" ? t.aDot(e) : e; };
const isHidden = () => doc.visibilityState === "hidden";
const sigOf = (d) => `${!!d?.connected}|${!!d?.portFile?.valid}|${Number(d?.portFile?.port) || 0}|${d?.activeTargetId || ""}|${d?.tabsHash || ""}|${d?.error || ""}`;
const canCall = () => { const c = app.getHostCapabilities(); return !c || c.serverTools !== false; };
const rpcSc = (name, a) => app.callServerTool({ name, arguments: a }).then((r) => r?.structuredContent);
const sendIn = (events, targetId) => rpcSc("chrome_stream_input", { targetId, events });
const bt = (x, r = "") => `<button type=button ${x}>${r}</button>`;
const bH = (id, c = "", tk = "", ak = "", x = "", raw = "") => bt(`class="${c ? `b ${c}` : "b"}" id=${id}${tk ? ` data-t=${tk}` : ""}${ak ? ` data-a=${ak}` : ""}${x}`, raw);
const evIn = (a, o, imm = true) => pushIn({ action: a, ...o }, imm);
const keyIn = (key, modifiers, imm = false) => evIn("key", { key, modifiers }, imm);
const txtIn = (text, pressEnter = false, imm = true) => evIn("type", { text, pressEnter }, imm);
const modsOf = (e) => (e.altKey ? 1 : 0) | (e.ctrlKey ? 2 : 0) | (e.metaKey ? 4 : 0) | (e.shiftKey ? 8 : 0);
const focusSt = () => (D.si || D.st)?.focus({ preventScroll: true });
const errMsg = (e) => e?.message || String(e);
const toast = (m) => { if (!D.ts || !m) return; tx(D.ts, m); tg(D.ts, "on", 1); cTm(tT); tT = sTm(() => tg(D.ts, "on", 0), 1600); };
const toastK = (k) => toast(L()[k]);
const setPf = (d) => { if (d?.portFile) S.pf = d.portFile; };
const ripple = (px, py) => { if (!D.vw) return; const d = mk("div", "rp"); d.style.left = `${px}%`; d.style.top = `${py}%`; D.vw.appendChild(d); sTm(() => d.remove(), 420); };

const fitStage = (nw = painted ? D.vc?.width : 0, nh = painted ? D.vc?.height : 0) => {
  const a = resolveStageAspect(S.pg.viewportWidth, S.pg.viewportHeight, nw, nh);
  if (a && a !== lAr && D.vw) D.vw.style.setProperty("--ar", lAr = a);
};

const paint = (url) => {
  if (!isSafeImageUrl(url) || !D.vc) return;
  const gen = ++pGen, im = new Image(), [vw, vh] = vp();
  im.decoding = "async";
  const blit = (src, nw, nh) => {
    if (gen !== pGen || !D.vc) { src.close?.(); return; }
    fitStage(nw, nh);
    if (D.vc.width !== nw || D.vc.height !== nh) { D.vc.width = nw; D.vc.height = nh; }
    D.vc.getContext("2d", { alpha: false })?.drawImage(src, 0, 0, nw, nh);
    src.close?.(); painted = 1; hd(D.vc, 0); hd(D.ph, 1); setSw(false);
  };
  const go = () => {
    if (gen !== pGen || !D.vc) return;
    const nw = im.naturalWidth || vw, nh = im.naturalHeight || vh;
    typeof createImageBitmap === "function"
      ? createImageBitmap(im).then((b) => blit(b, b.width || nw, b.height || nh), () => blit(im, nw, nh))
      : blit(im, nw, nh);
  };
  im.src = url;
  im.decode ? im.decode().then(go, () => { if (im.complete && im.naturalWidth) go(); }) : (im.onload = go);
};

const setErr = (m) => { S.er = m || null; tx(D.em, S.er || ""); hd(D.eb, !S.er); tx(D.om, S.er || ""); hd(D.oe, !S.er); };
const sLoad = () => tg(D.lb, "on", S.nv || S.sw);
const setSw = (v) => { S.sw = v; tg(D.vc, "sw", v); sLoad(); };
const sUrl = (u) => { if (D.ui && doc.activeElement !== D.ui) D.ui.value = u || ""; };
const setPop = (o) => { S.po = !!o; hd(D.pp, !S.po); sa(D.bq, "aria-expanded", S.po); };
const setIme = (o) => { S.im = !!o; hd(D.im, !S.im); tg(D.bi, "on", S.im); sa(D.bi, "aria-expanded", S.im); if (S.im) D.ti?.focus(); };
const setMeta = (m) => { if (m) { S.pg = { ...S.pg, title: m.title || "", url: m.url || "" }; sUrl(m.url); } };
const rstF = (th) => { S.seq = 0; S.fh = ""; if (th) S.th = ""; };
const clrTabs = (th = "") => { S.aid = null; S.tabs = []; S.th = th; S.pg = {}; S.dlg = null; sUrl(""); rstF(); };

const syncDlg = () => {
  if (!D.db) return;
  const g = S.on ? S.dlg : null;
  if (!g || typeof g !== "object") { hd(D.db, 1); S.dk = ""; return; }
  const t = L(), tp = g.type || "alert", key = `${g.targetId || ""}:${tp}:${g.message || ""}`;
  tx(D.dtp, t[`d_${tp}`] || t.d_alert); tx(D.dm, g.message || g.url || "");
  hd(D.di, tp !== "prompt"); hd(D.dk, tp === "alert");
  if (key !== S.dk) { S.dk = key; if (tp === "prompt") { D.di.value = g.defaultPrompt || ""; sTm(() => D.di.focus(), 0); } }
  hd(D.db, 0);
};

const syncPh = () => {
  if (!D.ph || !D.pm) return;
  const t = L();
  if (S.nt) {
    painted = 0; hd(D.vc, 1); D.vc.classList.remove("sw", "pa");
    tx(D.pm, t.no); hd(D.nt2, 0); hd(D.ph, 0); hd(D.pb, 1); return;
  }
  hd(D.nt2, 1);
  if (painted) {
    hd(D.ph, 1);
    const dim = !!(S.pa || S.id);
    hd(D.vc, 0); tg(D.vc, "pa", dim);
    tx(D.pb, S.pa ? t.pb : t.ib); hd(D.pb, !dim);
  } else { tx(D.pm, S.pa ? t.ps : t.ld); hd(D.ph, 0); hd(D.pb, 1); }
};

const syncPauseUi = () => {
  if (!D.bp) return;
  const t = L();
  tg(D.bp, "on", S.pa); sa(D.bp, "aria-pressed", S.pa);
  tx(D.bp, S.pa ? t.rs : t.pa);
  tg(D.sd, "pa", S.pa || S.id); tg(D.sd, "lv", !S.pa && !S.id && !S.nt);
  syncPh(); uSt();
};

const offDelay = () => net.computeOfflinePollDelay({ hidden: isHidden(), errCount: offErr });
const stopOffP = () => { cTm(oT); oT = 0; };
const schedOffP = (ms = offDelay()) => { stopOffP(); if (!S.on && !S.cn && ready) oT = sTm(() => { oT = 0; probeOff(); }, ms); };

const probeOff = async () => {
  if (S.on || S.cn || oB || !ready || !canCall()) return;
  oB = 1;
  const t0 = pNow();
  try {
    const d = await reqFrame(false, 0, false);
    lastFetchAt = nowMs();
    offErr = d?.error ? Math.min(4, offErr + 1) : 0;
    if (!d?.screencastPausedIdle) net.recordSample({ totalMs: rnd(pNow() - t0), serverMs: d?.serverMs || 0, imageBytes: 0 });
    if (S.cn) return;
    if (d?.portFile) { setPf(d); if (!S.on) uOff(); }
    if (d?.connected) {
      act = nowMs(); onPoll(d, 0, S.ep);
      if (S.on && S.st && !S.pa) sched(nextMs(0), false);
      return;
    }
  } catch { offErr = Math.min(4, offErr + 1); } finally {
    oB = 0;
    if (!S.on && !S.cn) { if (offUrg) { offUrg = 0; schedOffP(20); } else schedOffP(); }
    else offUrg = 0;
  }
};

const goOff = (err, pf) => {
  S.on = false; S.pa = false; S.id = false; S.dlg = null; S.st = true; S.nt = false; S.th = ""; S.er = err || null;
  if (pf) S.pf = pf;
  stopP(); sView(); schedOffP();
};

const uOff = () => {
  const t = L(), ok = !!S.pf?.valid, port = Number(S.pf?.port) || 9222;
  tg(D.od, "on", ok); tg(D.od, "lv", ok);
  tx(D.op, ok ? t.rdy(port) : t.of);
  tx(D.om, S.er || ""); hd(D.oe, !S.er); D.ob.disabled = !!S.cn;
  tx(D.ob, S.cn ? t.wt : t.co);
  if (!ok && !D.dt.dataset.u) D.dt.open = true;
};

const sView = () => {
  hd(D.vo, S.on); hd(D.vn, !S.on);
  if (!S.on) {
    setPop(false); painted = 0; syncDlg();
    hd(D.vc, 1); D.vc.classList.remove("sw", "pa");
    hd(D.ph, 0); hd(D.pb, 1); uOff();
  } else { stopOffP(); syncDlg(); syncPauseUi(); fitStage(); }
};

const applyLangUi = () => {
  const t = L();
  doc.documentElement.lang = S.lg;
  qa(root, "[data-t]").forEach((e) => { const v = t[e.dataset.t]; if (v) tx(e, v); });
  qa(root, "[data-h]").forEach((e) => { const v = t[e.dataset.h]; if (v) e.innerHTML = v; });
  qa(root, "[data-a]").forEach((e) => { const v = t[e.dataset.a]; if (v) { sa(e, "aria-label", v); if (e.tagName === "BUTTON") e.title = v; } });
  qa(root, "[data-p]").forEach((e) => { const v = t[e.dataset.p]; if (v) e.placeholder = v; });
  for (const [, n] of tMap) { const x = n.querySelector(".tc"); if (x) sa(x, "aria-label", t.cl); }
  uOff(); syncDlg(); syncPauseUi();
};

const toggleLang = () => { S.lg = S.lg === "zh-CN" ? "en" : "zh-CN"; lsS("chrome_stream_lang", S.lg); applyLangUi(); };

const mount = () => {
  const ok = !!S.pf?.valid, li = (n, k) => `<li><span class=n>${n}</span><span data-h=${k}></span></li>`;
  const pr = (k, i) => `<div class=pr><span data-t=${k}></span><span class=nm id=${i}>-</span></div>`;
  const ebH = (a, b, c) => `<div class=eb id=${a} hidden><span id=${b}></span>${bH(c, "bg", "ry")}</div>`;
  const br = (tag) => `<div class=bm><span class=bd></span><${tag}>Chrome Stream</${tag.split(" ")[0]}></div>`;
  const qb = QK.map((v) => bt(`role=radio data-q=${v} data-t=q_${v} aria-checked=${v === S.qm}`)).join("");
  const ik = IK.map((k) => bt(`class=ch data-k=${k} data-t=k_${k}`)).join("");
  const html =
    `<div class=a><div class=v id=vo${S.on ? " hidden" : ""}><section class=sh><div class=pr>${br("h1")}` +
    `<div class=ha><span class="tg nm"><span class=dt id=od></span><span id=op></span></span>${bH("lg0", "bg", "lg", "lg")}</div></div>` +
    `${ebH("oe", "om", "or")}${bH("ob", "bp")}` +
    `<details class=d id=dt${ok ? "" : " open"}><summary data-t=gd></summary><ol class=ol>${li("01", "g1")}${li("02", "g2")}${li("03", "g3")}</ol></details></section></div>` +
    `<div class=v id=vn${S.on ? "" : " hidden"}><header class=hd>${br("span class=bn")}` +
    `<div class=tb id=tb role=tablist data-a=tb>${bH("nt", "bg", "", "nt", "", "+")}</div>` +
    `<div class=ha>${bH("lg1", "bg", "lg", "lg")}${bH("bp", "bg", "", "pa", " aria-pressed=false")}${bH("dc", "bg dg", "dc", "dc")}</div></header>` +
    `<div class=rw><div class=pg>${bH("bk", "bg", "", "bk", "", "←")}${bH("fw", "bg", "", "fw", "", "→")}${bH("rl", "bg", "", "rl", "", "↻")}</div>` +
    `<form class=uf id=uf><input class=in id=ui data-a=up data-p=up></form>` +
    `<div class=pw>${bH("bq", "nm", "", "qa", " aria-haspopup=dialog aria-expanded=false", `<span class="dt on lv" id=sd></span><span id=ql></span>`)}` +
    `<div class=p id=pp role=dialog data-a=qa hidden><div class=pl data-t=qa></div><div class=sg id=qs role=radiogroup>${qb}</div>` +
    `${pr("tr", "sr")}${pr("vp", "sv")}<div class=pr>${bH("sn", "", "sn")}${bH("dc2", "dg", "dc")}</div></div></div>` +
    `${bH("bi", "", "", "kb", " aria-expanded=false", "⌨")}</div>` +
    `${ebH("eb", "em", "er")}` +
    `<div class=db id=db hidden><span class=dtg id=dtp></span><span class=dm id=dm></span><input class=in id=di data-p=pp hidden>` +
    `<div class=da>${bH("dk", "bg", "cn")}${bH("do", "bp", "ok")}</div></div>` +
    `<div class=st id=st tabindex=0 data-a=sa><textarea class=si id=si autocomplete=off spellcheck=false data-a=sa></textarea>` +
    `<div class=lb id=lb></div><div class=ph id=ph><span id=pm></span>${bH("nt2", "bp", "nt", "", " hidden")}</div>` +
    `${bt("class=pb id=pb hidden")}` +
    `<div class=vw id=vw><canvas class=vc id=vc data-a=vp hidden></canvas></div><div class=ts id=ts role=status aria-live=polite></div></div>` +
    `<div class=i id=im hidden><div class=rw><input class=in id=ti data-p=ip>${bH("ty", "", "ty")}${bH("te", "bp", "se")}</div>` +
    `<div class=ik>${ik}${bt("class=ch data-sc=-360 data-a=su", "↑")}${bt("class=ch data-sc=360 data-a=sd", "↓")}</div></div></div></div>`;

  root.innerHTML = html;
  for (const [, i] of html.matchAll(/\bid=(\w+)/g)) D[i] = $(i);
  applyLangUi();

  const wake = () => { S.id = false; act = nowMs(); };
  const sendTx = (pe) => { const v = D.ti.value; if (!v && !pe) return; D.ti.value = ""; toastK("s1"); txtIn(v, pe); };
  const openNT = () => { S.nt = false; S.id = false; S.ep++; rstF(1); setSw(true); evIn("new_tab", { url: "about:blank" }); };
  const doDisc = () => {
    setPop(false); S.ep++; qIn = []; S.tabs = []; S.aid = null; rstF(1); goOff(null); toastK("s2");
    if (canCall()) rpcSc("chrome_stream_disconnect", {}).then((d) => { if (d?.portFile) { setPf(d); uOff(); } }, () => {});
  };
  const togglePa = () => {
    if (!S.on) return;
    S.pa = !S.pa; S.st = !S.pa; S.ep++; S.pe++; wake(); syncPauseUi();
    const syncPe = (d) => { const pe = Number(d?.pauseEpoch); if (Number.isFinite(pe) && pe > S.pe) S.pe = pe; };
    if (S.pa) { stopP(); toastK("ps"); if (canCall()) sendIn([{ action: "pause" }]).then(syncPe, () => {}); }
    else { toastK("s3"); if (canCall()) sendIn([{ action: "resume" }]).then(syncPe, () => {}).finally(() => { if (S.on && !S.pa) askUrg(true); }); }
  };
  const wakeId = () => { if (!S.on) return; if (S.pa) return togglePa(); wake(); syncPauseUi(); poll(true); };
  const ansDlg = (accept) => {
    const g = S.dlg; if (!g) return;
    const promptText = g.type === "prompt" ? D.di.value : undefined;
    S.dlg = null; syncDlg(); evIn("dialog", { accept, promptText }); askUrg(true); focusSt();
  };

  for (const [id, fn] of [
    ["ob", () => conn(false)], ["or", () => conn(true)], ["lg0", toggleLang], ["lg1", toggleLang],
    ["nt", openNT], ["nt2", openNT], ["er", () => { setErr(null); askUrg(true); }],
    ["sn", () => { S.fh = ""; wake(); setPop(false); toastK("s4"); poll(true); }],
    ["dc", doDisc], ["dc2", doDisc], ["bp", togglePa], ["pb", wakeId],
    ["dk", () => ansDlg(false)], ["do", () => ansDlg(true)],
    ["bi", () => setIme(!S.im)], ["ty", () => sendTx(false)], ["te", () => sendTx(true)],
    ["bq", (e) => { e.stopPropagation(); setPop(!S.po); }],
  ]) ck(D[id], fn);
  for (const [id, a, p] of [["bk", "back", "← "], ["fw", "forward", "→ "], ["rl", "reload", "↻ "]]) ck(D[id], () => { S.nv = true; S.id = false; sLoad(); toast(p + L()[id]); evIn(a); });

  on(D.dt, "toggle", () => { D.dt.dataset.u = "1"; });
  on(D.di, "keydown", (e) => { if (e.key === "Enter" && !e.isComposing) { pd(e); ansDlg(true); } else if (e.key === "Escape") { pd(e); ansDlg(S.dlg?.type === "alert"); } });
  on(D.ui, "focus", () => D.ui.select());
  on(D.ui, "keydown", (e) => { if (e.key === "Escape") { pd(e); D.ui.value = S.pg.url || ""; focusSt(); } });
  on(D.uf, "submit", (e) => { pd(e); const u = D.ui.value.trim(); if (!u) return; S.nv = true; S.id = false; S.pg = { ...S.pg, url: u }; sLoad(); evIn("navigate", { url: u }); focusSt(); });

  const setQ = (v) => {
    S.qm = v; net.setMode(v); S.qa = net.getEffectiveQuality(); S.fh = ""; wake();
    qa(D.qs, "[data-q]").forEach((el) => sa(el, "aria-checked", el.dataset.q === v));
    uSt(); toast(`✓ ${qLbl()}`); askUrg(true);
  };
  qa(D.qs, "[data-q]").forEach((el) => ck(el, () => setQ(el.dataset.q)));
  on(D.qs, "keydown", (e) => {
    if (e.key !== "ArrowLeft" && e.key !== "ArrowRight") return;
    pd(e); const i = QK.indexOf(S.qm), nx = QK[(i + (e.key === "ArrowRight" ? 1 : QK.length - 1)) % QK.length];
    setQ(nx); D.qs.querySelector(`[data-q="${nx}"]`)?.focus();
  });

  on(D.ti, "keydown", (e) => { if (e.key === "Enter" && !e.isComposing) { pd(e); sendTx(true); } else if (e.key === "Escape") { pd(e); setIme(false); focusSt(); } });
  qa(D.im, "[data-k]").forEach((el) => ck(el, async () => {
    const k = el.dataset.k;
    if (k === "Paste" && navigator.clipboard?.readText) {
      try { const txt = await navigator.clipboard.readText(); if (txt) { toastK("s0"); txtIn(txt); return; } } catch {}
    }
    toast(`✓ ${el.textContent}`); keyIn(k, undefined, true);
  }));
  qa(D.im, "[data-sc]").forEach((el) => ck(el, () => { const [w, h] = vp(); evIn("scroll", { x: rnd(w / 2), y: rnd(h / 2), deltaY: Number(el.dataset.sc) || 360 }); }));

  const ptOf = (e) => {
    const r = D.vc.getBoundingClientRect(); if (!r.width || !r.height) return null;
    const rx = cl((e.clientX - r.left) / r.width, 0, 1), ry = cl((e.clientY - r.top) / r.height, 0, 1), [w, h] = vp();
    const vw = S.pg.viewportWidth || D.vc.width || w, vh = S.pg.viewportHeight || D.vc.height || h;
    return { x: rnd(rx * vw), y: rnd(ry * vh), px: +(rx * 100).toFixed(2), py: +(ry * 100).toFixed(2), sx: vw / r.width, sy: vh / r.height };
  };

  let mM = 0, mN = "left", mvT = 0, mvP = null, lRD = 0, supM = 0;
  const bInf = (b) => (b === 2 ? ["right", 2] : b === 1 ? ["middle", 4] : ["left", 1]);
  const flMv = () => { if (mvT) { cTm(mvT); mvT = 0; } if (mvP) { const p = mvP; mvP = null; evIn("mousemove", p, false); } };
  const ptrEv = (act, e, p, button, extra) => evIn(act, { x: p.x, y: p.y, button, clickCount: cl(e.detail || 1, 1, 3), modifiers: modsOf(e) || undefined, ...extra });

  const sup = () => nowMs() < supM, hyp = Math.hypot;
  const onV = (ev, fn, o) => on(D.vc, ev, fn, o);
  onV("mousedown", (e) => {
    if (sup() || e.button > 2) return;
    if (e.button === 1) pd(e);
    if (S.po) setPop(false);
    const p = ptOf(e); if (!p) return;
    focusSt();
    const [btn, mask] = bInf(e.button);
    mM = mask; mN = btn; if (e.button === 2) lRD = nowMs();
    ripple(p.px, p.py);
    ptrEv("mousedown", e, p, btn, { buttons: mask });
  });

  const onMv = (e) => {
    if (sup() || (!mM && (S.id || S.pa || !S.on))) return;
    const p = ptOf(e); if (!p) return;
    mvP = { x: p.x, y: p.y, button: mM ? mN : "none", buttons: mM, modifiers: modsOf(e) || undefined };
    if (!mvT) mvT = sTm(flMv, mM ? 32 : net.ewmaRttMs > 350 ? 150 : 75);
  };
  onV("mousemove", onMv);
  on(window, "mousemove", (e) => { if (mM && e.target !== D.vc) onMv(e); });
  on(window, "mouseup", (e) => {
    if (!mM || sup()) return;
    flMv();
    const [btn] = bInf(e.button), p = ptOf(e) || { x: 0, y: 0 };
    mM = 0;
    ptrEv("mouseup", e, p, btn, { buttons: 0 });
  });
  onV("auxclick", (e) => { if (e.button === 1) pd(e); });
  onV("contextmenu", (e) => {
    pd(e);
    if (sup() || nowMs() - lRD < 450) return;
    const p = ptOf(e); if (p) { ripple(p.px, p.py); ptrEv("click", e, p, "right", { clickCount: 1 }); }
  });

  let tch = null, lTp = 0, lTx = 0, lTy = 0;
  onV("touchstart", (e) => {
    if (e.touches.length !== 1) { if (tch?.lp) cTm(tch.lp); tch = null; return; }
    if (S.po) setPop(false);
    const t0 = e.touches[0], p = ptOf(t0); if (!p) return;
    const c = { x0: t0.clientX, y0: t0.clientY, lx: t0.clientX, ly: t0.clientY, p, mv: 0, lg: 0, lp: 0 };
    c.lp = sTm(() => { if (!c.mv && tch === c) { c.lg = 1; ripple(p.px, p.py); toastK("sr"); evIn("click", { x: p.x, y: p.y, button: "right", clickCount: 1 }); } }, 500);
    tch = c;
  }, { passive: true });
  onV("touchmove", (e) => {
    if (!tch || e.touches.length !== 1) return;
    const t0 = e.touches[0], dx = tch.lx - t0.clientX, dy = tch.ly - t0.clientY;
    if (!tch.mv && hyp(t0.clientX - tch.x0, t0.clientY - tch.y0) > 8) { tch.mv = 1; cTm(tch.lp); }
    if (tch.mv && !tch.lg) { pd(e); tch.lx = t0.clientX; tch.ly = t0.clientY; qScr(dx * tch.p.sx, dy * tch.p.sy, tch.p); }
  }, { passive: false });
  const endT = (can) => {
    if (!tch) return;
    const c = tch; tch = null; cTm(c.lp); supM = nowMs() + 500;
    if (can || c.mv || c.lg) return;
    const ts = nowMs(), dbl = ts - lTp < 320 && hyp(c.x0 - lTx, c.y0 - lTy) < 24;
    lTp = dbl ? 0 : ts; lTx = c.x0; lTy = c.y0;
    focusSt(); ripple(c.p.px, c.p.py); evIn("click", { x: c.p.x, y: c.p.y, button: "left", clickCount: dbl ? 2 : 1 });
  };
  onV("touchend", () => endT(false));
  onV("touchcancel", () => endT(true));

  let wx = 0, wy = 0, wp = null, wt = 0;
  const qScr = (dx, dy, p) => {
    wx += dx; wy += dy; wp = p || wp;
    if (!wt) wt = sTm(() => {
      const sx = cl(rnd(wx), -1400, 1400), sy = cl(rnd(wy), -1400, 1400), pt = wp || { x: 500, y: 350 };
      wx = wy = wt = 0;
      if (Math.abs(sx) >= 2 || Math.abs(sy) >= 2) evIn("scroll", { x: pt.x, y: pt.y, deltaX: sx, deltaY: sy });
    }, net.getWheelCoalesceMs());
  };
  onV("wheel", (e) => { pd(e); const sc = e.deltaMode === 1 ? 40 : e.deltaMode === 2 ? vp()[1] * 0.85 : 1; qScr(e.deltaX * sc, e.deltaY * sc, ptOf(e)); }, { passive: false });

  let comp = 0, pArm = 0;
  on(D.st, "focus", () => { if (doc.activeElement === D.st) focusSt(); });
  on(D.si, "compositionstart", () => { comp = 1; });
  on(D.si, "compositionend", (e) => { comp = 0; const v = e.data || D.si.value; D.si.value = ""; if (v) txtIn(v); });
  on(D.si, "beforeinput", (e) => {
    if (comp || e.isComposing) return;
    if (e.inputType === "deleteContentBackward") { pd(e); keyIn("Backspace"); }
    else if (e.inputType === "insertLineBreak") { pd(e); keyIn("Enter", undefined, true); }
  });
  on(D.si, "input", (e) => { if (comp || e.isComposing) return; const v = D.si.value; if (v) { D.si.value = ""; txtIn(v, false, false); } });

  const onKey = (e) => {
    if (comp || e.isComposing || e.key === "Process" || e.keyCode === 229) return;
    const mods = modsOf(e), low = e.key.toLowerCase();
    if ((e.metaKey || e.ctrlKey) && !e.altKey) {
      if (low === "l") return;
      if (low === "v") { cTm(pArm); pArm = sTm(() => { pArm = 0; keyIn("Paste", undefined, true); }, 45); return; }
      const cmd = { a: ["SelectAll"], c: ["Copy", L().s9], x: ["Cut"], z: [e.shiftKey ? "Redo" : "Undo"], y: ["Redo"] }[low];
      if (cmd) { pd(e); if (cmd[1]) toast(cmd[1]); keyIn(cmd[0], undefined, true); }
      return;
    }
    if (KEYS.has(e.key)) { pd(e); keyIn(e.key, mods || undefined, e.key === "Enter" || e.key === "Escape"); return; }
    if (e.key === " ") { pd(e); keyIn("Space", mods || undefined); return; }
    if (e.key.length === 1 && !e.metaKey && (!e.ctrlKey || e.altKey)) {
      pd(e); const c = e.key.charCodeAt(0);
      if (!e.altKey && c >= 33 && c <= 126) keyIn(e.key, mods || undefined);
      else txtIn(e.key, false, false);
    }
  };
  on(D.si, "keydown", onKey);
  on(D.st, "keydown", (e) => { if (e.target === D.st) onKey(e); });

  on(doc, "paste", (e) => {
    if (!S.on || [D.ui, D.ti, D.di].includes(e.target)) return;
    if (pArm) { cTm(pArm); pArm = 0; }
    pd(e);
    const txt = e.clipboardData?.getData("text/plain") || "";
    if (D.si) D.si.value = "";
    if (txt) { toastK("s0"); txtIn(txt); }
    else keyIn("Paste", undefined, true);
  });
};

const swTab = (tid) => {
  if (!tid || tid === S.aid) return;
  S.aid = tid; S.id = false; S.dlg = null; S.ep++; rstF(); setMeta(S.tabs.find((t) => t.targetId === tid));
  syncDlg(); setSw(true); uTabs(); uSt(); evIn("switch_tab", { targetId: tid });
};

const clTab = (tid) => {
  if (!tid || closing.has(tid)) return;
  closing.add(tid);
  const o = applyOptimisticTabClose(S.tabs, S.aid, tid);
  S.tabs = o.tabs; S.th = ""; S.id = false;
  if (o.wasActive) { S.aid = o.activeTargetId; S.dlg = null; S.ep++; rstF(); syncDlg(); setSw(true); setMeta(S.tabs.find((t) => t.targetId === S.aid)); }
  uTabs(); uSt(); toastK("s5"); evIn("close_tab", { targetId: tid });
};

const mkTab = (tid) => {
  const el = mk("div", "tp"), sp = mk("span", "tt"), x = mk("button", "tc");
  el.tabIndex = 0; sa(el, "role", "tab"); el.dataset.id = tid;
  x.type = "button"; x.textContent = "×"; sa(x, "aria-label", L().cl);
  el.append(sp, x);
  ck(el, (e) => { if (e.target !== x) swTab(tid); });
  on(el, "auxclick", (e) => { if (e.button === 1) { pd(e); clTab(tid); } });
  on(el, "keydown", (e) => {
    if (e.key === "Enter" || e.key === " ") { pd(e); swTab(tid); }
    else if (e.key === "ArrowLeft" || e.key === "ArrowRight") {
      pd(e); const i = S.tabs.findIndex((t) => t.targetId === tid);
      if (i < 0 || S.tabs.length < 2) return;
      const nx = S.tabs[(i + (e.key === "ArrowRight" ? 1 : S.tabs.length - 1)) % S.tabs.length].targetId;
      swTab(nx); tMap.get(nx)?.focus();
    }
  });
  ck(x, (e) => { e.stopPropagation(); clTab(tid); });
  return el;
};

const uTabs = () => {
  if (!D.tb) return;
  const sig = `${S.aid}|${S.tabs.map((t) => `${t.targetId}:${t.title}:${t.url}`).join("|")}`;
  if (sig === S.ts) return;
  S.ts = sig;
  const live = new Set(S.tabs.map((t) => t.targetId));
  for (const [id, n] of tMap) if (!live.has(id)) { n.remove(); tMap.delete(id); }
  let cur = D.tb.firstChild;
  for (const t of S.tabs) {
    let el = tMap.get(t.targetId);
    if (!el) { el = mkTab(t.targetId); tMap.set(t.targetId, el); }
    const a = t.targetId === S.aid;
    tg(el, "on", a); sa(el, "aria-selected", a);
    if (el.title !== (t.url || "")) el.title = t.url || "";
    tx(el.firstChild, t.title || t.url || "Tab");
    if (cur !== el) D.tb.insertBefore(el, cur || D.nt);
    cur = el.nextSibling;
  }
};

const uSt = () => {
  if (!D.ql) return;
  const t = L(), rtt = net.lastNetRttMs || net.ewmaRttMs, pend = (inB ? 1 : 0) + qIn.length, [w, h] = vp();
  sa(D.sd, "data-n", net.getNetworkGrade());
  tg(D.sd, "pa", S.pa || S.id);
  tx(D.ql, S.pa ? t.ps : S.id ? t.is : S.nt ? t.no : rtt ? `${qLbl()} · ${rtt}ms` : qLbl());
  tx(D.sr, `${S.uc ? t.ru : kb(S.by)} · #${S.seq}${pend ? t.qu(pend) : ""}`);
  tx(D.sv, `${w}×${h}`);
  sLoad();
};

const recover = (m) => { S.aid = null; S.dlg = null; rstF(1); if (m) toast(m); askUrg(true); };

const onPoll = (d, totalMs, reqEpoch = S.ep) => {
  if (!d) return;
  if (totalMs > 0) synced = 1;
  setPf(d);
  if (d.connected === false) return (d.recoverable || isTargetErrorText(d.error)) ? recover(L().s6) : goOff(d.error, d.portFile);

  const wasPa = S.pa;
  const paintIt = applyFramePayloadState(S, d, reqEpoch, totalMs);
  if (paintIt === null) {
    if (S.on && S.st && !S.pa) askUrg(false);
    return;
  }
  if (totalMs > 0 && !d.screencastPausedIdle) {
    const s = net.recordSample({ totalMs, serverMs: d.serverMs || 0, imageBytes: d.changed ? d.imageBytes || 0 : 0 });
    S.qa = d.qualityApplied || (s.tierChanged ? s.effectiveTier : S.qa);
  } else if (d.qualityApplied) S.qa = d.qualityApplied;

  sView();
  if (wasPa !== S.pa) { syncPauseUi(); if (S.pa) stopP(); }
  if (S.nt) sUrl(""); else if (d.page) sUrl(d.page.url);
  if (d.popupSwitched || d.recovered) { setErr(null); toastK(d.popupSwitched ? "s8" : "s7"); }
  else setErr(d.error || null);
  uTabs(); syncDlg();
  if (paintIt) paint(d.imageDataUrl);
  else {
    fitStage();
    if (S.sw && painted && S.fh) setSw(false);
    syncPh();
  }
  uSt();
};

const pushIn = (ev, imm = true) => {
  const wasIdle = S.id;
  act = nowMs();
  if (ev.action !== "pause" && ev.action !== "disconnect") S.id = false;
  qIn.push(ev);
  if (wasIdle && !S.id) syncPauseUi(); else uSt();
  if (inB) return;
  const ms = imm ? 0 : net.getTypeBatchMs();
  if (ms <= 0) { cTm(inT); inT = 0; flIn(); }
  else if (!inT) inT = sTm(() => { inT = 0; flIn(); }, ms);
};

const flIn = async () => {
  if (inB || !qIn.length || !canCall()) return;
  inB = 1;
  const batch = coalesceInputEvents(qIn); qIn = [];
  const closed = batch.filter((e) => e.action === "close_tab").map((e) => e.targetId), hadDisc = batch.some((e) => e.action === "disconnect");
  const ep = S.ep; uSt();
  try {
    const d = await sendIn(batch, S.aid || undefined);
    synced = 1;
    if (hadDisc || (d?.connected === false && !d?.recoverable && !isTargetErrorText(d?.error))) return goOff(d?.error, d?.portFile);
    if (d?.error && !d?.recovered && !isTargetErrorText(d.error)) setErr(d.error);
    else if (d?.recovered) { setErr(null); toastK("s7"); }
    const pe = Number(d?.pauseEpoch);
    if (Number.isFinite(pe) && pe > S.pe) { S.pe = pe; if (typeof d.paused === "boolean") { S.pa = d.paused; S.st = !d.paused; syncPauseUi(); } }
    if (ep === S.ep) {
      if ("dialog" in (d || {})) { S.dlg = d.dialog || null; syncDlg(); }
      const noTabs = isNoTabsPayload(d);
      S.nt = noTabs;
      if (noTabs) { clrTabs(d?.tabsHash || ""); syncDlg(); uTabs(); syncPh(); }
      else {
        if (d?.activeTargetId && d.activeTargetId !== S.aid) { S.aid = d.activeTargetId; rstF(); }
        if (isA(d?.tabs)) { S.tabs = d.tabs; S.th = d.tabsHash || ""; setMeta(S.tabs.find((t) => t.targetId === S.aid)); uTabs(); }
      }
    }
    if (!S.nt && !S.pa) askUrg(false);
  } catch (err) {
    isTargetErrorText(err?.message) ? recover() : setErr(errMsg(err));
  } finally {
    for (const id of closed) closing.delete(id);
    inB = 0; uSt(); if (qIn.length) flIn();
  }
};

const stopP = () => { cTm(pT); pT = 0; };
const sched = (ms, f = false) => { stopP(); if (S.on && (!S.pa || f)) pT = sTm(() => { pT = 0; poll(f); }, ms); };
const askUrg = (f = false) => { if (!S.on || (S.pa && !f)) return; if (pB) urg = { f: !!(f || urg?.f) }; else sched(20, f); };
const timing = (idleMs, inputBusy = false) => net.computePollTiming({ idleMs, inputBusy, screencastIdle: S.id, hidden: isHidden(), errCount: pErr });
const nextMs = (idleMs, busy = false) => !isHidden() && S.nt ? 1200 : timing(idleMs, busy).nextDelayMs;
const reqFrame = (f, w, keep = true) => rpcSc("chrome_stream_frame", { targetId: S.aid || undefined, sinceSeq: !f && keep ? S.seq : 0, lastFrameHash: S.fh || undefined, knownTabsHash: S.th || undefined, qualityPreset: net.getEffectiveQuality(), rttMs: net.ewmaRttMs || 0, waitMs: f ? 0 : w, forceRefresh: f });

const poll = async (f = false) => {
  if (pB || !canCall() || (!f && S.pa)) { if (f && pB) urg = { f: true }; return; }
  pB = 1;
  const ep = S.ep, t0 = pNow(), hid = isHidden();
  try {
    const d = await reqFrame(f, hid ? 0 : S.nt ? 1000 : timing(nowMs() - act).waitMs);
    lastFetchAt = nowMs();
    pErr = d?.error && !d?.recovered ? Math.min(4, pErr + 1) : 0;
    onPoll(d, rnd(pNow() - t0), ep);
  } catch (err) {
    pErr = Math.min(4, pErr + 1);
    if (isTargetErrorText(err?.message)) { S.aid = null; rstF(1); urg = { f: true }; }
    else setErr(errMsg(err));
  } finally {
    pB = 0;
    if (S.on && !S.pa) {
      if (urg) { const u = urg; urg = 0; sched(20, u.f); }
      else if (S.st) sched(nextMs(nowMs() - act, !!(inB || qIn.length)), false);
    }
  }
};

const conn = async (f = false) => {
  if (S.cn || !canCall()) return;
  S.cn = true; S.er = null; stopOffP(); if (!S.on) uOff();
  const ep = S.ep, t0 = pNow();
  try {
    const c = await rpcSc("chrome_stream_connect", { qualityPreset: net.getEffectiveQuality(), rttMs: net.ewmaRttMs || 0 });
    if (!c?.connected) { S.cn = false; return goOff(c?.error || null, c?.portFile); }
    act = nowMs(); S.on = true; S.pa = false; S.id = false; S.st = true; S.pe = Number(c.pauseEpoch) || 0;
    setPf(c);
    if (isA(c.tabs)) S.tabs = c.tabs;
    if (c.tabsHash) S.th = c.tabsHash;
    if (c.activeTargetId) S.aid = c.activeTargetId;
    S.dlg = c.dialog || null;
    S.nt = isNoTabsPayload(c);
    const d = await reqFrame(f || true, 0, false);
    S.cn = false; lastFetchAt = nowMs(); pErr = 0; offErr = 0; onPoll(d, rnd(pNow() - t0), ep);
    if (S.on && S.st && !S.pa) sched(nextMs(0), false);
  } catch (err) {
    S.cn = false;
    if (S.on && isTargetErrorText(err?.message)) return recover();
    goOff(errMsg(err));
  }
};

const forceNow = () => {
  if (!ready) return;
  const t = nowMs();
  if (t - lastForceAt < 250) return;
  lastForceAt = t;
  pErr = 0; offErr = 0;
  if (S.on) {
    if (S.pa) return;
    act = t;
    if (S.id) { S.id = false; syncPauseUi(); }
    stopP(); poll(true);
  } else if (!S.cn) {
    stopOffP();
    if (oB) offUrg = 1; else probeOff();
  }
};

const onHide = () => {
  if (!ready) return;
  if (S.on && !S.pa && !pB && pT) sched(nextMs(nowMs() - act, !!(inB || qIn.length)), false);
  else if (!S.on && !S.cn && !oB && oT) schedOffP();
};

ck(doc, (e) => { if (S.po && !e.target.closest?.(".pw")) setPop(false); });
on(doc, "keydown", (e) => {
  if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "l" && D.ui && S.on) { pd(e); D.ui.focus(); D.ui.select(); }
  else if (e.key === "Escape" && S.po) { pd(e); setPop(false); D.bq?.focus(); }
});

const bootGo = () => { if (!started && ready) { started = 1; lastForceAt = nowMs(); if (S.on && !S.pa) poll(false); else if (!S.on) schedOffP(); } };

const syncHostCtx = (ctx) => {
  applyHostContext(ctx);
  const nextLg = resolvePanelLang({ saved: lsG("chrome_stream_lang"), hostLocale: ctx?.locale || ctx?.language || "", navLang: globalThis.navigator?.language || "", bootLocale: boot?.locale || "" });
  if (nextLg !== S.lg) { S.lg = nextLg; applyLangUi(); }
};

const onResult = (r) => {
  const d = r?.structuredContent; if (!d) return;
  const sig = sigOf(d);
  if (sig === tSig) { if (d.connected) bootGo(); return; }
  tSig = sig;
  setPf(d);
  if (typeof d.paused === "boolean") {
    const pe = Number(d.pauseEpoch);
    if (Number.isFinite(pe) && pe > S.pe) { S.pe = pe; S.pa = d.paused; S.st = !d.paused; }
    else if (!synced) { S.pa = d.paused; S.st = !d.paused; }
  }
  if (d.activeTargetId && !synced) S.aid = d.activeTargetId;
  if ("dialog" in d) S.dlg = d.dialog || null;
  if (d.connected) { S.on = true; act = nowMs(); lastFetchAt = nowMs(); sView(); uTabs(); uSt(); if (ready) poll(false); return; }
  if (synced && S.on && !d.connected) return goOff(d.error, d.portFile);
  if (synced || S.cn) return;
  goOff(d.error, d.portFile);
};

app.ontoolinput = () => {};
app.ontoolresult = onResult;
app.onhostcontextchanged = syncHostCtx;
app.onteardown = () => { stopP(); stopOffP(); stopReveal?.(); stopReveal = null; return {}; };

mount();
sView();
if (boot?.connected) {
  if (S.nt) sUrl(""); else if (boot.page) sUrl(boot.page.url);
  uTabs(); uSt();
  if (!S.nt && boot.changed && isSafeImageUrl(boot.imageDataUrl)) paint(boot.imageDataUrl);
}

app.connect().then(() => {
  ready = 1;
  syncHostCtx(app.getHostContext());
  stopReveal = setupRevealObservers({ onReveal: forceNow, onHide, getLastFetchAt: () => lastFetchAt, bridgeApp: app });
  bootGo();
}).catch((err) => { setErr(L().iEr(errMsg(err))); });
