//! The ports of the inspection core, one file per port.
//!
//! A port declares what the core needs from the outside world as a trait plus
//! the value and error types that cross the boundary. Adapter crates
//! implement these traits; the core never learns which technology sits
//! behind them.

pub mod source_tree;
pub mod syntax_analyzer;
