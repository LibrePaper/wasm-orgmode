//! SPIKE (librepaper#sm8z): the CST as a Lezer tree in the vocabulary of
//! LibrePaper's JS Org parser (`web/src/lib/org-language/parser.js`).
//!
//! The editor hands the host a postorder `u32` buffer, four words per node,
//! `[kind, from, to, size]`, offsets in UTF-16 code units, root last, `size`
//! counting the words of the node and all its descendants. This module builds
//! that buffer by converting the rowan tree to a small intermediate tree
//! (`N`), then flattening it.
//!
//! The mapping lives in [`conv`]: one `match` arm per CST kind. Kinds with no
//! JS counterpart fall through to the default arm, which drops the node and
//! lifts its children. Tokens are dropped unless an arm synthesises a JS node
//! from them (emphasis marks, table delimiters, tags...).

use crate::config::ParseConfig;
use crate::syntax::{SyntaxKind as K, SyntaxNode};
use crate::Org;
use rowan::NodeOrToken;

/// The JS node vocabulary, in id order. Must match `NODE_NAMES` in parser.js.
pub const NAMES: &str = "Document Section Headline Stars TodoKeyword Priority Title Tags Tag
Planning PlanningKeyword Clock ClockKeyword ClockDuration
PropertyDrawer Property PropertyKey PropertyValue Drawer DrawerName DrawerEnd
Keyword KeywordKey KeywordValue
Block BlockBegin BlockName BlockParameters BlockLanguage VerbatimBody CommentBody BlockBody BlockEnd
LatexEnvironment LatexEnvBegin LatexEnvName LatexEnvEnd
Comment FixedWidth HorizontalRule
Table TableRow TableCell TableDelimiter TableRule TableFormula
List ListItem ListBullet Checkbox ListTag FootnoteDefinition FootnoteLabel Paragraph
Bold Italic Underline StrikeThrough Verbatim Code EmphasisMark
Link LinkTarget LinkDescription FootnoteReference Citation CitationKey
TimestampActive TimestampInactive Macro MacroName Target RadioTarget
InlineMath DisplayMath Entity LatexCommand InlineSrc InlineSrcLanguage InlineSrcBody
InlineBabelCall InlineBabelName LineBreak";

/// The names as the host reads them: one per line, id = line index.
pub fn names() -> String {
    NAMES.split_whitespace().collect::<Vec<_>>().join("\n")
}

/// Intermediate node: byte offsets into the source.
#[derive(Debug)]
pub struct N {
    pub name: &'static str,
    pub from: usize,
    pub to: usize,
    pub kids: Vec<N>,
}

fn n(name: &'static str, from: usize, to: usize, kids: Vec<N>) -> N {
    N { name, from, to, kids }
}

struct Cx<'a> {
    src: &'a str,
}

fn is_ws(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\n' | b'\r')
}

