#!/usr/bin/env node
// Zero-dependency panel bundler & minifier for Chrome Stream MCP -> panel/dist/panel.html (< 44 KiB).

import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);

function minifyCss(css) {
  return css
    .replace(/\/\*[\s\S]*?\*\//g, "")
    .replace(/\s+/g, " ")
    .replace(/\s*([{}:;,>~+])\s*/g, "$1")
    .replace(/\(\s+/g, "(")
    .replace(/\s+\)/g, ")")
    .replace(/;}/g, "}")
    .replace(/(^|[:\s(,])0+\.(\d)/g, "$1.$2")
    .replace(/([:\s(,])0(?:px|em|rem)\b/g, "$10")
    .replace(/#([0-9a-fA-F])\1([0-9a-fA-F])\2([0-9a-fA-F])\3\b/g, "#$1$2$3")
    .trim();
}

const KEYWORDS_AND_GLOBALS = new Set(
  (
    "break case catch class const continue debugger default delete do else export extends false finally for function if import in instanceof let new null return super switch this throw true try typeof var void while with yield async await of static get set " +
    "window document navigator localStorage performance console setTimeout clearTimeout setInterval clearInterval requestAnimationFrame cancelAnimationFrame ResizeObserver Image Map Set WeakMap WeakSet Promise Error TypeError Number String Boolean Array Object Math Date JSON RegExp Symbol BigInt Function parseInt parseFloat isNaN isFinite encodeURIComponent decodeURIComponent createImageBitmap getComputedStyle undefined Infinity NaN arguments globalThis"
  ).split(" ")
);

// Internal class properties/methods owned exclusively by our bundle (never present on DOM/MCP payloads).
const INTERNAL_PROPS = new Set(
  "effectiveTier calibrated ewmaRttMs lastNetRttMs lastKbps fastStreak lastUpgradeAt setMode getEffectiveQuality recordSample getNetworkGrade computePollTiming getWheelCoalesceMs getTypeBatchMs getHostContext getHostCapabilities callServerTool notification _watchSize ontoolinput ontoolresult ontoolcancelled onhostcontextchanged onteardown App AdaptiveBandwidthController parseAlpha lum applyDocumentTheme applyHostStyleVariables applyHostFonts applyHostContext isTargetErrorText parsePanelLocaleTag resolvePanelLang coalesceInputEvents shouldApplyFrameResponse applyOptimisticTabClose _i _c _o _n _p _up _t _m _ctx _hc _ro _send _req _note connect request".split(" ")
);

function tokenizeJs(src) {
  const tokens = [];
  let i = 0;
  const n = src.length;
  let prevSig = "";
  const tplDepths = [];

  const canPrecedeRegex = (tok) =>
    !tok ||
    "([{:;,=!?&|+-*%^~<>".includes(tok[tok.length - 1]) ||
    tok === "return" ||
    tok === "typeof" ||
    tok === "case" ||
    tok === "throw" ||
    tok === "delete" ||
    tok === "void" ||
    tok === "in" ||
    tok === "of";

  const readTemplateChunk = (openChar) => {
    let chunk = openChar;
    while (i < n) {
      const c = src[i];
      if (c === "\\" && i + 1 < n) {
        chunk += c + src[i + 1];
        i += 2;
        continue;
      }
      if (c === "`") {
        chunk += "`";
        i += 1;
        tokens.push({ k: "lit", v: chunk });
        prevSig = "`";
        return;
      }
      if (c === "$" && src[i + 1] === "{") {
        chunk += "${";
        i += 2;
        tokens.push({ k: "tpl_open", v: chunk });
        tplDepths.push(0);
        prevSig = "{";
        return;
      }
      chunk += c;
      i += 1;
    }
    tokens.push({ k: "lit", v: chunk });
  };

  while (i < n) {
    const ch = src[i];
    const next = src[i + 1] || "";

    if (ch === "/" && next === "/") {
      i += 2;
      while (i < n && src[i] !== "\n") i += 1;
      continue;
    }
    if (ch === "/" && next === "*") {
      i += 2;
      while (i < n && !(src[i] === "*" && src[i + 1] === "/")) i += 1;
      i += 2;
      continue;
    }

    if (ch === "}" && tplDepths.length > 0) {
      const top = tplDepths[tplDepths.length - 1];
      if (top === 0) {
        tplDepths.pop();
        i += 1;
        readTemplateChunk("}");
        continue;
      }
      tplDepths[tplDepths.length - 1] = top - 1;
      tokens.push({ k: "op", v: "}" });
      prevSig = "}";
      i += 1;
      continue;
    }

    if (ch === "{" && tplDepths.length > 0) {
      tplDepths[tplDepths.length - 1] += 1;
      tokens.push({ k: "op", v: "{" });
      prevSig = "{";
      i += 1;
      continue;
    }

    if (ch === "`") {
      i += 1;
      readTemplateChunk("`");
      continue;
    }

    if (ch === "'" || ch === '"') {
      let s = ch;
      i += 1;
      while (i < n) {
        const c = src[i];
        s += c;
        i += 1;
        if (c === "\\" && i < n) {
          s += src[i];
          i += 1;
          continue;
        }
        if (c === ch) break;
      }
      tokens.push({ k: "lit", v: s });
      prevSig = ch;
      continue;
    }

    if (ch === "/" && canPrecedeRegex(prevSig)) {
      let rx = "/";
      i += 1;
      let inClass = false;
      while (i < n) {
        const c = src[i];
        rx += c;
        i += 1;
        if (c === "\\" && i < n) {
          rx += src[i];
          i += 1;
          continue;
        }
        if (c === "[") inClass = true;
        else if (c === "]") inClass = false;
        else if (c === "/" && !inClass) break;
      }
      while (i < n && /[a-z]/i.test(src[i])) {
        rx += src[i];
        i += 1;
      }
      tokens.push({ k: "lit", v: rx });
      prevSig = "/";
      continue;
    }

    if (/\s/.test(ch)) {
      let nl = ch === "\n";
      i += 1;
      while (i < n && /\s/.test(src[i])) {
        if (src[i] === "\n") nl = true;
        i += 1;
      }
      tokens.push({ k: "ws", nl });
      continue;
    }

    if (/[0-9]/.test(ch) || (ch === "." && /[0-9]/.test(next))) {
      let num = ch;
      i += 1;
      while (i < n && /[0-9a-fA-FxXoObBeE._]/.test(src[i])) {
        num += src[i];
        i += 1;
      }
      tokens.push({ k: "num", v: num });
      prevSig = num;
      continue;
    }

    if (/[A-Za-z_$]/.test(ch)) {
      let id = ch;
      i += 1;
      while (i < n && /[A-Za-z0-9_$]/.test(src[i])) {
        id += src[i];
        i += 1;
      }
      tokens.push({ k: "id", v: id });
      prevSig = id;
      continue;
    }

    const three = src.slice(i, i + 3);
    const two = src.slice(i, i + 2);
    if (["===", "!==", "...", "??=", "&&=", "||=", ">>>"].includes(three)) {
      tokens.push({ k: "op", v: three });
      prevSig = three;
      i += 3;
      continue;
    }
    if (["?.", "=>", "==", "!=", "<=", ">=", "&&", "||", "??", "++", "--", "+=", "-=", "*=", "/=", "**", "<<", ">>"].includes(two)) {
      tokens.push({ k: "op", v: two });
      prevSig = two;
      i += 2;
      continue;
    }

    tokens.push({ k: "op", v: ch });
    prevSig = ch;
    i += 1;
  }
  return tokens;
}

function foldTokens(tokens) {
  const out = [];
  for (let i = 0; i < tokens.length; i++) {
    const t = tokens[i];
    if (t.k === "ws") {
      out.push(t);
      continue;
    }
    // Fold adjacent string/static-template literals joined by `+`
    if (t.k === "lit" && (t.v[0] === '"' || t.v[0] === "'" || t.v.endsWith("`"))) {
      let cur = t;
      while (true) {
        let j = i + 1;
        while (j < tokens.length && tokens[j].k === "ws") j++;
        if (tokens[j]?.v !== "+") break;
        let k = j + 1;
        while (k < tokens.length && tokens[k].k === "ws") k++;
        const nx = tokens[k];
        if (!nx || nx.k !== "lit") break;
        const canJoinQuote = (cur.v[0] === '"' || cur.v[0] === "'") && nx.v[0] === cur.v[0];
        const canJoinTpl = cur.v.endsWith("`") && nx.v[0] === "`";
        if (!canJoinQuote && !canJoinTpl) break;
        let after = k + 1;
        while (after < tokens.length && tokens[after].k === "ws") after++;
        if (tokens[after]?.v === "*" || tokens[after]?.v === "/" || tokens[after]?.v === "%" || tokens[after]?.v === "." || tokens[after]?.v === "?.") break;
        cur = { k: "lit", v: cur.v.slice(0, -1) + nx.v.slice(1) };
        i = k;
      }
      out.push(cur);
      continue;
    }
    out.push(t);
  }
  return out;
}

function minifyJsTokens(src) {
  const raw = foldTokens(tokenizeJs(src));
  const prevNonWs = (idx) => {
    for (let j = idx - 1; j >= 0; j--) if (raw[j].k !== "ws") return raw[j];
    return null;
  };
  const nextNonWs = (idx) => {
    for (let j = idx + 1; j < raw.length; j++) if (raw[j].k !== "ws") return raw[j];
    return null;
  };

  // Unwrap single-identifier arrow parens `(x) =>` -> `x =>` when precedence-safe
  const safeBeforeArrow = new Set(["=", "(", "[", ",", ":", "?", "return", "=>", "async", "{", ";"]);
  for (let idx = 0; idx < raw.length; idx++) {
    if (raw[idx].v !== "(") continue;
    const p = prevNonWs(idx);
    if (p && !safeBeforeArrow.has(p.v)) continue;
    let i1 = idx + 1;
    while (i1 < raw.length && raw[i1].k === "ws") i1++;
    if (raw[i1]?.k !== "id") continue;
    let i2 = i1 + 1;
    while (i2 < raw.length && raw[i2].k === "ws") i2++;
    if (raw[i2]?.v !== ")") continue;
    let i3 = i2 + 1;
    while (i3 < raw.length && raw[i3].k === "ws") i3++;
    if (raw[i3]?.v !== "=>") continue;
    raw[idx] = { k: "ws", nl: false };
    raw[i2] = { k: "ws", nl: false };
  }

  // Exclude identifiers inside `{ x }` or `{ a, x }` object literals/destructuring,
  // and tag each `{` / `}` token with `isBlock` so emission can drop newlines after block `}`.
  const braceProtected = new Set();
  const dStack = [];
  const objBracePrev = new Set(["=", "(", ",", ":", "[", "?", "...", "const", "let", "var", "return", "&&", "||", "??"]);
  for (let idx = 0; idx < raw.length; idx++) {
    const t = raw[idx];
    if (t.k === "ws") continue;
    const p = prevNonWs(idx);
    const nx = nextNonWs(idx);
    if (t.v === "(" || t.v === "[" || t.v === "{") {
      const isObj = t.v === "{" && (!p || objBracePrev.has(p.v));
      if (t.v === "{") t.isBlock = !isObj;
      dStack.push(isObj ? "{" : t.v === "{" ? "B" : t.v);
      continue;
    }
    if (t.v === ")" || t.v === "]" || t.v === "}") {
      const top = dStack.pop();
      if (t.v === "}") t.isBlock = top === "B";
      continue;
    }
    if (t.k === "id" && dStack[dStack.length - 1] === "{" && (p?.v === "{" || p?.v === ",") && (nx?.v === "," || nx?.v === "}" || nx?.v === "=")) {
      braceProtected.add(t.v);
    }
  }

  // Count frequency of renameable identifiers (`len >= 3`) across var/let/const lists, fn/class decls, and parameter lists
  const counts = new Map();
  const declared = new Set(INTERNAL_PROPS);
  const canDecl = (name) => name.length >= 3 && !KEYWORDS_AND_GLOBALS.has(name) && !braceProtected.has(name);

  let inVarDecl = false;
  let varDepth = 0;
  for (let idx = 0; idx < raw.length; idx++) {
    const t = raw[idx];
    if (t.k === "ws") continue;
    const p = prevNonWs(idx);

    if (t.k === "id" && (t.v === "const" || t.v === "let" || t.v === "var")) {
      inVarDecl = true;
      varDepth = 0;
      if (t.v === "const") t.v = "let";
      continue;
    }

    if (inVarDecl) {
      if (t.v === "(" || t.v === "[" || t.v === "{") varDepth += 1;
      else if (t.v === ")" || t.v === "]" || t.v === "}") {
        if (varDepth === 0) inVarDecl = false;
        else varDepth -= 1;
      } else if (varDepth === 0 && (t.v === ";" || t.v === "in" || t.v === "of")) {
        inVarDecl = false;
      } else if (varDepth === 0 && t.k === "id" && (p?.v === "let" || p?.v === "var" || p?.v === ",")) {
        if (canDecl(t.v)) declared.add(t.v);
      }
    }

    if (t.k === "id" && p && (p.v === "function" || p.v === "class") && canDecl(t.v)) {
      declared.add(t.v);
    }

    // Collect plain parameter identifiers in `(a, b = 1) =>` or `function(a, b)` or `catch(err)`
    if (t.v === "(") {
      let depth = 1;
      let closeIdx = -1;
      for (let j = idx + 1; j < raw.length; j++) {
        if (raw[j].k === "ws") continue;
        if (raw[j].v === "(") depth += 1;
        else if (raw[j].v === ")") {
          depth -= 1;
          if (depth === 0) {
            closeIdx = j;
            break;
          }
        }
      }
      if (closeIdx > idx) {
        const afterClose = nextNonWs(closeIdx);
        const beforeOpen = p;
        const isParamList =
          afterClose?.v === "=>" ||
          beforeOpen?.v === "catch" ||
          beforeOpen?.v === "function" ||
          (afterClose?.v === "{" && beforeOpen && beforeOpen.v !== "if" && beforeOpen.v !== "for" && beforeOpen.v !== "while" && beforeOpen.v !== "switch");
        if (isParamList) {
          let pd = 0;
          for (let j = idx + 1; j < closeIdx; j++) {
            const pt = raw[j];
            if (pt.k === "ws") continue;
            if (pt.v === "(" || pt.v === "[" || pt.v === "{") pd += 1;
            else if (pt.v === ")" || pt.v === "]" || pt.v === "}") pd -= 1;
            else if (pd === 0 && pt.k === "id") {
              const pp = prevNonWs(j);
              const pn = nextNonWs(j);
              if ((pp?.v === "(" || pp?.v === "," || pp?.v === "...") && (pn?.v === "," || pn?.v === ")" || pn?.v === "=")) {
                if (canDecl(pt.v)) declared.add(pt.v);
              }
            }
          }
        }
      }
    }
  }

  for (let idx = 0; idx < raw.length; idx++) {
    const t = raw[idx];
    if (t.k !== "id") continue;
    const p = prevNonWs(idx);
    const nx = nextNonWs(idx);
    const afterDot = p?.v === "." || p?.v === "?.";
    const isObjKey = nx?.v === ":" && (p?.v === "{" || p?.v === ",");
    if (INTERNAL_PROPS.has(t.v) || (!afterDot && !isObjKey && declared.has(t.v) && !braceProtected.has(t.v))) {
      counts.set(t.v, (counts.get(t.v) || 0) + 1);
    }
  }

  // Generate short names avoiding existing undeclared identifiers & keywords
  const usedShort = new Set(KEYWORDS_AND_GLOBALS);
  for (let idx = 0; idx < raw.length; idx++) {
    const t = raw[idx];
    if (t.k !== "id") continue;
    const p = prevNonWs(idx);
    const nx = nextNonWs(idx);
    const afterDot = p?.v === "." || p?.v === "?.";
    const isObjKey = nx?.v === ":" && (p?.v === "{" || p?.v === ",");
    if (!afterDot && !isObjKey && !declared.has(t.v)) usedShort.add(t.v);
  }
  const alphabet1 = "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ_$";
  const alphabet2 = alphabet1 + "0123456789";
  const genNames = [];
  for (const c of alphabet1) if (!usedShort.has(c)) genNames.push(c);
  for (const c1 of alphabet1) {
    for (const c2 of alphabet2) {
      const s = c1 + c2;
      if (!usedShort.has(s)) genNames.push(s);
      if (genNames.length > 600) break;
    }
    if (genNames.length > 600) break;
  }

  const renameMap = new Map();
  const sorted = [...counts.entries()].filter(([k, cnt]) => k.length * cnt >= 6).sort((a, b) => b[1] * b[0].length - a[1] * a[0].length);
  let gi = 0;
  for (const [orig] of sorted) {
    if (gi < genNames.length && genNames[gi].length < orig.length) {
      renameMap.set(orig, genNames[gi++]);
    }
  }

  // Apply renames + boolean literals (`true` -> `!0`, `false` -> `!1`)
  for (let idx = 0; idx < raw.length; idx++) {
    const t = raw[idx];
    if (t.k !== "id") continue;
    const p = prevNonWs(idx);
    const nx = nextNonWs(idx);
    const afterDot = p?.v === "." || p?.v === "?.";
    const isObjKey = nx?.v === ":" && (p?.v === "{" || p?.v === ",");

    if (!afterDot && !isObjKey && (t.v === "true" || t.v === "false")) {
      t.k = "op";
      t.v = t.v === "true" ? "!0" : "!1";
      continue;
    }
    if (!afterDot && !isObjKey && t.v === "undefined" && !["=", ".", "?.", "[", "("].includes(nx?.v)) {
      t.k = "op";
      t.v = "void 0";
      continue;
    }
    if (INTERNAL_PROPS.has(t.v) && renameMap.has(t.v)) {
      t.v = renameMap.get(t.v);
      continue;
    }
    if (!afterDot && !isObjKey && renameMap.has(t.v)) {
      t.v = renameMap.get(t.v);
    }
  }

  // Hoist repeated global identifiers & quoted string literals (`len >= 7`) into top-level `let` bindings
  const hoistGlobals = new Set("clearTimeout setTimeout clearInterval setInterval document window navigator localStorage performance Math Date Array Number String Object Boolean ResizeObserver createImageBitmap encodeURIComponent".split(" "));
  const globalCounts = new Map();
  const strCounts = new Map();
  for (let idx = 0; idx < raw.length; idx++) {
    const t = raw[idx];
    const p = prevNonWs(idx);
    const nx = nextNonWs(idx);
    const afterDot = p?.v === "." || p?.v === "?.";
    const isObjKey = nx?.v === ":" && (p?.v === "{" || p?.v === ",");
    if (t.k === "id" && hoistGlobals.has(t.v) && !afterDot && !isObjKey && !declared.has(t.v) && !braceProtected.has(t.v) && p?.v !== "typeof") {
      globalCounts.set(t.v, (globalCounts.get(t.v) || 0) + 1);
      continue;
    }
    if (t.k !== "lit" || (t.v[0] !== '"' && t.v[0] !== "'") || t.v.length < 6) continue;
    if (isObjKey) continue;
    strCounts.set(t.v, (strCounts.get(t.v) || 0) + 1);
  }
  const strMap = new Map();
  const strDecls = [];
  const hoistCandidates = [
    ...[...globalCounts.entries()].map(([s, cnt]) => [s, cnt, (s.length - 2) * cnt - s.length - 4, "g"]),
    ...[...strCounts.entries()].map(([s, cnt]) => [s, cnt, (s.length - 2) * cnt - s.length - 4, "s"]),
  ]
    .filter(([, cnt, gain]) => cnt >= 2 && gain >= 2)
    .sort((a, b) => b[2] - a[2]);
  for (const [s] of hoistCandidates) {
    if (gi >= genNames.length) break;
    const alias = genNames[gi++];
    strMap.set(s, alias);
    strDecls.push(`${alias}=${s}`);
  }
  if (strMap.size > 0) {
    for (let idx = 0; idx < raw.length; idx++) {
      const t = raw[idx];
      const p = prevNonWs(idx);
      const nx = nextNonWs(idx);
      const afterDot = p?.v === "." || p?.v === "?.";
      const isObjKey = nx?.v === ":" && (p?.v === "{" || p?.v === ",");
      if (isObjKey) continue;
      if (t.k === "id" && !afterDot && p?.v !== "typeof" && strMap.has(t.v)) {
        t.v = strMap.get(t.v);
      } else if (t.k === "lit" && strMap.has(t.v)) {
        t.k = "id";
        t.v = strMap.get(t.v);
      }
    }
  }

  // Emit compact token stream with ASI-safe newline & space preservation
  const isWordChar = (c) => Boolean(c) && /[A-Za-z0-9_$\u0080-\uffff]/.test(c);
  const asiSensitivePrev = new Set(["return", "throw", "break", "continue", "yield"]);
  let res = strDecls.length ? `let ${strDecls.join(",")};` : "";
  let lastTok = null;
  let parenDepth = 0;
  let stmtIsLet = false;
  const stmtStack = [];

  for (let idx = 0; idx < raw.length; idx++) {
    const t = raw[idx];
    if (t.k === "ws") {
      const nx = nextNonWs(idx);
      if (!lastTok || !nx) continue;
      if (parenDepth === 0 && stmtIsLet && nx.v === "let" && (lastTok.v !== "}" || lastTok.isBlock)) {
        res += ",";
        lastTok = { k: "op", v: "," };
        idx = raw.indexOf(nx);
        continue;
      }
      if (t.nl && asiSensitivePrev.has(lastTok.v)) {
        res += "\n";
        lastTok = { k: "ws_out", v: "\n" };
        stmtIsLet = false;
        continue;
      }
      // Keep newline if a statement lacks a trailing semicolon before a new statement
      if (
        t.nl &&
        !(lastTok.v === "}" && lastTok.isBlock) &&
        ![";", "{", "(", "[", ",", ":", "?", "=", "=>", "&&", "||", "??", "+", "-", "*", "."].includes(lastTok.v) &&
        !["}", ")", "]", ",", ";", ":", "?", ".", "?.", "&&", "||", "??"].includes(nx.v) &&
        lastTok.k !== "tpl_open"
      ) {
        res += "\n";
        lastTok = { k: "ws_out", v: "\n" };
        stmtIsLet = false;
        continue;
      }
      const lc = res[res.length - 1] || "";
      const nc = nx.v[0] || "";
      if (
        (isWordChar(lc) && isWordChar(nc)) ||
        (lc === "+" && nc === "+") ||
        (lc === "-" && nc === "-") ||
        (lc === "/" && (nc === "/" || nc === "*")) ||
        (lc === "!" && nc === "!")
      ) {
        res += " ";
      }
      continue;
    }

    // Drop redundant `;` immediately before `}` or merge consecutive `let` statements
    if (t.v === ";") {
      const nx = nextNonWs(idx);
      if (nx?.v === "}") { stmtIsLet = false; continue; }
      if (parenDepth === 0 && stmtIsLet && nx?.v === "let") {
        res += ",";
        lastTok = { k: "op", v: "," };
        idx = raw.indexOf(nx);
        continue;
      }
      stmtIsLet = false;
    }

    if (t.v === "(") parenDepth++;
    else if (t.v === ")") parenDepth = Math.max(0, parenDepth - 1);
    else if (t.v === "{") { stmtStack.push(stmtIsLet); stmtIsLet = false; }
    else if (t.v === "}") { stmtIsLet = Boolean(stmtStack.pop()); }
    else if (parenDepth === 0 && t.k === "id" && t.v === "let" && (!lastTok || lastTok.v === ";" || lastTok.v === "{" || (lastTok.v === "}" && lastTok.isBlock) || lastTok.v === "\n")) {
      stmtIsLet = true;
    } else if (parenDepth === 0 && t.k === "id" && ["if", "else", "for", "while", "do", "try", "catch", "finally", "switch", "case", "default", "function", "class", "return", "throw", "break", "continue"].includes(t.v) && lastTok?.v !== "." && lastTok?.v !== "?.") {
      stmtIsLet = false;
    }

    res += t.v;
    lastTok = t;
  }
  return res;
}

function stripModuleSyntax(code) {
  return code
    .replace(/^\s*import\s+[\s\S]*?from\s+["'][^"']+["'];?\s*$/gm, "")
    .replace(/^\s*export\s+(?=(?:async\s+)?(?:function|class|const|let|var)\b)/gm, "");
}

async function bundleAssets() {
  const rawCss = fs.readFileSync(path.join(__dirname, "styles.css"), "utf8");

  try {
    const esbuild = await import("esbuild");
    const res = await esbuild.build({
      entryPoints: [path.join(__dirname, "stage.js")],
      bundle: true,
      minify: true,
      format: "iife",
      platform: "browser",
      target: ["es2022"],
      write: false,
    });
    const minCss = await esbuild.transform(rawCss, { loader: "css", minify: true });
    return {
      js: res.outputFiles[0].text,
      css: minCss.code.trim(),
      engine: "esbuild",
    };
  } catch {
    const parts = ["bridge.js", "net-policy.js", "stage.js"].map((file) =>
      stripModuleSyntax(fs.readFileSync(path.join(__dirname, file), "utf8"))
    );
    const combined = `(()=>{"use strict";${minifyJsTokens(parts.join("\n"))}})();`;
    return {
      js: combined,
      css: minifyCss(rawCss),
      engine: "builtin",
    };
  }
}

const { js, css, engine } = await bundleAssets();
const jsCode = js.replace(/<\/script/gi, "<\\/script");
const cssCode = css.replace(/<\/style/gi, "<\\/style");

// Validate generated JS syntax before writing
new Function(jsCode);

const template = fs.readFileSync(path.join(__dirname, "template.html"), "utf8").replace(/>\s+</g, "><").trim();
const html = template
  .replace("/*__INLINE_CSS__*/", () => cssCode)
  .replace("/*__INLINE_JS__*/", () => jsCode);

const distDir = path.join(__dirname, "dist");
fs.mkdirSync(distDir, { recursive: true });
const outHtml = path.join(distDir, "panel.html");
fs.writeFileSync(outHtml, html, "utf8");

const kb = Buffer.byteLength(html, "utf8") / 1024;
console.log(`Built ${outHtml} (${kb.toFixed(1)} KiB, engine=${engine})`);
if (kb >= 44.0) {
  console.error(`ERROR: panel bundle ${kb.toFixed(1)} KiB exceeds 44 KiB budget`);
  process.exit(1);
}
