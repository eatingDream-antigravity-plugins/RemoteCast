// Lightweight SEP-1865 (2026-01-26) MCP Apps PostMessage bridge.
export const LATEST_PROTOCOL_VERSION = "2026-01-26";
const de = () => document.documentElement, HX = /^#([0-9a-f]{3}|[0-9a-f]{4}|[0-9a-f]{6}|[0-9a-f]{8})$/i, UN = "ui/notifications/";
const px = (h, i) => parseInt(h.length <= 4 ? h[i] + h[i] : h.slice(i * 2, i * 2 + 2), 16);

const parseAlpha = (v) => {
  const s = String(v || "").trim().toLowerCase();
  if (/^(|transparent|none|initial|inherit|unset)$/.test(s)) return 0;
  const hx = s.match(HX);
  if (hx) return hx[1].length % 4 === 0 ? px(hx[1], 3) / 255 : 1;
  const fn = s.match(/^(?:rgba?|hsla?)\([^,)]+,[^,)]+,[^,)]+,\s*(\d*\.?\d+%?)\s*\)$/) || s.match(/\/\s*(\d*\.?\d+%?)\s*\)$/);
  if (!fn) return 1;
  const a = parseFloat(fn[1]) / (fn[1].endsWith("%") ? 100 : 1);
  return Number.isFinite(a) ? Math.max(0, Math.min(1, a)) : 0;
};