impl Cx<'_> {
    /// `[from, to)` with surrounding blanks and line breaks removed.
    fn trim(&self, from: usize, to: usize) -> (usize, usize) {
        let b = self.src.as_bytes();
        let (mut f, mut t) = (from, to);
        while f < t && is_ws(b[f]) {
            f += 1;
        }
        while t > f && is_ws(b[t - 1]) {
            t -= 1;
        }
        (f, t)
    }

    fn range(&self, node: &SyntaxNode) -> (usize, usize) {
        let r = node.text_range();
        (usize::from(r.start()), usize::from(r.end()))
    }

    /// A node with the trimmed extent of a CST node and the converted children.
    fn wrap(&self, name: &'static str, node: &SyntaxNode) -> N {
        let (f, t) = self.range(node);
        let (f, t) = self.trim(f, t);
        let mut kids = vec![];
        self.kids(node, &mut kids);
        n(name, f, t, kids)
    }

    /// Convert every child node of `node`; tokens are dropped.
    /// The few tokens that map to a JS node of their own are converted here.
    fn kids(&self, node: &SyntaxNode, out: &mut Vec<N>) {
        for el in node.children_with_tokens() {
            match el {
                NodeOrToken::Node(c) => self.conv(&c, out),
                NodeOrToken::Token(t) => {
                    let r = t.text_range();
                    let (f, e) = (usize::from(r.start()), usize::from(r.end()));
                    match t.kind() {
                        K::LIST_ITEM_BULLET => {
                            let (f, e) = self.trim(f, e);
                            out.push(n("ListBullet", f, e, vec![]));
                        }
                        K::FN_LABEL if e > f => out.push(n("FootnoteLabel", f, e, vec![])),
                        K::TEXT => self.commands(f, e, out),
                        _ => {}
                    }
                }
            }
        }
    }

    /// `\name` in plain text is a LatexCommand in JS (the CST only knows entities).
    /// `\\` is skipped as a pair, like the JS parser does.
    fn commands(&self, f: usize, e: usize, out: &mut Vec<N>) {
        let b = self.src.as_bytes();
        let mut i = f;
        while i < e {
            if b[i] != b'\\' {
                i += 1;
            } else if i + 1 < e && b[i + 1] == b'\\' {
                i += 2;
            } else {
                let mut k = i + 1;
                while k < e && b[k].is_ascii_alphabetic() {
                    k += 1;
                }
                if k > i + 1 {
                    out.push(n("LatexCommand", i, k, vec![]));
                }
                i = k.max(i + 1);
            }
        }
    }

    /// Text of a single line `[f,t)` up to its first `needle` byte.
    fn find(&self, f: usize, t: usize, b: u8) -> Option<usize> {
        self.src.as_bytes()[f..t].iter().position(|&c| c == b).map(|p| f + p)
    }

    // ---------------------------------------------------------------- the table

    /// Rust keeps affiliated keywords (`#+NAME:`, `#+CAPTION:`) inside the element they
    /// precede; JS has them as preceding siblings. Hoist them out of the element.
    fn conv(&self, node: &SyntaxNode, out: &mut Vec<N>) {
        let start = out.len();
        self.conv_inner(node, out);
        for i in start..out.len() {
            let lead = out[i].kids.iter().take_while(|k| k.name == "Keyword").count();
            if lead > 0 && matches!(out[i].name, "Table" | "Paragraph" | "List" | "LatexEnvironment" | "FootnoteDefinition" | "Drawer" | "HorizontalRule") {
                let kws: Vec<N> = out[i].kids.drain(..lead).collect();
                let kw_end = kws.last().unwrap().to;
                let (nf, nt) = (out[i].from, out[i].to);
                out[i].from = self.trim(kw_end, nt).0.max(nf);
                let at = i;
                for (j, k) in kws.into_iter().enumerate() {
                    out.insert(at + j, k);
                }
                break; // one element per call
            }
        }
    }

    fn conv_inner(&self, node: &SyntaxNode, out: &mut Vec<N>) {
        let (nf, nt) = self.range(node);
        match node.kind() {
            // ---- structure: Rust nests body SECTION inside HEADLINE; JS nests
            // Headline + body + subsections inside Section.
            K::DOCUMENT => self.kids(node, out),
            K::SECTION => self.kids(node, out), // a body: lift its elements
            K::HEADLINE => out.push(self.headline(node)),

            // ---- planning / clock / drawers
            K::PLANNING => {
                let mut kids = vec![];
                for part in node.children() {
                    // PLANNING_SCHEDULED etc: [TEXT "SCHEDULED:", ws, TIMESTAMP_*]
                    for el in part.children_with_tokens() {
                        match el {
                            NodeOrToken::Token(t) if t.kind() == K::TEXT => {
                                let r = t.text_range();
                                let (f, e) = (usize::from(r.start()), usize::from(r.end()));
                                kids.push(n("PlanningKeyword", f, e, vec![]));
                            }
                            NodeOrToken::Node(c) => self.conv(&c, &mut kids),
                            _ => {}
                        }
                    }
                }
                let (f, t) = self.trim(nf, nt);
                out.push(n("Planning", f, t, kids));
            }
            K::CLOCK => {
                let mut kids = vec![];
                let mut after_arrow = false;
                for el in node.children_with_tokens() {
                    match el {
                        NodeOrToken::Token(t) => {
                            let r = t.text_range();
                            let (f, e) = (usize::from(r.start()), usize::from(r.end()));
                            match t.kind() {
                                K::TEXT if !after_arrow => kids.push(n("ClockKeyword", f, e, vec![])),
                                K::DOUBLE_ARROW => after_arrow = true,
                                K::TEXT => {
                                    // JS: ClockDuration covers `=> 1:30`; start at the arrow.
                                    let arrow = self.src[nf..f].rfind("=>").map(|p| nf + p).unwrap_or(f);
                                    kids.push(n("ClockDuration", arrow, e, vec![]));
                                }
                                _ => {}
                            }
                        }
                        NodeOrToken::Node(c) => self.conv(&c, &mut kids),
                    }
                }
                let (f, t) = self.trim(nf, nt);
                out.push(n("Clock", f, t, kids));
            }
            K::PROPERTY_DRAWER => out.push(self.drawer("PropertyDrawer", node)),
            K::DRAWER => out.push(self.drawer("Drawer", node)),
            K::NODE_PROPERTY => {
                // `:KEY: value` -- read from the line, the CST keeps these as tokens.
                let (f, t) = self.trim(nf, nt);
                let mut kids = vec![];
                if let Some(c1) = self.find(f, t, b':') {
                    if let Some(c2) = self.find(c1 + 1, t, b':') {
                        kids.push(n("PropertyKey", c1 + 1, c2, vec![]));
                        let (vf, vt) = self.trim(c2 + 1, t);
                        if vt > vf {
                            kids.push(n("PropertyValue", vf, vt, vec![]));
                        }
                    }
                }
                out.push(n("Property", f, t, kids));
            }

            // ---- keywords (`#+KEY: value`; JS treats affiliated ones the same)
            K::KEYWORD | K::AFFILIATED_KEYWORD | K::BABEL_CALL | K::TABLE_FORMULA => {
                let (f, t) = self.trim(nf, nt);
                let mut kw = self.keyword(f, t);
                if node.kind() == K::TABLE_FORMULA {
                    kw.name = "TableFormula";
                }
                // Inline markup in the value (TITLE, SUBTITLE, AUTHOR, DATE, CAPTION).
                if let Some(v) = node.children().find(|c| c.kind() == K::KEYWORD_VALUE) {
                    if let Some(kv) = kw.kids.iter_mut().find(|k| k.name == "KeywordValue") {
                        self.kids(&v, &mut kv.kids);
                    }
                }
                out.push(kw);
            }

            // ---- citations: the CST has one reference per key; JS lists the keys only
            K::CITATION => {
                let mut kids = vec![];
                for r in node.descendants() {
                    if r.kind() == K::CITATION_REFERENCE {
                        for t in r.children_with_tokens().filter_map(|e| e.into_token()) {
                            if t.kind() == K::CITATION_KEY {
                                let tr = t.text_range();
                                kids.push(n("CitationKey", usize::from(tr.start()), usize::from(tr.end()), vec![]));
                            }
                        }
                    }
                }
                out.push(n("Citation", nf, nt, kids));
            }

            // ---- blocks: JS has one Block with Begin/Body/End; Rust one kind per block name
            K::SOURCE_BLOCK | K::EXAMPLE_BLOCK | K::EXPORT_BLOCK | K::COMMENT_BLOCK | K::QUOTE_BLOCK
            | K::CENTER_BLOCK | K::VERSE_BLOCK | K::SPECIAL_BLOCK | K::DYN_BLOCK => {
                self.block(node, out);
            }
            K::LATEX_ENVIRONMENT => {
                // affiliated keywords come first among the children; `conv` hoists them out
                let mut kw = vec![];
                let mut body_from = nf;
                for c in node.children().take_while(|c| c.kind() == K::AFFILIATED_KEYWORD) {
                    self.conv(&c, &mut kw);
                    body_from = self.range(&c).1;
                }
                let (f, t) = self.trim(body_from, nt);
                let s = &self.src[f..t];
                let mut kids = vec![];
                if let Some(open) = s.find('{') {
                    if let Some(close) = s[open..].find('}') {
                        let bend = f + open + close + 1;
                        kids.push(n("LatexEnvBegin", f, bend, vec![n("LatexEnvName", f + open + 1, bend - 1, vec![])]));
                        if let Some(e) = s.rfind("\\end{") {
                            // JS emits no LatexEnvEnd for a one-line environment
                            if f + e >= bend && s[bend - f..e].contains('\n') {
                                kids.push(n("LatexEnvEnd", f + e, t, vec![]));
                            }
                        }
                    }
                }
                kw.push(n("LatexEnvironment", f, t, kids));
                out.append(&mut kw);
            }

            // ---- lesser elements. The CST groups consecutive lines; JS has one node per line.
            K::COMMENT => self.per_line("Comment", nf, nt, out),
            K::FIXED_WIDTH => {
                let from = self.after_keywords(node, out);
                self.per_line("FixedWidth", from, nt, out);
            }
            K::RULE => {
                let from = self.after_keywords(node, out);
                let (f, t) = self.trim(from, nt);
                out.push(n("HorizontalRule", f, t, vec![]));
            }
            K::FN_DEF => {
                let mut kids = vec![];
                for el in node.children_with_tokens() {
                    match el {
                        NodeOrToken::Token(t) if t.kind() == K::FN_LABEL => {
                            let r = t.text_range();
                            kids.push(n("FootnoteLabel", usize::from(r.start()), usize::from(r.end()), vec![]));
                        }
                        // Rust keeps the definition's text as one FN_CONTENT; JS has a Paragraph.
                        NodeOrToken::Node(c) if c.kind() == K::FN_CONTENT => {
                            if c.children().any(|k| k.kind() == K::PARAGRAPH) {
                                // multi-line definitions: the content holds real elements
                                self.kids(&c, &mut kids);
                            } else {
                                let (f, t) = self.range(&c);
                                let (f, t) = self.trim(f, t);
                                let mut pk = vec![];
                                self.kids(&c, &mut pk);
                                if t > f {
                                    kids.push(n("Paragraph", f, t, pk));
                                }
                            }
                        }
                        NodeOrToken::Node(c) => self.conv(&c, &mut kids),
                        _ => {}
                    }
                }
                let (f, t) = self.trim(nf, nt);
                out.push(n("FootnoteDefinition", f, t, kids));
            }
            K::PARAGRAPH => {
                // A stray `#+end_x` line is a BlockEnd of its own in JS.
                let (f, t) = self.trim(nf, nt);
                let line = &self.src[f..t];
                if !line.contains('\n') && line.len() > 6 && line[..6].eq_ignore_ascii_case("#+end_") {
                    out.push(n("BlockEnd", f, t, vec![]));
                } else if t > f {
                    out.push(self.wrap("Paragraph", node));
                }
            }

            // ---- tables
            K::ORG_TABLE => out.push(self.wrap("Table", node)),
            K::ORG_TABLE_RULE_ROW => {
                let (f, t) = self.trim(nf, nt);
                let line = &self.src[f..t];
                // JS: a rule is only `|`, `-` and `+` ; anything else (`|---|:--:|`) is a row of cells.
                if line.bytes().all(|c| matches!(c, b'|' | b'-' | b'+')) {
                    out.push(n("TableRule", f, t, vec![]));
                } else {
                    let mut kids = vec![];
                    let mut start = f + 1;
                    kids.push(n("TableDelimiter", f, f + 1, vec![]));
                    for (i, c) in line.bytes().enumerate().skip(1) {
                        if c == b'|' {
                            let (cf, ct) = self.trim(start, f + i);
                            if ct > cf {
                                kids.push(n("TableCell", cf, ct, vec![]));
                            }
                            kids.push(n("TableDelimiter", f + i, f + i + 1, vec![]));
                            start = f + i + 1;
                        }
                    }
                    let (cf, ct) = self.trim(start, t);
                    if ct > cf {
                        kids.push(n("TableCell", cf, ct, vec![]));
                    }
                    out.push(n("TableRow", f, t, kids));
                }
            }
            K::ORG_TABLE_STANDARD_ROW => {
                let mut kids = vec![];
                for el in node.children_with_tokens() {
                    match el {
                        NodeOrToken::Token(t) if t.kind() == K::PIPE => {
                            let r = t.text_range();
                            kids.push(n("TableDelimiter", usize::from(r.start()), usize::from(r.end()), vec![]));
                        }
                        NodeOrToken::Node(c) => self.conv(&c, &mut kids),
                        _ => {}
                    }
                }
                let (f, t) = self.trim(nf, nt);
                out.push(n("TableRow", f, t, kids));
            }
            K::ORG_TABLE_CELL => out.push(self.wrap("TableCell", node)),

            // ---- lists
            K::LIST => out.push(self.wrap("List", node)),
            K::LIST_ITEM => out.push(self.wrap("ListItem", node)),
            K::LIST_ITEM_CONTENT => self.kids(node, out), // lifted: JS has the elements directly in ListItem
            K::LIST_ITEM_INDENT | K::LIST_ITEM_COUNTER => {} // no JS node
            K::LIST_ITEM_BULLET => {
                let (f, t) = self.trim(nf, nt);
                out.push(n("ListBullet", f, t, vec![]));
            }
            K::LIST_ITEM_CHECK_BOX => out.push(n("Checkbox", nf, nt, vec![])),
            K::LIST_ITEM_TAG => {
                // Rust includes the trailing `::`; JS has the term only.
                let mut end = nt;
                for t in node.children_with_tokens().filter_map(|e| e.into_token()) {
                    if t.kind() == K::COLON2 {
                        end = usize::from(t.text_range().start());
                    }
                }
                let (f, t) = self.trim(nf, end);
                let mut kids = vec![];
                self.kids(node, &mut kids);
                out.push(n("ListTag", f, t, kids));
            }

            // ---- inline markup
            K::BOLD => out.push(self.emphasis("Bold", node, false)),
            K::ITALIC => out.push(self.emphasis("Italic", node, false)),
            K::UNDERLINE => out.push(self.emphasis("Underline", node, false)),
            K::STRIKE => out.push(self.emphasis("StrikeThrough", node, false)),
            K::VERBATIM => out.push(self.emphasis("Verbatim", node, true)),
            K::CODE => out.push(self.emphasis("Code", node, true)),
            K::LINK => {
                let mut kids = vec![];
                let mut desc: Option<(usize, usize, Vec<N>)> = None;
                let mut in_desc = false;
                for el in node.children_with_tokens() {
                    match el {
                        NodeOrToken::Token(t) if t.kind() == K::LINK_PATH => {
                            let r = t.text_range();
                            kids.push(n("LinkTarget", usize::from(r.start()), usize::from(r.end()), vec![]));
                        }
                        NodeOrToken::Node(c) if in_desc => {
                            if let Some((_, _, k)) = desc.as_mut() {
                                self.conv(&c, k);
                            }
                        }
                        NodeOrToken::Token(t) if t.kind() == K::TEXT && in_desc => {
                            let r = t.text_range();
                            if let Some((_, _, k)) = desc.as_mut() {
                                self.commands(usize::from(r.start()), usize::from(r.end()), k);
                            }
                        }
                        NodeOrToken::Token(t) if t.kind() == K::L_BRACKET && !kids.is_empty() && !in_desc => {
                            in_desc = true;
                            let p = usize::from(t.text_range().end());
                            desc = Some((p, p, vec![]));
                        }
                        NodeOrToken::Token(t) if t.kind() == K::R_BRACKET2 && in_desc => {
                            if let Some(d) = desc.as_mut() {
                                d.1 = usize::from(t.text_range().start());
                            }
                        }
                        _ => {}
                    }
                }
                if let Some((f, t, k)) = desc {
                    kids.push(n("LinkDescription", f, t, k));
                }
                out.push(n("Link", nf, nt, kids));
            }
            K::FN_REF => {
                let mut kids = vec![];
                self.kids(node, &mut kids); // FN_LABEL token, FN_CONTENT's objects lifted
                out.push(n("FootnoteReference", nf, nt, kids));
            }
            K::FN_CONTENT => self.kids(node, out),
            // Rust makes `[a]--[b]` one timestamp; JS makes two.
            K::TIMESTAMP_ACTIVE | K::TIMESTAMP_INACTIVE => {
                let name = if node.kind() == K::TIMESTAMP_ACTIVE { "TimestampActive" } else { "TimestampInactive" };
                let s = &self.src[nf..nt];
                match s.find(">--<").or_else(|| s.find("]--[")) {
                    Some(p) => {
                        out.push(n(name, nf, nf + p + 1, vec![]));
                        out.push(n(name, nf + p + 3, nt, vec![]));
                    }
                    None => out.push(n(name, nf, nt, vec![])),
                }
            }
            K::MACROS => {
                // `{{{name(args)}}}`: name = first text token
                let name = node
                    .children_with_tokens()
                    .filter_map(|e| e.into_token())
                    .find(|t| t.kind() == K::TEXT)
                    .map(|t| (usize::from(t.text_range().start()), usize::from(t.text_range().end())));
                let kids = name.map(|(f, t)| vec![n("MacroName", f, t, vec![])]).unwrap_or_default();
                out.push(n("Macro", nf, nt, kids));
            }
            K::TARGET => out.push(n("Target", nf, nt, vec![])),
            K::RADIO_TARGET => out.push(n("RadioTarget", nf, nt, vec![])),
            K::LATEX_FRAGMENT => {
                let s = &self.src[nf..nt];
                let display = s.starts_with("$$") || s.starts_with("\\[");
                // `\name{arg}` (and a `\begin{x}` with no matching `\end`) is only a command to JS;
                // what is inside the braces reads as text, with further commands.
                if s.as_bytes().get(1).is_some_and(|c| c.is_ascii_alphabetic()) && s.starts_with('\\') {
                    self.commands(nf, nt, out);
                    return;
                }
                out.push(n(if display { "DisplayMath" } else { "InlineMath" }, nf, nt, vec![]));
            }
            K::ENTITY => out.push(n("Entity", nf, nt, vec![])),
            K::LINE_BREAK => {
                let (f, t) = self.trim(nf, nt);
                out.push(n("LineBreak", f, t, vec![]));
            }
            K::INLINE_SRC => {
                // `src_lang[header]{code}`
                let s = &self.src[nf..nt];
                let mut kids = vec![];
                let lang_end = s.find(['[', '{']).unwrap_or(s.len());
                kids.push(n("InlineSrcLanguage", nf + 4, nf + lang_end, vec![]));
                if let (Some(o), true) = (s.rfind('{'), s.ends_with('}')) {
                    kids.push(n("InlineSrcBody", nf + o + 1, nt - 1, vec![]));
                }
                out.push(n("InlineSrc", nf, nt, kids));
            }
            K::INLINE_CALL => {
                let s = &self.src[nf..nt];
                let end = s.find(['[', '(']).unwrap_or(s.len());
                out.push(n("InlineBabelCall", nf, nt, vec![n("InlineBabelName", nf + 5, nf + end, vec![])]));
            }

            // ---- headline parts are handled in `headline`; these arise only if
            // they appear outside a headline, which the parser does not produce.
            // ---- no JS counterpart: drop the node, lift its children
            // (COOKIE, SNIPPET, SUBSCRIPT, SUPERSCRIPT, TABLE_EL, DRAWER_CONTENT, BLOCK_CONTENT...)
            _ => self.kids(node, out),
        }
    }

    fn per_line(&self, name: &'static str, nf: usize, nt: usize, out: &mut Vec<N>) {
        let mut pos = nf;
        while pos < nt {
            let end = self.src[pos..nt].find('\n').map(|p| pos + p).unwrap_or(nt);
            let (f, t) = self.trim(pos, end);
            if t > f {
                out.push(n(name, f, t, vec![]));
            }
            pos = end + 1;
        }
    }

    /// `#+KEY[opt]: value` on one line.
    fn keyword(&self, f: usize, t: usize) -> N {
        let mut kids = vec![];
        if let Some(c) = self.find(f, t, b':') {
            // Option brackets belong to the key, so look for the colon after `]`.
            let c = match self.find(f, c, b'[') {
                Some(_) => self.src[f..t].find("]:").map(|p| f + p + 1).unwrap_or(c),
                None => c,
            };
            kids.push(n("KeywordKey", f + 2, c, vec![]));
            let (vf, vt) = self.trim(c + 1, t);
            if vt > vf {
                kids.push(n("KeywordValue", vf, vt, vec![]));
            }
        }
        n("Keyword", f, t, kids)
    }

    fn drawer(&self, name: &'static str, node: &SyntaxNode) -> N {
        let mut kids = vec![];
        for c in node.children() {
            match c.kind() {
                K::DRAWER_BEGIN => {
                    let (f, t) = self.trim(self.range(&c).0, self.range(&c).1);
                    kids.push(n("DrawerName", f + 1, t - 1, vec![]));
                }
                K::DRAWER_END => {
                    let (f, t) = self.trim(self.range(&c).0, self.range(&c).1);
                    kids.push(n("DrawerEnd", f, t, vec![]));
                }
                _ => self.conv(&c, &mut kids),
            }
        }
        let (f, t) = self.trim(self.range(node).0, self.range(node).1);
        n(name, f, t, kids)
    }

    fn block(&self, node: &SyntaxNode, out: &mut Vec<N>) {
        let kind = node.kind();
        let mut kids = vec![];
        let mut before = vec![]; // affiliated keywords are children in Rust, siblings in JS
        for c in node.children() {
            match c.kind() {
                K::AFFILIATED_KEYWORD => self.conv(&c, &mut before),
                K::BLOCK_BEGIN => {
                    let (f, t) = self.trim(self.range(&c).0, self.range(&c).1);
                    kids.push(self.block_begin(f, t));
                }
                K::BLOCK_CONTENT | K::DRAWER_CONTENT => {
                    let (f, t) = self.range(&c);
                    let (tf, tt) = self.trim(f, t);
                    let name = match kind {
                        K::SOURCE_BLOCK | K::EXAMPLE_BLOCK | K::EXPORT_BLOCK => "VerbatimBody",
                        K::COMMENT_BLOCK => "CommentBody",
                        _ => "BlockBody",
                    };
                    let mut bk = vec![];
                    if name == "BlockBody" {
                        self.kids(&c, &mut bk);
                        // verse: the CST holds the objects directly, JS wraps them in a Paragraph
                        if kind == K::VERSE_BLOCK && tt > tf {
                            bk = vec![n("Paragraph", tf, tt, bk)];
                        }
                    }
                    // Verbatim bodies keep their leading indentation and end before the final newline.
                    let tt2 = if name == "BlockBody" { tt } else { t.saturating_sub((self.src.as_bytes()[..t].last() == Some(&b'\n')) as usize) };
                    // JS has no body for an empty one, and starts a BlockBody at its line start.
                    if tt2 > f && tt > tf {
                        kids.push(n(name, f, tt2, bk));
                    }
                }
                K::BLOCK_END | K::DYN_BLOCK_END => {
                    let (f, t) = self.trim(self.range(&c).0, self.range(&c).1);
                    kids.push(n("BlockEnd", f, t, vec![]));
                }
                K::DYN_BLOCK_BEGIN => {
                    let (f, t) = self.trim(self.range(&c).0, self.range(&c).1);
                    kids.push(self.block_begin(f, t));
                }
                _ => self.conv(&c, &mut kids),
            }
        }
        // Block extent: from the begin line to the end line (not the affiliated keywords).
        let first = kids.first().map(|k| k.from).unwrap_or(self.range(node).0);
        let last = kids.last().map(|k| k.to).unwrap_or(first);
        out.append(&mut before);
        out.push(n("Block", first, last, kids));
    }

    /// Converts the affiliated keywords leading `node` into siblings (JS has them as such) and
    /// returns where the element itself starts.
    fn after_keywords(&self, node: &SyntaxNode, out: &mut Vec<N>) -> usize {
        let mut from = self.range(node).0;
        for c in node.children().take_while(|c| c.kind() == K::AFFILIATED_KEYWORD) {
            self.conv(&c, out);
            from = self.range(&c).1;
        }
        from
    }

    fn block_begin(&self, f: usize, t: usize) -> N {
        // `#+begin_NAME params` (dynamic blocks: `#+BEGIN: name params`)
        let s = &self.src[f..t];
        let after = s.find('_').map(|p| p + 1).unwrap_or(2);
        let name_end = s[after..].find([' ', '\t']).map(|p| after + p).unwrap_or(s.len());
        let mut kids = vec![n("BlockName", f + after, f + name_end, vec![])];
        let (pf, pt) = self.trim(f + name_end, t);
        if pt > pf {
            let mut pk = vec![];
            let lname = s[after..name_end].to_ascii_lowercase();
            if lname == "src" || lname == "export" {
                let lang_end = self.src[pf..pt].find([' ', '\t']).map(|p| pf + p).unwrap_or(pt);
                pk.push(n("BlockLanguage", pf, lang_end, vec![]));
            }
            kids.push(n("BlockParameters", pf, pt, pk));
        }
        n("BlockBegin", f, t, kids)
    }

    fn emphasis(&self, name: &'static str, node: &SyntaxNode, raw: bool) -> N {
        let (nf, nt) = self.range(node);
        let mut kids = vec![n("EmphasisMark", nf, nf + 1, vec![])];
        if !raw {
            self.kids(node, &mut kids);
        }
        kids.push(n("EmphasisMark", nt - 1, nt, vec![]));
        n(name, nf, nt, kids)
    }

    /// JS `Section[Headline, body..., Section...]` from Rust `HEADLINE[parts..., SECTION, HEADLINE...]`.
    fn headline(&self, node: &SyntaxNode) -> N {
        let (nf, nt) = self.range(node);
        let mut parts = vec![];
        let mut rest = vec![];
        for el in node.children_with_tokens() {
            match el {
                NodeOrToken::Token(t) if t.kind() == K::HEADLINE_STARS => {
                    let r = t.text_range();
                    parts.push(n("Stars", usize::from(r.start()), usize::from(r.end()), vec![]));
                }
                NodeOrToken::Token(t) if matches!(t.kind(), K::HEADLINE_KEYWORD_TODO | K::HEADLINE_KEYWORD_DONE) => {
                    let r = t.text_range();
                    parts.push(n("TodoKeyword", usize::from(r.start()), usize::from(r.end()), vec![]));
                }
                NodeOrToken::Node(c) => match c.kind() {
                    K::HEADLINE_STARS => {
                        let (f, t) = self.range(&c);
                        parts.push(n("Stars", f, t, vec![]));
                    }
                    K::HEADLINE_KEYWORD_TODO | K::HEADLINE_KEYWORD_DONE => {
                        let (f, t) = self.range(&c);
                        parts.push(n("TodoKeyword", f, t, vec![]));
                    }
                    K::HEADLINE_PRIORITY => {
                        let (f, t) = self.range(&c);
                        parts.push(n("Priority", f, t, vec![]));
                    }
                    K::HEADLINE_TITLE => parts.push(self.wrap("Title", &c)),
                    K::HEADLINE_TAGS => {
                        let (f, t) = self.range(&c);
                        let mut tags = vec![];
                        for tk in c.children_with_tokens().filter_map(|e| e.into_token()) {
                            if tk.kind() == K::TEXT {
                                let r = tk.text_range();
                                tags.push(n("Tag", usize::from(r.start()), usize::from(r.end()), vec![]));
                            }
                        }
                        parts.push(n("Tags", f, t, tags));
                    }
                    _ => self.conv(&c, &mut rest), // planning, drawer, SECTION body, sub-HEADLINEs, in order
                },
                _ => {}
            }
        }
        let hf = parts.first().map(|p| p.from).unwrap_or(nf);
        let ht = parts.last().map(|p| p.to).unwrap_or(nf);
        let mut kids = vec![n("Headline", hf, ht, parts)];
        kids.extend(rest);
        let to = kids.last().map(|k| k.to).unwrap_or(nt);
        n("Section", nf, to, kids)
    }
}

