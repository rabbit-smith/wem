//! Pure readiness and overlap planning for Wwise analysis frames
//! (Python: `scheduling/planner.py`).

use crate::model::{FramePlan, SchedulerState, DEFAULT_BLOCKSIZES};

/// Rejection conditions for the scheduler (Python `ValueError` /
/// `AssertionError` family in `scheduling/planner.py`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlannerError {
    /// "the Wwise scheduler has exactly short/long block sizes"
    BlockSizeCount,
    /// "block sizes must be positive even values"
    BlockSizeNotEven,
    /// "mode must be 0 (short) or 1 (long)"
    ModeNotBit,
    /// "append count must be non-negative"
    AppendCountNegative,
    /// "need {required} buffered samples for mode {current}->{following};
    /// have {filled}"
    InsufficientSamples {
        required: i64,
        have: i64,
        current: i64,
        following: i64,
    },
    /// "scheduler ring advance must be positive"
    NonPositiveAdvance,
    /// "mode sequence and scheduler state diverged"
    StateDiverged,
}

impl std::fmt::Display for PlannerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        use PlannerError::*;
        match self {
            BlockSizeCount => {
                write!(f, "the Wwise scheduler has exactly short/long block sizes")
            }
            BlockSizeNotEven => write!(f, "block sizes must be positive even values"),
            ModeNotBit => write!(f, "mode must be 0 (short) or 1 (long)"),
            AppendCountNegative => write!(f, "append count must be non-negative"),
            InsufficientSamples {
                required,
                have,
                current,
                following,
            } => {
                write!(
                    f,
                    "need {required} buffered samples for mode {current}->{following}; \
                     have {have}"
                )
            }
            NonPositiveAdvance => {
                write!(f, "scheduler ring advance must be positive")
            }
            StateDiverged => write!(f, "mode sequence and scheduler state diverged"),
        }
    }
}

impl std::error::Error for PlannerError {}

/// Validate the block sizes and every supplied mode bit
/// (Python `_validate_modes`).
pub fn validate_modes(blocksizes: &[i64], modes: &[i64]) -> Result<(), PlannerError> {
    if blocksizes.len() != 2 {
        return Err(PlannerError::BlockSizeCount);
    }
    if blocksizes.iter().any(|size| *size < 2 || *size & 1 != 0) {
        return Err(PlannerError::BlockSizeNotEven);
    }
    if modes.iter().any(|mode| *mode != 0 && *mode != 1) {
        return Err(PlannerError::ModeNotBit);
    }
    Ok(())
}

/// Return the overlap state before the first PCM append
/// (Python `initial_state`).
pub fn initial_state(blocksizes: &[i64]) -> Result<SchedulerState, PlannerError> {
    validate_modes(blocksizes, &[0, 0])?;
    let long_half = blocksizes[1] / 2;
    Ok(SchedulerState::new(0, 0, long_half, long_half))
}

/// Return the exact readiness threshold for a chosen following mode
/// (Python `required_samples`).
pub fn required_samples(
    state: &SchedulerState,
    following: i64,
    blocksizes: &[i64],
) -> Result<i64, PlannerError> {
    validate_modes(blocksizes, &[state.previous, state.current, following])?;
    let current_size = blocksizes[state.current as usize];
    let following_size = blocksizes[following as usize];
    Ok(state.cursor + following_size / 4 + current_size / 4 + following_size / 2)
}

/// Emit one frame plan and return the post-slide state
/// (Python `emit_block`).
pub fn emit_block(
    state: &SchedulerState,
    following: i64,
    blocksizes: &[i64],
) -> Result<(FramePlan, SchedulerState), PlannerError> {
    validate_modes(blocksizes, &[state.previous, state.current, following])?;
    let required = required_samples(state, following, blocksizes)?;
    if state.filled < required {
        return Err(PlannerError::InsufficientSamples {
            required,
            have: state.filled,
            current: state.current,
            following,
        });
    }

    let current_size = blocksizes[state.current as usize];
    let following_size = blocksizes[following as usize];
    let long_half = blocksizes[1] / 2;
    let sample_start = state.buffer_base + state.cursor - current_size / 2;
    let advance = state.cursor + following_size / 4 + current_size / 4 - long_half;
    if advance <= 0 {
        return Err(PlannerError::NonPositiveAdvance);
    }

    let plan = FramePlan {
        index: state.emitted,
        previous: state.previous,
        current: state.current,
        following,
        sample_start,
        sample_end: sample_start + current_size,
        required_filled: required,
        advance,
    };
    let next_state = SchedulerState {
        previous: state.current,
        current: following,
        cursor: long_half,
        filled: state.filled - advance,
        buffer_base: state.buffer_base + advance,
        emitted: state.emitted + 1,
    };
    Ok((plan, next_state))
}

