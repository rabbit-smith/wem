//! Transient-driven short/long block mode selection (Python:
//! `scheduling/selector.py`).
//!
//! The lower-level detector supplies one combined integer flag word per
//! 64-sample quantum. This module owns queue timing, look-ahead scanning, and
//! the psychoacoustic profile variant derived for each scheduled frame.

/// Rejection conditions for the mode selector (Python `ValueError` family).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SelectorError {
    /// "selector hop/capacity must be positive"
    HopCapacityNonPositive,
    /// "selector queue slot {slot} exceeds capacity"
    QueueSlotOutOfRange { slot: i64 },
    /// "selector expects Wwise short/long block sizes"
    GenerationGeometry,
    /// "transition look expects short/long mode bits"
    TransitionLookModesInvalid,
    /// "mode generation lacks transient flag rows"
    MissingFlagRows,
    /// "mode generation received surplus transient flag rows"
    SurplusFlagRows,
}

impl std::fmt::Display for SelectorError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        use SelectorError::*;
        match self {
            HopCapacityNonPositive => write!(f, "selector hop/capacity must be positive"),
            QueueSlotOutOfRange { slot } => {
                write!(f, "selector queue slot {slot} exceeds capacity")
            }
            GenerationGeometry => {
                write!(f, "selector expects Wwise short/long block sizes")
            }
            TransitionLookModesInvalid => {
                write!(f, "transition look expects short/long mode bits")
            }
            MissingFlagRows => write!(f, "mode generation lacks transient flag rows"),
            SurplusFlagRows => {
                write!(f, "mode generation received surplus transient flag rows")
            }
        }
    }
}

impl std::error::Error for SelectorError {}

/// Persistent transient queue and short/long selection cursors
/// (Python `ModeSelector`).
pub struct ModeSelector {
    pub hop: i64,
    pub capacity: i64,
    pub cooldown: i64,
    pub generated: i64,
    pub selected: i64,
    pub scan_cursor: i64,
    pub queue: Vec<i64>,
}

/// A summary: every counter, with the queue reported by length.
///
/// The queue is a fixed-capacity ring of transient indices (its full contents
/// are `capacity` values of bookkeeping, not a diagnostic), so its length is
/// shown instead of its contents; everything else here is a scalar and is
/// printed as it stands.
impl std::fmt::Debug for ModeSelector {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ModeSelector")
            .field("hop", &self.hop)
            .field("capacity", &self.capacity)
            .field("cooldown", &self.cooldown)
            .field("generated", &self.generated)
            .field("selected", &self.selected)
            .field("scan_cursor", &self.scan_cursor)
            .field("queue_len", &self.queue.len())
            .finish()
    }
}

impl ModeSelector {
    /// Construct and pad the queue to its capacity
    /// (Python `__post_init__`).
    pub fn new(
        hop: i64,
        capacity: i64,
        cooldown: i64,
        generated: i64,
        selected: i64,
        scan_cursor: i64,
        queue: Vec<i64>,
    ) -> Result<Self, SelectorError> {
        if hop <= 0 || capacity <= 0 {
            return Err(SelectorError::HopCapacityNonPositive);
        }
        let mut queue = queue;
        if (queue.len() as i64) < capacity {
            queue.resize(capacity as usize, 0);
        }
        Ok(Self {
            hop,
            capacity,
            cooldown,
            generated,
            selected,
            scan_cursor,
            queue,
        })
    }

    fn check_slot(&self, index: i64) -> Result<(), SelectorError> {
        if index < 0 || index >= self.capacity {
            return Err(SelectorError::QueueSlotOutOfRange { slot: index });
        }
        Ok(())
    }

    /// Advance and return the shared detector history window.
    ///
    /// Every channel in one quantum receives the same value, and the window
    /// saturates after 24 consecutive non-resetting quanta.
    pub fn begin_quantum(&mut self) -> i64 {
        self.cooldown = (self.cooldown + 1).min(24);
        self.cooldown
    }

    /// Store one already-computed multichannel transient flag word.
    pub fn finish_quantum(&mut self, index: i64, flags: i64) -> Result<(), SelectorError> {
        self.check_slot(index + 2)?;
        self.queue[(index + 2) as usize] = 0;
        if flags & 1 != 0 {
            self.check_slot(index + 1)?;
            self.queue[index as usize] = 1;
            self.queue[(index + 1) as usize] = 1;
        }
        if flags & 2 != 0 {
            self.queue[index as usize] = 1;
            if index != 0 {
                self.queue[(index - 1) as usize] = 1;
            }
        }
        if flags & 4 != 0 {
            self.cooldown = -1;
        }
        Ok(())
    }

    /// Return the selector's native `-1/0/1` look-ahead decision.
    pub fn scan(
        &mut self,
        center: i64,
        current_mode: i64,
        blocksizes: &[i64],
    ) -> Result<i64, SelectorError> {
        if blocksizes.len() != 2 || (current_mode != 0 && current_mode != 1) {
            return Err(SelectorError::GenerationGeometry);
        }
        let boundary =
            blocksizes[current_mode as usize] / 4 + center + blocksizes[1] / 2 + blocksizes[0] / 4;
        let limit = self.generated - self.hop;
        let mut position = self.scan_cursor;
        if position >= limit {
            return Ok(-1);
        }
        loop {
            if position >= boundary {
                return Ok(1);
            }
            self.scan_cursor = position;
            let slot = position.div_euclid(self.hop);
            self.check_slot(slot)?;
            if self.queue[slot as usize] != 0 && position > center {
                self.selected = position;
                return Ok(0);
            }
            position += self.hop;
            if position >= limit {
                return Ok(-1);
            }
        }
    }

