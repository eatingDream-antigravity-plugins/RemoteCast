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
    .replace(/\s*!important/g, "!important")
    .replace(/;}/g, "}")
    .replace(/\[([^=\]]+)="([a-zA-Z0-9_-]+)"\]/g, "[$1=$2]")
    .replace(/(^|[:\s(,])0+\.(\d)/g, "$1.$2")
    .replace(/([:\s(,])0(?:px|em|rem)\b/g, "$10")
    .replace(/#([0-9a-fA-F])\1([0-9a-fA-F])\2([0-9a-fA-F])\3\b/g, "#$1$2$3")
    .trim();
}

const KEYWORDS_AND_GLOBALS = new Set(
  (
    "break case catch class const continue debugger default delete do else export extends false finally for function if import in instanceof let new null return super switch this throw true try typeof var void while with yield async await of static get set " +
    "window document navigator localStorage performance console setTimeout clearTimeout setInterval clearInterval requestAnimationFrame cancelAnimationFrame ResizeObserver IntersectionObserver Image Map Set WeakMap WeakSet Promise Error TypeError Number String Boolean Array Object Math Date JSON RegExp Symbol BigInt Function parseInt parseFloat isNaN isFinite encodeURIComponent decodeURIComponent createImageBitmap getComputedStyle undefined Infinity NaN arguments globalThis"
  ).split(" ")
);

  // Internal class properties/methods owned exclusively by our bundle (never present on DOM/MCP payloads).
const INTERNAL_PROPS = new Set(
  "effectiveTier calibrated ewmaRttMs lastNetRttMs lastKbps fastStreak lastUpgradeAt setMode getEffectiveQuality recordSample getNetworkGrade computePollTiming computeOfflinePollDelay getWheelCoalesceMs getTypeBatchMs getHostContext getHostCapabilities callServerTool notification _watchSize ontoolinput ontoolresult ontoolcancelled onhostcontextchanged onteardown App AdaptiveBandwidthController parseAlpha lum applyDocumentTheme applyHostStyleVariables applyHostFonts applyHostContext isTargetErrorText parsePanelLocaleTag resolvePanelLang coalesceInputEvents shouldApplyFrameResponse applyOptimisticTabClose isNoTabsPayload isSafeImageUrl applyFramePayloadState resolveStageAspect setupRevealObservers rootDoc rootWin bridgeApp clock onReveal onHide getLastFetchAt staleMs throttleMs ResizeObs IntersectionObs errCount reqEpoch currentEpoch currentTargetId idleMs inputBusy screencastIdle nextDelayMs bootLocale hostLocale navLang saved totalMs netRttMs tierChanged wasActive _i _c _o _n _p _up _t _m _ctx _hc _ro _send _req _note connect request tierVal clamp".split(" ")
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
      const isObj = t.v === "{" && (!p || p.k === "tpl_open" || objBracePrev.has(p.v));
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

  // Build lexical scope tree: scope 0 is bundle root; block scopes hold local `let`/`const`/param/`catch` decls.
  const mkScope = (parent) => ({ parent, children: [], decls: new Set(), refs: [], rename: new Map(), extUsed: new Set(), capFrom: new Set() });
  const rootScope = mkScope(null);
  const allScopes = [rootScope];
  const tokenScope = new Array(raw.length).fill(null);
  const tokenTargetScope = new Array(raw.length).fill(null);
  const canDeclLocal = (name) => name.length >= 2 && !KEYWORDS_AND_GLOBALS.has(name) && !braceProtected.has(name) && !INTERNAL_PROPS.has(name);
  const canDeclRoot = (name) => name.length >= 2 && name !== "bH" && !KEYWORDS_AND_GLOBALS.has(name) && !braceProtected.has(name) && !INTERNAL_PROPS.has(name);

  let curScope = rootScope;
  let pendingBlockDecls = null; // Array<{ idx, name }> for upcoming `{` block scope
  let inVarDecl = false;
  let varDepth = 0;
  let varTargetScope = rootScope;
  const exprArrowParams = new Set();

  for (let idx = 0; idx < raw.length; idx++) {
    const t = raw[idx];
    tokenScope[idx] = curScope;
    if (t.k === "ws") continue;
    const p = prevNonWs(idx);

    if (t.v === "{" && t.isBlock) {
      const child = mkScope(curScope);
      curScope.children.push(child);
      allScopes.push(child);
      if (pendingBlockDecls) {
        for (const item of pendingBlockDecls) {
          if (canDeclLocal(item.name)) {
            child.decls.add(item.name);
            tokenScope[item.idx] = child;
          } else {
            child.extUsed.add(item.name);
          }
        }
        pendingBlockDecls = null;
      }
      curScope = child;
      tokenScope[idx] = curScope;
      continue;
    }
    if (t.v === "}" && t.isBlock) {
      curScope = curScope.parent || rootScope;
      pendingBlockDecls = null;
      continue;
    }

    if (t.k === "id" && (t.v === "const" || t.v === "let" || t.v === "var")) {
      inVarDecl = true;
      varDepth = 0;
      varTargetScope = curScope;
      if (p?.v === "(" && prevNonWs(raw.indexOf(p))?.v === "for") {
        let fd = 1, fj = idx + 1;
        while (fj < raw.length && fd > 0) {
          if (raw[fj].v === "(") fd++;
          else if (raw[fj].v === ")") fd--;
          fj++;
        }
        if (nextNonWs(fj - 1)?.v === "{") {
          pendingBlockDecls = pendingBlockDecls || [];
          varTargetScope = null;
        }
      }
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
        if (varTargetScope === null && pendingBlockDecls) pendingBlockDecls.push({ idx, name: t.v });
        else if (varTargetScope === rootScope ? canDeclRoot(t.v) : canDeclLocal(t.v)) varTargetScope.decls.add(t.v);
        else if (varTargetScope) varTargetScope.extUsed.add(t.v);
      }
    }

    if (t.k === "id" && p && (p.v === "function" || p.v === "class")) {
      if (curScope === rootScope ? canDeclRoot(t.v) : canDeclLocal(t.v)) curScope.decls.add(t.v);
      else curScope.extUsed.add(t.v);
    }

    if (t.k === "id" && nextNonWs(idx)?.v === "=>" && p?.v !== ")") {
      const afterArrow = nextNonWs(raw.indexOf(nextNonWs(idx)));
      if (afterArrow?.v === "{") {
        pendingBlockDecls = pendingBlockDecls || [];
        pendingBlockDecls.push({ idx, name: t.v });
      } else {
        exprArrowParams.add(t.v);
      }
    }
    if (t.v === "(") {
      let depth = 1, closeIdx = -1;
      for (let j = idx + 1; j < raw.length; j++) {
        if (raw[j].k === "ws") continue;
        if (raw[j].v === "(") depth += 1;
        else if (raw[j].v === ")") { depth -= 1; if (depth === 0) { closeIdx = j; break; } }
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
          const bodyBrace = afterClose?.v === "=>" ? nextNonWs(raw.indexOf(afterClose)) : afterClose;
          const hasBlockBody = bodyBrace?.v === "{" && bodyBrace.isBlock;
          let pd = 0;
          for (let j = idx + 1; j < closeIdx; j++) {
            const pt = raw[j];
            if (pt.k === "ws") continue;
            if (pt.v === "(" || pt.v === "[" || pt.v === "{") pd += 1;
            else if (pt.v === ")" || pt.v === "]" || pt.v === "}") pd -= 1;
            else if (pd === 0 && pt.k === "id") {
              const pp = prevNonWs(j), pn = nextNonWs(j);
              if ((pp?.v === "(" || pp?.v === "," || pp?.v === "...") && (pn?.v === "," || pn?.v === ")" || pn?.v === "=")) {
                if (hasBlockBody) {
                  pendingBlockDecls = pendingBlockDecls || [];
                  pendingBlockDecls.push({ idx: j, name: pt.v });
                } else {
                  exprArrowParams.add(pt.v);
                }
              }
            }
          }
        }
      }
    }
  }

  // Bind every identifier occurrence to its resolving lexical scope
  const rootCounts = new Map();
  for (const ip of INTERNAL_PROPS) rootScope.decls.add(ip);
  const usedShortRoot = new Set([...KEYWORDS_AND_GLOBALS, ...exprArrowParams, ...rootScope.decls, ...rootScope.extUsed]);

  for (let idx = 0; idx < raw.length; idx++) {
    const t = raw[idx];
    if (t.k !== "id") continue;
    const p = prevNonWs(idx), nx = nextNonWs(idx);
    const afterDot = p?.v === "." || p?.v === "?.";
    const isObjKey = nx?.v === ":" && (p?.v === "{" || p?.v === ",");
    if (INTERNAL_PROPS.has(t.v)) {
      rootCounts.set(t.v, (rootCounts.get(t.v) || 0) + 1);
      tokenTargetScope[idx] = rootScope;
      continue;
    }
    if (afterDot || isObjKey || braceProtected.has(t.v)) continue;
    let sc = tokenScope[idx];
    let resolved = null;
    while (sc) {
      if (sc.decls.has(t.v)) { resolved = sc; break; }
      sc = sc.parent;
    }
    if (!resolved) {
      usedShortRoot.add(t.v);
      let s = tokenScope[idx];
      while (s) { s.extUsed.add(t.v); s = s.parent; }
    } else {
      tokenTargetScope[idx] = resolved;
      if (resolved === rootScope) {
        rootCounts.set(t.v, (rootCounts.get(t.v) || 0) + 1);
      } else {
        resolved.refs.push(idx);
      }
      let s = tokenScope[idx];
      while (s && s !== resolved) {
        s.capFrom.add(`${resolved === rootScope ? "R" : allScopes.indexOf(resolved)}:${t.v}`);
        s = s.parent;
      }
    }
  }

  const alphabet1 = "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ_$";
  const alphabet2 = alphabet1 + "0123456789";
  const genNames = [];
  const rootCandidatesFor1Char = new Set([...rootCounts.keys()]);
  for (const c of alphabet1) if (!usedShortRoot.has(c) || rootCandidatesFor1Char.has(c)) {
    if (!KEYWORDS_AND_GLOBALS.has(c) && !exprArrowParams.has(c) && !rootScope.extUsed.has(c)) {
      // Only allow reusing `c` if `c` wasn't an undeclared/1-char non-renamed global
      if (!usedShortRoot.has(c)) genNames.push(c);
    }
  }
  for (const c1 of alphabet1) {
    for (const c2 of alphabet2) {
      const s = c1 + c2;
      if (!usedShortRoot.has(s)) genNames.push(s);
      if (genNames.length > 600) break;
    }
    if (genNames.length > 600) break;
  }

  // Sort root identifiers + hoisted globals/strings together so highest-gain items get 1-char names first
  const hoistGlobals = new Set("document window Math Date Array Number String Object Boolean encodeURIComponent Promise Error Set Map JSON globalThis requestAnimationFrame cancelAnimationFrame ResizeObserver IntersectionObserver getComputedStyle null undefined".split(" "));
  const globalCounts = new Map();
  const strCounts = new Map();
  for (let idx = 0; idx < raw.length; idx++) {
    const t = raw[idx];
    const p = prevNonWs(idx), nx = nextNonWs(idx);
    const afterDot = p?.v === "." || p?.v === "?.";
    const isObjKey = nx?.v === ":" && (p?.v === "{" || p?.v === ",");
    if (t.k === "id" && hoistGlobals.has(t.v) && !afterDot && !isObjKey && !tokenTargetScope[idx] && !braceProtected.has(t.v) && p?.v !== "typeof" && (t.v !== "undefined" || !["=", ".", "?.", "[", "("].includes(nx?.v))) {
      globalCounts.set(t.v, (globalCounts.get(t.v) || 0) + 1);
      continue;
    }
    if (t.k !== "lit" || (t.v[0] !== '"' && t.v[0] !== "'") || t.v.length < 5) continue;
    if (isObjKey) continue;
    strCounts.set(t.v, (strCounts.get(t.v) || 0) + 1);
  }
  const hoistCandidates = [
    ...[...globalCounts.entries()].map(([s, cnt]) => [s, cnt, ((s === "undefined" ? 6 : s.length) - 1) * cnt - (s === "undefined" ? 2 : s.length + 3)]),
    ...[...strCounts.entries()].map(([s, cnt]) => [s, cnt, (s.length - 1) * cnt - s.length - 3]),
  ].filter(([, cnt, gain]) => cnt >= 2 && gain >= 2);

  const strMap = new Map();
  const strDecls = [];
  const combined = [
    ...[...rootCounts.entries()].map(([k, cnt]) => ({ k, cnt, gain: (k.length - 1) * cnt, kind: "id" })),
    ...hoistCandidates.map(([s, cnt, gain]) => ({ k: s, cnt, gain, kind: "hoist" })),
  ].sort((a, b) => b.gain - a.gain);
  let gi = 0;
  for (const item of combined) {
    if (gi >= genNames.length) break;
    const alias = genNames[gi];
    if (item.kind === "id") {
      if (alias.length < item.k.length) { rootScope.rename.set(item.k, alias); gi++; }
    } else {
      const origLen = item.k === "undefined" ? 6 : item.k.length;
      const declCost = item.k === "undefined" ? alias.length + 1 : alias.length + 2 + item.k.length;
      if ((origLen - alias.length) * item.cnt - declCost > 0) {
        strMap.set(item.k, alias);
        strDecls.push(item.k === "undefined" ? alias : `${alias}=${item.k}`);
        gi++;
      }
    }
  }

  for (let idx = 0; idx < raw.length; idx++) {
    const t = raw[idx];
    const p = prevNonWs(idx), nx = nextNonWs(idx);
    const afterDot = p?.v === "." || p?.v === "?.";
    const isObjKey = nx?.v === ":" && (p?.v === "{" || p?.v === ",");
    if (!isObjKey && strMap.has(t.v) && (t.k === "lit" || (t.k === "id" && !afterDot && p?.v !== "typeof" && !tokenTargetScope[idx]))) {
      const alias = strMap.get(t.v);
      let s = tokenScope[idx];
      while (s) { s.extUsed.add(alias); s = s.parent; }
    }
  }

  // Assign local 1-char names top-down per block scope, avoiding anything captured or referenced inside the subtree
  const assignChildScopes = (sc) => {
    for (const child of sc.children) {
      const childIdx = allScopes.indexOf(child);
      const localCounts = new Map();
      for (const rIdx of child.refs) {
        const nm = raw[rIdx].v;
        localCounts.set(nm, (localCounts.get(nm) || 0) + 1);
      }
      const forbidden = new Set([...KEYWORDS_AND_GLOBALS, ...exprArrowParams, ...child.extUsed, ...child.decls]);
      const capturedByDesc = new Map();
      const collectCaps = (node) => {
        for (const key of node.capFrom) {
          const sep = key.indexOf(":");
          const sid = key.slice(0, sep), orig = key.slice(sep + 1);
          const src = sid === "R" ? rootScope : allScopes[Number(sid)];
          if (src !== child) forbidden.add(src.rename.get(orig) || orig);
          else {
            let s = capturedByDesc.get(orig);
            if (!s) { s = new Set(); capturedByDesc.set(orig, s); }
            for (const x of node.extUsed) s.add(x);
            for (const d of node.decls) s.add(d);
          }
        }
        for (const gc of node.children) collectCaps(gc);
      };
      collectCaps(child);

      const sortedLocals = [...localCounts.entries()].sort((a, b) => b[1] * b[0].length - a[1] * a[0].length);
      for (const [orig] of sortedLocals) {
        const extra = capturedByDesc.get(orig);
        const bad = (c) => forbidden.has(c) || (extra && extra.has(c));
        let picked = null;
        for (const c of alphabet1) {
          if (!bad(c)) { picked = c; break; }
        }
        if (!picked) {
          outer: for (const c1 of alphabet1) {
            for (const c2 of alphabet2) {
              const s = c1 + c2;
              if (!bad(s)) { picked = s; break outer; }
            }
          }
        }
        if (picked && picked.length < orig.length) {
          child.rename.set(orig, picked);
          forbidden.add(picked);
        } else {
          forbidden.add(orig);
        }
      }
      void childIdx;
      assignChildScopes(child);
    }
  };
  assignChildScopes(rootScope);

  // Apply renames + boolean literals (`true` -> `!0`, `false` -> `!1`) + hoisted globals/strings
  for (let idx = 0; idx < raw.length; idx++) {
    const t = raw[idx];
    const p = prevNonWs(idx), nx = nextNonWs(idx);
    const afterDot = p?.v === "." || p?.v === "?.";
    const isObjKey = nx?.v === ":" && (p?.v === "{" || p?.v === ",");
    if (t.k === "id") {
      if (!afterDot && !isObjKey && (t.v === "true" || t.v === "false")) {
        t.k = "op"; t.v = t.v === "true" ? "!0" : "!1"; continue;
      }
      if (!afterDot && !isObjKey && p?.v !== "typeof" && t.v === "undefined" && !["=", ".", "?.", "[", "("].includes(nx?.v)) {
        if (strMap.has("undefined")) { t.v = strMap.get("undefined"); continue; }
        t.k = "op"; t.v = "void 0"; continue;
      }
      const ts = tokenTargetScope[idx];
      if (ts && ts.rename.has(t.v)) {
        t.v = ts.rename.get(t.v);
        continue;
      }
      if (!afterDot && !isObjKey && p?.v !== "typeof" && !ts && strMap.has(t.v)) {
        t.v = strMap.get(t.v);
      }
      if (t.k === "num" && /^0\.\d+$/.test(t.v)) t.v = t.v.slice(1);
    } else if (t.k === "lit" && !isObjKey && strMap.has(t.v)) {
      t.k = "id"; t.v = strMap.get(t.v);
    } else if (t.k === "num" && /^0\.\d+$/.test(t.v)) {
      t.v = t.v.slice(1);
    }
  }

  // Emit compact token stream with ASI-safe newline & space preservation
  const isWordChar = (c) => Boolean(c) && /[A-Za-z0-9_$\u0080-\uffff]/.test(c);
  const asiSensitivePrev = new Set(["return", "throw", "break", "continue", "yield"]);
  let res = strDecls.length ? `let ${strDecls.join(",")};` : "";
  let lastTok = null;
  let parenDepth = 0;
  let blockParenDepth = 0;
  let stmtIsLet = false;
  const stmtStack = [];

  for (let idx = 0; idx < raw.length; idx++) {
    const t = raw[idx];
    if (t.k === "ws") {
      const nx = nextNonWs(idx);
      if (!lastTok || !nx) continue;
      if (parenDepth === blockParenDepth && stmtIsLet && nx.v === "let" && (lastTok.v !== "}" || lastTok.isBlock)) {
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
        !(lastTok.v === "}" && lastTok.isBlock && !stmtIsLet) &&
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
      if (parenDepth === blockParenDepth && stmtIsLet && nx?.v === "let") {
        res += ",";
        lastTok = { k: "op", v: "," };
        idx = raw.indexOf(nx);
        continue;
      }
      stmtIsLet = false;
    }

    if (t.v === "(") parenDepth++;
    else if (t.v === ")") parenDepth = Math.max(0, parenDepth - 1);
    else if (t.v === "{") { stmtStack.push([stmtIsLet, blockParenDepth]); stmtIsLet = false; blockParenDepth = parenDepth; }
    else if (t.v === "}") { const prevSt = stmtStack.pop(); stmtIsLet = Boolean(prevSt?.[0]); blockParenDepth = prevSt ? prevSt[1] : 0; }
    else if (parenDepth === blockParenDepth && t.k === "id" && t.v === "let" && (!lastTok || lastTok.v === ";" || lastTok.v === "{" || (lastTok.v === "}" && lastTok.isBlock) || lastTok.v === "\n")) {
      stmtIsLet = true;
    } else if (parenDepth === blockParenDepth && t.k === "id" && ["if", "else", "for", "while", "do", "try", "catch", "finally", "switch", "case", "default", "function", "class", "return", "throw", "break", "continue"].includes(t.v) && lastTok?.v !== "." && lastTok?.v !== "?.") {
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
if (kb >= 45.0) {
  console.error(`ERROR: panel bundle ${kb.toFixed(1)} KiB exceeds 45 KiB budget`);
  process.exit(1);
}
