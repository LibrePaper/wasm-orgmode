# wasm-orgmode

Org-mode to a standalone HTML page, compiled to WebAssembly.

Part of [LibrePaper](https://github.com/LibrePaper). The module is what a
LibrePaper editor previews an Org document with.

This is early work: see "What does not work yet" before relying on it.

## What it renders

Headlines, paragraphs, emphasis, links, lists, tables, quote, example and
source blocks, and images. On top of that:

- TODO keywords and priorities on headlines, as `<span class="todo">` and
  `<span class="priority">`.
- `SCHEDULED`, `DEADLINE` and `CLOSED` planning lines, as `<p class="planning">`.
- `CLOCK:` entries, as `<p class="clock">`.
- Math, kept as TeX and escaped inside a span tagged `data-math-style="inline"`
  or `"display"`. Typesetting is the host's.
- Block elements carry `data-source="START-END"`, their byte range in the
  source.

The page it produces is self-contained: styles inline, no webfonts, no
scripts, nothing to fetch.

## What does not work yet

- A table preceded by `#+NAME:` or `#+CAPTION:` renders as a paragraph.
- `$x$` followed directly by punctuation is not recognised as math; `\(x\)` is.
- Footnote references and definitions are not rendered.
- `CLOCK:` entries inside a `:LOGBOOK:` drawer are dropped with the drawer.
- A blank line after a headline produces an empty paragraph.
- Babel code blocks are shown as source and never executed.

## Building

```sh
rustup target add wasm32-unknown-unknown   # once
make build      # dist/orgmode.wasm
make test       # the tests, natively
make compress   # dist/orgmode.wasm.br and .gz
```

## Layout

- `src/syntax`, `src/ast`, `src/config.rs`, `src/org.rs`: the parser and its
  lossless syntax tree.
- `src/export`: the plain HTML and Markdown exporters and the tree traverser.
- `src/render.rs`: the LibrePaper page renderer built on the exporter.
- `src/abi.rs`: the WebAssembly exports a host calls.

## Credits

The parser, syntax tree and exporters began as
[orgize](https://github.com/PoiScript/orgize) by Alex Lin (poi), taken at
commit `5f26c94` (version 0.10.0-alpha.10, July 2024) and developed here since.
orgize is MIT licensed, and its copyright notice is kept in `LICENSE`.

## License

MIT. See `LICENSE`.
