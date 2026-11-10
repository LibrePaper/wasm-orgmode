# Spike sm8z: the Rust Org CST as the editor's Lezer tree

Question: can the Rust parser produce a tree in the vocabulary of the JS parser
(`web/src/lib/org-language/parser.js`), close enough and fast enough to replace it?

Answer in one line: **close enough, yes (about 94% of JS nodes match exactly on the real
fixtures, 100% apart from citations on generated documents); fast enough, no, not
today** (about 2x slower than the JS parser on a 200 KB document, on par on a 1 MB
flat one) **and there is no incremental reparse**. Recommendation below.

## What was built

- `src/lezer.rs`: `parse_to_buffer(&str) -> Vec<u32>`. Walks the rowan tree into a small
  intermediate tree, normalises it (children inside parents), and flattens it to the
  postorder `[kind, from, to, size]` buffer with UTF-16 offsets. `config_for` reads
  `#+TODO:` / `#+SEQ_TODO:` / `#+TYP_TODO:` into `ParseConfig`.
- wasm exports added to `src/abi.rs` (existing ones untouched): `lezer_parse(ptr,len) -> words`,
  `lezer_tree_ptr`, `lezer_names_len`, `lezer_names_ptr`. Same alloc/dealloc as before.
  Unlike typst's buffer the root `Document` is **not** in the buffer: `Tree.build({topID})`
  already adds the top node, so a root in the buffer ends up as a nested second Document.
- `tools/compare-lezer.mjs` (compare + timing), `fixtures/` (17 strings lifted from
  `tests/unit/org-language.mjs`, two hand-written files, one Unicode file),
  `examples/dump.rs` (CST dump for debugging).

Run: `cargo build --release --target wasm32-unknown-unknown && node tools/compare-lezer.mjs`
(`QUIET=1` hides per-file differences, `--first N` sets how many are listed).

## Mapping table (SyntaxKind -> JS node)

Structure is the main shape difference. Rust nests: `HEADLINE[STARS, TODO, PRIORITY, TITLE, TAGS, PLANNING, PROPERTY_DRAWER, SECTION(body), HEADLINE...]`,
where `SECTION` is only the *body* and sub-headlines are children of the headline.
JS: `Section[Headline[Stars,...], body elements..., Section...]`. So `HEADLINE` becomes
`Section`+`Headline`, and the body `SECTION` is lifted into the `Section`.

