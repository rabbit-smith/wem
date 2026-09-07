//! WEM encoder scheduling domain: immutable frame plans and mode-selection
//! policy. This crate is the bottom of the dependency chain and must never
//! import analysis, profiles, vorbis, or container crates
//! (see `docs/architecture.md`).
//!
//! Mirrors the Python `wwise_wem/scheduling` package layout.

/// `FramePlan`, `WindowedFrame`, `DEFAULT_BLOCKSIZES`, and scheduling model
/// types (Python: `scheduling/model.py`). Filled in by the scheduling port.
pub mod model {
    // Intentionally empty: type definitions land with the scheduling port.
}

/// Frame planning: the deterministic short/long mode sequence
/// (`plan_mode_sequence`; Python: `scheduling/planner.py`).
pub mod planner {
    // Intentionally empty: planning functions land with the scheduling port.
}

/// Mode selection policy (`ModeSelector`; Python: `scheduling/selector.py`).
pub mod selector {
    // Intentionally empty: selector lands with the scheduling port.
}