    /// Return whether the frame's overlap window contains a transient.
    pub fn has_transient_in_window(
        &self,
        center: i64,
        previous_mode: i64,
        current_mode: i64,
        following_mode: i64,
        blocksizes: &[i64],
    ) -> Result<bool, SelectorError> {
        if blocksizes.len() != 2
            || (previous_mode != 0 && previous_mode != 1)
            || (current_mode != 0 && current_mode != 1)
            || (following_mode != 0 && following_mode != 1)
        {
            return Err(SelectorError::TransitionLookModesInvalid);
        }
        let quarter = blocksizes[current_mode as usize] / 4;
        let mut lower = center - quarter;
        let mut upper = center + quarter;
        if current_mode != 0 {
            lower -= blocksizes[previous_mode as usize] / 4;
            upper += blocksizes[following_mode as usize] / 4;
        } else {
            let short_quarter = blocksizes[0] / 4;
            lower -= short_quarter;
            upper += short_quarter;
        }

        // Check the exact selected position before scanning queue slots.
        // Profile positions are non-negative; the division deliberately
        // retains truncation toward zero for a generic early negative
        // boundary (matching Python `int(lower / self.hop)`).
        if lower <= self.selected && self.selected < upper {
            return Ok(true);
        }
        let first = lower / self.hop;
        let limit = upper / self.hop;
        for slot in first..limit {
            if 0 <= slot && slot < self.capacity && self.queue[slot as usize] != 0 {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// Return the psychoacoustic variant code for one scheduled frame.
    ///
    /// The low bit selects the profile variant and the high bit represents
    /// the current block mode. Long blocks combine adjacent long modes;
    /// short blocks invert the transient-window result.
    pub fn transition_code(
        &self,
        center: i64,
        previous_mode: i64,
        current_mode: i64,
        following_mode: i64,
        blocksizes: &[i64],
    ) -> Result<i64, SelectorError> {
        let variant = if current_mode != 0 {
            i64::from(previous_mode != 0 && following_mode != 0)
        } else {
            i64::from(!self.has_transient_in_window(
                center,
                previous_mode,
                current_mode,
                following_mode,
                blocksizes,
            )?)
        };
        Ok(2 * current_mode + variant)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constructor_validation_and_padding() {
        assert!(ModeSelector::new(0, 4, 0, 0, 0, 0, vec![]).is_err());
        assert!(ModeSelector::new(4, 0, 0, 0, 0, 0, vec![]).is_err());
        let sel = ModeSelector::new(4, 4, 0, 0, 0, 0, vec![1]).expect("ok");
        assert_eq!(sel.queue, vec![1, 0, 0, 0]);
    }

    #[test]
    fn quantum_timing_saturation_and_reset() {
        let mut sel = ModeSelector::new(64, 8, 0, 0, 0, 0, vec![0; 8]).expect("ok");
        assert_eq!(sel.begin_quantum(), 1);
        sel.cooldown = 23;
        assert_eq!(sel.begin_quantum(), 24);
        assert_eq!(sel.begin_quantum(), 24); // saturates
        sel.cooldown = 10;
        sel.finish_quantum(0, 4).expect("reset flag");
        assert_eq!(sel.cooldown, -1);
        sel.finish_quantum(0, 1).expect("transient");
        assert_eq!(sel.queue[0], 1);
        assert_eq!(sel.queue[1], 1);
        sel.finish_quantum(0, 2).expect("look-ahead");
        assert_eq!(sel.queue[0], 1);
        assert_eq!(sel.queue[2], 0);
    }

    #[test]
    fn queue_slot_rejection() {
        let mut sel = ModeSelector::new(64, 4, 0, 0, 0, 0, vec![0; 4]).expect("ok");
        let err = sel.finish_quantum(2, 1).unwrap_err();
        assert!(matches!(
            err,
            SelectorError::QueueSlotOutOfRange { slot: 4 }
        ));
    }

    #[test]
    fn scan_decisions() {
        let bs = [256, 2048];
        let mut sel = ModeSelector::new(64, 32, 0, 1024, 0, 0, vec![0; 32]).expect("ok");
        // No queue entry yet beyond the cursor.
        assert_eq!(sel.scan(0, 0, &bs).expect("ok"), -1);
        assert_eq!(sel.scan_cursor, 896);
        // A transient past the center selects 0 and pins it.
        sel.queue[2] = 1; // slot 2 -> position 128
        sel.scan_cursor = 0;
        assert_eq!(sel.scan(0, 0, &bs).expect("ok"), 0);
        assert_eq!(sel.selected, 128);
        assert_eq!(sel.scan_cursor, 128);
    }

    #[test]
    fn transition_code_short_inversion() {
        let bs = [256, 2048];
        // Fresh selector with no transients: short blocks invert an empty
        // window (no transient -> code 1) and long blocks combine.
        let sel = ModeSelector::new(64, 8, 0, 0, 0, 2048, vec![0; 8]).expect("ok");
        assert_eq!(sel.transition_code(1024, 0, 0, 0, &bs).expect("ok"), 1);
        assert_eq!(sel.transition_code(1024, 1, 1, 1, &bs).expect("ok"), 3);
        assert_eq!(sel.transition_code(1024, 1, 1, 0, &bs).expect("ok"), 2);
    }
}