| Rust CST | JS node | How |
|---|---|---|
| DOCUMENT | Document | supplied by `Tree.build`, not in buffer |
| HEADLINE | Section + Headline | split; Headline = parts on the first line |
| HEADLINE_STARS (token) | Stars | rename |
| HEADLINE_KEYWORD_TODO / _DONE (token) | TodoKeyword | merged |
| HEADLINE_PRIORITY | Priority | rename |
| HEADLINE_TITLE | Title | rename, inline children converted |
| HEADLINE_TAGS | Tags + Tag | Tag synthesised from the TEXT tokens |
| SECTION (body) | (lifted) | children go into the JS Section / Document |
| PLANNING (+ _SCHEDULED/_DEADLINE/_CLOSED) | Planning, PlanningKeyword | keyword = leading TEXT token; timestamps converted |
| CLOCK | Clock, ClockKeyword, ClockDuration | duration synthesised from `=>` + TEXT |
| PROPERTY_DRAWER, NODE_PROPERTY | PropertyDrawer, Property, PropertyKey, PropertyValue | key/value read from the line text (CST has bare tokens) |
| DRAWER, DRAWER_BEGIN/END | Drawer, DrawerName, DrawerEnd | name read from the line |
| KEYWORD, AFFILIATED_KEYWORD, BABEL_CALL | Keyword, KeywordKey, KeywordValue | from the line text; key keeps `[opt]` |
| SOURCE/EXAMPLE/EXPORT/COMMENT/QUOTE/CENTER/VERSE/SPECIAL_BLOCK, DYN_BLOCK | Block, BlockBegin, BlockName, BlockParameters, BlockLanguage, BlockEnd | begin line parsed from text; body = VerbatimBody (src/example/export), CommentBody, else BlockBody (children lifted) |
| AFFILIATED_KEYWORD inside a block/table/paragraph/list | Keyword (preceding sibling) | hoisted out of the element, element start moved |
| LATEX_ENVIRONMENT | LatexEnvironment, LatexEnvBegin, LatexEnvName, LatexEnvEnd | from text (CST is bare tokens) |
| COMMENT, FIXED_WIDTH | Comment, FixedWidth | CST groups consecutive lines; split per line |
| RULE | HorizontalRule | rename, trim |
| FN_DEF (+FN_LABEL token, FN_CONTENT) | FootnoteDefinition, FootnoteLabel, Paragraph | FN_CONTENT becomes a Paragraph |
| PARAGRAPH | Paragraph | trimmed of trailing newline/blank line |
| ORG_TABLE, ORG_TABLE_STANDARD_ROW, ORG_TABLE_RULE_ROW, ORG_TABLE_CELL | Table, TableRow, TableRule, TableCell | PIPE tokens become TableDelimiter |
| LIST, LIST_ITEM | List, ListItem | trimmed |
| LIST_ITEM_BULLET (token) | ListBullet | trimmed |
| LIST_ITEM_CHECK_BOX | Checkbox | rename |
| LIST_ITEM_TAG | ListTag | `::` cut off |
| LIST_ITEM_CONTENT | (lifted) | |
| LIST_ITEM_INDENT, LIST_ITEM_COUNTER | (dropped) | no JS node |
| BOLD, ITALIC, UNDERLINE, STRIKE | Bold, Italic, Underline, StrikeThrough | first and last char become EmphasisMark |
| VERBATIM, CODE | Verbatim, Code | two EmphasisMarks, content dropped |
| LINK (+LINK_PATH token) | Link, LinkTarget, LinkDescription | description range from the brackets |
| FN_REF | FootnoteReference (+FootnoteLabel) | |
| TIMESTAMP_ACTIVE / _INACTIVE | TimestampActive / TimestampInactive | `a--b` range split in two (Rust makes one node) |
| MACROS | Macro, MacroName | |
| TARGET, RADIO_TARGET | Target, RadioTarget | |
| LATEX_FRAGMENT | InlineMath or DisplayMath | by delimiter (`$$`, `\[` = display) |
| ENTITY | Entity | |
| INLINE_SRC / INLINE_CALL | InlineSrc(+Language,Body) / InlineBabelCall(+Name) | from text |
| LINE_BREAK | LineBreak | trimmed |
| COOKIE, SNIPPET, SUBSCRIPT, SUPERSCRIPT, TABLE_EL, BLANK_LINE, all other tokens | (dropped, children lifted) | no JS node |
| (none) | Citation, CitationKey | **Rust parser has no citations** |
| (none) | LatexCommand | Rust ENTITY only matches its entity table; other `\cmd` stay text |
| (none) | TableFormula | Rust gives `#+TBLFM:` as a plain Keyword |

Not mapped / incomplete: `Rust-only` constructs with no JS node were dropped (cookies, sub/superscripts,
`[@n]` counters); JS `TableFormula`, `LatexCommand`, `Citation` have no source in the CST.

## Differences, grouped by cause

Totals over the real fixtures (2 web `.org` files, `samples/paper.org`, 17 unit strings,
3 own files): **936 of 995 JS nodes match on name+from+to**. Per-name counts:
JS-only `LatexCommand 7, CitationKey 7, EmphasisMark 6, Paragraph 5, Link 4, LinkTarget 4,
Citation 4, Bold 3, FootnoteDefinition 3, Entity 3, Block 2 (+Begin/Name/Params/Lang/Body),
TimestampActive 1, TableFormula 1, InlineMath 1`; Rust-only `Paragraph 8, FootnoteDefinition 3,
DisplayMath 2, FootnoteLabel 1, InlineMath 1, Keyword 1`. On the generated 200 KB and 1 MB
documents the only difference is citations (200 KB: 640 Citation + 1280 CitationKey).

1. **Rust parser lacks a construct** (needs work in the Rust parser, or a JS/Rust post-pass):
   - Citations `[cite/style:@key;...]`: whole Citation/CitationKey family. Largest and the only one that shows on ordinary documents. Easy: org-cite is a regular grammar; add an object parser.
   - Plain URL and `<angle>` links (`https://x.org`, `<https://x.org>`): Rust leaves them as text. JS Link/LinkTarget.
   - Inline markup inside keyword values (`#+TITLE: Hi *there*`, `#+CAPTION:`, `#+DATE: <...>`): Rust keeps the value as TEXT. JS parses TITLE, SUBTITLE, AUTHOR, DATE, CAPTION.
   - `#+TBLFM:` as TableFormula; `\unknowncmd` as LatexCommand.
   - Unclosed `#+begin_...` (typing state): JS makes a Block running to the end of the section, Rust falls back to a Paragraph. This one matters for an editor, which sees unclosed blocks while the user types.
2. **JS lacks a construct / is narrower** (these are arguably Rust being better):
   - Multi-line `$$ ... $$` and `\[ ... \]`: Rust gives DisplayMath, JS parses the inside as ordinary text (so JS yields Entity/LatexCommand there).
   - `\begin{equation}` without a matching `\end` is InlineMath in Rust, LatexCommand in JS.
