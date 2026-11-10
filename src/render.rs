//! Org-mode, rendered to the page a document is stored as.
//!
//! The HTML exporter in `crate::export` does most of the work. This module
//! takes over the few places a LibrePaper document needs more: headlines show
//! their TODO keyword, priority and planning; math is carried as TeX for the host to typeset; and the
//! figures a document names are pointed at what the host says is there.
//!
//! Rendering at publish time rather than per request keeps one thing true that
//! everything else depends on: a document is HTML, addressed by the hash of the
//! bytes actually served. The reader anchors comments into those bytes, so they
//! must not change underneath a comment.

use std::cmp::min;

use rowan::ast::AstNode;

use crate::ast::{Clock, Paragraph, Planning, Timestamp};
use crate::export::{Container, Event, HtmlExport, TraversalContext, Traverser};
use crate::page;
use crate::{Org, SyntaxElement, SyntaxKind, SyntaxNode};

/// Where a figure in the document actually lives. An Org document names its
/// figures by path -- `[[file:fig/one.png]]` -- and a path means nothing to a
/// browser showing a page that was never written to a disk. The host is the
/// only thing that knows where those bytes ended up, so it answers.
pub type Resolve<'a> = &'a dyn Fn(&str) -> Option<String>;

/// Nothing beside the document, which is what a one-file Org source has
/// always compiled against.
pub fn no_assets(_: &str) -> Option<String> {
    None
}

/// Says whether a filename is one this renders.
pub fn is_org(name: &str) -> bool {
    name.to_lowercase().ends_with(".org")
}

/// The value of the first `#+TITLE:` keyword, or else the text of the first
/// top-level headline, or else nothing.
pub fn title_of(source: &str) -> String {
    let org = Org::parse(source);
    let document = org.document();
    let keyword = document
        .keywords()
        .filter(|kw| kw.key().eq_ignore_ascii_case("TITLE"))
        .map(|kw| {
            // Markup in the value is shown as the text it marks up.
            match kw
                .syntax()
                .children()
                .find(|n| n.kind() == SyntaxKind::KEYWORD_VALUE)
            {
                Some(value) => plain_text(value.children_with_tokens()),
                None => kw.value().trim().to_string(),
            }
        })
        .find(|value| !value.is_empty());
    if let Some(title) = keyword {
        return title;
    }
    document
        .headlines()
        .next()
        .map(|headline| plain_text(headline.title()))
        .unwrap_or_default()
}

/// The text of some elements with their markup removed: what a title reads as
/// when it is shown in a tab or a list.
fn plain_text(elements: impl Iterator<Item = SyntaxElement>) -> String {
    let mut out = String::new();
    for element in elements {
        match element {
            SyntaxElement::Token(token) => {
                if matches!(token.kind(), SyntaxKind::TEXT | SyntaxKind::WHITESPACE) {
                    out.push_str(token.text());
                }
            }
            SyntaxElement::Node(node) => push_text_tokens(&node, &mut out),
        }
    }
    out.trim().to_string()
}

fn push_text_tokens(node: &SyntaxNode, out: &mut String) {
    for element in node.descendants_with_tokens() {
        if let SyntaxElement::Token(token) = element {
            if matches!(token.kind(), SyntaxKind::TEXT | SyntaxKind::WHITESPACE) {
                out.push_str(token.text());
            } else if token.kind() == SyntaxKind::LINK_PATH {
                // A link with no description reads as its target.
                let described = token
                    .parent()
                    .is_some_and(|link| {
                        link.children_with_tokens()
                            .any(|e| e.kind() == SyntaxKind::L_BRACKET)
                    });
                if !described {
                    out.push_str(token.text());
                }
            }
        }
    }
}

/// Turns an Org source into the HTML of its body, with no page around it.
pub fn render_body(source: &str) -> String {
    let org = Org::parse(source);
    let mut render = Render::default();
    org.traverse(&mut render);
    render.html.finish()
}