/// Return the same state after the feeder appends `count` samples
/// (Python `append_samples`).
pub fn append_samples(state: &SchedulerState, count: i64) -> Result<SchedulerState, PlannerError> {
    if count < 0 {
        return Err(PlannerError::AppendCountNegative);
    }
    Ok(SchedulerState {
        filled: state.filled + count,
        ..*state
    })
}

/// Turn a complete mode sequence into its immutable frame plans
/// (Python `plan_mode_sequence`).
///
/// `FramePlan` intervals use the scheduler timeline, whose zero precedes
/// PCM sample zero by one long half-block. PCM materialization applies that
/// single origin offset; all overlap advances and transition modes remain
/// owned here.
pub fn plan_mode_sequence(
    modes: &[i64],
    blocksizes: &[i64],
    terminal_following: i64,
) -> Result<Vec<FramePlan>, PlannerError> {
    if modes.is_empty() {
        return Ok(Vec::new());
    }
    let mut all_modes = modes.to_vec();
    all_modes.push(terminal_following);
    validate_modes(blocksizes, &all_modes)?;

    let long_half = blocksizes[1] / 2;
    let mut state = SchedulerState {
        previous: 0,
        current: modes[0],
        cursor: long_half,
        filled: long_half,
        buffer_base: 0,
        emitted: 0,
    };
    let mut plans: Vec<FramePlan> = Vec::with_capacity(modes.len());
    for (index, &current) in modes.iter().enumerate() {
        if state.current != current {
            return Err(PlannerError::StateDiverged);
        }
        let following = if index + 1 < modes.len() {
            modes[index + 1]
        } else {
            terminal_following
        };
        let required = required_samples(&state, following, blocksizes)?;
        state = append_samples(&state, (required - state.filled).max(0))?;
        let (plan, next_state) = emit_block(&state, following, blocksizes)?;
        plans.push(plan);
        state = next_state;
    }
    Ok(plans)
}

/// Convenience overload defaulting to `DEFAULT_BLOCKSIZES`.
pub fn plan_mode_sequence_default(
    modes: &[i64],
    terminal_following: i64,
) -> Result<Vec<FramePlan>, PlannerError> {
    plan_mode_sequence(modes, &DEFAULT_BLOCKSIZES, terminal_following)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::DEFAULT_BLOCKSIZES;

    #[test]
    fn plan_validation_rejections() {
        assert!(matches!(
            plan_mode_sequence(&[0], &[256, 2048, 4096], 1).unwrap_err(),
            PlannerError::BlockSizeCount
        ));
        assert!(matches!(
            plan_mode_sequence(&[0], &[255, 2048], 1).unwrap_err(),
            PlannerError::BlockSizeNotEven
        ));
        assert!(matches!(
            plan_mode_sequence(&[2], &DEFAULT_BLOCKSIZES, 1).unwrap_err(),
            PlannerError::ModeNotBit
        ));
        assert!(matches!(
            plan_mode_sequence(&[0, 1], &DEFAULT_BLOCKSIZES, 9).unwrap_err(),
            PlannerError::ModeNotBit
        ));
        assert!(plan_mode_sequence(&[], &DEFAULT_BLOCKSIZES, 1)
            .unwrap()
            .is_empty());
        assert!(append_samples(&SchedulerState::new(0, 0, 0, 0), -1,).is_err());
    }

    #[test]
    fn insufficient_samples_message() {
        let state = SchedulerState::new(0, 0, 1024, 5);
        let err = emit_block(&state, 1, &DEFAULT_BLOCKSIZES).unwrap_err();
        assert!(matches!(
            err,
            PlannerError::InsufficientSamples {
                required: 2624,
                have: 5,
                current: 0,
                following: 1,
            }
        ));
    }

    #[test]
    fn plan_short_sequence_geometry() {
        // Two short blocks then a long terminal; scheduler timeline origin
        // precedes PCM sample zero by one long half-block (oracle values).
        let plans = plan_mode_sequence(&[0, 0, 1], &DEFAULT_BLOCKSIZES, 1).expect("plans");
        assert_eq!(plans.len(), 3);
        assert_eq!(
            (plans[0].previous, plans[0].current, plans[0].following),
            (0, 0, 0)
        );
        assert_eq!(plans[0].sample_start, 896);
        assert_eq!(plans[0].sample_end, 1152);
        assert_eq!(plans[0].advance, 128);
        assert_eq!(
            (plans[1].previous, plans[1].current, plans[1].following),
            (0, 0, 1)
        );
        assert_eq!(plans[1].sample_start, 1024);
        assert_eq!(
            (plans[2].previous, plans[2].current, plans[2].following),
            (0, 1, 1)
        );
        assert_eq!(plans[2].sample_start, 704);
        assert_eq!(plans[2].sample_end, 2752);
    }
}
