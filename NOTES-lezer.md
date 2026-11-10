# The Org syntax tree for LibrePaper's editor (librepaper#sgcd)

`src/lezer.rs` converts the rowan CST into the node vocabulary of
`web/src/lib/org-language/parser.js` (its NODE VOCABULARY comment is the contract) and
flattens it to a Lezer buffer. Background and the first measurements: `SPIKE-sm8z.md`.

## Wasm exports (all added to the existing ones, none changed)

```
lezer_parse(source: *const u8, len: usize) -> usize
lezer_parse_chunk(source: *const u8, len: usize, todo: *const u8, todo_len: usize) -> usize
lezer_tree_ptr() -> *const u32
lezer_names_len() -> usize
lezer_names_ptr() -> *const u8
alloc(len) / dealloc(ptr, len)        (existing)
```

- Both parse calls return the buffer length in `u32` words and keep the buffer in the module
  until the next parse. Read it with `new Uint32Array(memory.buffer, lezer_tree_ptr(), words)`
  and copy it out before the next call (memory may grow).
- Buffer: postorder, four words per node `[kind, from, to, size]`, offsets in UTF-16 code
  units, `size` counts the words of the node and its descendants. The root `Document` is not in
  the buffer: `Tree.build({ buffer, nodeSet, topID, length })` adds it.
- `lezer_names_len()` fills the name table (call it first); the bytes at `lezer_names_ptr()` are
  the node names separated by `\n`, kind = line index. The order equals `NODE_NAMES` in parser.js.
- `todo` of `lezer_parse_chunk` is UTF-8 text holding the *values* of the document's `#+TODO:`,
  `#+SEQ_TODO:` and `#+TYP_TODO:` lines, whitespace separated (`NEXT WAIT | FIN`, lines may be
  joined with `\n`, `(k)` fast keys are fine). The `|` is optional: a TodoKeyword node is produced
  either way. `TODO` and `DONE` are always states. An empty string is allowed (pass any valid
  pointer, e.g. `alloc(1)`). `lezer_parse` reads the `#+TODO:` lines out of the text itself.

## Section-reuse call sequence for the loader