// ------------------------------------------------------------------ buffer

/// Byte offset -> UTF-16 offset.
struct Utf16<'a> {
    src: &'a str,
    table: Option<Vec<u32>>,
}

impl<'a> Utf16<'a> {
    fn new(src: &'a str) -> Self {
        let table = if src.is_ascii() {
            None
        } else {
            let mut t = vec![0u32; src.len() + 1];
            let mut u = 0u32;
            for (i, c) in src.char_indices() {
                for k in 0..c.len_utf8() {
                    t[i + k] = u;
                }
                u += c.len_utf16() as u32;
            }
            t[src.len()] = u;
            Some(t)
        };
        Utf16 { src, table }
    }
    fn at(&self, byte: usize) -> u32 {
        let b = byte.min(self.src.len());
        match &self.table {
            None => b as u32,
            Some(t) => t[b],
        }
    }
}

/// Make every node enclose its children and stay inside its parent, in order.
fn normalize(node: &mut N, lo: usize, hi: usize) {
    node.from = node.from.clamp(lo, hi);
    node.to = node.to.clamp(node.from, hi);
    let mut cursor = node.from;
    let (f, t) = (node.from, node.to);
    for k in node.kids.iter_mut() {
        k.from = k.from.max(cursor);
        normalize(k, f, t);
        cursor = k.to.max(cursor);
    }
}