/// Turns an Org source into the standalone HTML page a document is stored as.
pub fn render(source: &str, title: &str) -> String {
    render_with(source, title, &no_assets)
}

/// The same, with somewhere for the figures to come from.
pub fn render_with(source: &str, title: &str, assets: Resolve) -> String {
    let body = rewrite_images(&render_body(source), assets);
    page::page(title, "", &body)
}

/// Points every relative `src` at what the host says is there, and leaves
/// everything else alone: an absolute URL is the author's own and a data URL
/// is already the bytes. A path the host has nothing for is left as written,
/// so a figure that has not arrived yet is a broken image rather than a
/// rewritten one pointing nowhere.
pub fn rewrite_images(html: &str, assets: Resolve) -> String {
    let mut out = String::with_capacity(html.len());
    let mut rest = html;
    while let Some(at) = rest.find("src=\"") {
        let (before, after) = rest.split_at(at + 5);
        out.push_str(before);
        let Some(end) = after.find('"') else {
            rest = after;
            break;
        };
        let (path, tail) = after.split_at(end);
        // The path is HTML-escaped by the exporter, so the few entities it can
        // carry are undone before it is compared with the names the document
        // knows.
        let decoded = path
            .replace("&amp;", "&")
            .replace("&lt;", "<")
            .replace("&gt;", ">")
            .replace("&quot;", "\"")
            .replace("&#39;", "'");
        let external = decoded.contains("://")
            || decoded.starts_with("data:")
            || decoded.starts_with("blob:")
            || decoded.starts_with('/')
            || decoded.starts_with('#');
        // Only the path component is decoded, and only for a destination this
        // function is about to hand to the resolver: a query or fragment is not
        // part of a filename and stays as written, and a destination that fails
        // to decode as UTF-8 is looked up by its raw, still-encoded spelling
        // rather than dropped.
        let lookup = if external {
            decoded.clone()
        } else {
            let (path_only, _query_or_fragment) = split_path_query_fragment(&decoded);
            percent_decode_path(path_only)
        };
        match (external, assets(&lookup)) {
            (false, Some(url)) => out.push_str(&crate::page::escape(&url)),
            _ => out.push_str(path),
        }
        rest = tail;
    }
    out.push_str(rest);
    out
}

/// Splits a URL-shaped string at its first `?` or `#`, so the path can be
/// decoded without touching a query string or fragment that happens to
/// follow it.
fn split_path_query_fragment(s: &str) -> (&str, &str) {
    match s.find(['?', '#']) {
        Some(i) => (&s[..i], &s[i..]),
        None => (s, ""),
    }
}

/// A small, dependency-free percent-decoder for the path component of a
/// figure's destination. A destination that decodes to invalid UTF-8 is
/// returned unchanged: a raw, still-encoded lookup that fails is never worse
/// than a panic.
fn percent_decode_path(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(hi), Some(lo)) = (hex_digit(bytes[i + 1]), hex_digit(bytes[i + 2])) {
                out.push(hi * 16 + lo);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8(out).unwrap_or_else(|_| s.to_string())
}

