// Spike librepaper#sm8z: compare the Lezer tree built from the Rust CST with the
// one the JS Org parser builds, and time both.
//   node tools/compare-lezer.mjs [--first N] [--wasm path]
// Needs `cargo build --release --target wasm32-unknown-unknown` first.
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const WEB = "/home/vincent/.kwt/worktrees/github.com/LibrePaper/librepaper/kata-cx8b-index/web";
const { NodeSet, NodeType, Tree } = await import(`${WEB}/node_modules/@lezer/common/dist/index.js`);
const { parseOrg } = await import(`${WEB}/src/lib/org-language/parser.js`);

const here = path.dirname(fileURLToPath(import.meta.url));
const root = path.join(here, "..");
const args = process.argv.slice(2);
const opt = (name, dflt) => (args.includes(name) ? args[args.indexOf(name) + 1] : dflt);
const FIRST = Number(opt("--first", 15));
const wasmPath = opt("--wasm", path.join(root, "target/wasm32-unknown-unknown/release/wasm_orgmode.wasm"));

// ---- the wasm side
const { instance } = await WebAssembly.instantiate(fs.readFileSync(wasmPath), {});
const x = instance.exports;
const names = (() => {
  const len = x.lezer_names_len();
  return new TextDecoder().decode(new Uint8Array(x.memory.buffer, x.lezer_names_ptr(), len)).split("\n");
})();
const nodeSet = new NodeSet(names.map((name, id) => NodeType.define({ id, name, top: name === "Document" })));
const topID = names.indexOf("Document");
const encoder = new TextEncoder();

function put(text) {
  const bytes = encoder.encode(text);
  const ptr = x.alloc(Math.max(bytes.length, 1));
  new Uint8Array(x.memory.buffer, ptr, bytes.length).set(bytes);
  return [ptr, bytes.length];
}
const take = (words) => Array.from(new Uint32Array(x.memory.buffer, x.lezer_tree_ptr(), words));

/** The postorder buffer of a whole text. */
function rustBuffer(text) {
  const [ptr, len] = put(text);
  const words = x.lezer_parse(ptr, len);
  x.dealloc(ptr, Math.max(len, 1));
  return take(words);
}
/** The postorder buffer of a chunk, given the TODO words of the whole document. */
function rustChunk(text, todo) {
  const [ptr, len] = put(text);
  const [tp, tl] = put(todo);
  const words = x.lezer_parse_chunk(ptr, len, tp, tl);
  x.dealloc(ptr, Math.max(len, 1));
  x.dealloc(tp, Math.max(tl, 1));
  return take(words);
}
function rustTree(text) {
  return Tree.build({ buffer: rustBuffer(text), nodeSet, topID, length: text.length });
}

