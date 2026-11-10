//! Org-mode to the standalone HTML page a LibrePaper document is stored as,
//! compiled to WebAssembly.
//!
//! Two layers live here. The parser and its syntax tree (`ast`, `syntax`,
//! `config`, `export`, [`Org`]) began as orgize by Alex Lin (poi),
//! <https://github.com/PoiScript/orgize>, and are developed here from that
//! starting point. The renderer (`render`) turns a parsed document into the
//! page LibrePaper stores and previews.
//!
//! The page template, the diagnostics and the WebAssembly interface come from
//! `wasm-helpers`, as they do for every other renderer.

/// The shared page template and the shape a compile answers in, re-exported so
/// that `crate::page` and `crate::diagnostic` mean here what they mean in every
/// other renderer.
pub use wasm_helpers::{diagnostic, page};

pub mod ast;
pub mod config;
mod entities;
pub mod lezer;
pub mod export;
mod org;
pub mod render;
mod replace;
mod syntax;
#[cfg(test)]
mod tests;

// Re-export of the rowan crate.
pub use rowan;

pub use config::ParseConfig;
pub use org::Org;
pub use rowan::{TextRange, TextSize};
pub use syntax::{
    SyntaxElement, SyntaxElementChildren, SyntaxKind, SyntaxNode, SyntaxNodeChildren, SyntaxToken,
};

pub(crate) use syntax::combinator::lossless_parser;

#[cfg(all(target_arch = "wasm32", feature = "exports"))]
mod abi;
