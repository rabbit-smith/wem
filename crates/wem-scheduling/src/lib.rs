//! WEM encoder scheduling domain: immutable frame plans and mode-selection
//! policy. This crate is the bottom of the dependency chain and must never
//! import analysis, profiles, vorbis, or container crates
//! (see `docs/reference/architecture.md`).
//!
//! Mirrors the Python `wwise_wem/scheduling` package layout.

mod model;
mod planner;
mod selector;

pub use model::{FramePlan, SchedulerState, DEFAULT_BLOCKSIZES};
pub use planner::{
    append_samples, emit_block, initial_state, plan_mode_sequence, plan_mode_sequence_default,
    required_samples, validate_modes, PlannerError,
};
pub use selector::{ModeSelector, SelectorError};
