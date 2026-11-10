use nom::{
    bytes::complete::{tag, take_while1},
    combinator::map,
    sequence::tuple,
    IResult, Slice,
};

use super::{
    combinator::{
        blank_lines, colon_token, l_bracket_token, line_ends_iter, node, r_bracket_token,
        GreenElement, NodeBuilder,
    },
    element::element_nodes,
    input::Input,
    keyword::affiliated_keyword_nodes,
    object::standard_object_nodes,
    SyntaxKind::*,
};

/// Whether a line starts another footnote definition.
fn starts_fn_def(line: &str) -> bool {
    let Some(rest) = line.strip_prefix("[fn:") else {
        return false;
    };
    let label = rest
        .find(|c: char| !(c.is_ascii_alphanumeric() || c == '-' || c == '_'))
        .unwrap_or(rest.len());
    label > 0 && rest[label..].starts_with(']')
}

/// Whether a line is a headline: stars at column 0 and a space.
fn starts_headline(line: &str) -> bool {
    let stars = line.bytes().take_while(|&b| b == b'*').count();
    stars > 0 && matches!(line.as_bytes().get(stars), Some(b' ' | b'\t'))
}

/// Whether the text has a blank line between two lines with text.
fn has_inner_blank_line(content: &str) -> bool {
    let mut seen_text = false;
    let mut seen_blank = false;
    for line in content.split_inclusive(['\n', '\r']) {
        if line.trim().is_empty() {
            seen_blank |= seen_text;
        } else if seen_blank {
            return true;
        } else {
            seen_text = true;
        }
    }
    false
}

/// The text of a definition after its label: the rest of the line and every line
/// after it up to the next definition, a headline, or two blank lines in a row.
/// Returns the content, its trailing blanks and its line end.
fn fn_def_body(input: Input) -> IResult<Input, (Input, Input, Input), ()> {
    let s = input.as_str();
    let mut end = 0;
    let mut start = 0;
    let mut blanks = 0;
    for (n, line_end) in line_ends_iter(s).enumerate() {
        if start == line_end {
            break;
        }
        let line = &s[start..line_end];
        if n > 0 {
            if line.trim().is_empty() {
                blanks += 1;
                if blanks == 2 {
                    break;
                }
                start = line_end;
                continue;
            }
            if starts_fn_def(line) || starts_headline(line) {
                break;
            }
        }
        blanks = 0;
        end = line_end;
        start = line_end;
    }

    let text = &s[..end];
    let trimmed = text.trim_end_matches(|c: char| c.is_ascii_whitespace()).len();
    let nl_at = trimmed + text[trimmed..].find(['\n', '\r']).unwrap_or(end - trimmed);
    Ok((
        input.slice(end..),
        (
            input.slice(..trimmed),
            input.slice(trimmed..nl_at),
            input.slice(nl_at..end),
        ),
    ))
}

#[cfg_attr(
  feature = "tracing",
  tracing::instrument(level = "debug", skip(input), fields(input = input.s))
)]
pub fn fn_def_node(input: Input) -> IResult<Input, GreenElement, ()> {
    crate::lossless_parser!(fn_def_node_base, input)
}

fn fn_def_node_base(input: Input) -> IResult<Input, GreenElement, ()> {
    let mut parser = map(
        tuple((
            affiliated_keyword_nodes,
            l_bracket_token,
            tag("fn"),
            colon_token,
            take_while1(|c: char| c.is_ascii_alphanumeric() || c == '-' || c == '_'),
            r_bracket_token,
            fn_def_body,
            blank_lines,
        )),
        |(
            affiliated_keywords,
            l_bracket,
            fn_,
            colon,
            label,
            r_bracket,
            (content, ws_, nl),
            post_blank,
        )| {
            let mut b = NodeBuilder::new();

            b.children.extend(affiliated_keywords);
            b.push(l_bracket);
            b.push(fn_.token(KEYWORD));
            b.push(colon);
            b.push(label.token(FN_LABEL));
            b.push(r_bracket);

            // A definition of one paragraph holds its objects; one with a blank
            // line inside holds elements.
            let content_node = if has_inner_blank_line(content.as_str()) || content.as_str().trim_end().contains('\n') {
                let children = element_nodes(content).unwrap_or_else(|_| vec![content.text_token()]);
                node(FN_CONTENT, children)
            } else {
                node(FN_CONTENT, standard_object_nodes(content))
            };
            b.push(content_node);

            b.ws(ws_);
            b.nl(nl);
            b.children.extend(post_blank);
            b.finish(FN_DEF)
        },
    );
    let (i, fn_def) = parser(input)?;
    Ok((i, fn_def))
}

