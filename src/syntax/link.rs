use nom::{
    bytes::complete::take_while,
    combinator::{map, opt},
    sequence::tuple,
    IResult, InputTake,
};

use super::{
    combinator::{
        l_bracket2_token, l_bracket_token, node, r_bracket2_token, r_bracket_token, GreenElement,
    },
    input::Input,
    object::link_description_object_nodes,
    SyntaxKind::*,
};

#[cfg_attr(
    feature = "tracing",
    tracing::instrument(level = "debug", skip(input), fields(input = input.s))
)]
pub fn link_node(input: Input) -> IResult<Input, GreenElement, ()> {
    let mut parser = map(
        tuple((
            l_bracket2_token,
            take_while(|c: char| c != '<' && c != '>' && c != ']'),
            opt(tuple((
                r_bracket_token,
                l_bracket_token,
                take_while(|c: char| c != '[' && c != ']'),
            ))),
            r_bracket2_token,
        )),
        |(l_bracket2, path, desc, r_bracket2)| {
            let mut children = vec![l_bracket2, path.token(LINK_PATH)];

            if let Some((r_bracket, l_bracket, desc)) = desc {
                children.extend([r_bracket, l_bracket]);
                children.extend(link_description_object_nodes(desc));
            }

            children.push(r_bracket2);

            node(LINK, children)
        },
    );
    crate::lossless_parser!(parser, input)
}

/// Schemes a bare URL may start with, as in `https://orgmode.org`.
const PLAIN_SCHEMES: &[&str] = &[
    "https", "http", "ftp", "file", "mailto", "doi", "irc", "news", "ssh",
];

/// Where a plain link's scheme starts, given the text before the `:` that follows it.
/// A scheme glued to a longer word (`xhttp`) is not one.
pub fn plain_link_start(before_colon: &str) -> Option<usize> {
    PLAIN_SCHEMES.iter().find_map(|scheme| {
        let start = before_colon.len().checked_sub(scheme.len())?;
        if !before_colon.is_char_boundary(start) || &before_colon[start..] != *scheme {
            return None;
        }
        match before_colon[..start].chars().next_back() {
            Some(c) if c.is_alphanumeric() => None,
            _ => Some(start),
        }
    })
}

/// A bare URL: a known scheme, a colon, then anything up to whitespace or a bracket,
/// less the punctuation that ends a sentence.
#[cfg_attr(
    feature = "tracing",
    tracing::instrument(level = "debug", skip(input), fields(input = input.s))
)]
pub fn plain_link_node(input: Input) -> IResult<Input, GreenElement, ()> {
    fn f(input: Input) -> IResult<Input, GreenElement, ()> {
        let s = input.as_str();
        let scheme = PLAIN_SCHEMES
            .iter()
            .find(|scheme| s.starts_with(**scheme) && s[scheme.len()..].starts_with(':'))
            .ok_or(nom::Err::Error(()))?;
        let rest = &s[scheme.len() + 1..];
        let body = rest
            .find(|c: char| c.is_whitespace() || matches!(c, '<' | '>' | '[' | ']' | '{' | '}'))
            .unwrap_or(rest.len());
        let body = rest[..body].trim_end_matches(['.', ',', ';', ':', '!', '?', '\'', '"', ')']);
        if body.is_empty() {
            return Err(nom::Err::Error(()));
        }
        let (input, path) = input.take_split(scheme.len() + 1 + body.len());
        Ok((input, node(LINK, [path.token(LINK_PATH)])))
    }
    crate::lossless_parser!(f, input)
}

/// `<scheme:target>`, with any scheme.
#[cfg_attr(
    feature = "tracing",
    tracing::instrument(level = "debug", skip(input), fields(input = input.s))
)]
pub fn angle_link_node(input: Input) -> IResult<Input, GreenElement, ()> {
    fn f(input: Input) -> IResult<Input, GreenElement, ()> {
        let s = input.as_str();
        let inner = s.strip_prefix('<').ok_or(nom::Err::Error(()))?;
        let colon = inner.find(':').ok_or(nom::Err::Error(()))?;
        let scheme = &inner[..colon];
        let mut chars = scheme.chars();
        let valid = chars.next().is_some_and(|c| c.is_ascii_alphabetic())
            && chars.all(|c| c.is_alphanumeric() || matches!(c, '_' | '+' | '.' | '-'));
        if !valid {
            return Err(nom::Err::Error(()));
        }
        let target = &inner[colon + 1..];
        let end = target
            .find(|c: char| c.is_whitespace() || c == '<' || c == '>')
            .ok_or(nom::Err::Error(()))?;
        if end == 0 || !target[end..].starts_with('>') {
            return Err(nom::Err::Error(()));
        }
        let (input, l) = input.take_split(1);
        let (input, path) = input.take_split(colon + 1 + end);
        let (input, r) = input.take_split(1);
        Ok((
            input,
            node(
                LINK,
                [
                    l.token(L_ANGLE),
                    path.token(LINK_PATH),
                    r.token(R_ANGLE),
                ],
            ),
        ))
    }
    crate::lossless_parser!(f, input)
}

