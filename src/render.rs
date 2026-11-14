//! Org-mode, rendered to the page a document is stored as.
//!
//! The HTML exporter in `crate::export` does most of the work. This module
//! takes over the few places a LibrePaper document needs more: headlines show
//! their TODO keyword, priority and planning; math is carried as TeX for the
//! host to typeset; the figures a document names are pointed at what the host
//! says is there; and the title, subtitle, author and date keywords open the
//! page as a title block.
//!
//! Rendering at publish time rather than per request keeps one thing true that
//! everything else depends on: a document is HTML, addressed by the hash of the
//! bytes actually served. The reader anchors comments into those bytes, so they
//! must not change underneath a comment.

use std::cmp::min;

use rowan::ast::AstNode;

use crate::ast::{AffiliatedKeyword, Citation, Clock, Document, Link, Paragraph, Planning, Timestamp};
use crate::export::{Container, Event, HtmlEscape, HtmlExport, TraversalContext, Traverser};
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
/// Placeholder citation rendering: the keys as written, parenthesised unless
/// the style is textual (`/t`, `/text`). A later module will replace this
/// with CSL-formatted output; `data-keys` is what that module will read.
pub(crate) fn citation_html(citation: &Citation) -> String {
    use crate::SyntaxKind::*;
    let text_of = |n: &SyntaxNode| page::escape(n.text().to_string().trim()).to_string();
    let (mut keys, mut shown) = (Vec::new(), Vec::new());
    let (mut head, mut tail) = (String::new(), String::new());
    for child in citation.syntax().children() {
        match child.kind() {
            CITATION_PREFIX => head = text_of(&child),
            CITATION_SUFFIX => tail = text_of(&child),
            CITATION_REFERENCE => {
                let mut one = String::new();
                for part in child.children_with_tokens() {
                    match part.kind() {
                        CITATION_PREFIX | CITATION_SUFFIX => {
                            let t = match &part {
                                SyntaxElement::Node(n) => text_of(n),
                                _ => String::new(),
                            };
                            if t.is_empty() {
                            } else if part.kind() == CITATION_PREFIX {
                                one.push_str(&t);
                                one.push(' ');
                            } else if t.starts_with(|c: char| c.is_alphanumeric()) {
                                one.push(' ');
                                one.push_str(&t);
                            } else {
                                one.push_str(&t);
                            }
                        }
                        CITATION_KEY => {
                            if let SyntaxElement::Token(t) = &part {
                                keys.push(t.text().to_string());
                                one.push_str(&page::escape(t.text()));
                            }
                        }
                        _ => {}
                    }
                }
                shown.push(one);
            }
            _ => {}
        }
    }
    let mut body = shown.join("; ");
    if !head.is_empty() {
        body = format!("{head} {body}");
    }
    if !tail.is_empty() {
        body = format!("{body}{}{tail}", if tail.starts_with(|c: char| c.is_alphanumeric()) { " " } else { "" });
    }
    let textual = matches!(citation.style().as_deref().map(|s| s.split('/').next().unwrap_or("")), Some("t" | "text"));
    let body = if textual { body } else { format!("({body})") };
    format!(
        "<cite class=\"citation\" data-keys=\"{}\">{body}</cite>",
        page::escape(&keys.join(" "))
    )
}

fn has_text(reference: &SyntaxNode) -> bool {
    reference
        .children()
        .any(|child| child.kind() == SyntaxKind::FN_CONTENT)
}

/// A paragraph that is a figure: an image link, with the affiliated keywords
/// that describe it.
struct Figure {
    /// The link's path, with a leading `file:` taken off.
    path: String,
    caption: Option<String>,
    name: Option<String>,
    alt: Option<String>,
    /// The width, as a CSS length.
    width: Option<String>,
    /// The height, as a CSS length.
    height: Option<String>,
}

impl Figure {
    /// Whether the keywords say anything about the figure. A bare image with
    /// none is written as an ordinary paragraph.
    fn is_marked(&self) -> bool {
        self.caption.is_some()
            || self.name.is_some()
            || self.alt.is_some()
            || self.width.is_some()
            || self.height.is_some()
    }
}