fn emit(node: &N, u: &Utf16, ids: &std::collections::HashMap<&str, u32>, out: &mut Vec<u32>) {
    let start = out.len();
    for k in &node.kids {
        emit(k, u, ids, out);
    }
    out.extend_from_slice(&[ids[node.name], u.at(node.from), u.at(node.to), 0]);
    let size = (out.len() - start) as u32;
    let last = out.len() - 1;
    out[last] = size;
}

/// Add the states of one `#+TODO:` value (`NEXT WAIT | FIN`) to `config`: everything
/// before `|` is a todo state, after it a done state. `(x)` fast keys are dropped.
fn add_todo_words(config: &mut ParseConfig, rest: &str) {
    let mut done = false;
    for w in rest.split_whitespace() {
        if w == "|" {
            done = true;
            continue;
        }
        let name = w.split('(').next().unwrap_or(w).to_string();
        if name.is_empty() {
            continue;
        }
        if done { config.todo_keywords.1.push(name) } else { config.todo_keywords.0.push(name) }
    }
}

/// The TODO words `#+TODO:`, `#+SEQ_TODO:` and `#+TYP_TODO:` lines declare, as
/// the JS parser reads them.
pub fn config_for(src: &str) -> ParseConfig {
    let mut config = ParseConfig::default();
    for line in src.lines() {
        let l = line.trim_end();
        let up = l.to_ascii_uppercase();
        let rest = ["#+TODO:", "#+SEQ_TODO:", "#+TYP_TODO:"]
            .iter()
            .find(|p| up.starts_with(*p))
            .map(|p| &l[p.len()..]);
        if let Some(rest) = rest {
            add_todo_words(&mut config, rest);
        }
    }
    config
}

