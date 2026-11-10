use rowan::{ast::AstNode, TextRange, TextSize};

use crate::syntax::{OrgLanguage, SyntaxKind, SyntaxNode};

/// An Org 9.5 citation, `[cite/style:global prefix; prefix @key suffix; global suffix]`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Citation {
    pub(crate) syntax: SyntaxNode,
}

impl AstNode for Citation {
    type Language = OrgLanguage;
    fn can_cast(kind: SyntaxKind) -> bool {
        kind == SyntaxKind::CITATION
    }
    fn cast(node: SyntaxNode) -> Option<Citation> {
        Self::can_cast(node.kind()).then(|| Citation { syntax: node })
    }
    fn syntax(&self) -> &SyntaxNode {
        &self.syntax
    }
}

impl Citation {
    pub fn start(&self) -> TextSize {
        self.syntax.text_range().start()
    }
    pub fn end(&self) -> TextSize {
        self.syntax.text_range().end()
    }
    pub fn text_range(&self) -> TextRange {
        self.syntax.text_range()
    }

    /// The style, without its leading slash: `t/b` for `[cite/t/b:...]`.
    pub fn style(&self) -> Option<String> {
        self.syntax
            .children_with_tokens()
            .filter_map(|e| e.into_token())
            .find(|t| t.kind() == SyntaxKind::CITATION_STYLE)
            .map(|t| t.text().trim_start_matches('/').to_string())
    }
}