/// The figure a paragraph is, if it is one. Apart from its affiliated keywords
/// and line breaks, a figure holds exactly one image link and nothing else.
fn figure_of(paragraph: &Paragraph) -> Option<Figure> {
    let mut links = Vec::new();
    for child in paragraph.syntax().children_with_tokens() {
        match child {
            SyntaxElement::Token(token)
                if matches!(
                    token.kind(),
                    SyntaxKind::WHITESPACE | SyntaxKind::NEW_LINE | SyntaxKind::BLANK_LINE
                ) => {}
            // The line break after the link is text of the paragraph.
            SyntaxElement::Token(token)
                if token.kind() == SyntaxKind::TEXT && token.text().trim().is_empty() => {}
            SyntaxElement::Node(node) if node.kind() == SyntaxKind::AFFILIATED_KEYWORD => {}
            SyntaxElement::Node(node) if node.kind() == SyntaxKind::LINK => links.push(node),
            _ => return None,
        }
    }
    let link = Link::cast(links.pop()?)?;
    if !links.is_empty() || !link.is_image() {
        return None;
    }

    // Keywords are matched without regard to case, as Org matches them. The
    // generated accessors compare with upper-case names only.
    let keywords: Vec<AffiliatedKeyword> = paragraph
        .syntax()
        .children()
        .filter_map(AffiliatedKeyword::cast)
        .collect();
    let keyword = |key: &str| {
        keywords
            .iter()
            .find(|k| k.key().eq_ignore_ascii_case(key))
            .and_then(|k| k.value())
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
    };
    let attr = |backend: &str, key: &str| {
        keywords
            .iter()
            .filter(|k| k.key().eq_ignore_ascii_case(backend))
            .filter_map(|k| k.value())
            .find_map(|line| plist_value(&line, key))
    };
    // A size that is not safe to write is ignored, so the next source in the
    // precedence order is tried.
    let size = |backend: &str, key: &str| attr(backend, key).and_then(|raw| css_length(&raw));

    let path = link.path();
    Some(Figure {
        path: path.trim_start_matches("file:").to_string(),
        caption: keyword("CAPTION"),
        name: keyword("NAME"),
        alt: attr("ATTR_HTML", "alt"),
        width: size("ATTR_HTML", "width")
            .or_else(|| size("ATTR_ORG", "width"))
            .or_else(|| size("ATTR_LATEX", "width")),
        height: size("ATTR_HTML", "height"),
    })
}

/// The value of one property in an attribute line, which is a plist: `:width
/// 300px :alt A plot` gives `300px` for `width`. A value runs to the next word
/// that starts with a colon, and an empty value is nothing.
fn plist_value(line: &str, key: &str) -> Option<String> {
    let mut words = line.split_whitespace().peekable();
    while let Some(word) = words.next() {
        if word
            .strip_prefix(':')
            .is_some_and(|name| name.eq_ignore_ascii_case(key))
        {
            let mut value = Vec::new();
            while let Some(part) = words.next_if(|part| !part.starts_with(':')) {
                value.push(part);
            }
            return (!value.is_empty()).then(|| value.join(" "));
        }
    }
    None
}

/// The CSS length a figure size is written as, or nothing if the size is not
/// one that is safe to write. A bare number is in pixels, a number with a CSS
/// unit keeps it, and a LaTeX fraction of the line becomes a percentage. Only
/// digits, one dot and a known unit get through, so nothing else can reach a
/// style attribute.
fn css_length(raw: &str) -> Option<String> {
    let raw = raw.trim().to_ascii_lowercase();
    for line in ["\\textwidth", "\\linewidth", "\\columnwidth"] {
        if let Some(fraction) = raw.strip_suffix(line) {
            let fraction = if fraction.is_empty() { "1" } else { fraction };
            if !is_number(fraction) {
                return None;
            }
            let percent = fraction.parse::<f64>().ok()? * 100.0;
            // Six places, then the trailing zeros and dot are dropped.
            let text = format!("{percent:.6}");
            return Some(format!("{}%", text.trim_end_matches('0').trim_end_matches('.')));
        }
    }
    let split = raw
        .find(|c: char| !(c.is_ascii_digit() || c == '.'))
        .unwrap_or(raw.len());
    let (number, unit) = raw.split_at(split);
    if !is_number(number) {
        return None;
    }
    match unit {
        "" => Some(format!("{number}px")),
        "px" | "em" | "rem" | "%" | "cm" | "mm" | "in" | "pt" | "pc" | "ex" | "ch" | "vw"
        | "vh" => Some(format!("{number}{unit}")),
        _ => None,
    }
}