#[test]
fn parse() {
    use crate::{ast::Link, tests::to_ast, ParseConfig};

    let to_link = to_ast::<Link>(link_node);

    let link = to_link("[[#id]]");
    insta::assert_debug_snapshot!(
        link.syntax,
        @r###"
    LINK@0..7
      L_BRACKET2@0..2 "[["
      LINK_PATH@2..5 "#id"
      R_BRACKET2@5..7 "]]"
    "###
    );

    let link = to_link("[[#id][desc]]");
    insta::assert_debug_snapshot!(
        link.syntax,
        @r###"
    LINK@0..13
      L_BRACKET2@0..2 "[["
      LINK_PATH@2..5 "#id"
      R_BRACKET@5..6 "]"
      L_BRACKET@6..7 "["
      TEXT@7..11 "desc"
      R_BRACKET2@11..13 "]]"
    "###
    );

    let link = to_link("[[file:/home/dominik/images/jupiter.jpg]]");
    insta::assert_debug_snapshot!(
        link.syntax,
        @r###"
    LINK@0..41
      L_BRACKET2@0..2 "[["
      LINK_PATH@2..39 "file:/home/dominik/im ..."
      R_BRACKET2@39..41 "]]"
    "###
    );

    let link = to_link("[[https://orgmode.org][*bold* description]]");
    insta::assert_debug_snapshot!(
        link.syntax,
        @r###"
    LINK@0..43
      L_BRACKET2@0..2 "[["
      LINK_PATH@2..21 "https://orgmode.org"
      R_BRACKET@21..22 "]"
      L_BRACKET@22..23 "["
      BOLD@23..29
        STAR@23..24 "*"
        TEXT@24..28 "bold"
        STAR@28..29 "*"
      TEXT@29..41 " description"
      R_BRACKET2@41..43 "]]"
    "###
    );

    let config = &ParseConfig::default();

    assert!(link_node(("[[#id][desc]", config).into()).is_err());
}

/// Emacs org-mode accepts a single newline inside a bracket link — both inside
/// the path and inside the description.  These tests document the discrepancy
/// against orgize and serve as the reproducer for the upstream issue/PR.
#[test]
fn parse_multiline_link() {
    use crate::{ast::Link, tests::to_ast};

    let to_link = to_ast::<Link>(link_node);

    // Path-only link with a newline inside the path.
    let link = to_link("[[really really long link\nthat just keeps going]]");
    insta::assert_debug_snapshot!(
        link.syntax,
        @r###"
    LINK@0..49
      L_BRACKET2@0..2 "[["
      LINK_PATH@2..47 "really really long li ..."
      R_BRACKET2@47..49 "]]"
    "###
    );

    // Link with separate description that contains a newline.
    let link = to_link("[[https://example.com][some\ndescription]]");
    insta::assert_debug_snapshot!(
        link.syntax,
        @r###"
    LINK@0..41
      L_BRACKET2@0..2 "[["
      LINK_PATH@2..21 "https://example.com"
      R_BRACKET@21..22 "]"
      L_BRACKET@22..23 "["
      TEXT@23..39 "some\ndescription"
      R_BRACKET2@39..41 "]]"
    "###
    );
}

/// `Org::parse` should also pick up multi-line links inside paragraphs.  This
/// is the end-to-end case relevant to downstream formatters (e.g. org-fmt).
#[test]
fn parse_multiline_link_in_paragraph() {
    use crate::{ast::Link, Org};
    use rowan::ast::AstNode;

    let org = Org::parse(
        "Here is a [[really really long link\nthat just keeps going]] in prose.\n",
    );

    let links: Vec<_> = org
        .document()
        .syntax()
        .descendants()
        .filter_map(Link::cast)
        .collect();

    assert_eq!(
        links.len(),
        1,
        "expected one link, found {}",
        links.len(),
    );
}

#[test]
fn plain_and_angle_links() {
    use crate::{ast::Link, Org};
    use rowan::ast::AstNode;

    let paths = |s: &str| {
        Org::parse(s)
            .document()
            .syntax()
            .descendants()
            .filter_map(Link::cast)
            .map(|l| l.path().to_string())
            .collect::<Vec<_>>()
    };

    assert_eq!(paths("see https://orgmode.org."), ["https://orgmode.org"]);
    assert_eq!(paths("(http://a.org/x?y=1), ftp://h/f"), ["http://a.org/x?y=1", "ftp://h/f"]);
    assert_eq!(paths("mail mailto:a@b.org now"), ["mailto:a@b.org"]);
    assert_eq!(paths("doi:10.1000/182 and file:/tmp/a.txt"), ["doi:10.1000/182", "file:/tmp/a.txt"]);
    assert_eq!(paths("ssh://host and news:comp.lang"), ["ssh://host", "news:comp.lang"]);
    assert_eq!(paths("<https://x.org/a>"), ["https://x.org/a"]);
    assert_eq!(paths("<foo+bar:baz>"), ["foo+bar:baz"]);
    // not links
    assert!(paths("xhttp://a.org").is_empty());
    assert!(paths("http: nothing").is_empty());
    assert!(paths("=https://a.org= and ~http://b.org~").is_empty());
    assert!(paths("<2024-01-31 Wed 10:00>").is_empty());
    // bracket links stay one link
    assert_eq!(paths("[[https://a.org][desc]]"), ["https://a.org"]);
    assert_eq!(paths("[[https://a.org]]"), ["https://a.org"]);
    // in a table cell
    assert_eq!(paths("| https://a.org | x |"), ["https://a.org"]);
}
