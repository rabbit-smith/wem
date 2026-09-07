//! Immutable scheduling values shared by frame planning and PCM
//! materialization (Python: `scheduling/model.py`).

/// The default short/long block sizes (Python `DEFAULT_BLOCKSIZES`).
pub const DEFAULT_BLOCKSIZES: [i64; 2] = [256, 2048];

/// Persistent overlap state, expressed without PCM buffers
/// (Python `SchedulerState`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SchedulerState {
    pub previous: i64,
    pub current: i64,
    pub cursor: i64,
    pub filled: i64,
    pub buffer_base: i64,
    pub emitted: i64,
}

impl SchedulerState {
    /// Construct with zero `buffer_base` and `emitted`
    /// (Python dataclass defaults).
    pub fn new(previous: i64, current: i64, cursor: i64, filled: i64) -> Self {
        Self {
            previous,
            current,
            cursor,
            filled,
            buffer_base: 0,
            emitted: 0,
        }
    }
}

/// One ready analysis frame and its exact source interval
/// (Python `FramePlan`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FramePlan {
    pub index: i64,
    pub previous: i64,
    pub current: i64,
    pub following: i64,
    pub sample_start: i64,
    pub sample_end: i64,
    pub required_filled: i64,
    pub advance: i64,
}