/// Whether a string is a plain decimal: at least one digit and at most one dot,
/// and the dot, if there is one, is not the last character.
fn is_number(s: &str) -> bool {
    !s.is_empty()
        && s.bytes().any(|b| b.is_ascii_digit())
        && s.bytes().all(|b| b.is_ascii_digit() || b == b'.')
        && s.matches('.').count() <= 1
        && !s.ends_with('.')
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

    /// The title block is the document's `#+TITLE:`, `#+SUBTITLE:`, `#+AUTHOR:`
    /// and `#+DATE:` keywords as the header that opens the page, with the
    /// classes the house stylesheet already styles for Quarto title blocks. A
    /// repeated keyword is joined with a space, as Org joins it. Nothing is
    /// written when none of them is set.
    fn title_block(&mut self, document: &Document, ctx: &mut TraversalContext) {
        const PARTS: [(&str, &str, &str); 4] = [
            ("TITLE", "h1", "title"),
            ("SUBTITLE", "p", "subtitle"),
            ("AUTHOR", "p", "author"),
            ("DATE", "p", "date"),
        ];
        let blank = |element: &SyntaxElement| {
            element
                .as_token()
                .is_some_and(|token| token.text().trim().is_empty())
        };

        let mark = self.html.len();
        self.html.push_str("<header class=\"title-block\">");
        let opened = self.html.len();
        for (key, tag, class) in PARTS {
            let values: Vec<SyntaxNode> = document
                .keywords()
                .filter(|kw| kw.key().eq_ignore_ascii_case(key))
                .filter_map(|kw| {
                    kw.syntax()
                        .children()
                        .find(|n| n.kind() == SyntaxKind::KEYWORD_VALUE)
                })
                .filter(|value| !value.text().to_string().trim().is_empty())
                .collect();
            if values.is_empty() {
                continue;
            }
            self.html.push_str(format!("<{tag} class=\"{class}\">"));
            for (i, value) in values.iter().enumerate() {
                if i > 0 {
                    self.html.push_str(" ");
                }
                // The value's leading whitespace is not part of its text.
                let children: Vec<SyntaxElement> = value.children_with_tokens().collect();
                let skip = children.iter().take_while(|element| blank(element)).count();
                for (j, child) in children.into_iter().skip(skip).enumerate() {
                    match child.as_token() {
                        Some(token) if j == 0 && token.text().starts_with(char::is_whitespace) => {
                            self.html.push_str(page::escape(token.text().trim_start()));
                        }
                        _ => self.element(child, ctx),
                    }
                }
            }
            self.html.push_str(format!("</{tag}>"));
        }
        if self.html.len() == opened {
            self.html.split_off(mark);
            return;
        }
        self.html.push_str("</header>");
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

    /// A figure: its image in a `<figure>` with the caption and the name as its
    /// id, or in a plain `<p>` when it has neither.
    fn figure(&mut self, figure: &Figure, ctx: &mut TraversalContext) {
        let wrapped = figure.caption.is_some() || figure.name.is_some();
        if !wrapped {
            self.html.push_str("<p>");
        } else if let Some(name) = &figure.name {
            self.html.push_str(format!(
                "<figure class=\"figure\" id=\"{}\">",
                page::escape(name)
            ));
        } else {
            self.html.push_str("<figure class=\"figure\">");
        }

        let mut image = format!("<img src=\"{}\"", HtmlEscape(&figure.path));
        if let Some(alt) = &figure.alt {
            image.push_str(&format!(" alt=\"{}\"", page::escape(alt)));
        }
        let style: Vec<String> = figure
            .width
            .iter()
            .map(|width| format!("width:{width}"))
            .chain(figure.height.iter().map(|height| format!("height:{height}")))
            .collect();
        if !style.is_empty() {
            image.push_str(&format!(" style=\"{}\"", style.join(";")));
        }
        image.push('>');
        self.html.push_str(image);

        if let Some(caption) = &figure.caption {
            self.html.push_str("<figcaption>");
            self.inline_org(caption, ctx);
            self.html.push_str("</figcaption>");
        }
        self.html.push_str(if wrapped { "</figure>" } else { "</p>" });
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

            // An image paragraph with a caption, a name or a size is a figure. It
            // is written here and its children are skipped, so they are not walked
            // again. Any other paragraph, including a bare image, is left to the
            // exporter exactly as before.
            Event::Enter(Container::Paragraph(paragraph)) => match figure_of(&paragraph) {
                Some(figure) if figure.is_marked() => {
                    self.figure(&figure, ctx);
                    ctx.skip();
                }
                _ => self.html.event(Event::Enter(Container::Paragraph(paragraph)), ctx),
            },

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
            // wrapper of its own. Its title block comes first and its footnotes
            // last.
            Event::Enter(Container::Document(document)) => self.title_block(&document, ctx),
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
    fn the_title_keywords_open_the_page_as_a_title_block() {
        let out = render_body(
            "#+TITLE: Learn *LibrePaper* with Org\n#+AUTHOR: LibrePaper\n#+bibliography: references.bib\n\nBody text.\n",
        );
        assert!(out.starts_with("<header class=\"title-block\">"), "{out}");
        assert!(
            out.contains("<h1 class=\"title\">Learn <b>LibrePaper</b> with Org</h1>"),
            "{out}"
        );
        assert!(out.contains("<p class=\"author\">LibrePaper</p>"), "{out}");
        assert!(out.contains("</header>"), "{out}");
        assert!(!out.contains("references.bib"), "{out}");
        let close = out.find("</header>").expect("a closed header");
        let body = out.find("Body text.").expect("the body");
        assert!(close < body, "{out}");
    }

    #[test]
    fn a_subtitle_and_a_date_join_the_title_block() {
        let out = render_body("#+TITLE: T\n#+SUBTITLE: S\n#+DATE: 2026-11-13\n");
        assert!(out.contains("<p class=\"subtitle\">S</p>"), "{out}");
        assert!(out.contains("<p class=\"date\">2026-11-13</p>"), "{out}");
        let title = out.find("class=\"title\"").expect("a title");
        let subtitle = out.find("class=\"subtitle\"").expect("a subtitle");
        let date = out.find("class=\"date\"").expect("a date");
        assert!(title < subtitle && subtitle < date, "{out}");
    }

    #[test]
    fn a_repeated_title_keyword_is_joined_with_a_space() {
        let out = render_body("#+TITLE: One\n#+TITLE: Two\n");
        assert!(out.contains("<h1 class=\"title\">One Two</h1>"), "{out}");
    }

    #[test]
    fn a_document_without_title_keywords_has_no_title_block() {
        for source in [
            "* Heading\n\nText.\n",
            "#+OPTIONS: toc:nil\n\nText.\n",
            "#+TITLE:\n#+AUTHOR:   \n\nText.\n",
        ] {
            let out = render_body(source);
            assert!(!out.contains("title-block"), "{out}");
        }
    }

    #[test]
    fn a_title_keyword_below_a_headline_is_not_the_document_title() {
        let out = render_body("* Heading\n#+TITLE: Not this\n");
        assert!(!out.contains("title-block"), "{out}");
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

    #[test]
    fn list_checkboxes_render_as_task_items() {
        let out = render_body("- [ ] a\n- [/] b\n- [-] c\n- [X] d\n- [x] e\n- plain\n");
        assert!(
            out.contains("<li class=\"task task-todo\"><input type=\"checkbox\" disabled=\"\" />"),
            "{out}"
        );
        assert!(
            out.contains("<li class=\"task task-doing\"><input type=\"checkbox\" disabled=\"\" />"),
            "{out}"
        );
        assert_eq!(
            out.matches("<li class=\"task task-doing\"><input type=\"checkbox\" disabled=\"\" />")
                .count(),
            2,
            "{out}"
        );
        assert_eq!(out.matches("task-done").count(), 2, "{out}");
        assert!(
            out.contains("<li class=\"task task-done\"><input type=\"checkbox\" checked=\"\" disabled=\"\" />"),
            "{out}"
        );
        assert!(out.contains("<li><p>plain"), "{out}");
    }

    #[test]
    fn ordered_list_checkboxes_are_tasks_too() {
        let out = render_body("1. [ ] ordered\n2. plain\n");
        assert!(out.contains("<ol>"), "{out}");
        assert!(out.contains("task-todo"), "{out}");
    }

    #[test]
    fn statistics_cookies_render_as_code() {
        let out = render_body("- [ ] parent [1/2]\n");
        assert!(out.contains("<code>[1/2]</code>"), "{out}");
        assert!(out.contains("task-todo"), "{out}");

        let out = render_body("- shopping [/]\n");
        assert!(out.contains("<code>[/]</code>"), "{out}");
        assert!(!out.contains("class=\"task"), "{out}");
    }

    #[test]
    fn an_unknown_checkbox_state_is_plain_text() {
        let out = render_body("- [?] x\n");
        assert!(!out.contains("class=\"task"), "{out}");
    }

    #[test]
    fn a_captioned_named_figure_is_a_figure_element() {
        let out = render_body(
            "#+CAPTION: Outcome by *exposure*\n#+NAME: fig:one\n#+ATTR_HTML: :width 300px\n[[file:plot.png]]\n",
        );
        assert!(
            out.contains(
                "<figure class=\"figure\" id=\"fig:one\"><img src=\"plot.png\" style=\"width:300px\"><figcaption>"
            ),
            "{out}"
        );
        assert!(out.contains("<b>exposure</b>"), "{out}");
        assert!(out.contains("</figcaption></figure>"), "{out}");
    }

    #[test]
    fn lower_case_figure_keywords_work_the_same() {
        let out = render_body(
            "#+caption: Outcome by *exposure*\n#+name: fig:one\n#+attr_html: :width 300px\n[[file:plot.png]]\n",
        );
        assert!(
            out.contains(
                "<figure class=\"figure\" id=\"fig:one\"><img src=\"plot.png\" style=\"width:300px\"><figcaption>"
            ),
            "{out}"
        );
    }

    #[test]
    fn a_sized_figure_without_caption_or_name_is_a_plain_paragraph() {
        let out = render_body("#+ATTR_HTML: :width 50%\n[[file:plot.png]]\n");
        assert!(
            out.contains("<p><img src=\"plot.png\" style=\"width:50%\"></p>"),
            "{out}"
        );
        assert!(!out.contains("<figure"), "{out}");
    }

    #[test]
    fn the_width_comes_from_html_then_org_then_latex() {
        let latex = render_body("#+ATTR_LATEX: :width 0.5\\textwidth\n[[file:plot.png]]\n");
        assert!(latex.contains("style=\"width:50%\""), "{latex}");

        let org = render_body("#+ATTR_ORG: :width 300\n[[file:plot.png]]\n");
        assert!(org.contains("style=\"width:300px\""), "{org}");

        let all = render_body(
            "#+ATTR_LATEX: :width 0.5\\textwidth\n#+ATTR_ORG: :width 300\n#+ATTR_HTML: :width 200px\n[[file:plot.png]]\n",
        );
        assert!(all.contains("style=\"width:200px\""), "{all}");
        assert!(!all.contains("50%"), "{all}");
        assert!(!all.contains("300px"), "{all}");
    }

    #[test]
    fn alt_text_is_escaped_and_height_is_written() {
        let out = render_body(
            "#+ATTR_HTML: :alt A <plot> & \"x\" :height 2in\n[[file:plot.png]]\n",
        );
        assert!(out.contains("alt=\"A &lt;plot&gt;"), "{out}");
        assert!(!out.contains("<plot>"), "{out}");
        assert!(out.contains("style=\"height:2in\""), "{out}");
    }

    #[test]
    fn an_unsafe_size_writes_no_style() {
        for source in [
            "#+ATTR_HTML: :width 100px;background:url(x)\n[[file:plot.png]]\n",
            "#+ATTR_HTML: :width expression(1)\n[[file:plot.png]]\n",
        ] {
            let out = render_body(source);
            assert!(!out.contains("style="), "{out}");
            assert!(!out.contains("background"), "{out}");
            assert!(!out.contains("expression"), "{out}");
            assert!(out.contains("<img src=\"plot.png\">"), "{out}");
        }
    }

    #[test]
    fn an_image_in_running_text_or_with_no_keywords_renders_as_before() {
        let plain = render_body("[[file:plot.png]]\n");
        assert!(plain.contains("<p><img src=\"plot.png\">\n</p>"), "{plain}");
        assert!(!plain.contains("<figure"), "{plain}");
        assert!(!plain.contains("style="), "{plain}");

        let inline = render_body("See [[file:plot.png]] here.\n");
        assert!(inline.contains("<p>See <img src=\"plot.png\"> here."), "{inline}");
        assert!(!inline.contains("style="), "{inline}");
        assert!(!inline.contains("<figure"), "{inline}");
    }

    #[test]
    fn a_css_length_is_written_only_when_it_is_safe() {
        for (raw, want) in [
            ("300", "300px"),
            ("12.5", "12.5px"),
            (".5", ".5px"),
            ("2in", "2in"),
            ("1.5EM", "1.5em"),
            ("50%", "50%"),
            ("0.5\\textwidth", "50%"),
            ("\\textwidth", "100%"),
            (".8\\linewidth", "80%"),
            ("1.0\\columnwidth", "100%"),
        ] {
            assert_eq!(css_length(raw).as_deref(), Some(want), "{raw}");
        }
        for raw in [
            "",
            ".",
            "-3px",
            "1e3px",
            "100px;background:url(x)",
            "expression(1)",
            "calc(1px)",
            "300 px",
            "10.px",
            "1.2.3px",
            "3pz",
            "300px\\textwidth",
        ] {
            assert_eq!(css_length(raw), None, "{raw}");
        }
    }

    #[test]
    fn a_figure_image_with_a_style_is_still_pointed_at_its_asset() {
        let assets = |path: &str| (path == "plot.png").then(|| "/assets/plot.png".to_string());
        let out = render_with(
            "#+CAPTION: Cap\n#+ATTR_HTML: :width 300px\n[[file:plot.png]]\n",
            "t",
            &assets,
        );
        assert!(
            out.contains("<img src=\"/assets/plot.png\" style=\"width:300px\">"),
            "{out}"
        );
    }
}

#[cfg(test)]
mod footnote_tests {
    use super::render_body;

    #[test]
    fn a_footnote_definition_spans_lines_until_a_definition_or_two_blank_lines() {
        let out = render_body(
            "A[fn:a] and B[fn:b].\n\n[fn:a] First line\nsecond line\n\nthird after one blank.\n[fn:b] Other.\n\n\nOutside.\n",
        );
        let notes = out.find("class=\"footnotes\"").expect("notes");
        let one = out.find("<li id=\"fn-1\">").expect("note 1");
        let two = out.find("<li id=\"fn-2\">").expect("note 2");
        assert!(notes < one && one < two, "{out}");
        assert!(out[one..two].contains("First line\nsecond line"), "{out}");
        assert!(out[one..two].contains("third after one blank."), "{out}");
        assert!(out[two..].contains("Other."), "{out}");
        // the text after two blank lines is body text, before the notes
        assert!(out[..notes].contains("Outside."), "{out}");
        assert!(!out[two..].contains("Outside."), "{out}");
    }
}

#[cfg(test)]
mod unclosed_block_tests {
    use super::render_body;
    use crate::{ast::SourceBlock, Org, SyntaxKind};
    use rowan::ast::AstNode;

    #[test]
    fn an_unclosed_block_runs_to_the_end_of_the_document() {
        let source = "Intro.\n\n#+begin_src python\nprint('a')\nx = 1\n";
        let org = Org::parse(source);
        let block = org.first_node::<SourceBlock>().expect("a source block");
        assert!(
            block
                .syntax()
                .children()
                .all(|n| n.kind() != SyntaxKind::BLOCK_END),
            "no end line"
        );
        assert_eq!(org.document().syntax().to_string(), source, "lossless");
        let out = render_body(source);
        assert!(out.contains("print(&apos;a&apos;)\nx = 1"), "{out}");
        assert!(out.contains("<pre"), "{out}");
        assert!(!out.contains("#+begin"), "{out}");
    }

    #[test]
    fn a_headline_ends_an_unclosed_block() {
        let source = "* One\n#+begin_quote\nQuoted *text*.\n* Two\nAfter.\n";
        let out = render_body(source);
        assert!(out.contains("<blockquote"), "{out}");
        assert!(out.contains("<b>text</b>"), "{out}");
        let quote = out.find("<blockquote").unwrap();
        let two = out.find("<h1>Two</h1>").expect("second headline");
        assert!(quote < two, "{out}");
        assert!(out[two..].contains("After."), "{out}");
        assert!(!out[two..].contains("blockquote"), "{out}");
    }

    #[test]
    fn a_closed_block_still_stops_at_its_end_line() {
        let out = render_body("#+begin_src c\nint a;\n#+end_src\n\nAfter.\n");
        assert!(out.contains("int a;"), "{out}");
        assert!(out.contains("<p>After."), "{out}");
    }
}