/// One hex nibble, or nothing if the byte is not one.
fn hex_digit(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

/// The same, in the shape every renderer answers in.
pub fn compile(source: &str, title: &str) -> crate::diagnostic::Compiled {
    compile_with(source, title, &no_assets)
}

pub fn compile_with(source: &str, title: &str, assets: Resolve) -> crate::diagnostic::Compiled {
    crate::diagnostic::Compiled::page(render_with(source, title, assets))
}

/// The plain exporter, with the pieces a document needs changed. Every event
/// this does not handle goes to the exporter unchanged.
#[derive(Default)]
struct Render {
    html: HtmlExport,
    /// Labelled footnotes, by label, with the number each was given.
    numbers: Vec<(String, usize)>,
    /// How many footnote numbers have been handed out.
    count: usize,
    /// Bodies of `[fn:LABEL] text` definitions, by label.
    definitions: Vec<(String, String)>,
    /// Bodies of inline footnotes, by number.
    bodies: Vec<(usize, String)>,
    /// Footnote bodies being rendered, innermost last.
    captures: Vec<Capture>,
}

/// Where the body captured for a footnote element goes when the element ends.
enum Target {
    /// A `[fn:LABEL] text` definition.
    Label(String),
    /// The body of a numbered footnote, from an inline reference.
    Number(usize),
    /// A body with nowhere to go.
    Discard,
}

/// A footnote body being rendered: the output length it starts at, and where
/// it goes.
struct Capture {
    mark: usize,
    target: Target,
}

/// Whether a footnote reference has text of its own, as `[fn:LABEL:text]` does.
fn has_text(reference: &SyntaxNode) -> bool {
    reference
        .children()
        .any(|child| child.kind() == SyntaxKind::FN_CONTENT)
}

/// Splits a raw LaTeX fragment into its display style and its TeX, with the
/// delimiters taken off. A fragment that matches none of the delimiter pairs
/// is kept whole and set inline.
fn math_parts(raw: &str) -> (&'static str, &str) {
    let raw = raw.trim();
    if let Some(tex) = raw.strip_prefix("$$").and_then(|s| s.strip_suffix("$$")) {
        return ("display", tex.trim());
    }
    if let Some(tex) = raw.strip_prefix("\\[").and_then(|s| s.strip_suffix("\\]")) {
        return ("display", tex.trim());
    }
    if let Some(tex) = raw.strip_prefix("\\(").and_then(|s| s.strip_suffix("\\)")) {
        return ("inline", tex.trim());
    }
    if let Some(tex) = raw.strip_prefix('$').and_then(|s| s.strip_suffix('$')) {
        return ("inline", tex.trim());
    }
    ("inline", raw)
}

impl Render {
    /// One clock entry, as a paragraph of its own raw text.
    fn clock(&mut self, clock: &Clock) {
        self.html.push_str(format!(
            "<p class=\"clock\">{}</p>",
            page::escape(clock.syntax().to_string().trim())
        ));
    }

    /// The planning line of a headline, as one paragraph. Nothing is written
    /// when no planning timestamp is set.
    fn planning(&mut self, planning: &Planning) {
        let stamps: [(&str, &str, Option<Timestamp>); 3] = [
            ("planning-scheduled", "SCHEDULED", planning.scheduled()),
            ("planning-deadline", "DEADLINE", planning.deadline()),
            ("planning-closed", "CLOSED", planning.closed()),
        ];
        let mut spans = String::new();
        for (class, label, stamp) in stamps {
            if let Some(stamp) = stamp {
                spans.push_str(&format!(
                    "<span class=\"{class}\">{label}: {}</span>",
                    page::escape(stamp.syntax().to_string().trim())
                ));
            }
        }
        if !spans.is_empty() {
            self.html
                .push_str(format!("<p class=\"planning\">{spans}</p>"));
        }
    }

    /// The number of a labelled footnote, given on its first reference. The
    /// flag says whether this reference is that first one.
    fn number_of(&mut self, label: &str) -> (usize, bool) {
        if let Some((_, n)) = self.numbers.iter().find(|(known, _)| known.as_str() == label) {
            return (*n, false);
        }
        let n = self.fresh_number();
        self.numbers.push((label.to_string(), n));
        (n, true)
    }

    fn fresh_number(&mut self) -> usize {
        self.count += 1;
        self.count
    }

    /// The reference marker for footnote `n`. Only the first reference carries
    /// the anchor that the notes link back to.
    fn footnote_marker(&mut self, n: usize, first: bool) {
        let id = if first {
            format!(" id=\"fnref-{n}\"")
        } else {
            String::new()
        };
        self.html.push_str(format!(
            "<sup class=\"footnote-ref\"><a href=\"#fn-{n}\"{id}>{n}</a></sup>"
        ));
    }

    /// Ends the innermost footnote body being rendered and keeps it.
    fn end_capture(&mut self) {
        let Some(capture) = self.captures.pop() else {
            return;
        };
        let body = self.html.split_off(capture.mark).trim().to_string();
        match capture.target {
            Target::Label(label) => self.definitions.push((label, body)),
            Target::Number(n) => self.bodies.push((n, body)),
            Target::Discard => {}
        }
    }

    /// The body shown for footnote `n` in the notes: its inline text, else the
    /// definition of its label, else nothing.
    fn body_of(&self, n: usize) -> String {
        if let Some((_, body)) = self.bodies.iter().find(|(k, _)| *k == n) {
            return body.clone();
        }
        self.numbers
            .iter()
            .find(|(_, k)| *k == n)
            .and_then(|(label, _)| self.definitions.iter().find(|(d, _)| d == label))
            .map(|(_, body)| body.clone())
            .unwrap_or_default()
    }

    /// The notes at the end of the document: one item per footnote number, in
    /// order. Nothing is written when no footnote was referenced.
    fn footnotes(&mut self) {
        if self.count == 0 {
            return;
        }
        self.html.push_str("<section class=\"footnotes\"><ol>");
        for n in 1..=self.count {
            let body = self.body_of(n);
            self.html.push_str(format!(
                "<li id=\"fn-{n}\">{body} <a href=\"#fnref-{n}\" class=\"footnote-back\">↩</a></li>"
            ));
        }
        self.html.push_str("</ol></section>");
    }

    /// A caption's text as inline Org, rendered through this traverser so that
    /// math and markup in it come out as they do in the body. Text that does
    /// not parse as a paragraph is written escaped.
    fn inline_org(&mut self, text: &str, ctx: &mut TraversalContext) {
        let org = Org::parse(text);
        match org.first_node::<Paragraph>() {
            Some(paragraph) => {
                for child in paragraph.syntax().children_with_tokens() {
                    self.element(child, ctx);
                }
            }
            None => self.html.push_str(page::escape(text)),
        }
    }
}

impl Traverser for Render {
    fn event(&mut self, event: Event, ctx: &mut TraversalContext) {
        match event {
            Event::Enter(Container::Headline(headline)) => {
                let level = min(headline.level(), 6);
                self.html.push_str(format!("<h{level}>"));
                if let Some(keyword) = headline.todo_keyword() {
                    self.html.push_str(format!(
                        "<span class=\"todo\">{}</span> ",
                        page::escape(&keyword)
                    ));
                }
                if let Some(priority) = headline.priority() {
                    self.html.push_str(format!(
                        "<span class=\"priority\">[#{}]</span> ",
                        page::escape(&priority)
                    ));
                }
                // The title is walked through this traverser, so markup nested
                // in it still comes through this handler.
                for elem in headline.title() {
                    self.element(elem, ctx);
                }
                self.html.push_str(format!("</h{level}>"));
                if let Some(planning) = headline.planning() {
                    self.planning(&planning);
                }
            }

            Event::Enter(Container::OrgTable(table)) => {
                self.html.enter_table(&table);
                if let Some(caption) = table.caption().and_then(|k| k.value()) {
                    self.html.push_str("<caption>");
                    self.inline_org(caption.trim(), ctx);
                    self.html.push_str("</caption>");
                }
            }

            Event::Enter(Container::FnRef(reference)) => {
                let label = reference
                    .label()
                    .map(|label| label.syntax().text().to_string())
                    .unwrap_or_default();
                let text = has_text(reference.syntax());
                // An anonymous footnote, `[fn::text]`, is always a new note.
                let (n, first) = if label.is_empty() {
                    (self.fresh_number(), true)
                } else {
                    self.number_of(&label)
                };
                self.footnote_marker(n, first);
                if text {
                    let mark = self.html.len();
                    self.captures.push(Capture {
                        mark,
                        target: Target::Number(n),
                    });
                }
            }
            Event::Leave(Container::FnRef(reference)) => {
                if has_text(reference.syntax()) {
                    self.end_capture();
                }
            }

            // A definition is shown in the notes at the end, not where it stands.
            Event::Enter(Container::FnDef(definition)) => {
                let target = match definition.label() {
                    Some(label) => Target::Label(label.syntax().text().to_string()),
                    None => Target::Discard,
                };
                let mark = self.html.len();
                self.captures.push(Capture { mark, target });
            }
            Event::Leave(Container::FnDef(_)) => self.end_capture(),

            // The body of a footnote is captured as it is traversed, so its
            // delimiters and label add nothing of their own.
            Event::Enter(Container::FnContent(_)) | Event::Leave(Container::FnContent(_)) => {}
            Event::FnLabel(_) => {}

            Event::LatexFragment(latex) => {
                let raw = latex.syntax().to_string();
                let (style, tex) = math_parts(&raw);
                self.html.push_str(format!(
                    "<span data-math-style=\"{style}\">{}</span>",
                    page::escape(tex)
                ));
            }

            Event::LatexEnvironment(latex) => {
                // The whole `\begin{..}...\end{..}` is the TeX, so it is kept
                // as it is written.
                let raw = latex.syntax().to_string();
                self.html.push_str(format!(
                    "<span data-math-style=\"display\">{}</span>",
                    page::escape(raw.trim())
                ));
            }

            Event::Clock(clock) => self.clock(&clock),

            // The page template supplies the `<body>`, so a document adds no
            // wrapper of its own. Its footnotes come last.
            Event::Enter(Container::Document(_)) => {}
            Event::Leave(Container::Document(_)) => self.footnotes(),

            // A LOGBOOK drawer is the one drawer whose clock entries are shown.
            // Its contents are emitted here and then skipped, so they are not
            // walked again. Skipping an Enter also suppresses its Leave, so no
            // closing tag is needed for a drawer handled this way.
            Event::Enter(Container::Drawer(drawer)) => {
                if drawer.name().eq_ignore_ascii_case("LOGBOOK") {
                    let clocks: Vec<Clock> = drawer
                        .syntax()
                        .descendants()
                        .filter_map(Clock::cast)
                        .collect();
                    if !clocks.is_empty() {
                        self.html.push_str("<div class=\"logbook\">");
                        for clock in &clocks {
                            self.clock(clock);
                        }
                        self.html.push_str("</div>");
                    }
                    ctx.skip();
                } else {
                    self.html.event(Event::Enter(Container::Drawer(drawer)), ctx);
                }
            }

            other => self.html.event(other, ctx),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_paragraph_is_a_plain_p_element() {
        let out = render_body("Hello.\n");
        assert!(out.contains("<p>Hello."), "{out}");
        assert!(!out.contains("data-source"), "{out}");
    }

    #[test]
    fn a_table_caption_renders_math_and_markup() {
        let out = render_body("#+CAPTION: Error by $\\tau$ in *bold*\n| a | b |\n| 1 | 2 |\n");
        assert!(out.contains("<caption>"), "{out}");
        assert!(
            out.contains("<span data-math-style=\"inline\">\\tau</span>"),
            "{out}"
        );
        assert!(out.contains("<b>bold</b>"), "{out}");
        assert!(!out.contains('$'), "{out}");
    }

    #[test]
    fn footnotes_are_numbered_and_collected_at_the_end() {
        let out = render_body(
            "A[fn:x] and B[fn:y] and again[fn:x].\n\n[fn:y] Why.\n\n[fn:x] Ex *bold*.\n",
        );
        assert!(out.contains("href=\"#fn-1\" id=\"fnref-1\">1</a>"), "{out}");
        assert!(out.contains("href=\"#fn-2\" id=\"fnref-2\">2</a>"), "{out}");
        assert_eq!(out.matches("id=\"fnref-1\"").count(), 1, "{out}");
        assert!(!out.contains("footnote-definition"), "{out}");

        let notes = out
            .find("<section class=\"footnotes\">")
            .expect("a footnotes section");
        assert!(out[..notes].contains("again"), "{out}");
        let one = out.find("<li id=\"fn-1\">").expect("note 1");
        let two = out.find("<li id=\"fn-2\">").expect("note 2");
        assert!(notes < one && one < two, "{out}");
        assert!(out[one..two].contains("Ex <b>bold</b>"), "{out}");
        assert!(out[two..].contains("Why."), "{out}");
    }

    #[test]
    fn an_inline_footnote_is_numbered_and_its_text_moves_to_the_notes() {
        let out = render_body("A[fn::anon note] b.\n");
        assert_eq!(
            out.matches("href=\"#fn-1\" id=\"fnref-1\">1</a>").count(),
            1,
            "{out}"
        );
        let notes = out
            .find("<section class=\"footnotes\">")
            .expect("a footnotes section");
        assert!(!out[..notes].contains("anon note"), "{out}");
        let one = out.find("<li id=\"fn-1\">").expect("note 1");
        assert!(out[one..].contains("anon note"), "{out}");
    }

    #[test]
    fn a_document_without_footnotes_has_no_notes_section() {
        assert!(!render_body("Hello.\n").contains("class=\"footnotes\""));
    }

    #[test]
    fn a_headline_shows_its_todo_keyword_and_priority() {
        let out = render_body("* TODO [#A] Write the paper\n");
        assert!(out.contains("<span class=\"todo\">TODO</span> "), "{out}");
        assert!(
            out.contains("<span class=\"priority\">[#A]</span> "),
            "{out}"
        );
    }

    #[test]
    fn math_is_carried_as_tex_in_a_tagged_span() {
        let out = render_body("Let $x^2$ hold.\n\n\\[ y \\]\n");
        assert!(
            out.contains("<span data-math-style=\"inline\">x^2</span>"),
            "inline math was not kept as TeX: {out}"
        );
        assert!(
            out.contains("<span data-math-style=\"display\">y</span>"),
            "display math was not kept as TeX: {out}"
        );
    }

    #[test]
    fn title_is_the_title_keyword_else_the_first_headline() {
        assert_eq!(title_of("#+TITLE: Hello\n* Other\n"), "Hello");
        assert_eq!(title_of("#+title: lower case\n"), "lower case");
        assert_eq!(title_of("* First heading\n** sub\n"), "First heading");
        assert_eq!(title_of("just a paragraph\n"), "");
    }

    #[test]
    fn detects_org_by_extension() {
        for (name, want) in [
            ("paper.org", true),
            ("PAPER.ORG", true),
            ("paper.org.html", false),
            ("org", false),
            ("paper.md", false),
        ] {
            assert_eq!(is_org(name), want, "{name}");
        }
    }

    #[test]
    fn a_planning_line_renders_a_planning_paragraph() {
        let out = render_body("* Task\nSCHEDULED: <2026-01-02 Fri> DEADLINE: <2026-01-05 Mon>\n");
        assert!(out.contains("<p class=\"planning\">"), "{out}");
        assert!(out.contains("planning-scheduled"), "{out}");
        assert!(out.contains("planning-deadline"), "{out}");
    }

    #[test]
    fn logbook_clocks_render_inside_a_logbook_div() {
        let out = render_body(
            "* TODO Task\nSCHEDULED: <2026-11-12 Thu>\n:LOGBOOK:\n\
             CLOCK: [2026-11-09 Mon 09:00]--[2026-11-09 Mon 10:30] =>  1:30\n\
             CLOCK: [2026-11-10 Tue 14:00]--[2026-11-10 Tue 15:15] =>  1:15\n\
             :END:\nBody.\n",
        );
        assert!(out.contains("<div class=\"logbook\">"), "{out}");
        assert_eq!(out.matches("<p class=\"clock\">").count(), 2, "{out}");
        assert_eq!(out.matches("<p class=\"clock\">CLOCK:").count(), 2, "{out}");
        assert!(out.contains("Body."), "{out}");
    }

    #[test]
    fn a_bare_clock_line_still_renders_a_clock_paragraph() {
        let out = render_body("* Task\nCLOCK: [2026-11-09 Mon 09:00]--[2026-11-09 Mon 10:30] =>  1:30\n");
        assert_eq!(out.matches("<p class=\"clock\">").count(), 1, "{out}");
    }

    #[test]
    fn other_drawers_render_no_logbook_div() {
        let properties = render_body("* Task\n:PROPERTIES:\n:ID: abc\n:END:\n");
        assert!(!properties.contains("class=\"logbook\""), "{properties}");

        let notes = render_body("* Task\n:NOTES:\nA note.\n:END:\n");
        assert!(!notes.contains("class=\"logbook\""), "{notes}");
    }

    #[test]
    fn markup_in_title_like_keywords_is_parsed() {
        use crate::ast::Keyword;
        for key in ["TITLE", "SUBTITLE", "AUTHOR", "DATE", "CAPTION"] {
            let org = Org::parse(&format!("#+{key}: A *bold* [[https://x.org][l]] $x$ \\alpha\n"));
            let kw = org.first_node::<Keyword>().expect(key);
            let value = kw
                .syntax()
                .children()
                .find(|n| n.kind() == SyntaxKind::KEYWORD_VALUE)
                .unwrap_or_else(|| panic!("{key} has no KEYWORD_VALUE"));
            let kinds: Vec<_> = value.children().map(|n| n.kind()).collect();
            assert_eq!(
                kinds,
                [
                    SyntaxKind::BOLD,
                    SyntaxKind::LINK,
                    SyntaxKind::LATEX_FRAGMENT,
                    SyntaxKind::ENTITY
                ],
                "{key}"
            );
            assert_eq!(kw.value(), " A *bold* [[https://x.org][l]] $x$ \\alpha");
        }
        // other keywords stay raw
        let org = Org::parse("#+OPTIONS: *bold* https://x.org\n");
        let kw = org.first_node::<crate::ast::Keyword>().unwrap();
        assert!(kw.syntax().children().next().is_none());
    }

    #[test]
    fn a_title_reads_without_its_markup() {
        assert_eq!(title_of("#+TITLE: A *bold* /title/ at https://x.org\n"), "A bold title at https://x.org");
        assert_eq!(title_of("#+TITLE: See [[https://x.org][the site]]\n"), "See the site");
    }

    #[test]
    fn a_caption_with_a_link_renders_the_anchor() {
        let out = render_body("#+CAPTION: From [[https://x.org][x]] and https://y.org\n| a |\n");
        assert!(out.contains("<caption>"), "{out}");
        assert!(out.contains("<a href=\"https://x.org\">x</a>"), "{out}");
        assert!(out.contains("<a href=\"https://y.org\">"), "{out}");
    }

    #[test]
    fn a_table_formula_belongs_to_the_table_and_is_not_shown() {
        let source = "| 1 | 2 |\n| 3 | 4 |\n#+TBLFM: $2=$1*2\n\nAfter.\n";
        let org = Org::parse(source);
        let table = org.first_node::<crate::ast::OrgTable>().unwrap();
        let kinds: Vec<_> = table.syntax().children().map(|n| n.kind()).collect();
        assert_eq!(kinds.last(), Some(&SyntaxKind::TABLE_FORMULA), "{kinds:?}");
        assert_eq!(table.tblfm().count(), 1);
        let out = render_body(source);
        assert!(!out.contains("TBLFM"), "{out}");
        assert!(!out.contains("$2=$1*2"), "{out}");
        assert!(out.contains("After."), "{out}");
    }

    #[test]
    fn a_body_has_no_main_wrapper() {
        assert!(!render_body("Hello.\n").contains("<main>"));
    }
}
