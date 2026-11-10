//! Org 9.5 citations: `[cite/style:global prefix; prefix @key suffix; global suffix]`.
//!
//! Tree shape (lossless):
//!
//! ```text
//! CITATION
//!   L_BRACKET "[", KEYWORD "cite", CITATION_STYLE "/t/b" (optional), COLON ":"
//!   CITATION_PREFIX      (global prefix, optional, before the first reference)
//!   CITATION_REFERENCE
//!     CITATION_PREFIX    (optional)
//!     AT "@", CITATION_KEY "doe2020"
//!     CITATION_SUFFIX    (optional)
//!   SEMICOLON ";"
//!   CITATION_REFERENCE ...
//!   CITATION_SUFFIX      (global suffix, optional, after the last reference)
//!   R_BRACKET "]"
//! ```

use nom::{bytes::complete::tag, Err, IResult, InputTake};

use super::{
    combinator::{colon_token, l_bracket_token, node, r_bracket_token, token, GreenElement},
    input::Input,
    SyntaxKind::*,
};

fn is_key_char(c: char) -> bool {
    c.is_alphanumeric() || matches!(c, '-' | '_' | ':' | '.' | '+' | '/')
}

#[cfg_attr(
    feature = "tracing",
    tracing::instrument(level = "debug", skip(input), fields(input = input.s))
)]
pub fn citation_node(input: Input) -> IResult<Input, GreenElement, ()> {
    crate::lossless_parser!(citation_node_base, input)
}

fn citation_node_base(input: Input) -> IResult<Input, GreenElement, ()> {
    let (input, l_bracket) = l_bracket_token(input)?;
    let (input, cite) = tag("cite")(input)?;

    // optional `/style` or `/style/variant`, up to the colon
    let style_len = input
        .s
        .find(|c: char| !(c.is_alphanumeric() || c == '/' || c == '-' || c == '_'))
        .unwrap_or(input.s.len());
    let (input, style) = input.take_split(style_len);
    if !style.s.is_empty() && (!style.s.starts_with('/') || style.s.len() == 1) {
        return Err(Err::Error(()));
    }
    let (input, colon) = colon_token(input)?;

    // the body ends at the first `]`
    let end = input.s.find(']').ok_or(Err::Error(()))?;
    let (rest, body) = input.take_split(end);
    let (rest, r_bracket) = r_bracket_token(rest)?;

    let segments: Vec<&str> = body.s.split(';').collect();
    let has_key = |s: &str| key_start(s).is_some();
    let n = segments.len();
    if !segments.iter().any(|s| has_key(s)) {
        return Err(Err::Error(()));
    }

    let mut children = vec![l_bracket, token(KEYWORD, cite.s)];
    if !style.s.is_empty() {
        children.push(token(CITATION_STYLE, style.s));
    }
    children.push(colon);

    for (i, seg) in segments.iter().enumerate() {
        if i > 0 {
            children.push(token(SEMICOLON, ";"));
        }
        match key_start(seg) {
            Some((at, key_end)) => {
                let mut r = vec![];
                if at > 0 {
                    r.push(node(CITATION_PREFIX, [token(TEXT, &seg[..at])]));
                }
                r.push(token(AT, "@"));
                r.push(token(CITATION_KEY, &seg[at + 1..key_end]));
                if key_end < seg.len() {
                    r.push(node(CITATION_SUFFIX, [token(TEXT, &seg[key_end..])]));
                }
                children.push(node(CITATION_REFERENCE, r));
            }
            // a keyless segment is only a global prefix (first) or suffix (last)
            None if i == 0 && n > 1 => {
                if !seg.is_empty() {
                    children.push(node(CITATION_PREFIX, [token(TEXT, seg)]));
                }
            }
            None if i == n - 1 && n > 1 => {
                if !seg.is_empty() {
                    children.push(node(CITATION_SUFFIX, [token(TEXT, seg)]));
                }
            }
            None => return Err(Err::Error(())),
        }
    }
    children.push(r_bracket);

    Ok((rest, node(CITATION, children)))
}

/// Byte offset of the first `@key` in a segment, and where the key ends.
fn key_start(seg: &str) -> Option<(usize, usize)> {
    let mut from = 0;
    while let Some(i) = seg[from..].find('@') {
        let at = from + i;
        let len = seg[at + 1..]
            .find(|c: char| !is_key_char(c))
            .unwrap_or(seg.len() - at - 1);
        if len > 0 {
            return Some((at, at + 1 + len));
        }
        from = at + 1;
    }
    None
}

#[test]
fn parse() {
    use crate::{ast::Citation, tests::to_ast, ParseConfig};

    let to = to_ast::<Citation>(citation_node);

    insta::assert_debug_snapshot!(
        to("[cite:@doe2020]").syntax,
        @r#"
    CITATION@0..15
      L_BRACKET@0..1 "["
      KEYWORD@1..5 "cite"
      COLON@5..6 ":"
      CITATION_REFERENCE@6..14
        AT@6..7 "@"
        CITATION_KEY@7..14 "doe2020"
      R_BRACKET@14..15 "]"
    "#
    );

    insta::assert_debug_snapshot!(
        to("[cite/t/b:see ;pre @a.b-c p. 3;@b; end]").syntax,
        @r#"
    CITATION@0..39
      L_BRACKET@0..1 "["
      KEYWORD@1..5 "cite"
      CITATION_STYLE@5..9 "/t/b"
      COLON@9..10 ":"
      CITATION_PREFIX@10..14
        TEXT@10..14 "see "
      SEMICOLON@14..15 ";"
      CITATION_REFERENCE@15..30
        CITATION_PREFIX@15..19
          TEXT@15..19 "pre "
        AT@19..20 "@"
        CITATION_KEY@20..25 "a.b-c"
        CITATION_SUFFIX@25..30
          TEXT@25..30 " p. 3"
      SEMICOLON@30..31 ";"
      CITATION_REFERENCE@31..33
        AT@31..32 "@"
        CITATION_KEY@32..33 "b"
      SEMICOLON@33..34 ";"
      CITATION_SUFFIX@34..38
        TEXT@34..38 " end"
      R_BRACKET@38..39 "]"
    "#
    );

    let config = &ParseConfig::default();
    // lossless, and mixed with other objects in a paragraph
    for src in ["[cite:@a]", "[cite/t:x;@a y;@b;z]", "a [cite:@a] [fn:1] [[l]] [50%] [2024-01-01] b"] {
        let nodes = crate::syntax::object::standard_object_nodes((src, config).into());
        let text: String = nodes.iter().map(|n| n.to_string()).collect();
        assert_eq!(text, src);
    }
    let nodes = crate::syntax::object::standard_object_nodes(("a [cite:@a] [fn:1] [50%]", config).into());
    let kinds: Vec<_> = nodes.iter().filter_map(|n| n.as_node().map(|n| n.kind())).collect();
    assert_eq!(kinds.len(), 3);
    for bad in ["[cite:]", "[cite:foo]", "[cite:@]", "[cite/:@a]", "[cite:@a", "[cite:a;b;@c;d]", "[cited:@a]", "[fn:1]", "[2024-01-01]", "[50%]"] {
        let r = citation_node((bad, config).into());
        assert!(r.is_err(), "{bad}");
    }
}