/// A config from words given by the host: the values of the document's `#+TODO:`
/// lines, whitespace separated, `|` optional, lines may be joined with newlines.
/// TODO and DONE are always states.
pub fn config_with(todo_words: &str) -> ParseConfig {
    let mut config = ParseConfig::default();
    add_todo_words(&mut config, todo_words);
    config
}

/// The intermediate tree (byte offsets), for tests and the comparison tool.
pub fn tree_of(org: &Org, src: &str) -> N {
    let cx = Cx { src };
    let root = SyntaxNode::new_root(org.green().clone());
    let mut kids = vec![];
    cx.kids(&root, &mut kids);
    let mut doc = n("Document", 0, src.len(), kids);
    normalize(&mut doc, 0, src.len());
    doc
}

/// Parse and flatten: postorder `[kind, from, to, size]` words, UTF-16 offsets, root last.
pub fn parse_to_buffer(src: &str) -> Vec<u32> {
    buffer_of(src, config_for(src))
}

/// Like [`parse_to_buffer`] for a chunk of a larger document: the TODO states come
/// from the host (see [`config_with`]) instead of the chunk's own `#+TODO:` lines.
/// Offsets are relative to the chunk.
pub fn parse_chunk_to_buffer(src: &str, todo_words: &str) -> Vec<u32> {
    buffer_of(src, config_with(todo_words))
}

