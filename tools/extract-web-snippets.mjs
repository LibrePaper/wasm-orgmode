// Extract every Org snippet LibrePaper's web tests use, as fixtures for compare-lezer.mjs.
//   node tools/extract-web-snippets.mjs [path-to-librepaper-worktree]
// String and template literals (with `${}` replaced by "x", `+` and `[...].join()` folded) of
// the Org-related unit tests go to fixtures/web-snippets/<test>.json; the .org files the web
// app ships or tests with are copied to fixtures/web-org/.
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const here = path.dirname(fileURLToPath(import.meta.url));
const root = path.join(here, "..");
const LP = process.argv[2] ?? "/home/vincent/.kwt/worktrees/github.com/LibrePaper/librepaper/kata-f9qv-org-wasm-editor";
const { parse } = await import(`${LP}/web/node_modules/acorn/dist/acorn.mjs`);

const unitDir = path.join(LP, "web/tests/unit");
const named = new Set(["pandoc-org.mjs", "org-include.mjs", "visual-org-facts.mjs", "opening.mjs", "document-index.mjs"]);
const tests = fs.readdirSync(unitDir).filter((f) => f.endsWith(".mjs") && (f.includes("org") || named.has(f) || f.startsWith("pipe-table")));

function fold(n) {
  if (!n) return undefined;
  switch (n.type) {
    case "Literal": return typeof n.value === "string" ? n.value : undefined;
    case "TemplateLiteral": return n.quasis.map((q, i) => q.value.cooked + (i < n.expressions.length ? (fold(n.expressions[i]) ?? "x") : "")).join("");
    case "BinaryExpression": {
      if (n.operator !== "+") return undefined;
      const a = fold(n.left), b = fold(n.right);
      return a !== undefined && b !== undefined ? a + b : undefined;
    }
    case "CallExpression": {
      const c = n.callee;
      if (c.type === "MemberExpression" && c.property.name === "join" && c.object.type === "ArrayExpression") {
        const sep = n.arguments.length ? fold(n.arguments[0]) : ",";
        const parts = c.object.elements.map(fold);
        if (sep !== undefined && parts.every((p) => p !== undefined)) return parts.join(sep);
      }
      return undefined;
    }
    default: return undefined;
  }
}
const MARKUP = /[*+#\[\]|\\$=~\/_<:^-]/;
const keep = (s) => s.length >= 2 && (s.includes("\n") || (/\s/.test(s) && MARKUP.test(s)) || /^(\*+|#\+|\||:|\[\[|\\)/.test(s)) && !/^(https?:|\.{0,2}\/|[\w./-]+\.(m?js|css|svg|org|json|md)$)/.test(s);

function collect(node, out) {
  if (!node || typeof node.type !== "string") return;
  const s = ["Literal", "TemplateLiteral", "BinaryExpression", "CallExpression"].includes(node.type) ? fold(node) : undefined;
  if (s !== undefined) { if (keep(s)) out.add(s); return; }
  for (const k of Object.keys(node)) {
    const v = node[k];
    if (Array.isArray(v)) v.forEach((c) => collect(c, out));
    else if (v && typeof v === "object") collect(v, out);
  }
}

fs.rmSync(path.join(root, "fixtures/web-snippets"), { recursive: true, force: true });
fs.rmSync(path.join(root, "fixtures/web-org"), { recursive: true, force: true });
fs.mkdirSync(path.join(root, "fixtures/web-snippets"), { recursive: true });
fs.mkdirSync(path.join(root, "fixtures/web-org"), { recursive: true });
let total = 0;
for (const f of tests) {
  const out = new Set();
  collect(parse(fs.readFileSync(path.join(unitDir, f), "utf8"), { ecmaVersion: "latest", sourceType: "module" }), out);
  fs.writeFileSync(path.join(root, "fixtures/web-snippets", f.replace(/\.mjs$/, ".json")), JSON.stringify([...out], null, 1) + "\n");
  total += out.size;
  console.log(`${f}: ${out.size} snippets`);
}
function orgFiles(dir, out = []) {
  if (!fs.existsSync(dir)) return out;
  for (const e of fs.readdirSync(dir, { withFileTypes: true })) {
    const p = path.join(dir, e.name);
    if (e.isDirectory()) orgFiles(p, out);
    else if (e.name.endsWith(".org")) out.push(p);
  }
  return out;
}
let files = 0;
for (const dir of ["web/tests/fixtures/visual-tokens/edge", "web/src/templates", "docs/examples/tutorial-org"]) {
  for (const p of orgFiles(path.join(LP, dir))) {
    const rel = path.relative(LP, p).replace(/[\\/]/g, "__");
    fs.copyFileSync(p, path.join(root, "fixtures/web-org", rel));
    files++;
  }
}
console.log(`${total} snippets in ${tests.length} files, ${files} .org files`);
