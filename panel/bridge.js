// Lightweight SEP-1865 (2026-01-26) MCP Apps PostMessage bridge.
export const LATEST_PROTOCOL_VERSION = "2026-01-26";
const de = () => document.documentElement;

function parseAlpha(v) {
  const s = String(v || "").trim().toLowerCase();
  if (!s || s === "transparent" || s === "none" || s === "initial" || s === "inherit" || s === "unset") return 0;
  const hx = s.match(/^#([0-9a-f]{3}|[0-9a-f]{4}|[0-9a-f]{6}|[0-9a-f]{8})$/i);
  if (hx) {
    const h = hx[1];
    if (h.length === 4) return parseInt(h[3] + h[3], 16) / 255;
    if (h.length === 8) return parseInt(h.slice(6, 8), 16) / 255;
    return 1;
  }
  const fn = s.match(/^(?:rgba?|hsla?)\([^,)]+,[^,)]+,[^,)]+,\s*(\d*\.?\d+%?)\s*\)$/) ||
    s.match(/\/\s*(\d*\.?\d+%?)\s*\)$/);
  if (fn) {
    const a = fn[1].endsWith("%") ? parseFloat(fn[1]) / 100 : parseFloat(fn[1]);
    return Number.isFinite(a) ? Math.max(0, Math.min(1, a)) : 0;
  }
  return 1;
}