#[test]
fn parse() {
    use crate::ParseConfig;
    use crate::{ast::FnDef, tests::to_ast};

    let to_fn_def = to_ast::<FnDef>(fn_def_node);

    insta::assert_debug_snapshot!(
         to_fn_def("[fn:1] *bold* -  https://orgmode.org").syntax,
         @r#"
    FN_DEF@0..36
      L_BRACKET@0..1 "["
      KEYWORD@1..3 "fn"
      COLON@3..4 ":"
      FN_LABEL@4..5 "1"
      R_BRACKET@5..6 "]"
      FN_CONTENT@6..36
        TEXT@6..7 " "
        BOLD@7..13
          STAR@7..8 "*"
          TEXT@8..12 "bold"
          STAR@12..13 "*"
        TEXT@13..17 " -  "
        LINK@17..36
          LINK_PATH@17..36 "https://orgmode.org"
    "#
    );

    insta::assert_debug_snapshot!(
         to_fn_def("[fn:word_1] https://orgmode.org").syntax,
         @r#"
    FN_DEF@0..31
      L_BRACKET@0..1 "["
      KEYWORD@1..3 "fn"
      COLON@3..4 ":"
      FN_LABEL@4..10 "word_1"
      R_BRACKET@10..11 "]"
      FN_CONTENT@11..31
        TEXT@11..12 " "
        LINK@12..31
          LINK_PATH@12..31 "https://orgmode.org"
    "#
    );

    insta::assert_debug_snapshot!(
         to_fn_def("[fn:WORD-1] https://orgmode.org").syntax,
         @r#"
    FN_DEF@0..31
      L_BRACKET@0..1 "["
      KEYWORD@1..3 "fn"
      COLON@3..4 ":"
      FN_LABEL@4..10 "WORD-1"
      R_BRACKET@10..11 "]"
      FN_CONTENT@11..31
        TEXT@11..12 " "
        LINK@12..31
          LINK_PATH@12..31 "https://orgmode.org"
    "#
    );

    insta::assert_debug_snapshot!(
         to_fn_def("[fn:WORD]").syntax,
         @r#"
    FN_DEF@0..9
      L_BRACKET@0..1 "["
      KEYWORD@1..3 "fn"
      COLON@3..4 ":"
      FN_LABEL@4..8 "WORD"
      R_BRACKET@8..9 "]"
      FN_CONTENT@9..9
    "#
    );

    insta::assert_debug_snapshot!(
         to_fn_def("[fn:1] In particular, the parser requires stars at column 0 to be\n").syntax,
         @r#"
    FN_DEF@0..66
      L_BRACKET@0..1 "["
      KEYWORD@1..3 "fn"
      COLON@3..4 ":"
      FN_LABEL@4..5 "1"
      R_BRACKET@5..6 "]"
      FN_CONTENT@6..65
        TEXT@6..65 " In particular, the p ..."
      NEW_LINE@65..66 "\n"
    "#
    );

    let config = &ParseConfig::default();

    assert!(fn_def_node(("[fn:] https://orgmode.org", config).into()).is_err());
    assert!(fn_def_node(("[fn:wor d] https://orgmode.org", config).into()).is_err());
    assert!(fn_def_node(("[fn:WORD https://orgmode.org", config).into()).is_err());

    insta::assert_debug_snapshot!(
         to_fn_def("#+ATTR_poi: 1\n[fn:WORD-1] https://orgmode.org").syntax,
         @r##"
    FN_DEF@0..45
      AFFILIATED_KEYWORD@0..14
        HASH_PLUS@0..2 "#+"
        TEXT@2..10 "ATTR_poi"
        COLON@10..11 ":"
        TEXT@11..13 " 1"
        NEW_LINE@13..14 "\n"
      L_BRACKET@14..15 "["
      KEYWORD@15..17 "fn"
      COLON@17..18 ":"
      FN_LABEL@18..24 "WORD-1"
      R_BRACKET@24..25 "]"
      FN_CONTENT@25..45
        TEXT@25..26 " "
        LINK@26..45
          LINK_PATH@26..45 "https://orgmode.org"
    "##
    );
}

#[test]
fn multiline_definitions() {
    use crate::tests::to_ast;
    use crate::ast::FnDef;
    use rowan::ast::AstNode;

    let to_fn_def = to_ast::<FnDef>(fn_def_node);
    let content = |s: &str| {
        let def = to_fn_def(s);
        assert_eq!(def.syntax().to_string(), s, "lossless");
        def.syntax()
            .children()
            .find(|n| n.kind() == FN_CONTENT)
            .unwrap()
            .to_string()
    };

    // continuation lines belong to the definition
    assert_eq!(content("[fn:1] one\ntwo\nthree\n"), " one\ntwo\nthree");
    // one blank line does not end it, and the content holds elements
    let def = to_fn_def("[fn:1] one\n\ntwo\n");
    let content = def.syntax().children().find(|n| n.kind() == FN_CONTENT).unwrap();
    assert_eq!(content.to_string(), " one\n\ntwo");
    assert!(content.children().all(|n| n.kind() == PARAGRAPH), "{content:#?}");
}

#[test]
fn definition_extents() {
    let config = crate::ParseConfig::default();
    let rest = |s: &str| {
        fn_def_node((s, &config).into())
            .unwrap()
            .0
            .as_str()
            .to_string()
    };
    assert_eq!(rest("[fn:1] one\n[fn:2] two\n"), "[fn:2] two\n");
    assert_eq!(rest("[fn:1] one\ntwo\n* Head\nbody\n"), "* Head\nbody\n");
    assert_eq!(rest("[fn:1] one\n\n[fn:2] two\n"), "[fn:2] two\n");
    assert_eq!(rest("[fn:1] one\n\n\nafter\n"), "after\n");
    assert_eq!(rest("[fn:1] one\n\nmore\n\n\nafter\n"), "after\n");
}
