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

#[cfg(test)]
mod parity {
    //! Oracle tests against the checked stage index
    //! (`tests/data/stage-records/stages/index.json`).

    use serde_json::Value;

    use crate::{plan_mode_sequence, DEFAULT_BLOCKSIZES};

    fn stage_index() -> Value {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(|p| p.parent())
            .expect("repo root")
            .join("tests/data/stage-records/stages/index.json");
        let text = std::fs::read_to_string(path).expect("stage index reads");
        serde_json::from_str(&text).expect("stage index parses")
    }

    fn frames_of(index: &Value) -> &Vec<Value> {
        index
            .get("frames")
            .and_then(Value::as_array)
            .expect("frames array")
    }

    fn int_field(frame: &Value, field: &str) -> i64 {
        frame
            .get(field)
            .and_then(Value::as_i64)
            .unwrap_or_else(|| panic!("frame field {field}"))
    }

    #[test]
    fn stage_index_header() {
        let index = stage_index();
        assert_eq!(index["audio_packets"].as_i64(), Some(205));
        assert_eq!(index["channels"].as_i64(), Some(6));
        assert_eq!(index["sample_rate"].as_i64(), Some(44100));
    }

    #[test]
    fn plan_sequence_matches_index_all_frames() {
        let index = stage_index();
        let frames = frames_of(&index);
        assert_eq!(frames.len(), 205, "stage index records 205 frames");

        let modes: Vec<i64> = frames.iter().map(|f| int_field(f, "mode")).collect();
        let plans = plan_mode_sequence(&modes, &DEFAULT_BLOCKSIZES, 1).expect("plans build");
        assert_eq!(plans.len(), 205);
        for (plan, frame) in plans.iter().zip(frames.iter()) {
            assert_eq!(
                (plan.index, plan.previous, plan.current, plan.following,),
                (
                    int_field(frame, "index"),
                    int_field(frame, "previous"),
                    int_field(frame, "current"),
                    int_field(frame, "following"),
                ),
                "frame {} transition modes",
                plan.index
            );
            // Window centre = (sample_start - long_half) + block_size / 2.
            let block_size = DEFAULT_BLOCKSIZES[plan.current as usize];
            let center = plan.sample_start - DEFAULT_BLOCKSIZES[1] / 2 + block_size / 2;
            assert_eq!(
                center,
                int_field(frame, "window_center"),
                "frame {} window center",
                plan.index
            );
        }
    }
}