export function lum(raw) {
  const s = String(raw || "").trim();
  if (parseAlpha(s) < 0.5) return null;
  const m = s.match(/^#([0-9a-f]{3}|[0-9a-f]{4}|[0-9a-f]{6}|[0-9a-f]{8})$/i);
  if (m) {
    const h = m[1], p = h.length <= 4 ? [h[0]+h[0], h[1]+h[1], h[2]+h[2]] : [h.slice(0,2), h.slice(2,4), h.slice(4,6)];
    return (0.2126 * parseInt(p[0], 16) + 0.7152 * parseInt(p[1], 16) + 0.0722 * parseInt(p[2], 16)) / 255;
  }
  const r = s.match(/^rgba?\(\s*(\d*\.?\d+)[\s,]+(\d*\.?\d+)[\s,]+(\d*\.?\d+)/i);
  return r ? (0.2126 * +r[1] + 0.7152 * +r[2] + 0.0722 * +r[3]) / 255 : null;
}

export function applyDocumentTheme(t) {
  if (t !== "dark" && t !== "light") return;
  const e = de(), bg = t === "light" ? "var(--color-background-primary,#FAF8F5)" : "var(--color-background-primary,#141311)";
  if (e.getAttribute("data-theme") !== t) e.setAttribute("data-theme", t);
  e.style.colorScheme = t; e.style.backgroundColor = bg;
  if (document.body) document.body.style.backgroundColor = bg;
  const m = document.querySelector?.('meta[name="color-scheme"]');
  if (m && m.getAttribute("content") !== t) m.setAttribute("content", t);
}

export function applyHostStyleVariables(v, el = de()) {
  if (v && typeof v === "object") {
    for (const [k, x] of Object.entries(v)) {
      if (x == null) continue;
      const val = String(x).trim();
      if (k === "--color-background-primary" && (!val || parseAlpha(val) < 0.9)) continue;
      el.style.setProperty(k, val);
    }
  }
}

export function applyHostFonts(css) {
  if (!css || document.getElementById("__mcp-hf")) return;
  const s = document.createElement("style"); s.id = "__mcp-hf"; s.textContent = css; document.head.appendChild(s);
}

export function applyHostContext(c) {
  if (!c || typeof c !== "object") return;
  if (c.styles?.variables) applyHostStyleVariables(c.styles.variables);
  if (c.styles?.css?.fonts) applyHostFonts(c.styles.css.fonts);
  const raw = String(c.theme || "").trim().toLowerCase();
  let t = raw.includes("dark") || raw.includes("night") || raw.includes("black") ? "dark"
    : raw.includes("light") || raw.includes("day") || raw.includes("white") ? "light" : null;
  if (!t) {
    const bg = c.styles?.variables?.["--color-background-primary"] ||
      (typeof getComputedStyle === "function" ? getComputedStyle(de()).getPropertyValue("--color-background-primary") : "");
    const l = lum(bg);
    if (l !== null) t = l < 0.45 ? "dark" : "light";
  }
  if (t) applyDocumentTheme(t);
}

export class App {
  constructor(i, c = {}, o = {}) {
    this._i = i; this._c = c || {}; this._o = { autoResize: true, ...o };
    this._n = 1; this._p = new Map(); this._up = false; this._t = window.parent;
    this._m = (e) => {
      if (e.source !== this._t) return;
      const m = e.data;
      if (!m || typeof m !== "object" || m.jsonrpc !== "2.0") return;
      if (m.id !== undefined && !m.method) {
        const p = this._p.get(m.id);
        if (!p) return;
        this._p.delete(m.id); clearTimeout(p.t);
        m.error ? p.rej(new Error(m.error.message || JSON.stringify(m.error))) : p.res(m.result);
      } else if (m.id !== undefined) this._req(m);
      else if (m.method) this._note(m);
    };
  }
  getHostContext() { return this._ctx; }
  getHostCapabilities() { return this._hc; }
  _send(p) { this._t.postMessage(p, "*"); }
  async _req(m) {
    const id = m.id, k = m.method;
    try {
      if (k === "ping") return this._send({ jsonrpc: "2.0", id, result: {} });
      if (k === "ui/resource-teardown") return this._send({ jsonrpc: "2.0", id, result: this.onteardown ? (await this.onteardown(m.params || {})) ?? {} : {} });
      this._send({ jsonrpc: "2.0", id, error: { code: -32601, message: k } });
    } catch (e) { this._send({ jsonrpc: "2.0", id, error: { code: -32603, message: e?.message || String(e) } }); }
  }
  _note(m) {
    const p = m.params || {}, k = m.method;
    if (k === "ui/notifications/tool-input") this.ontoolinput?.(p);
    else if (k === "ui/notifications/tool-result") this.ontoolresult?.(p);
    else if (k === "ui/notifications/tool-cancelled") this.ontoolcancelled?.(p);
    else if (k === "ui/notifications/host-context-changed") { this._ctx = { ...this._ctx, ...p }; this.onhostcontextchanged?.(p); }
    else if (k === "notifications/progress") for (const e of this._p.values()) e.reset();
  }
  request(method, params, { timeoutMs = 75000 } = {}) {
    const id = this._n++;
    return new Promise((res, rej) => {
      const arm = () => setTimeout(() => { this._p.delete(id); rej(new Error(`Timeout:${method}`)); }, timeoutMs);
      const e = { res, rej, t: arm(), reset() { clearTimeout(e.t); e.t = arm(); } };
      this._p.set(id, e);
      this._send({ jsonrpc: "2.0", id, method, params });
    });
  }
  notification(method, params) { this._send(params !== undefined ? { jsonrpc: "2.0", method, params } : { jsonrpc: "2.0", method }); }
  callServerTool(params, o = {}) { return this.request("tools/call", params, { timeoutMs: o.timeout ?? 75000 }); }
  _watchSize() {
    if (typeof ResizeObserver === "undefined" || !this._ctx?.containerDimensions?.maxHeight) return;
    let raf = 0, lw = 0, lh = 0;
    const go = () => {
      if (raf) return; raf = 1;
      requestAnimationFrame(() => {
        raf = 0;
        const r = de(), pv = r.style.height;
        r.style.height = "max-content";
        const h = Math.ceil(r.getBoundingClientRect().height), w = Math.ceil(window.innerWidth);
        r.style.height = pv;
        if (w !== lw || h !== lh) { lw = w; lh = h; this.notification("ui/notifications/size-changed", { width: w, height: h }); }
      });
    };
    go(); (this._ro = new ResizeObserver(go)).observe(de());
  }
  async connect() {
    if (this._up) return;
    this._up = true; window.addEventListener("message", this._m);
    try {
      const r = await this.request("ui/initialize", { appInfo: this._i, appCapabilities: this._c, protocolVersion: LATEST_PROTOCOL_VERSION }, { timeoutMs: 20000 });
      if (!r || typeof r !== "object") throw new Error("Bad init");
      this._ctx = r.hostContext; this._hc = r.hostCapabilities || {};
      applyHostContext(this._ctx); this.notification("ui/notifications/initialized");
      if (this._o.autoResize) this._watchSize();
    } catch (e) { this.close(); throw e; }
  }
  close() {
    this._up = false; window.removeEventListener("message", this._m);
    this._ro?.disconnect(); this._ro = null;
    for (const e of this._p.values()) { clearTimeout(e.t); e.rej(new Error("Closed")); }
    this._p.clear();
  }
}
