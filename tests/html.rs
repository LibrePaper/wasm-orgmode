use wasm_orgmode::Org;

#[test]
fn emphasis() {
    insta::assert_snapshot!(
        Org::parse("*bold*, /italic/,\n_underlined_, =verbatim= and ~code~").to_html(),
        @r###"
    <main><section><p><b>bold</b>, <i>italic</i>,
    <u>underlined</u>, <code>verbatim</code> and <code>code</code></p></section></main>
    "###
    );
}

#[test]
fn link() {
    insta::assert_snapshot!(
        Org::parse("Visit[[http://example.com][link1]]or[[http://example.com][link1]].").to_html(),
        @r###"<main><section><p>Visit<a href="http://example.com">link1</a>or<a href="http://example.com">link1</a>.</p></section></main>"###
    );
}

#[test]
fn section_and_headline() {
    insta::assert_snapshot!(
        Org::parse(r#"
* title 1
section 1
** title 2
section 2
* title 3
section 3
* title 4
section 4
"#).to_html(),
        @r###"
    <main><h1>title 1</h1><section><p>section 1
    </p></section><h2>title 2</h2><section><p>section 2
    </p></section><h1>title 3</h1><section><p>section 3
    </p></section><h1>title 4</h1><section><p>section 4
    </p></section></main>
    "###
    );
}

#[test]
fn list() {
    insta::assert_snapshot!(
        Org::parse(r#"
+ 1

+ 2

  - 3

  - 4

+ 5
"#).to_html(),
        @r###"
    <main><section><ul><li><p>1
    </p></li><li><p>2
    </p><ul><li><p>3
    </p></li><li><p>4
    </p></li></ul></li><li><p>5
    </p></li></ul></section></main>
    "###
    );
}

#[test]
fn snippet() {
    insta::assert_snapshot!(
        Org::parse("@@html:<del>@@delete this@@html:</del>@@").to_html(),
        @"<main><section><p><del>delete this</del></p></section></main>"
    );
}

#[test]
fn paragraphs() {
    insta::assert_snapshot!(
        Org::parse(r#"
* title

paragraph 1

paragraph 2

paragraph 3

paragraph 4
"#).to_html(),
        @r###"
    <main><h1>title</h1><section><p>paragraph 1
    </p><p>paragraph 2
    </p><p>paragraph 3
    </p><p>paragraph 4
    </p></section></main>
    "###
    );
}

#[test]
fn table() {
    // don't has table header
    insta::assert_snapshot!(
        Org::parse(r#"
|-----+-----+-----|
|   0 |   1 |   2 |
|   4 |   5 |   6 |
|-----+-----+-----|
"#).to_html(),
        @"<main><section><table><tbody><tr><td>0</td><td>1</td><td>2</td></tr><tr><td>4</td><td>5</td><td>6</td></tr></tbody></table></section></main>"
    );

    // has table header
    insta::assert_snapshot!(
        Org::parse(r#"
|   0 |   1 |   2 |
|-----+-----+-----|
|   4 |   5 |   6 |
|-----+-----+-----|
"#).to_html(),
        @"<main><section><table><thead><tr><td>0</td><td>1</td><td>2</td></tr></thead><tbody><tr><td>4</td><td>5</td><td>6</td></tr></tbody></table></section></main>"
    );

    // has two table body
    insta::assert_snapshot!(
        Org::parse(r#"
|   0 |   1 |   2 |
|-----+-----+-----|
|   4 |   5 |   6 |
|-----+-----+-----|
|   7 |   8 |   9 |
"#).to_html(),
        @"<main><section><table><thead><tr><td>0</td><td>1</td><td>2</td></tr></thead><tbody><tr><td>4</td><td>5</td><td>6</td></tr></tbody><tbody><tr><td>7</td><td>8</td><td>9</td></tr></tbody></table></section></main>"
    );

    // multiple row rule
    insta::assert_snapshot!(
        Org::parse(r#"
|   0 |   1 |   2 |
|-----+-----+-----|
|-----+-----+-----|
|   4 |   5 |   6 |
"#).to_html(),
        @"<main><section><table><thead><tr><td>0</td><td>1</td><td>2</td></tr></thead><tbody><tr><td>4</td><td>5</td><td>6</td></tr></tbody></table></section></main>"
    );

    // empty
    insta::assert_snapshot!(
        Org::parse(r#"
|-----+-----+-----|
|-----+-----+-----|
"#).to_html(),
        @"<main><section><table></table></section></main>"
    );

    insta::assert_snapshot!(
        Org::parse(r#"
|
|-
|
|-
|
"#).to_html(),
        @"<main><section><table><thead><tr></tr></thead><tbody><tr></tr></tbody><tbody><tr></tr></tbody></table></section></main>"
    );
}

#[test]
fn line_break() {
    insta::assert_debug_snapshot!(
        Org::parse("aa\\\\\nbb").to_html(),
        @r###""<main><section><p>aa<br/>bb</p></section></main>""###
    );
}

#[test]
fn footnote() {
    insta::assert_debug_snapshot!(
        Org::parse("[fn:1] In particular, the parser requires stars at column 0 to be\n").to_html(),
        @r##""<main><section><aside class=\"footnote-definition\" ><a href=\"#footnote_1\" class=\"footnote-reference\" >[1]</a><span class=\"footnote-content\" id=\"footnote_1\" > In particular, the parser requires stars at column 0 to be</span></aside></section></main>""##
    );
    // "~org-inlinetask-min-level~[fn:oiml:The default value of \n~org-inlinetask-min-level~ is =15=.]"
    insta::assert_debug_snapshot!(
        Org::parse(
            "~org-inlinetask-min-level~[fn:oiml:The default value of \n~org-inlinetask-min-level~ is =15=.]"
        ).to_html(),
        @r##""<main><section><p><code>org-inlinetask-min-level</code><a href=\"#footnote_oiml\" class=\"footnote-reference\">[oiml]</a><span class=\"footnote-content\" id=\"footnote_oiml\" >The default value of \n<code>org-inlinetask-min-level</code> is <code>15</code>.</span></p></section></main>""##
    );
}

#[test]
fn headline_followed_by_blank_line_has_no_empty_paragraph() {
    let html = Org::parse("* H\n\nPara.\n").to_html();
    assert!(!html.contains("<p></p>"), "{html}");
    assert!(html.contains("<p>Para."), "{html}");
}

#[test]
fn empty_greater_blocks_do_not_panic() {
    let html = Org::parse("#+begin_quote\n#+end_quote\n").to_html();
    assert!(html.contains("<blockquote>"), "{html}");

    let html = Org::parse("#+begin_center\n\n#+end_center\n").to_html();
    assert!(html.contains("<div"), "{html}");
}

#[test]
fn round_trip_blank_lines_after_headline() {
    let inputs = [
        "#+begin_quote\n#+end_quote\n",
        "#+begin_center\n\n#+end_center\n",
        "#+begin_note\n#+end_note\n",
        "* H\n\nPara.\n",
        "* H\n\n\n\nPara.\n",
        "* H\n\n",
    ];
    for s in inputs {
        assert_eq!(Org::parse(s).to_org(), s, "input: {s:?}");
    }
}

#[test]
fn table_caption() {
    let html = Org::parse("#+NAME: t\n#+CAPTION: cap\n| a | b |\n|---+---|\n| 1 | 2 |\n").to_html();
    assert!(html.contains("<table>"), "{html}");
    assert!(html.contains("<caption>cap</caption>"), "{html}");
    assert!(!html.contains("<p>|"), "{html}");
}

#[test]
fn table_empty_cell_keeps_columns() {
    let html = Org::parse("| column 1 | column 2 |\n|          | column 2 |\n").to_html();
    assert!(
        html.contains("<tr><td>column 1</td><td>column 2</td></tr>"),
        "{html}"
    );
    assert!(
        html.contains("<tr><td></td><td>column 2</td></tr>"),
        "{html}"
    );
}

#[test]
fn citations() {
    let h = |s: &str| Org::parse(s).to_html();
    assert_eq!(
        h("[cite:@doe2020]"),
        r#"<main><section><p><cite class="citation" data-keys="doe2020">(doe2020)</cite></p></section></main>"#
    );
    assert!(h("a [cite/p:@doe2020;@smith21] b")
        .contains(r#"<cite class="citation" data-keys="doe2020 smith21">(doe2020; smith21)</cite>"#));
    assert!(h("[cite/t:@doe2020]").contains(r#"data-keys="doe2020">doe2020</cite>"#));
    assert!(h("[cite/t/b:@doe2020]").contains(r#">doe2020</cite>"#));
    assert!(h("[cite:see @doe2020, p. 3]").contains(">(see doe2020, p. 3)</cite>"));
    assert!(h("[cite:cf. ;@a;@b; passim]").contains(">(cf. a; b passim)</cite>"));
    assert!(h("[cite:<b> @a&b]").contains("(&lt;b&gt; a&amp;b)"));
    // other bracket objects are unaffected
    assert!(!h("[fn:1] [[http://x.org][l]] [50%] [2024-01-01]").contains("cite"));
    assert!(h("* Title [cite:@a]\n").contains("<cite"));
    assert!(h("- item [cite:@a]\n").contains("<cite"));
    assert!(h("| [cite:@a] |\n").contains("<cite"));
}
