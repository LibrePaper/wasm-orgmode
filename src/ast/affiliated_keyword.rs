use crate::syntax::SyntaxKind;

use super::{filter_token, AffiliatedKeyword, Token};

impl AffiliatedKeyword {
    ///
    /// ```rust
    /// use wasm_orgmode::{Org, ast::AffiliatedKeyword};
    ///
    /// let keyword = Org::parse("#+CAPTION: VALUE\nabc").first_node::<AffiliatedKeyword>().unwrap();
    /// assert_eq!(keyword.key(), "CAPTION");
    /// ```
    pub fn key(&self) -> Token {
        self.syntax
            .children_with_tokens()
            .find_map(filter_token(SyntaxKind::TEXT))
            .expect("keyword must contains TEXT")
    }

    ///
    /// ```rust
    /// use wasm_orgmode::{Org, ast::AffiliatedKeyword};
    ///
    /// let keyword = Org::parse("#+CAPTION: VALUE\nabc").first_node::<AffiliatedKeyword>().unwrap();
    /// assert!(keyword.optional().is_none());
    /// let keyword = Org::parse("#+CAPTION[OPTIONAL]: VALUE\nabc").first_node::<AffiliatedKeyword>().unwrap();
    /// assert_eq!(keyword.optional().unwrap(), "OPTIONAL");
    /// ```
    pub fn optional(&self) -> Option<Token> {
        self.syntax
            .children_with_tokens()
            .skip_while(|it| it.kind() != SyntaxKind::L_BRACKET)
            .nth(1)
            .and_then(filter_token(SyntaxKind::TEXT))
    }

    ///
    /// ```rust
    /// use wasm_orgmode::{Org, ast::AffiliatedKeyword};
    ///
    /// let keyword = Org::parse("#+CAPTION: VALUE\nabc").first_node::<AffiliatedKeyword>().unwrap();
    /// assert_eq!(keyword.value().unwrap(), " VALUE");
    /// let keyword = Org::parse("#+CAPTION[OPTIONAL]:VALUE\nabc").first_node::<AffiliatedKeyword>().unwrap();
    /// assert_eq!(keyword.value().unwrap(), "VALUE");
    /// ```
    pub fn value(&self) -> Option<String> {
        super::keyword_value(&self.syntax)
    }

    /// The value as objects: the children of its `KEYWORD_VALUE` node, or its
    /// text. Empty when there is no value.
    pub fn value_nodes(&self) -> impl Iterator<Item = crate::SyntaxElement> {
        self.syntax
            .children()
            .find(|n| n.kind() == SyntaxKind::KEYWORD_VALUE)
            .into_iter()
            .flat_map(|n| n.children_with_tokens())
    }
}
