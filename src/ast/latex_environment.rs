use crate::SyntaxKind;

use super::LatexEnvironment;

impl LatexEnvironment {
    /// The environment's own text, without the affiliated keywords before it.
    pub fn value(&self) -> String {
        self.syntax
            .children_with_tokens()
            .filter(|c| c.kind() != SyntaxKind::AFFILIATED_KEYWORD)
            .map(|c| c.to_string())
            .collect()
    }
}
