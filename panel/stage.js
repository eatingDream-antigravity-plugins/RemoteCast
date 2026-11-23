import { App, applyHostContext } from "./bridge.js";
import {
  AdaptiveBandwidthController,
  applyOptimisticTabClose,
  coalesceInputEvents,
  isTargetErrorText,
  shouldApplyFrameResponse,
} from "./net-policy.js";

const doc = document, boot = typeof window !== "undefined" && window.__CHROME_STREAM_BOOT__ ? window.__CHROME_STREAM_BOOT__ : null;
const $ = (i) => doc.getElementById(i), on = (el, ev, fn, o) => el?.addEventListener(ev, fn, o), ck = (el, fn) => on(el, "click", fn);
const sa = (el, k, v) => el?.setAttribute(k, String(v)), qa = (p, s) => p.querySelectorAll(s), pd = (e) => e.preventDefault();
const tx = (el, v) => { if (el && el.textContent !== v) el.textContent = v; };
const mk = (t, c) => { const e = doc.createElement(t); if (c) e.className = c; return e; };
const rnd = Math.round, now = () => performance.now(), cl = (v, lo, hi) => Math.max(lo, Math.min(hi, v));
const root = $("app"), app = new App({ name: "Chrome Stream", version: "0.1.0" }, {}), net = new AdaptiveBandwidthController("auto");
const TL = { ultra: "极速", low: "弱网", medium: "均衡", high: "高清" };
const QO = [["auto", "自适应"], ...["low", "medium", "high"].map((k) => [k, TL[k]])];
const KEYS = new Set("Enter Backspace Delete Tab Escape ArrowUp ArrowDown ArrowLeft ArrowRight PageUp PageDown Home End".split(" "));
const TACT = new Set("switch_tab new_tab close_tab navigate back forward reload".split(" "));

const S = {
  on: !!boot?.connected, pf: boot?.portFile || null,
  tabs: Array.isArray(boot?.tabs) ? boot.tabs : [], ts: "", th: boot?.tabsHash || "",
  aid: boot?.activeTargetId || null, ep: Number(boot?.epoch) || 1, pg: boot?.page || {},
  seq: boot?.seq || 0, fh: boot?.frameHash || "", by: boot?.imageBytes || 0,
  uc: false, qa: boot?.qualityApplied || "low", er: null,
  cn: false, sw: false, nv: false, qm: "auto", st: !boot?.paused, pa: !!boot?.paused,
  nt: !!(boot?.connected && (boot?.noTabs || (!boot?.activeTargetId && (!Array.isArray(boot?.tabs) || boot.tabs.length === 0)))),
  im: false, po: false,
};

let qIn = [], inB = 0, inT = 0, pB = 0, pT = 0, urg = 0, act = 0, tT = 0, pGen = 0, painted = 0, started = 0, synced = 0, ready = 0, tSig = "";
const closing = new Set(), tMap = new Map(), D = {};

const kb = (b) => (!b ? "0 KB" : b < 1024 ? `${b} B` : `${(b / 1024).toFixed(b < 10240 ? 1 : 0)} KB`);
const vp = () => [Number(S.pg.viewportWidth) || 1280, Number(S.pg.viewportHeight) || 800];
const qLbl = () => { const e = TL[S.qa || net.getEffectiveQuality()] || "均衡"; return S.qm === "auto" ? `自适应·${e}` : e; };
const safeImg = (u) => typeof u === "string" && u.startsWith("data:image/");
const sigOf = (d) => `${!!d?.connected}|${!!d?.portFile?.valid}|${Number(d?.portFile?.port) || 0}|${d?.activeTargetId || ""}|${d?.tabsHash || ""}|${d?.error || ""}`;
const canCall = () => { const c = app.getHostCapabilities(); return !c || c.serverTools !== false; };
const rpcSc = (name, a) => app.callServerTool({ name, arguments: a }).then((r) => r?.structuredContent);
const bH = (id, c, t, l = "", x = "") => `<button type=button class="${c ? `b ${c}` : "b"}" id=${id}${l ? ` aria-label="${l}" title="${l}"` : ""}${x}>${t}</button>`;
const evIn = (a, o, imm = true) => pushIn({ action: a, ...o }, imm);
const toast = (m) => { if (!D.ts || !m) return; tx(D.ts, m); sa(D.ts, "data-show", true); clearTimeout(tT); tT = setTimeout(() => sa(D.ts, "data-show", false), 1600); };
const ripple = (px, py) => { if (!D.vw) return; const d = mk("div", "rip"); d.style.left = `${px}%`; d.style.top = `${py}%`; D.vw.appendChild(d); setTimeout(() => d.remove(), 420); };

