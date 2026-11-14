# wasm-orgmode

Org-mode to a standalone HTML page, compiled to WebAssembly.

Part of [LibrePaper](https://github.com/LibrePaper). The module is what a
LibrePaper editor previews an Org document with.

This is early work: see "What does not work yet" before relying on it.

## What it renders

Headlines, paragraphs, emphasis, links, lists, tables, quote, example and
source blocks, and images. On top of that:

- `#+TITLE:`, `#+SUBTITLE:`, `#+AUTHOR:` and `#+DATE:`, as a
  `<header class="title-block">` that opens the page.
- TODO keywords and priorities on headlines, as `<span class="todo">` and
  `<span class="priority">`.
- `SCHEDULED`, `DEADLINE` and `CLOSED` planning lines, as `<p class="planning">`.
- `CLOCK:` entries, as `<p class="clock">`.
- List checkboxes `[ ]`, `[X]`, `[-]` and `[/]`, as `<li class="task task-todo|task-doing|task-done">`
  with a disabled checkbox input, and statistics cookies as `<code>[1/2]</code>`.
- Footnotes, numbered in order of reference, with their text collected at the
  end of the document.
- Table captions from `#+CAPTION:`, with math and markup rendered.
- Math, kept as TeX and escaped inside a span tagged `data-math-style="inline"`
  or `"display"`. Typesetting is the host's.

The page it produces is self-contained: styles inline, no webfonts, no
scripts, nothing to fetch.

## What does not work yet

- Babel code blocks are shown as source and never executed.
- `- [[file.org::Heading]]` at the start of a list item is read as a
  description-list tag.
- `#+OPTIONS`, macros and citations are not interpreted.

## Building

```sh
rustup target add wasm32-unknown-unknown   # once
make build      # dist/org.wasm
make test       # the tests, natively
make compress   # dist/org.wasm.br and .gz
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

Two changes proposed to orgize and never merged there are included, with their
authors' commits kept in the history: footnote support by James Liu
([orgize#84](https://github.com/PoiScript/orgize/pull/84)) and newlines inside
bracket links by Logan Barnett
([orgize#94](https://github.com/PoiScript/orgize/pull/94)).

## License

MIT. See `LICENSE`.
