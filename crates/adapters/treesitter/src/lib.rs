//! Tree-sitter adapter: implements
//! [`cutaway_inspection::ports::syntax_analyzer::SyntaxAnalyzer`] per language.
//!
//! One analyzer per language, each backed by that language's tree-sitter
//! grammar. Rust is the first; nothing outside this crate knows which
//! languages exist, so adding a language means adding an analyzer here and
//! wiring it in the composition root.

mod rust;

pub use rust::RustSyntaxAnalyzer;