export const lum = (raw) => {
  const s = String(raw || "").trim();
  if (parseAlpha(s) < 0.5) return null;
  const m = s.match(HX), r = m ? [0, 1, 2].map((i) => px(m[1], i)) : s.match(/^rgba?\(\s*(\d*\.?\d+)[\s,]+(\d*\.?\d+)[\s,]+(\d*\.?\d+)/i)?.slice(1);
  return r ? (0.2126 * +r[0] + 0.7152 * +r[1] + 0.0722 * +r[2]) / 255 : null;
};

export const applyDocumentTheme = (t) => {
  if (t !== "dark" && t !== "light") return;
  const e = de(), bg = `var(--color-background-primary,${t === "light" ? "#FAF8F4" : "#141311"})`;
  e.setAttribute("data-theme", t); e.style.colorScheme = t; e.style.backgroundColor = bg;
  document.body && (document.body.style.backgroundColor = bg);
  document.querySelector?.('meta[name="color-scheme"]')?.setAttribute("content", t);
};

export const applyHostStyleVariables = (v, el = de()) => {
  if (!v || typeof v !== "object") return;
  for (const k in v) if (v[k] != null) {
    const val = String(v[k]).trim();
    if (k !== "--color-background-primary" || (val && parseAlpha(val) >= 0.9)) el.style.setProperty(k, val);
  }
};

export const applyHostFonts = (css) => {
  if (!css || document.getElementById("__mcp-hf")) return;
  const s = document.createElement("style"); s.id = "__mcp-hf"; s.textContent = css; document.head.appendChild(s);
};

export const applyHostContext = (c) => {
  if (!c || typeof c !== "object") return;
  const st = c.styles, vs = st?.variables;
  vs && applyHostStyleVariables(vs);
  st?.css?.fonts && applyHostFonts(st.css.fonts);
  const raw = String(c.theme || "").trim().toLowerCase();
  let t = /dark|night|black/.test(raw) ? "dark" : /light|day|white/.test(raw) ? "light" : null;
  if (!t) {
    const l = lum(vs?.["--color-background-primary"] || (typeof getComputedStyle === "function" ? getComputedStyle(de()).getPropertyValue("--color-background-primary") : ""));
    if (l !== null) t = l < 0.45 ? "dark" : "light";
  }
  t && applyDocumentTheme(t);
};

export class App {
  constructor(i, c = {}, o = {}) {
    const s = this; s._i = i; s._c = c || {}; s._o = { autoResize: true, ...o }; s._n = 1; s._p = new Map(); s._up = false; s._ci = null; s._t = window.parent;
    s._m = (e) => {
      if (e.source !== s._t) return;
      const m = e.data; if (!m || typeof m !== "object" || m.jsonrpc !== "2.0") return;
      if (m.id !== undefined && !m.method) {
        const p = s._p.get(m.id); if (!p) return;
        s._p.delete(m.id); globalThis.clearTimeout(p.t);
        m.error ? p.rej(new Error(m.error.message || JSON.stringify(m.error))) : p.res(m.result);
      } else m.id !== undefined ? s._req(m) : m.method && s._note(m);
    };
  }
  getHostContext() { return this._ctx; }
  getHostCapabilities() { return this._hc; }
  _send(p) { this._t.postMessage({ jsonrpc: "2.0", ...p }, "*"); }
  async _req(m) {
    const id = m.id, k = m.method;
    try {
      const r = k === "ping" ? {} : k === "ui/resource-teardown" ? (await this.onteardown?.(m.params || {})) ?? {} : null;
      this._send(r ? { id, result: r } : { id, error: { code: -32601, message: k } });
    } catch (e) { this._send({ id, error: { code: -32603, message: e?.message || String(e) } }); }
  }
  _note(m) {
    const s = this, p = m.params || {}, k = m.method;
    if (k === UN + "tool-input") s.ontoolinput?.(p);
    else if (k === UN + "tool-result") s.ontoolresult?.(p);
    else if (k === UN + "tool-cancelled") s.ontoolcancelled?.(p);
    else if (k === UN + "host-context-changed") { s._ctx = { ...s._ctx, ...p }; s.onhostcontextchanged?.(p); }
    else if (k === "notifications/progress") s._p.forEach((e) => e.reset());
  }
  request(method, params, { timeoutMs: tm = 75000 } = {}) {
    const s = this, id = s._n++;
    return new Promise((res, rej) => {
      const arm = () => globalThis.setTimeout(() => { s._p.delete(id); rej(new Error(`Timeout:${method}`)); }, tm);
      const e = { res, rej, t: arm(), reset() { globalThis.clearTimeout(e.t); e.t = arm(); } };
      s._p.set(id, e); s._send({ id, method, params });
    });
  }
  notification(method, params) { this._send(params !== undefined ? { method, params } : { method }); }
  callServerTool(params, o = {}) { return this.request("tools/call", params, { timeoutMs: o.timeout ?? 75000 }); }
  _watchSize() {
    const s = this; if (typeof ResizeObserver === "undefined" || !s._ctx?.containerDimensions?.maxHeight) return;
    let raf = 0, lw = 0, lh = 0;
    const go = () => !raf && (raf = 1, requestAnimationFrame(() => {
      raf = 0; const r = de(), pv = r.style.height; r.style.height = "max-content";
      const h = Math.ceil(r.getBoundingClientRect().height), w = Math.ceil(window.innerWidth);
      r.style.height = pv;
      if (w !== lw || h !== lh) { lw = w; lh = h; s.notification(UN + "size-changed", { width: w, height: h }); }
    }));
    go(); (s._ro = new ResizeObserver(go)).observe(de());
  }
  connect() {
    const s = this; if (s._up) return s._ci || Promise.resolve();
    s._up = true; window.addEventListener("message", s._m);
    return (s._ci = s.request("ui/initialize", { appInfo: s._i, appCapabilities: s._c, protocolVersion: LATEST_PROTOCOL_VERSION }, { timeoutMs: 20000 }).then((r) => {
      if (!r || typeof r !== "object" || r.protocolVersion !== LATEST_PROTOCOL_VERSION) throw new Error("Bad init");
      s._ctx = r.hostContext; s._hc = r.hostCapabilities || {};
      applyHostContext(s._ctx); s.notification(UN + "initialized");
      s._o.autoResize && s._watchSize();
    }).catch((e) => { s.close(); throw e; }));
  }
  close() {
    const s = this; s._up = false; s._ci = s._hc = null; window.removeEventListener("message", s._m);
    s._ro?.disconnect(); s._ro = null;
    s._p.forEach((e) => { globalThis.clearTimeout(e.t); e.rej(new Error("Closed")); });
    s._p.clear();
  }
}