// Offscreen createImageBitmap/decode -> persistent <canvas> blit (zero blank flash on frames & tab switches).
function paint(url) {
  if (!safeImg(url) || !D.vc) return;
  const gen = ++pGen, im = new Image(), [vw, vh] = vp();
  im.decoding = "async";
  const blit = (src, nw, nh) => {
    if (gen !== pGen || !D.vc) { src.close?.(); return; }
    if (D.vc.width !== nw || D.vc.height !== nh) { D.vc.width = nw; D.vc.height = nh; }
    D.vc.getContext("2d", { alpha: false })?.drawImage(src, 0, 0, nw, nh);
    src.close?.(); painted = 1; D.vc.hidden = false; D.ph.hidden = true; setSw(false);
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
}

const setErr = (m) => { S.er = m || null; tx(D.em, S.er || ""); D.eb.hidden = !S.er; tx(D.om, S.er || ""); D.oe.hidden = !S.er; };
const sLoad = () => sa(D.lb, "data-on", !!(S.nv || S.sw));
const setSw = (v) => { S.sw = v; D.vc?.classList.toggle("sw", !!v); sLoad(); };
const sUrl = (u) => { if (D.ui && doc.activeElement !== D.ui) D.ui.value = u || ""; };
const setPop = (o) => { S.po = !!o; D.pp.hidden = !S.po; sa(D.bq, "aria-expanded", S.po); };
const setIme = (o) => { S.im = !!o; D.im.hidden = !S.im; sa(D.bi, "data-active", S.im); sa(D.bi, "aria-expanded", S.im); if (S.im) D.ti?.focus(); };
const setMeta = (m) => { if (m) { S.pg = { ...S.pg, title: m.title || "", url: m.url || "" }; sUrl(m.url); } };
const rstF = (th) => { S.seq = 0; S.fh = ""; if (th) S.th = ""; };
const goOff = (err, pf) => { S.on = false; S.pa = false; S.st = true; S.nt = false; S.er = err || null; if (pf) S.pf = pf; stopP(); sView(); };

function syncPh() {
  if (!D.ph || !D.pm) return;
  if (S.nt) {
    painted = 0;
    if (D.vc) { D.vc.hidden = true; D.vc.classList.remove("sw", "pa"); }
    tx(D.pm, "Chrome 当前无打开标签页");
    if (D.nt2) D.nt2.hidden = false;
    D.ph.hidden = false;
    if (D.pb) D.pb.hidden = true;
    return;
  }
  if (D.nt2) D.nt2.hidden = true;
  if (painted) {
    D.ph.hidden = true;
    if (D.vc) { D.vc.hidden = false; D.vc.classList.toggle("pa", !!S.pa); }
    if (D.pb) D.pb.hidden = !S.pa;
  } else {
    tx(D.pm, S.pa ? "实时推流已暂停（连接保持中）" : "正在连接画面…");
    D.ph.hidden = false;
    if (D.pb) D.pb.hidden = true;
  }
}

function syncPauseUi() {
  if (!D.bp) return;
  sa(D.bp, "data-active", S.pa);
  sa(D.bp, "aria-pressed", S.pa);
  tx(D.bp, S.pa ? "▶ 继续" : "⏸ 暂停");
  if (D.sd) { sa(D.sd, "data-paused", S.pa); D.sd.classList.toggle("live", !S.pa && !S.nt); }
  syncPh();
  uSt();
}

function sView() {
  D.vo.hidden = S.on; D.vn.hidden = !S.on;
  if (!S.on) {
    setPop(false); painted = 0;
    if (D.vc) { D.vc.hidden = true; D.vc.classList.remove("sw", "pa"); }
    if (D.ph) D.ph.hidden = false;
    if (D.pb) D.pb.hidden = true;
    uOff();
  } else {
    syncPauseUi();
  }
}

function uOff() {
  const ok = !!S.pf?.valid, port = Number(S.pf?.port) || 9222;
  sa(D.od, "data-on", ok); D.od.classList.toggle("live", ok);
  tx(D.op, ok ? `Port ${port} · 已就绪` : "未检测到调试端口");
  tx(D.om, S.er || ""); D.oe.hidden = !S.er; D.ob.disabled = !!S.cn;
  tx(D.ob, S.cn ? "等待 Chrome 授权（请点 Allow）…" : "连接本机 Chrome");
  if (!ok && !D.dt.dataset.u) D.dt.open = true;
}

// Mount static outer .shell ONCE synchronously at boot; never wipe root.innerHTML on polls/ontoolresult.
function mount() {
  const ok = !!S.pf?.valid, li = (n, h) => `<li><span class=n>${n}</span><span>${h}</span></li>`;
  const pr = (a, i) => `<div class=pr><span>${a}</span><span class=num id=${i}>-</span></div>`;
  const qb = QO.map(([v, l]) => `<button type=button role=radio data-q=${v} aria-checked=${v === S.qm}>${l}</button>`).join("");
  const ik = "Enter:Enter Escape:Esc Tab:Tab Backspace:退格 Copy:⌘C Paste:⌘V SelectAll:全选".split(" ").map((s) => { const [a, b] = s.split(":"); return `<button type=button class=ch data-k=${a}>${b}</button>`; }).join("");
  root.innerHTML =
    `<div class=shell id=sh><div class=v id=vo${S.on ? " hidden" : ""}><section class=sh><div class=sh-h><div class=sh-t><span class=bd></span><h1>Chrome Stream</h1></div>` +
    `<span class="tag num"><span class=dot id=od></span><span id=op></span></span></div>` +
    `<p class=sub>原生 Rust 持久化 CDP 中继，弱网自适应画质与 SHA-1 零流量去重，即点即控。</p>` +
    `<div class=eb id=oe hidden><span id=om></span>${bH("or", "bg", "重试")}</div><div class=row>${bH("ob", "bp", "")}</div>` +
    `<details class=det id=dt${ok ? "" : " open"}><summary>首次连接指引（3 步）</summary><ol class=ol>` +
    `${li("01", "保持本机 Google Chrome 运行")}${li("02", "在 <span class=m>chrome://inspect/#remote-debugging</span> 开启远程调试")}${li("03", "首次点击「Allow」，0600 中继保活不再弹窗")}</ol></details>` +
    `<div class=sb><span>0600 Unix Socket · 12s 自动停流</span><span class=m>Local Loopback</span></div></section></div>` +
    `<div class=v id=vn${S.on ? "" : " hidden"}><header class=hd><div class=bm><span class=bd></span><span class=bn>Chrome Stream</span></div>` +
    `<div class=tb id=tb role=tablist aria-label=标签页>${bH("nt", "bg", "+", "新建标签页")}</div>` +
    `<div class=ha>${bH("bp", "bg", "⏸ 暂停", "暂停/继续实时推流（保持连接不弹窗）", " aria-pressed=false")}${bH("dc", "bg bdg", "断开", "关闭 Chrome 连接以省电")}</div></header>` +
    `<div class=row><div class=pg>${bH("bk", "bg", "←", "后退")}${bH("fw", "bg", "→", "前进")}${bH("rl", "bg", "↻", "刷新")}</div>` +
    `<form class=uf id=uf><input class=inp id=ui type=text aria-label=网址或搜索 placeholder="输入网址或搜索（⌘L）"/></form>` +
    `<div class=pw>${bH("bq", "num", `<span class="dot live" id=sd data-on=true></span><span id=ql>自适应</span>`, "画质与网络", " aria-haspopup=dialog aria-expanded=false")}` +
    `<div class=pop id=pp role=dialog aria-label=画质与网络 hidden><div class=pl>画质档位（←/→ 切换）</div><div class=seg id=qs role=radiogroup>${qb}</div>` +
    `${pr("流量与帧", "sr")}${pr("视口", "sv")}<div class=pr>${bH("sn", "", "抓一帧")}${bH("dc2", "bdg", "断开连接")}</div></div></div>` +
    `${bH("bi", "", "⌨", "快捷输入与 IME", " aria-expanded=false")}</div>` +
    `<div class=eb id=eb hidden><span id=em></span>${bH("er", "bg", "重试")}</div>` +
    `<div class=st id=st tabindex=0 aria-label="Chrome 实时画面"><div class=lb id=lb></div>` +
    `<div class=ph id=ph><span id=pm>正在连接画面…</span>${bH("nt2", "bp", "新建标签页", "", " hidden")}</div>` +
    `<div class=pbg id=pb hidden>已暂停推流 · 点击「继续」恢复</div>` +
    `<div class=vw id=vw><canvas class=vc id=vc width=960 height=600 aria-label="Chrome 视口" hidden></canvas></div><div class=tst id=ts role=status aria-live=polite></div></div>` +
    `<div class=ime id=im hidden><div class=row><input class=inp id=ti type=text placeholder="中文输入或粘贴，回车发送…"/>${bH("ty", "", "输入")}${bH("te", "bp", "发送 ↵")}</div>` +
    `<div class=ik>${ik}<button type=button class=ch data-sc=-360 aria-label=上滚>↑</button><button type=button class=ch data-sc=360 aria-label=下滚>↓</button></div></div></div></div>`;

  for (const id of "sh vo vn od op oe om ob dt tb nt nt2 bp dc uf ui bq ql pp qs bi im eb em st lb ph pm pb vw vc ts ti sd sr sv".split(" ")) D[id] = $(id);

  const sendTx = (pe) => { const t = D.ti.value; if (!t && !pe) return; D.ti.value = ""; toast("✓ 已发送"); evIn("type", { text: t, pressEnter: pe }); };
  const openNewTab = () => { S.nt = false; S.ep++; rstF(1); setSw(true); evIn("new_tab", { url: "about:blank" }); };
  const doDisc = () => {
    setPop(false); S.ep++; qIn = []; S.tabs = []; S.aid = null; rstF(1);
    goOff(null);
    toast("已关闭连接");
    if (canCall()) rpcSc("chrome_stream_disconnect", {}).then((d) => { if (d?.portFile) { S.pf = d.portFile; uOff(); } }, () => {});
  };
  const togglePause = () => {
    if (!S.on) return;
    S.pa = !S.pa; S.st = !S.pa; S.ep++;
    syncPauseUi();
    if (S.pa) {
      stopP();
      toast("已暂停推流（连接保持中）");
      if (canCall()) rpcSc("chrome_stream_input", { events: [{ action: "pause" }] }).catch(() => {});
    } else {
      toast("✓ 已继续实时推流");
      if (canCall()) rpcSc("chrome_stream_input", { events: [{ action: "resume" }] }).finally(() => { if (S.on && !S.pa) askUrg(true); });
    }
  };
  for (const [id, fn] of [
    ["ob", () => conn(false)], ["or", () => conn(true)],
    ["nt", openNewTab], ["nt2", openNewTab],
    ["er", () => { setErr(null); askUrg(true); }],
    ["sn", () => { S.fh = ""; setPop(false); toast("↻ 抓取画面"); poll(true); }],
    ["dc", doDisc], ["dc2", doDisc],
    ["bp", togglePause],
    ["bi", () => setIme(!S.im)], ["ty", () => sendTx(false)], ["te", () => sendTx(true)],
    ["bq", (e) => { e.stopPropagation(); setPop(!S.po); }],
  ]) ck($(id), fn);
  for (const [id, a, m] of [["bk", "back", "← 后退"], ["fw", "forward", "→ 前进"], ["rl", "reload", "↻ 刷新中"]]) ck($(id), () => { S.nv = true; sLoad(); toast(m); evIn(a); });
  on(D.dt, "toggle", () => { D.dt.dataset.u = "1"; });
  on(D.ui, "focus", () => D.ui.select());
  on(D.ui, "keydown", (e) => { if (e.key === "Escape") { pd(e); D.ui.value = S.pg.url || ""; D.st.focus(); } });
  on(D.uf, "submit", (e) => { pd(e); const u = D.ui.value.trim(); if (!u) return; S.nv = true; S.pg = { ...S.pg, url: u }; sLoad(); evIn("navigate", { url: u }); D.st.focus(); });

  const setQ = (v) => {
    S.qm = v; net.setMode(v); S.qa = net.getEffectiveQuality(); S.fh = "";
    qa(D.qs, "[data-q]").forEach((el) => sa(el, "aria-checked", el.dataset.q === v));
    uSt(); toast(`✓ ${qLbl()}`); askUrg(true);
  };
  qa(D.qs, "[data-q]").forEach((el) => ck(el, () => setQ(el.dataset.q)));
  on(D.qs, "keydown", (e) => {
    if (e.key !== "ArrowLeft" && e.key !== "ArrowRight") return;
    pd(e); const i = QO.findIndex(([v]) => v === S.qm), nx = QO[(i + (e.key === "ArrowRight" ? 1 : QO.length - 1)) % QO.length][0];
    setQ(nx); D.qs.querySelector(`[data-q="${nx}"]`)?.focus();
  });

  on(D.ti, "keydown", (e) => { if (e.key === "Enter" && !e.isComposing) { pd(e); sendTx(true); } else if (e.key === "Escape") { pd(e); setIme(false); D.st.focus(); } });
  qa(D.im, "[data-k]").forEach((el) => ck(el, () => { toast(`✓ ${el.textContent}`); evIn("key", { key: el.dataset.k }); }));
  qa(D.im, "[data-sc]").forEach((el) => ck(el, () => { const [w, h] = vp(); evIn("scroll", { x: rnd(w / 2), y: rnd(h / 2), deltaY: Number(el.dataset.sc) || 360 }); }));

  const ptOf = (e) => {
    const r = D.vc.getBoundingClientRect(); if (!r.width || !r.height) return null;
    const rx = cl((e.clientX - r.left) / r.width, 0, 1), ry = cl((e.clientY - r.top) / r.height, 0, 1), [w, h] = vp();
    return { x: rnd(rx * (S.pg.viewportWidth || D.vc.width || w)), y: rnd(ry * (S.pg.viewportHeight || D.vc.height || h)), px: +(rx * 100).toFixed(2), py: +(ry * 100).toFixed(2) };
  };
  for (const [ev, button, clickCount, pv] of [["click", "left", 1, 0], ["dblclick", "left", 2, 1], ["contextmenu", "right", 1, 1]]) {
    on(D.vc, ev, (e) => {
      if (pv) pd(e); if (S.po) setPop(false);
      const p = ptOf(e); if (!p) return;
      if (clickCount === 1) D.st.focus({ preventScroll: true });
      ripple(p.px, p.py); evIn("click", { x: p.x, y: p.y, button, clickCount });
    });
  }

  let wx = 0, wy = 0, wp = null, wt = 0;
  on(D.vc, "wheel", (e) => {
    pd(e); wx += e.deltaX; wy += e.deltaY; wp = ptOf(e) || wp;
    if (wt) return;
    wt = setTimeout(() => {
      const dx = cl(rnd(wx), -1200, 1200), dy = cl(rnd(wy), -1200, 1200), p = wp || { x: 500, y: 350 };
      wx = wy = wt = 0;
      if (Math.abs(dx) >= 12 || Math.abs(dy) >= 12) evIn("scroll", { x: p.x, y: p.y, deltaX: dx, deltaY: dy });
    }, net.getWheelCoalesceMs());
  }, { passive: false });

  on(D.st, "keydown", (e) => {
    if (e.isComposing) return;
    if (e.metaKey || e.ctrlKey) {
      const m = { a: ["SelectAll"], c: ["Copy", "✓ 复制"], v: ["Paste", "✓ 粘贴"] }[e.key.toLowerCase()];
      if (m) { pd(e); if (m[1]) toast(m[1]); evIn("key", { key: m[0] }); }
      return;
    }
    if (e.altKey) return;
    if (KEYS.has(e.key)) { pd(e); evIn("key", { key: e.key }, e.key === "Enter" || e.key === "Escape"); }
    else if (e.key.length === 1) { pd(e); evIn("type", { text: e.key, pressEnter: false }, false); }
  });
}

function swTab(tid) {
  if (!tid || tid === S.aid) return;
  S.aid = tid; S.ep++; rstF(); setMeta(S.tabs.find((t) => t.targetId === tid));
  setSw(true); uTabs(); uSt(); evIn("switch_tab", { targetId: tid });
}

function clTab(tid) {
  if (!tid || closing.has(tid)) return;
  closing.add(tid);
  const o = applyOptimisticTabClose(S.tabs, S.aid, tid);
  S.tabs = o.tabs; S.th = "";
  if (o.wasActive) { S.aid = o.activeTargetId; S.ep++; rstF(); setSw(true); setMeta(S.tabs.find((t) => t.targetId === S.aid)); }
  uTabs(); uSt(); toast("✓ 已关闭标签页"); evIn("close_tab", { targetId: tid });
}

function mkTab(tid) {
  const el = mk("div", "tp"), sp = mk("span", "tt"), x = mk("button", "tc");
  el.tabIndex = 0; sa(el, "role", "tab"); el.dataset.id = tid;
  x.type = "button"; x.textContent = "×"; sa(x, "aria-label", "关闭");
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
}

// Keyed in-place tab reconciliation by `data-id` (zero innerHTML rebuilds).
function uTabs() {
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
    sa(el, "data-active", a); sa(el, "aria-selected", a);
    if (el.title !== (t.url || "")) el.title = t.url || "";
    tx(el.firstChild, t.title || t.url || "Tab");
    if (cur !== el) D.tb.insertBefore(el, cur || D.nt);
    cur = el.nextSibling;
  }
}

