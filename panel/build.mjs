#!/usr/bin/env node
// Zero-dependency panel bundler for Chrome Stream MCP -> panel/dist/panel.html (< 36 KiB).
// Uses esbuild automatically if installed; otherwise bundles via built-in safe tokenizer.

import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);

function minifyCss(css) {
  return css
    .replace(/\/\*[\s\S]*?\*\//g, "")
    .replace(/\s+/g, " ")
    .replace(/\s*([{}:;,>~])\s*/g, "$1")
    .replace(/\(\s+/g, "(")
    .replace(/\s+\)/g, ")")
    .replace(/;}/g, "}")
    .trim();
}

// Strip comments and trim indentation safely while preserving strings, nested templates, and regexes.
function compactJs(src) {
  let out = "";
  let i = 0;
  const n = src.length;
  let prevNonWs = "";
  const templateBraceDepths = [];

  const canPrecedeRegex = (ch) =>
    !ch || "([{:;,=!?&|+-*%^~<>".includes(ch);

  const scanTemplateChunk = () => {
    while (i < n) {
      const c = src[i];
      if (c === "\\" && i + 1 < n) {
        out += c + src[i + 1];
        i += 2;
        continue;
      }
      if (c === "`") {
        out += c;
        i += 1;
        prevNonWs = "`";
        return;
      }
      if (c === "$" && src[i + 1] === "{") {
        out += "${";
        i += 2;
        templateBraceDepths.push(0);
        prevNonWs = "{";
        return;
      }
      out += c;
      i += 1;
    }
  };

  while (i < n) {
    const ch = src[i];
    const next = src[i + 1];

    // Resume template literal when closing `${ ... }`
    if (ch === "}" && templateBraceDepths.length > 0) {
      const top = templateBraceDepths[templateBraceDepths.length - 1];
      if (top === 0) {
        templateBraceDepths.pop();
        out += "}";
        i += 1;
        scanTemplateChunk();
        continue;
      }
      templateBraceDepths[templateBraceDepths.length - 1] = top - 1;
      out += ch;
      prevNonWs = ch;
      i += 1;
      continue;
    }

    if (ch === "{" && templateBraceDepths.length > 0) {
      templateBraceDepths[templateBraceDepths.length - 1] += 1;
      out += ch;
      prevNonWs = ch;
      i += 1;
      continue;
    }

    // Template literal start
    if (ch === "`") {
      out += ch;
      i += 1;
      scanTemplateChunk();
      continue;
    }

    // Single or double-quoted string
    if (ch === "'" || ch === '"') {
      const quote = ch;
      out += ch;
      i += 1;
      while (i < n) {
        const c = src[i];
        out += c;
        i += 1;
        if (c === "\\" && i < n) {
          out += src[i];
          i += 1;
          continue;
        }
        if (c === quote) break;
      }
      prevNonWs = quote;
      continue;
    }

    // Line comment
    if (ch === "/" && next === "/") {
      i += 2;
      while (i < n && src[i] !== "\n") i += 1;
      continue;
    }

    // Block comment
    if (ch === "/" && next === "*") {
      i += 2;
      while (i < n && !(src[i] === "*" && src[i + 1] === "/")) i += 1;
      i += 2;
      continue;
    }

    // Regex literal
    if (ch === "/" && canPrecedeRegex(prevNonWs)) {
      out += ch;
      i += 1;
      let inClass = false;
      while (i < n) {
        const c = src[i];
        out += c;
        i += 1;
        if (c === "\\" && i < n) {
          out += src[i];
          i += 1;
          continue;
        }
        if (c === "[") inClass = true;
        else if (c === "]") inClass = false;
        else if (c === "/" && !inClass) break;
      }
      prevNonWs = "/";
      continue;
    }

    out += ch;
    if (!/\s/.test(ch)) prevNonWs = ch;
    i += 1;
  }

  // Second pass: safely collapse redundant whitespace outside strings/templates/regexes.
  return collapseWhitespace(out);
}