3. **Extent conventions** (a naming/offset issue, fixed in the converter, still visible in one place):
   - Rust nodes include the trailing newline, blank lines, and indentation; JS nodes are trimmed. The converter trims every element; no residual diff.
   - Rust nests affiliated keywords (`#+NAME`, `#+CAPTION`) inside the table/paragraph/block; JS has them as preceding siblings. Hoisted in the converter.
   - Rust `SECTION` is the body, JS `Section` is headline+body+subsections. Restructured in the converter.
   - Timestamp ranges (`[a]--[b]`) are one node in Rust, two in JS. Split in the converter.
   - Footnote definitions: Rust ends the definition at the first line, JS continues over following lines until a blank line or element. Rust parser limitation (3 diffs, and each also adds a stray Paragraph).
4. **Naming only**: everything else was a rename or a token-to-node synthesis (EmphasisMark, Tag, TableDelimiter, ListBullet, FootnoteLabel). None remains.

## Timing (Node 24, wasm built with the release profile, median of 11 / 7 runs)

| Document | JS `parseOrg` | wasm path total | of which in wasm (UTF-8 encode, copy in, parse, flatten) | rest (`Array.from` + `Tree.build`) |
|---|---|---|---|---|
| ~200 KB typical (headlines, lists, tables, links, math, citations) | 14.6 ms | 28.5 ms | 22.4 ms | ~6 ms |
| ~1 MB, no headlines | 69 ms | 81 ms | 67.5 ms | ~14 ms |

So the Rust nom parser is slower than the hand-written JS one on this workload: about 9 MB/s
against about 14 MB/s. The Rust numbers are for the wasm module; the Rust parser is not
cheap per node (green nodes, then a second walk, then the buffer). Not tuned in this spike.

**No incremental reparse in the Rust crate** (nothing in `src/` reparses or reuses nodes; `Org::parse`
builds a whole `GreenNode`). The JS parser reuses unchanged top-level sections through Lezer
fragments, so on an edit it re-parses only one section. On a 200 KB document with many headlines the
JS cost per keystroke is a fraction of the 14.6 ms; the wasm path would pay the full 28 ms every
keystroke, on the main thread unless moved to a worker. A 1 MB flat document costs 80 ms per keystroke
in either parser; JS cannot reuse anything there either.

## wasm size

| | raw | gzip -9 |
|---|---|---|
| before (HEAD, existing exports) | 316,904 B | 91,917 B |
| after (+ `lezer.rs`, 4 exports) | 349,906 B | 105,028 B |

+33 KB raw, +13 KB gzip (+14%). Includes the node-name table and the converter.

## Recommendation

**Do not replace the JS parser with this now.** The tree shape is not the obstacle: with a 600-line
converter the Rust CST matches the JS vocabulary on 94% of nodes and on 100% of nodes in ordinary
documents except citations. The obstacles are performance and typing-time behaviour:

1. Rust is about 2x slower per parse than JS on a typical document and has no incremental reparse, while JS
   reuses all unchanged sections. For an editor that re-parses on every keystroke this is a regression
   unless the parse moves off the main thread or Rust learns to reparse a section.
2. The editor must handle half-typed text (unclosed blocks, unfinished links). The JS parser was written for
   that; the Rust one (an exporter parser) falls back to paragraphs.
3. Plain links, keyword-value inline markup, citations and multi-line footnotes would all have to be added
   to the Rust parser first, and the JS parser's 15 consumers rely on them.

What would change the answer, in order of value: (a) section-level incremental parse in Rust (split the text
at headline lines in Rust the way `parseOrg` does and cache per-section buffers on the host; this is cheap:
the host can do it with `lezer_parse` on slices and shift offsets, no change to the Rust parser); (b) Rust
parsers for citations, plain/angle links and inline markup in keyword values; (c) unclosed-block recovery;
(d) profile the nom parser (likely lots of small allocations and re-scans per line).

The converter is worth keeping as an *option*: if a single parser is wanted (the preview and the editor
disagreeing is a real cost), the cheaper path is to move the **JS parser's** guarantees into the Rust
one (a-d above, about the work of a few days each for (b), more for (a) and (c)), then switch; alternatively
keep both and use the Rust tree only for the live preview. If the single-parser goal is not pressing, leave the
editor on JS.

Incomplete in this spike: no worker/off-thread measurement; the converter's `Block` extents for
unclosed blocks are not handled; no check that every one of the ~15 consumer modules works on the
Rust-built tree (only node name+extent equality was checked, not child order or props); the 200 KB
document is synthetic.