function uSt() {
  if (!D.ql) return;
  const rtt = net.lastNetRttMs || net.ewmaRttMs, pend = (inB ? 1 : 0) + qIn.length, [w, h] = vp();
  sa(D.sd, "data-net", net.getNetworkGrade());
  sa(D.sd, "data-paused", S.pa);
  tx(D.ql, S.pa ? "已暂停" : S.nt ? "无标签页" : rtt ? `${qLbl()} · ${rtt}ms` : qLbl());
  tx(D.sr, `${S.uc ? "复用 0 KB" : kb(S.by)} · #${S.seq}${pend ? ` · 队列 ${pend}` : ""}`);
  tx(D.sv, `${w}×${h}`);
  sLoad();
}

const recover = (m) => { S.aid = null; rstF(1); if (m) toast(m); askUrg(true); };

function onPoll(d, totalMs, reqEpoch = S.ep) {
  if (!d) return;
  if (totalMs > 0) synced = 1;
  if (d.portFile) S.pf = d.portFile;
  if (d.connected === false) return (d.recoverable || isTargetErrorText(d.error)) ? recover("标签页已关闭，自动切换中…") : goOff(d.error);
  if (totalMs > 0) {
    const s = net.recordSample({ totalMs, serverMs: d.serverMs || 0, imageBytes: d.changed ? d.imageBytes || 0 : 0 });
    S.qa = d.qualityApplied || (s.tierChanged ? s.effectiveTier : S.qa);
  } else if (d.qualityApplied) S.qa = d.qualityApplied;

  if (!shouldApplyFrameResponse({ reqEpoch, currentEpoch: S.ep, currentTargetId: S.aid, payload: d }).apply) {
    if (S.on && S.st && !S.pa) askUrg(false);
    return;
  }
  S.on = true;
  if (typeof d.paused === "boolean" && totalMs === 0) { S.pa = d.paused; S.st = !d.paused; }
  const noTabs = !!(d.noTabs || (Array.isArray(d.tabs) && d.tabs.length === 0 && !d.activeTargetId));
  S.nt = noTabs;
  if (noTabs) { S.aid = null; S.tabs = []; S.pg = {}; sUrl(""); rstF(); }
  sView();
  if (d.recovered) { setErr(null); toast("✓ 已自动切换至存活标签页"); } else setErr(d.error || null);
  S.nv = false;
  if (Array.isArray(d.tabs)) S.tabs = d.tabs;
  if (d.tabsHash) S.th = d.tabsHash;
  if (d.activeTargetId) S.aid = d.activeTargetId;
  if (d.page && !noTabs) { S.pg = d.page; sUrl(d.page.url); }
  uTabs();
  if (!noTabs && d.changed && safeImg(d.imageDataUrl)) {
    S.seq = d.seq || S.seq + 1; S.fh = d.frameHash || S.fh; S.by = d.imageBytes || S.by; S.uc = false;
    paint(d.imageDataUrl);
  } else {
    S.uc = !!(d.unchanged || !d.changed);
    if (d.seq) S.seq = Math.max(S.seq, d.seq);
    if (d.frameHash) S.fh = d.frameHash;
    if (S.sw && painted && S.fh) setSw(false);
    syncPh();
  }
  uSt();
}