// ---- the loader's side of section reuse: split at top-level headlines (as parser.js
// sectionStarts does), parse each chunk with the whole document's TODO words, shift.
const HEAD_AT = /\*+(?=[ \t])/y;
function sectionStarts(text) {
  const starts = [];
  let base = 0;
  const see = (at) => {
    HEAD_AT.lastIndex = at;
    const m = HEAD_AT.exec(text);
    if (m && (!starts.length || m[0].length <= base)) { starts.push(at); base = m[0].length; }
  };
  if (text.charCodeAt(0) === 42) see(0);
  for (let i = text.indexOf("\n*"); i >= 0; i = text.indexOf("\n*", i + 1)) see(i + 1);
  return starts;
}
function todoLines(text) {
  return text.split("\n").map((l) => /^#\+(?:SEQ_|TYP_)?TODO:(.*)$/i.exec(l.replace(/\r$/, ""))).filter(Boolean).map((m) => m[1]).join("\n");
}
function chunkedBuffer(text) {
  const todo = todoLines(text);
  const starts = sectionStarts(text);
  const spans = [];
  if (!starts.length) spans.push([0, text.length]);
  else {
    if (starts[0] > 0) spans.push([0, starts[0]]);
    starts.forEach((f, k) => spans.push([f, k + 1 < starts.length ? starts[k + 1] : text.length]));
  }
  const out = [];
  let tops = 0;
  for (const [from, to] of spans) {
    const part = rustChunk(to < text.length ? text.slice(from, to - 1) : text.slice(from, to), todo);
    if (text.charCodeAt(from) === 42 && starts.length) {
      // exactly one top-level node, a Section
      let top = 0;
      for (let i = part.length; i > 0; i -= part[i - 1]) top++;
      if (top !== 1 || part[part.length - 4] !== names.indexOf("Section")) throw new Error(`chunk at ${from} is not one Section`);
    }
    tops++;
    for (let i = 0; i < part.length; i += 4) out.push(part[i], part[i + 1] + from, part[i + 2] + from, part[i + 3]);
  }
  return out;
}

// ---- diff
function flat(tree) {
  const out = [];
  let depth = 0;
  tree.iterate({ enter: (n) => { out.push(`${n.name}\t${n.from}\t${n.to}\t${depth++}`); }, leave: () => { depth--; } });
  return out;
}

function diff(text, a, b) {
  // a = JS, b = Rust. Multiset difference on "name from to".
  const count = new Map();
  for (const k of a) count.set(k, (count.get(k) ?? 0) + 1);
  const rustOnly = [];
  for (const k of b) {
    const c = count.get(k) ?? 0;
    if (c > 0) count.set(k, c - 1);
    else rustOnly.push(k);
  }
  const jsOnly = [];
  for (const [k, c] of count) for (let i = 0; i < c; i++) jsOnly.push(k);
  return { jsOnly, rustOnly };
}

// ---- fixtures
function findOrg(dir, out = []) {
  for (const e of fs.readdirSync(dir, { withFileTypes: true })) {
    if (e.name === "node_modules" || e.name === ".git") continue;
    const p = path.join(dir, e.name);
    if (e.isDirectory()) findOrg(p, out);
    else if (e.name.endsWith(".org")) out.push(p);
  }
  return out;
}
const fixtures = [];
for (const p of findOrg(WEB)) fixtures.push([`web:${path.relative(WEB, p)}`, fs.readFileSync(p, "utf8")]);
for (const p of findOrg(path.join(root, "samples"))) fixtures.push([`samples/${path.basename(p)}`, fs.readFileSync(p, "utf8")]);
for (const p of findOrg(path.join(root, "fixtures"))) fixtures.push([`fixtures/${path.basename(p)}`, fs.readFileSync(p, "utf8")]);

const totals = { jsOnly: new Map(), rustOnly: new Map(), match: 0, jsNodes: 0, order: 0, chunk: 0, intended: 0 };
const bump = (m, k) => m.set(k, (m.get(k) ?? 0) + 1);
const sample = new Map(); // "js-only Name" -> examples

for (const [label, text] of fixtures) {
  let a, b;
  try { a = flat(parseOrg(text)); } catch (e) { console.log(`${label}: JS parser threw ${e.message}`); continue; }
  try { b = flat(rustTree(text)); } catch (e) { console.log(`${label}: Rust tree failed: ${e.message}`); continue; }
  const d = diff(text, a, b);
  // Intended difference (NOTES-lezer.md): a multi-line $$..$$ or \[..\] is one DisplayMath in
  // Rust; JS parses its inside as text. Drop that DisplayMath and what JS found inside it.
  const spans = d.rustOnly.map((k) => k.split("\t")).filter(([n]) => n === "DisplayMath").map(([, f, t]) => [+f, +t]);
  const inside = (k) => { const [, f, t] = k.split("\t"); return spans.some(([a0, b0]) => +f >= a0 && +t <= b0); };
  const intended = d.jsOnly.filter(inside).length + spans.length;
  d.jsOnly = d.jsOnly.filter((k) => !inside(k));
  d.rustOnly = d.rustOnly.filter((k) => !k.startsWith("DisplayMath\t"));
  totals.intended += intended;
  totals.jsNodes += a.length;
  totals.match += a.length - d.jsOnly.length;
  for (const k of d.jsOnly) bump(totals.jsOnly, k.split("\t")[0]);
  for (const k of d.rustOnly) bump(totals.rustOnly, k.split("\t")[0]);
  if (!spans.length && !d.jsOnly.length && !d.rustOnly.length && a.join("|") !== b.join("|")) {
    totals.order++;
    const i = a.findIndex((v, j) => v !== b[j]);
    console.log(`   CHILD ORDER differs in ${label} at preorder ${i}: js ${a[i]} vs rust ${b[i]}`);
  }
  try {
    const whole = rustBuffer(text).join(",");
    const chunked = chunkedBuffer(text).join(",");
    if (whole !== chunked) { totals.chunk++; console.log(`   CHUNKED parse differs from whole parse in ${label}`); }
  } catch (e) { totals.chunk++; console.log(`   CHUNK check failed in ${label}: ${e.message}`); }
  console.log(`${label}: js ${a.length} nodes, rust ${b.length}; matching ${a.length - d.jsOnly.length}; js-only ${d.jsOnly.length}, rust-only ${d.rustOnly.length}`);
  const show = (tag, list) => {
    for (const k of list.slice(0, FIRST)) {
      const [name, f, t] = k.split("\t");
      const ex = `${tag} ${name} [${f},${t}] ${JSON.stringify(text.slice(Number(f), Number(t)).slice(0, 50))}`;
      const key = `${tag} ${name}`;
      const arr = sample.get(key) ?? [];
      if (arr.length < 3) { arr.push(`${label}: ${ex}`); sample.set(key, arr); }
      if (text.length < 3000 || process.env.VERBOSE) console.log("   " + ex);
    }
  };
  if (!process.env.QUIET) { show("JS-only  ", d.jsOnly); show("Rust-only", d.rustOnly); }
}

const table = (m) => [...m].sort((p, q) => q[1] - p[1]).map(([k, v]) => `${k}=${v}`).join(", ");
console.log(`\nfixtures with only a child-order difference: ${totals.order}; chunked != whole: ${totals.chunk}; nodes in intended multi-line DisplayMath differences: ${totals.intended}`);
console.log(`UNEXPLAINED differences: ${[...totals.jsOnly.values(), ...totals.rustOnly.values()].reduce((p, q) => p + q, 0)}`);
console.log(`TOTAL: ${totals.match}/${totals.jsNodes} JS nodes matched exactly (name+from+to)`);
console.log(`JS-only by name:   ${table(totals.jsOnly)}`);
console.log(`Rust-only by name: ${table(totals.rustOnly)}`);
console.log("\nExamples:");
for (const [k, v] of [...sample].sort()) for (const e of v) console.log("  " + e);

// ---- timing
function median(xs) { const s = [...xs].sort((p, q) => p - q); return s[s.length >> 1]; }
function time(fn, runs) {
  fn(); // warm up
  const ts = [];
  for (let i = 0; i < runs; i++) { const t0 = performance.now(); fn(); ts.push(performance.now() - t0); }
  return median(ts);
}

function typicalDoc(target) {
  const parts = [];
  let i = 0;
  while (parts.join("").length < target) {
    i++;
    parts.push(`* ${i % 7 === 0 ? "TODO " : ""}Heading number ${i} with /emphasis/ and $x_${i}$ :tag${i % 5}:\n`);
    parts.push(`:PROPERTIES:\n:ID: id-${i}\n:END:\n`);
    for (let p = 0; p < 10; p++) {
      parts.push(`Paragraph ${p} of section ${i} has *bold* text, a link [[https://example.org/${i}][site ${p}]], inline math $\\alpha + \\beta_${p}$, a citation [cite:@smith2020;@doe${i}] and =verbatim= bits, ~code~ too, plus a long sentence so the paragraph is of realistic size.\n\n`);
      if (p % 3 === 0) parts.push(`- item one with *bold*\n- [ ] item two with [[https://x.org]]\n  - nested ${p}\n1. numbered\n\n`);
      if (p % 4 === 1) parts.push(`| a | b | c |\n|---+---+---|\n| 1 | *2* | $3$ |\n| x | y | z |\n\n`);
    }
    parts.push(`#+begin_src python\nprint(${i})\n#+end_src\n\n`);
  }
  return parts.join("");
}
function flatDoc(target) {
  const parts = [];
  let n = 0;
  while (parts.join("").length < target) {
    n++;
    parts.push(`Paragraph ${n} has *bold*, a link [[https://example.org/${n}][site]], math $x_${n}^2$, =verb= and ~code~ with enough words to be a typical line of prose in a long document.\n`);
    if (n % 5 === 0) parts.push("\n");
    if (n % 40 === 0) parts.push("| a | b |\n| 1 | 2 |\n\n- one\n- two\n\n");
  }
  return parts.join("");
}

console.log("\nTIMING (median ms)");
for (const [label, doc, runs] of [["~200 KB typical", typicalDoc(200_000), 11], ["~1 MB no headlines", flatDoc(1_000_000), 7]]) {
  const js = time(() => parseOrg(doc), runs);
  const rs = time(() => rustTree(doc), runs);
  // Split the wasm side: parse+flatten in the module vs. the JS-side Tree.build.
  const bytes = encoder.encode(doc);
  const inOut = time(() => {
    const ptr = x.alloc(bytes.length);
    new Uint8Array(x.memory.buffer, ptr, bytes.length).set(encoder.encode(doc));
    const words = x.lezer_parse(ptr, bytes.length);
    x.dealloc(ptr, bytes.length);
    return words;
  }, runs);
  const tr = rustTree(doc);
  {
    const d = diff(doc, flat(parseOrg(doc)), flat(tr));
    const by = (l) => { const m = new Map(); for (const k of l) bump(m, k.split("\t")[0]); return table(m); };
    console.log(`   diff on this document: js-only ${d.jsOnly.length} (${by(d.jsOnly)}); rust-only ${d.rustOnly.length} (${by(d.rustOnly)})`);
  }
  if (process.env.DUMP_DOC && label.includes("typical")) fs.writeFileSync(process.env.DUMP_DOC, doc);
  if (label.includes("typical")) {
    const st = sectionStarts(doc);
    const chunk = doc.slice(st[3], st[4] - 1);
    const todo = todoLines(doc);
    const cs = time(() => rustChunk(chunk, todo), 31);
    console.log(`   single-section re-parse: ${chunk.length} chars, wasm ${cs.toFixed(2)} ms (JS parseOrg of the whole document with all other sections reused is not measured here)`);
  }
  console.log(`${label}: ${doc.length} chars, ${flat(tr).length} rust nodes / ${flat(parseOrg(doc)).length} js nodes`);
  console.log(`   JS parseOrg ${js.toFixed(1)} ms | wasm total ${rs.toFixed(1)} ms (encode+copy+parse+buffer ${inOut.toFixed(1)} ms, rest = Array.from + Tree.build)`);
}
