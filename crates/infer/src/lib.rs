//! Type and effect inference: symbolic generic schemes for reporting, and
//! concrete reachability-driven specialization for code generation.

pub mod definite;
pub mod generic;
pub mod ops;
pub mod spec;

pub use spec::{specialize, spec_label, Program, Slot, Spec, SpecKey};