function pushIn(ev, imm = true) {
  act = Date.now(); qIn.push(ev); uSt();
  if (inB) return;
  const ms = imm ? 0 : net.getTypeBatchMs();
  if (ms <= 0) { clearTimeout(inT); inT = 0; flIn(); }
  else if (!inT) inT = setTimeout(() => { inT = 0; flIn(); }, ms);
}

async function flIn() {
  if (inB || !qIn.length || !canCall()) return;
  inB = 1;
  const batch = coalesceInputEvents(qIn); qIn = [];
  const hadTab = batch.some((e) => TACT.has(e.action)), closed = batch.filter((e) => e.action === "close_tab").map((e) => e.targetId), hadDisc = batch.some((e) => e.action === "disconnect");
  const ep = S.ep, t0 = now(); uSt();
  try {
    const d = await rpcSc("chrome_stream_input", { targetId: S.aid || undefined, events: batch });
    synced = 1;
    if (!hadDisc) net.recordSample({ totalMs: rnd(now() - t0), serverMs: 8, imageBytes: 0 });
    if (hadDisc || (d?.connected === false && !d?.recoverable && !isTargetErrorText(d?.error))) return goOff(d?.error, d?.portFile);
    if (d?.error && !d?.recovered && !isTargetErrorText(d.error)) setErr(d.error);
    else if (d?.recovered) { setErr(null); toast("✓ 已自动恢复标签页"); }
    if (ep === S.ep) {
      const noTabs = !!(d?.noTabs || (Array.isArray(d?.tabs) && d.tabs.length === 0 && !d?.activeTargetId));
      S.nt = noTabs;
      if (noTabs) { S.aid = null; S.tabs = []; S.th = d?.tabsHash || ""; S.pg = {}; sUrl(""); rstF(); uTabs(); syncPh(); }
      else {
        if (d?.activeTargetId && d.activeTargetId !== S.aid) { S.aid = d.activeTargetId; rstF(); }
        if (Array.isArray(d?.tabs)) { S.tabs = d.tabs; S.th = d.tabsHash || ""; setMeta(S.tabs.find((t) => t.targetId === S.aid)); uTabs(); }
      }
    }
    if ((hadTab || d?.recovered) && !S.nt && !S.pa) askUrg(false); else if (!S.st && !S.pa && !S.nt) sched(90, false);
  } catch (err) {
    isTargetErrorText(err?.message) ? recover() : setErr(err?.message || String(err));
  } finally {
    for (const id of closed) closing.delete(id);
    inB = 0; uSt(); if (qIn.length) flIn();
  }
}