function collapseWhitespace(src) {
  const isWord = (c) => Boolean(c) && /[A-Za-z0-9_$\u0080-\uffff]/.test(c);
  const noNewlineAfter = new Set(["{", "(", "[", ",", ";", ":", "?", "=", "&", "|", "*", "<", ">", "!", "."]);
  const noNewlineBefore = new Set(["}", ")", "]", ",", ";", ":", "?", "=", "&", "|", "*", "<", ">", "."]);

  let res = "";
  let i = 0;
  const n = src.length;
  let lastCh = "";
  const templateBraceDepths = [];

  const canPrecedeRegex = (ch) => !ch || "([{:;,=!?&|+-*%^~<>".includes(ch);

  const scanTemplateChunk = () => {
    while (i < n) {
      const c = src[i];
      if (c === "\\" && i + 1 < n) {
        res += c + src[i + 1];
        lastCh = src[i + 1];
        i += 2;
        continue;
      }
      if (c === "`") {
        res += c;
        lastCh = c;
        i += 1;
        return;
      }
      if (c === "$" && src[i + 1] === "{") {
        res += "${";
        lastCh = "{";
        i += 2;
        templateBraceDepths.push(0);
        return;
      }
      res += c;
      lastCh = c;
      i += 1;
    }
  };

  while (i < n) {
    const ch = src[i];

    if (ch === "}" && templateBraceDepths.length > 0) {
      const top = templateBraceDepths[templateBraceDepths.length - 1];
      if (top === 0) {
        templateBraceDepths.pop();
        res += "}";
        lastCh = "}";
        i += 1;
        scanTemplateChunk();
        continue;
      }
      templateBraceDepths[templateBraceDepths.length - 1] = top - 1;
      res += ch;
      lastCh = ch;
      i += 1;
      continue;
    }

    if (ch === "{" && templateBraceDepths.length > 0) {
      templateBraceDepths[templateBraceDepths.length - 1] += 1;
      res += ch;
      lastCh = ch;
      i += 1;
      continue;
    }

    if (ch === "`") {
      res += ch;
      lastCh = ch;
      i += 1;
      scanTemplateChunk();
      continue;
    }

    if (ch === "'" || ch === '"') {
      const quote = ch;
      res += ch;
      i += 1;
      while (i < n) {
        const c = src[i];
        res += c;
        i += 1;
        if (c === "\\" && i < n) {
          res += src[i];
          i += 1;
          continue;
        }
        if (c === quote) break;
      }
      lastCh = quote;
      continue;
    }

    if (ch === "/" && canPrecedeRegex(lastCh)) {
      res += ch;
      i += 1;
      let inClass = false;
      while (i < n) {
        const c = src[i];
        res += c;
        i += 1;
        if (c === "\\" && i < n) {
          res += src[i];
          i += 1;
          continue;
        }
        if (c === "[") inClass = true;
        else if (c === "]") inClass = false;
        else if (c === "/" && !inClass) break;
      }
      lastCh = "/";
      continue;
    }

    if (ch === " " || ch === "\t" || ch === "\r" || ch === "\n") {
      let hasNewline = ch === "\n";
      while (i < n && (src[i] === " " || src[i] === "\t" || src[i] === "\r" || src[i] === "\n")) {
        if (src[i] === "\n") hasNewline = true;
        i += 1;
      }
      const nextCh = src[i] || "";
      if (!lastCh || !nextCh) continue;
      if (hasNewline && !noNewlineAfter.has(lastCh) && !noNewlineBefore.has(nextCh)) {
        res += "\n";
        lastCh = "\n";
      } else if (
        (isWord(lastCh) && isWord(nextCh)) ||
        (lastCh === "+" && nextCh === "+") ||
        (lastCh === "-" && nextCh === "-") ||
        (lastCh === "/" && (nextCh === "/" || nextCh === "*")) ||
        (lastCh === "<" && nextCh === "!")
      ) {
        res += " ";
        lastCh = " ";
      }
      continue;
    }

    res += ch;
    lastCh = ch;
    i += 1;
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

  // Try optional esbuild if available on NODE_PATH for maximum minification
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
    // Pure zero-dependency bundler fallback
    const parts = ["bridge.js", "net-policy.js", "stage.js"].map((file) =>
      stripModuleSyntax(fs.readFileSync(path.join(__dirname, file), "utf8"))
    );
    const combined = `(() => {\n"use strict";\n${compactJs(parts.join("\n"))}\n})();`;
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
if (kb >= 42.0) {
  console.error(`ERROR: panel bundle ${kb.toFixed(1)} KiB exceeds 42 KiB budget`);
  process.exit(1);
}
