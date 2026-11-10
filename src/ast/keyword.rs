use crate::SyntaxKind;

use super::{filter_token, Keyword, Token};

impl Keyword {
    ///
    /// ```rust
    /// use wasm_orgmode::{Org, ast::Keyword};
    ///
    /// let keyword = Org::parse("#+KEY: VALUE\nabc").first_node::<Keyword>().unwrap();
    /// assert_eq!(keyword.key(), "KEY");
    /// ```
    pub fn key(&self) -> Token {
        self.syntax
            .children_with_tokens()
            .find_map(filter_token(SyntaxKind::TEXT))
            .expect("keyword must contains TEXT")
    }

    /// The text after the colon. Markup in a TITLE, SUBTITLE, AUTHOR, DATE or
    /// CAPTION value is parsed, but this is still its source text.
    ///
    /// ```rust
    /// use wasm_orgmode::{Org, ast::Keyword};
    ///
    /// let keyword = Org::parse("#+KEY: VALUE\nabc").first_node::<Keyword>().unwrap();
    /// assert_eq!(keyword.value(), " VALUE");
    /// let keyword = Org::parse("#+KEY:").first_node::<Keyword>().unwrap();
    /// assert_eq!(keyword.value(), "");
    /// let keyword = Org::parse("#+TITLE: A *bold* title").first_node::<Keyword>().unwrap();
    /// assert_eq!(keyword.value(), " A *bold* title");
    /// ```
    pub fn value(&self) -> String {
        super::keyword_value(&self.syntax).unwrap_or_default()
    }
}