const stopP = () => { clearTimeout(pT); pT = 0; };
const sched = (ms, f = false) => { stopP(); if (S.on && (!S.pa || f)) pT = setTimeout(() => { pT = 0; poll(f); }, ms); };
const askUrg = (f = false) => { if (!S.on || (S.pa && !f)) return; if (pB) urg = { f: !!(f || urg?.f) }; else sched(20, f); };
const timing = (idleMs, inputBusy = false) => net.computePollTiming({ idleMs, inputBusy });
const reqFrame = (f, w, keep = true) => rpcSc("chrome_stream_frame", { targetId: S.aid || undefined, sinceSeq: !f && keep ? S.seq : 0, lastFrameHash: S.fh || undefined, knownTabsHash: S.th || undefined, qualityPreset: net.getEffectiveQuality(), rttMs: net.ewmaRttMs || 0, waitMs: f ? 0 : w, forceRefresh: f });

async function poll(f = false) {
  if (pB || !canCall() || (!f && S.pa)) { if (f && pB) urg = { f: true }; return; }
  if (doc.visibilityState === "hidden" && !f) return sched(1500, false);
  pB = 1;
  const ep = S.ep, t0 = now(), tm = timing(Date.now() - act);
  try {
    onPoll(await reqFrame(f, S.nt ? 1000 : tm.waitMs), rnd(now() - t0), ep);
  } catch (err) {
    if (isTargetErrorText(err?.message)) { S.aid = null; rstF(1); urg = { f: true }; }
    else setErr(err?.message || String(err));
  } finally {
    pB = 0;
    if (S.on && !S.pa) {
      if (urg) { const u = urg; urg = 0; sched(20, u.f); }
      else if (S.st) sched(S.nt ? 1200 : timing(Date.now() - act, !!(inB || qIn.length)).nextDelayMs, false);
    }
  }
}