1. Once per parse of the whole document: compute the TODO value text (every `#+TODO:`-like
   line's value, as `todoWords` does) and the top-level section starts (`sectionStarts`).
   The chunks are the text before the first headline, then one chunk per top-level section.
2. For each chunk that has no reusable fragment: `lezer_parse_chunk(chunk, todo)`, where `chunk`
   is `text.slice(from, to - 1)` for all but the last chunk (as `parseOrg` does) or
   `text.slice(from, to)` for the last. Offsets in the buffer are relative to `from`; the loader
   shifts them (add UTF-16 `from`) or builds a `Tree` for the chunk and positions it at `from`.
3. A chunk that starts with a headline yields exactly one top-level node, a `Section` (the buffer
   ends with that Section; its `size` word equals the buffer length). The chunk before the
   first headline yields the leading elements. Sub-headlines nest inside the Section.
4. Concatenate the chunks' children into the Document. `tools/compare-lezer.mjs` checks this on
   every fixture: the shifted, concatenated chunk buffers equal the buffer of the whole parse,
   word for word.

Chunk parsing is context free apart from the TODO words, which is why the loader must pass
them: a chunk need not contain the `#+TODO:` line.

## Mapping additions in this round

| CST | JS node |
|---|---|
| CITATION | `Citation[CitationKey...]`; one CitationKey per CITATION_KEY token (key without `@`) |
| LINK with LINK_PATH only / with `<` `>` | `Link[LinkTarget]`; the angle form's Link includes the brackets, the target does not |
| KEYWORD_VALUE (TITLE, SUBTITLE, AUTHOR, DATE, CAPTION) | `KeywordValue` holding the inline nodes |
| TABLE_FORMULA | `TableFormula[KeywordKey, KeywordValue?]` |
| FN_DEF with several lines | `FootnoteDefinition[FootnoteLabel, Paragraph...]` spanning all its content |
| block without end line | `Block[BlockBegin, body]`, no `BlockEnd`, ends with the body |
| stray `#+end_x` line | `BlockEnd` alone, outside a block |
| `\name` left as text | `LatexCommand` (also an unmatched `\begin{x}` / `\end{x}`, which the CST calls a fragment) |
| verse block content | `BlockBody[Paragraph[inline...]]` |
| empty `[fn::text]` label | no FootnoteLabel |

## Parser changes made on the Rust side (they change CSTs, tests pass)

- `fn_def.rs`: a multi-line footnote definition holds elements (paragraphs, lists, environments)
  as soon as it has a second line, not only when it has a blank line inside.
- `block.rs`: a verse block's content holds objects (bold, links, ...) instead of one TEXT token.
  The HTML export of verse text therefore renders markup it used to print raw.

## Intended differences from the JS parser

The comparison (`node tools/compare-lezer.mjs`, `QUIET=1` for the summary) ends with
**0 unexplained differences, 1223 of 1223 JS nodes matched** on name, extent and parent depth,
child order equal, on all fixtures. These stay:

1. **Multi-line `$$ ... $$` and `\[ ... \]` are one DisplayMath in Rust.** JS parses the inside
   as ordinary text (so it finds Entity/LatexCommand nodes there, e.g. `\frac`, `\tau`). Rust
   is right. JS tests that expect Entity/LatexCommand children inside a multi-line display
   formula will change. (3 fixtures; the script filters exactly this case and counts it.)
2. **Verse bodies** hold objects now; this matches JS, but a verse block is the one place where
   the Rust CST needed a parser change, see above.
3. Rust-only constructs are dropped, as in the spike: statistics cookies, sub/superscripts,
   `[@n]` counters, table.el tables, Snippets. JS reads them as text; neither side gives them a
   node.

4. **Three single-snippet differences, listed in `DELIBERATE` in `tools/compare-lezer.mjs`**
   (counted separately, 11 nodes in all):
   - `#+BEGIN_|` (unclosed, non-word name): JS makes a Block; Rust a Paragraph, as Org does.
   - `[fn:a]` alone on a line: JS ends the FootnoteDefinition before the `]`; Rust includes it.
   - `Para one.\n:smile:\nPara two.`: JS ends a paragraph at a line starting with `:word:`;
     Rust keeps one paragraph, as Org does.

Comparison corpus: `tools/extract-web-snippets.mjs` pulls every Org snippet out of LibrePaper's
web unit tests into `fixtures/web-snippets/*.json` (one fixture each) and copies the shipped
`.org` files into `fixtures/web-org/`; `compare-lezer.mjs` runs all of them against the old JS
parser (`OLD_PARSER`, extracted with `git show HEAD~1:web/src/lib/org-language/parser.js`).
Affiliated keywords before every element kind are in `fixtures/affiliated-elements.org`.

Not covered by fixtures: JS parser behaviour on inputs outside them (a fuzz comparison would be
the next step), and `Entity` choice for names that only one side's entity table contains
(Rust's table is the source for Entity nodes; text `\name` becomes LatexCommand).

## Timing and size (Node 24, release wasm, median)

| | JS `parseOrg` | wasm path |
|---|---|---|
| ~200 KB typical | 15 ms | 32 ms (27 ms in wasm incl. UTF-8 encode and copy; ~5 ms `Array.from` + `Tree.build`) |
| ~1 MB flat | 72 ms | 85 ms |
| one section, ~3.1 KB (`lezer_parse_chunk`) | n/a | 0.5 ms |

Native profile of the 200 KB parse (release): nom parse to rowan about 7-8 ms, CST to
intermediate tree about 4 ms, flatten plus the name-id map plus dropping the trees about 3 ms.
Hot spots by size, not measured with a profiler: green node allocation in the parser, the
intermediate `N` tree (one `Vec` per node) and its `normalize` pass. With section reuse a
keystroke re-parses one chunk (about 0.5 ms), so the full parse cost applies only to the first
load and to `#+TODO:` changes.

Wasm size: 373,882 B raw, 113,909 B gzip -9 (spike baseline before `lezer.rs`: 316,904 / 91,917).
