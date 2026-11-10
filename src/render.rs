//! Org-mode, rendered to the page a document is stored as.
//!
//! The HTML exporter in `crate::export` does most of the work. This module
//! takes over the few places a LibrePaper document needs more: blocks record
//! where they sit in the source; headlines show their TODO keyword, priority
//! and planning; math is carried as TeX for the host to typeset; and the
//! figures a document names are pointed at what the host says is there.
//!
//! Rendering at publish time rather than per request keeps one thing true that
//! everything else depends on: a document is HTML, addressed by the hash of the
//! bytes actually served. The reader anchors comments into those bytes, so they
//! must not change underneath a comment.

use std::cmp::min;

use rowan::ast::AstNode;

use crate::ast::{List, Planning, Timestamp};
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
        .map(|kw| kw.value().trim().to_string())
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
            }
        }
    }
}

/// Turns an Org source into the HTML of its body, with no page around it.
pub fn render_body(source: &str) -> String {
    let org = Org::parse(source);
    let mut render = Render {
        html: HtmlExport::default(),
    };
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
struct Render {
    html: HtmlExport,
}

/// The `data-source` attribute for a node: its byte range in the source.
fn source_attr(node: &SyntaxNode) -> String {
    let range = node.text_range();
    format!(
        "data-source=\"{}-{}\"",
        u32::from(range.start()),
        u32::from(range.end())
    )
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
}

impl Traverser for Render {
    fn event(&mut self, event: Event, ctx: &mut TraversalContext) {
        match event {
            Event::Enter(Container::Headline(headline)) => {
                let level = min(headline.level(), 6);
                self.html
                    .push_str(format!("<h{level} {}>", source_attr(headline.syntax())));
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

            Event::Enter(Container::Paragraph(paragraph)) => {
                self.html
                    .push_str(format!("<p {}>", source_attr(paragraph.syntax())));
            }

            Event::Enter(Container::ListItem(item)) => {
                // The exporter keeps the descriptive-list state privately, and
                // a description item's `<dt>`/`<dd>` pair is its own. Only an
                // ordinary item, whose parent list is not descriptive, is
                // opened here.
                let descriptive = item
                    .syntax()
                    .parent()
                    .and_then(List::cast)
                    .is_some_and(|list| list.is_descriptive());
                if descriptive {
                    self.html
                        .event(Event::Enter(Container::ListItem(item)), ctx);
                } else {
                    self.html
                        .push_str(format!("<li {}>", source_attr(item.syntax())));
                }
            }

            Event::Enter(Container::QuoteBlock(block)) => {
                self.html
                    .push_str(format!("<blockquote {}>", source_attr(block.syntax())));
            }

            Event::Enter(Container::ExampleBlock(block)) => {
                self.html.push_str(format!(
                    "<pre class=\"example\" {}>",
                    source_attr(block.syntax())
                ));
            }

            Event::Enter(Container::SourceBlock(block)) => {
                let attr = source_attr(block.syntax());
                match block.language() {
                    Some(language) => self.html.push_str(format!(
                        "<pre {attr}><code class=\"language-{}\">",
                        page::escape(&language)
                    )),
                    None => self.html.push_str(format!("<pre {attr}><code>")),
                }
            }

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

            Event::Clock(clock) => {
                self.html.push_str(format!(
                    "<p class=\"clock\">{}</p>",
                    page::escape(clock.syntax().to_string().trim())
                ));
            }

            other => self.html.event(other, ctx),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_paragraph_carries_its_source_range() {
        let out = render_body("First line.\n");
        assert!(out.contains("<p data-source=\"0-"), "{out}");
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
}
