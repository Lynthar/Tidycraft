#!/usr/bin/env node
// Palette audit for the compiled stylesheet (dist/assets/*.css): resolves every
// rule's var() chain to literal colours per theme, so a class rename can be
// checked as "same colours, new names". Modes and usage: docs/design-notes.md.

import fs from "node:fs";
import path from "node:path";
import postcss from "postcss";

const DIST = "dist/assets";
const UTILITY_PREFIXES = [
  "bg", "text", "border", "ring-offset", "ring", "divide", "placeholder",
  "from", "to", "via", "outline", "decoration", "caret", "fill", "stroke",
  "accent", "shadow",
];

function readDist() {
  if (!fs.existsSync(DIST)) fail(`${DIST} not found — run pnpm build first`);
  const files = fs.readdirSync(DIST).filter((f) => f.endsWith(".css"));
  if (files.length === 0) fail(`no .css in ${DIST} — run pnpm build first`);
  return postcss.parse(
    files.map((f) => fs.readFileSync(path.join(DIST, f), "utf8")).join("\n"),
  );
}

function fail(msg) {
  console.error(`check-palette: ${msg}`);
  process.exit(1);
}

// --- theme var tables ------------------------------------------------------

// Selectors here all have specificity 0-1-0, so the browser's tiebreak is document
// order; walking in source order and letting later writes win reproduces it.
// `*`-selector blocks are Tailwind's preflight defaults and apply everywhere.
function scopesOf(selector) {
  const s = { base: false, dark: false, light: false };
  for (const sel of selector.split(",").map((x) => x.trim())) {
    // The minifier strips the attribute quotes, so match both forms.
    if (/\[data-theme=["']?dark/.test(sel)) s.dark = true;
    else if (/\[data-theme=["']?light/.test(sel)) s.light = true;
    else if (sel.includes(":root") || sel === "*" || sel.startsWith("*,") || sel.startsWith("::"))
      s.base = true;
  }
  return s;
}

function buildVarTables(root) {
  const dark = {}, light = {};
  const darkBlockKeys = new Set(), lightBlockKeys = new Set();
  root.walkRules((rule) => {
    const s = scopesOf(rule.selector);
    if (!s.base && !s.dark && !s.light) return;
    rule.walkDecls((d) => {
      if (!d.prop.startsWith("--")) return;
      if (s.base) { dark[d.prop] = d.value; light[d.prop] = d.value; }
      if (s.dark) { dark[d.prop] = d.value; darkBlockKeys.add(d.prop); }
      if (s.light) { light[d.prop] = d.value; lightBlockKeys.add(d.prop); }
    });
  });
  return { dark, light, darkBlockKeys, lightBlockKeys };
}

// Innermost-first var() substitution; a second, one-level-nested pattern
// picks up var() calls whose fallback itself contains parentheses.
const VAR_FLAT = /var\(\s*(--[A-Za-z0-9-]+)\s*(?:,\s*([^()]*?))?\s*\)/;
const VAR_NESTED = /var\(\s*(--[A-Za-z0-9-]+)\s*(?:,\s*((?:[^()]|\([^()]*\))*?))?\s*\)/;

function resolveValue(value, table, dangling) {
  let v = value;
  for (let guard = 0; guard < 64 && v.includes("var("); guard++) {
    const m = v.match(VAR_FLAT) ?? v.match(VAR_NESTED);
    if (!m) break;
    const [whole, name, fallback] = m;
    let rep;
    if (table[name] !== undefined) rep = table[name];
    else if (fallback !== undefined) rep = fallback;
    else { dangling.add(name); rep = `<<unresolved:${name}>>`; }
    v = v.replace(whole, rep);
  }
  return v.replace(/\s+/g, " ").trim();
}

// --- snapshot --------------------------------------------------------------

function ruleKey(rule) {
  const ctx = [];
  for (let p = rule.parent; p && p.type === "atrule"; p = p.parent)
    ctx.unshift(`@${p.name} ${p.params}`);
  return ctx.length ? `${ctx.join(" | ")} | ${rule.selector}` : rule.selector;
}

function snapshot(root) {
  const { dark, light, darkBlockKeys, lightBlockKeys } = buildVarTables(root);
  const themes = { dark: {}, light: {} };
  const dangling = new Set();
  root.walkRules((rule) => {
    if (!rule.selector.includes(".")) return; // classless rules live in dangling's beat
    const scope = scopesOf(rule.selector);
    const themed = scope.dark || scope.light;
    const key = ruleKey(rule);
    // Rule-local custom properties (Tailwind's ring/shadow utilities define
    // their --tw-* inputs on the rule itself) overlay the global tables.
    const local = {};
    rule.walkDecls((d) => { if (d.prop.startsWith("--")) local[d.prop] = d.value; });
    for (const theme of ["dark", "light"]) {
      if (themed && !scope[theme]) continue;
      const table = { ...(theme === "dark" ? dark : light), ...local };
      const props = {};
      rule.walkDecls((d) => {
        if (d.prop.startsWith("--")) return;
        props[d.prop] = resolveValue(d.value, table, dangling);
      });
      if (Object.keys(props).length) themes[theme][key] = props;
    }
  });
  const dupes = {};
  for (const theme of ["dark", "light"]) {
    const byValue = new Map();
    const table = theme === "dark" ? dark : light;
    for (const [k, v] of Object.entries(table)) {
      const r = resolveValue(v, table, new Set());
      if (!byValue.has(r)) byValue.set(r, []);
      byValue.get(r).push(k);
    }
    dupes[theme] = [...byValue.entries()]
      .map(([val, ks]) => [val, ks.filter((k) => !/^--(tw|xy)-/.test(k))]) // third-party internals
      .filter(([, ks]) => ks.length > 1)
      .map(([val, ks]) => ({ value: val, tokens: ks.sort() }));
  }
  return {
    themes,
    blockKeys: {
      darkOnly: [...darkBlockKeys].filter((k) => !lightBlockKeys.has(k)).sort(),
      lightOnly: [...lightBlockKeys].filter((k) => !darkBlockKeys.has(k)).sort(),
    },
    duplicateValues: dupes,
  };
}

// --- class extraction & renaming ------------------------------------------

function classTokensOf(selector) {
  // `.hover\:bg-card-bg:hover` → "hover:bg-card-bg"
  const out = [];
  for (const m of selector.matchAll(/\.((?:[A-Za-z0-9_-]|\\.)+)/g))
    out.push(m[1].replace(/\\(.)/g, "$1"));
  return out;
}

function decompose(token, namemap) {
  const base = token.split(":").pop(); // strip variant prefixes
  for (const p of UTILITY_PREFIXES) {
    if (!base.startsWith(p + "-")) continue;
    const name = base.slice(p.length + 1);
    if (name in namemap) return { base, prefix: p, name };
  }
  return null;
}

function buildWorklist(root, namemap) {
  const classmap = {}; // base class → base class, changed names only
  const repointOnly = new Set();
  const unknownLegacy = new Set(); // utility-shaped: ends in a legacy name → hard error
  const handwritten = new Set(); // hand-written CSS on legacy vars → migrate by hand
  const names = Object.keys(namemap);
  root.walkRules((rule) => {
    let touchesLegacyVar = false;
    rule.walkDecls((d) => { if (d.value.includes("var(--color-")) touchesLegacyVar = true; });
    if (!touchesLegacyVar) {
      // Renamed-but-not-legacy classes (the soft tints) still need entries.
      for (const token of classTokensOf(rule.selector)) {
        const d = decompose(token, namemap);
        if (d && namemap[d.name] !== d.name) classmap[d.base] = `${d.prefix}-${namemap[d.name]}`;
      }
      return;
    }
    for (const token of classTokensOf(rule.selector)) {
      const d = decompose(token, namemap);
      if (!d) {
        const base = token.split(":").pop();
        if (names.some((n) => base === n || base.endsWith(`-${n}`))) unknownLegacy.add(token);
        else handwritten.add(token);
        continue;
      }
      const to = namemap[d.name];
      if (to === d.name) repointOnly.add(d.base);
      else classmap[d.base] = `${d.prefix}-${to}`;
    }
  });
  return {
    classmap,
    repointOnly: [...repointOnly].sort(),
    unknownLegacy: [...unknownLegacy].sort(),
    handwritten: [...handwritten].sort(),
  };
}

// Longest key first, and a match may not start or end inside a longer token
// (a plain \b treats "-" as a boundary and would hit `bg-card-bg` inside
// `bg-card-bg-hover`).
function replacerFor(classmap) {
  const entries = Object.entries(classmap)
    .filter(([a, b]) => a !== b)
    .sort((a, b) => b[0].length - a[0].length);
  const regexes = entries.map(([from, to]) => [
    new RegExp(`(?<![-A-Za-z0-9])${from.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")}(?![-A-Za-z0-9])`, "g"),
    to,
  ]);
  return (text) => {
    let count = 0;
    for (const [re, to] of regexes)
      text = text.replace(re, () => { count++; return to; });
    return { text, count };
  };
}

function renameInDir(dir, classmap) {
  const replace = replacerFor(classmap);
  let total = 0;
  const walk = (d) => {
    for (const e of fs.readdirSync(d, { withFileTypes: true })) {
      const p = path.join(d, e.name);
      if (e.isDirectory()) walk(p);
      else if (/\.(tsx?|css)$/.test(e.name)) {
        const { text, count } = replace(fs.readFileSync(p, "utf8"));
        if (count) { fs.writeFileSync(p, text); total += count; console.log(`  ${count}\t${p}`); }
      }
    }
  };
  walk(dir);
  console.log(`renamed ${total} occurrences`);
}

// --- modes -----------------------------------------------------------------

const [mode, ...args] = process.argv.slice(2);

if (mode === "snapshot") {
  const [out] = args;
  if (!out) fail("snapshot needs an output path");
  const snap = snapshot(readDist());
  fs.writeFileSync(out, JSON.stringify(snap, null, 1));
  const nDark = Object.keys(snap.themes.dark).length;
  console.log(`snapshot: ${nDark} dark keys, ${Object.keys(snap.themes.light).length} light keys → ${out}`);
  if (snap.blockKeys.darkOnly.length || snap.blockKeys.lightOnly.length)
    console.log(`theme block key mismatch — dark-only: [${snap.blockKeys.darkOnly}] light-only: [${snap.blockKeys.lightOnly}]`);
  for (const theme of ["dark", "light"])
    for (const g of snap.duplicateValues[theme])
      console.log(`same value in ${theme}: ${g.tokens.join(" = ")} (${g.value})`);
} else if (mode === "worklist") {
  const [namemapPath, out] = args;
  if (!namemapPath || !out) fail("worklist needs <namemap.json> <classmap-out.json>");
  const namemap = JSON.parse(fs.readFileSync(namemapPath, "utf8"));
  const { classmap, repointOnly, unknownLegacy, handwritten } = buildWorklist(readDist(), namemap);
  if (unknownLegacy.length)
    fail(`utility-shaped classes on var(--color-*) with no mapping: ${unknownLegacy.join(", ")}`);
  fs.writeFileSync(out, JSON.stringify(classmap, null, 1));
  console.log(`worklist: ${Object.keys(classmap).length} classes to rename → ${out}`);
  console.log(`repoint-only (name kept, config must move off --color-*): ${repointOnly.join(", ")}`);
  if (handwritten.length)
    console.log(`hand-written rules still on var(--color-*), migrate by hand: ${handwritten.join(", ")}`);
} else if (mode === "rename") {
  const [mapPath, dir] = args;
  if (!mapPath || !dir) fail("rename needs <classmap.json> <dir>");
  renameInDir(dir, JSON.parse(fs.readFileSync(mapPath, "utf8")));
} else if (mode === "compare") {
  const [basePath, newPath, mapPath, gonePath] = args;
  if (!basePath || !newPath || !mapPath) fail("compare needs <base.json> <new.json> <classmap.json>");
  const base = JSON.parse(fs.readFileSync(basePath, "utf8"));
  const next = JSON.parse(fs.readFileSync(newPath, "utf8"));
  const replace = replacerFor(JSON.parse(fs.readFileSync(mapPath, "utf8")));
  const expectedGone = new Set(gonePath ? JSON.parse(fs.readFileSync(gonePath, "utf8")) : []);
  let bad = 0;
  for (const theme of ["dark", "light"]) {
    const mappedKeys = new Set();
    for (const [key, props] of Object.entries(base.themes[theme])) {
      const mapped = replace(key).text;
      mappedKeys.add(mapped);
      const got = next.themes[theme][mapped];
      if (!got && expectedGone.has(key)) { console.log(`gone as expected ${theme}: ${key}`); continue; }
      if (!got) { console.log(`MISSING ${theme}: ${mapped}  (from: ${key})`); bad++; continue; }
      const want = JSON.stringify(props), have = JSON.stringify(got);
      if (want !== have) { console.log(`MISMATCH ${theme}: ${mapped}\n  base: ${want}\n  new:  ${have}`); bad++; }
    }
    for (const key of Object.keys(next.themes[theme]))
      if (!mappedKeys.has(key)) { console.log(`EXTRA ${theme}: ${key}`); bad++; }
  }
  if (bad) fail(`${bad} differences`);
  console.log("compare: identical resolved palettes");
} else if (mode === "dangling") {
  // Existence check, not value resolution: a var() reference is dangling when
  // its name is defined nowhere in the product — wherever the definition lives
  // (theme block, `.react-flow`, the rule itself) — and it carries no fallback.
  const root = readDist();
  const defined = new Set();
  const referencedBare = new Set();
  root.walkDecls((d) => {
    if (d.prop.startsWith("--")) defined.add(d.prop);
    for (const m of d.value.matchAll(/var\(\s*(--[A-Za-z0-9-]+)\s*(,)?/g))
      if (!m[2]) referencedBare.add(m[1]);
  });
  // --tw-*/--xy-* are Tailwind/XYFlow internals with library-managed
  // lifecycles (e.g. --tw-shadow-color is only defined by shadow-colour
  // utilities); our invariant is about this repo's tokens.
  const dangling = [...referencedBare]
    .filter((n) => !defined.has(n) && !/^--(tw|xy)-/.test(n))
    .sort();
  if (dangling.length) fail(`unresolved custom properties: ${dangling.join(", ")}`);
  console.log(`dangling: every bare var() has a definition (${defined.size} defined)`);
} else {
  fail("mode must be snapshot | worklist | rename | compare | dangling");
}
