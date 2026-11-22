//! Shared data structures: source spans, diagnostics, the structured HIR
//! produced by the frontend, and the IR type lattice used by inference.
//!
//! M0/M1 note: codegen currently lowers from the structured HIR. The
//! block/instruction IR described in the design (section 9) is introduced
//! when hybrid planning (M3) needs explicit exceptional edges.

pub mod diag;
pub mod hir;
pub mod types;

pub use diag::{Diagnostic, Severity};
pub use hir::*;
pub use types::Ty;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize)]
pub struct Span {
    pub line: u32,
    /// 1-based column (CPython's `col_offset` + 1).
    pub col: u32,
    pub end_line: u32,
    pub end_col: u32,
    /// Index into `Module::units` (0 = entry file).
    pub unit: u32,
}

impl std::fmt::Display for Span {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}", self.line, self.col)
    }
}

/// Compilation mode (design section 4).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Mode {
    #[default]
    Hybrid,
    NativeOnly,
}