async function conn(f = false) {
  if (S.cn || !canCall()) return;
  S.cn = true; S.er = null; if (!S.on) uOff();
  const ep = S.ep, t0 = now();
  try {
    const d = await reqFrame(f, 0, S.on && S.seq > 0);
    S.cn = false; onPoll(d, rnd(now() - t0), ep);
    if (S.on && S.st && !S.pa) sched(S.nt ? 1200 : timing(0).nextDelayMs, false);
  } catch (err) {
    S.cn = false;
    if (S.on && isTargetErrorText(err?.message)) return recover();
    goOff(err?.message || String(err));
  }
}

ck(doc, (e) => { if (S.po && !e.target.closest?.(".pw")) setPop(false); });
on(doc, "keydown", (e) => {
  if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "l" && D.ui && S.on) { pd(e); D.ui.focus(); D.ui.select(); }
  else if (e.key === "Escape" && S.po) { pd(e); setPop(false); D.bq?.focus(); }
});
on(doc, "visibilitychange", () => { if (doc.visibilityState === "visible" && S.on && S.st && !S.pa) sched(80, false); });
// Only auto-start polling if relay is ALREADY connected; NEVER auto-dial Chrome when disconnected (avoids Allow popup & focus switch).
const bootGo = () => { if (!started && ready && S.on) { started = 1; if (!S.pa) conn(false); } };

function onResult(r) {
  const d = r?.structuredContent;
  if (!d) return;
  const sig = sigOf(d);
  if (sig === tSig) { if (d.connected) bootGo(); return; }
  tSig = sig;
  if (d.portFile) S.pf = d.portFile;
  if (typeof d.paused === "boolean" && !synced) { S.pa = d.paused; S.st = !d.paused; }
  if (d.activeTargetId && !synced) S.aid = d.activeTargetId;
  if (d.connected) { S.on = true; sView(); uTabs(); uSt(); bootGo(); return; }
  if (synced || S.cn) return;
  goOff(d.error);
}

app.ontoolinput = () => {};
app.ontoolresult = onResult;
app.ontoolcancelled = () => { S.cn = false; uOff(); };
app.onhostcontextchanged = applyHostContext;

mount();
if (boot) tSig = sigOf(boot);
boot?.connected ? onPoll(boot, 0, S.ep) : sView();

(async () => {
  try {
    await app.connect(); ready = 1; applyHostContext(app.getHostContext()); uSt();
    if (S.on) bootGo();
  } catch (err) { goOff(`初始化失败：${err?.message || err}`); }
})();