fn buffer_of(src: &str, config: ParseConfig) -> Vec<u32> {
    let org = config.parse(src);
    let doc = tree_of(&org, src);
    let ids: std::collections::HashMap<&str, u32> =
        NAMES.split_whitespace().enumerate().map(|(i, n)| (n, i as u32)).collect();
    // The root is left out: `Tree.build({ topID })` supplies the Document node,
    // and a root in the buffer would nest a second Document inside it.
    let mut out = Vec::new();
    let u = Utf16::new(src);
    for k in &doc.kids {
        emit(k, &u, &ids, &mut out);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn root_covers_all_words() {
        let b = parse_to_buffer("* TODO a *b* é\nx\n");
        // One top-level Section: its size covers the whole buffer.
        assert_eq!(*b.last().unwrap() as usize, b.len());
        assert_eq!(b.len() % 4, 0);
    }

    fn names_of(b: &[u32]) -> Vec<&'static str> {
        let names: Vec<&str> = NAMES.split_whitespace().collect();
        b.chunks(4).map(|w| names[w[0] as usize]).collect()
    }

    #[test]
    fn chunk_with_host_todo_words_is_one_section() {
        let b = parse_chunk_to_buffer("* NEXT a
text
** WAIT b
", "NEXT WAIT | FIN");
        let n = names_of(&b);
        assert_eq!(n.last(), Some(&"Section"));
        assert_eq!(n.iter().filter(|x| **x == "TodoKeyword").count(), 2);
        assert_eq!(*b.last().unwrap() as usize, b.len());
        // without the words NEXT is plain title text
        let b = parse_chunk_to_buffer("* NEXT a
", "");
        assert!(!names_of(&b).contains(&"TodoKeyword"));
    }

    #[test]
    fn citation_keys_and_unclosed_block() {
        let n = names_of(&parse_to_buffer("[cite:@a;@b]

#+begin_src c
x
"));
        assert_eq!(n.iter().filter(|x| **x == "CitationKey").count(), 2);
        assert!(n.contains(&"Block") && !n.contains(&"BlockEnd"));
    }
}
