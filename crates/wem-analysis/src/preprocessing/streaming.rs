//! Incremental PCM feeder for the true streaming path.
//!
//! The batch paths ([`iter_detector_quanta`](crate::preprocessing::detector_input::iter_detector_quanta)
//! and
//! [`iter_planned_pcm_windows`](crate::preprocessing::windowing::iter_planned_pcm_windows))
//! materialize from the complete
//! source in memory. [`StreamingPcmFeeder`] shares the exact same kernel —
//! the same LPC functions, the same sample-selection views — but retains
//! only a bounded window, so input memory stays O(ring + tail) instead of
//! O(source length):
//!
//! * the most recent [`STREAM_RING_KEEP`] samples per channel, which covers
//!   the 4096-sample LPC tail model, the 8192-sample detector
//!   end-of-stream tail (whose detector windows can reference source
//!   samples up to 8192 + 1024 = 9216 positions before the endpoint), and
//!   every in-flight 2048-sample frame window;
//! * the cached first-frame prime (one long half-block per channel),
//!   computed from the first 4096 samples once they have been seen;
//! * the end-of-stream tail (8192 samples per channel), computed once by
//!   [`StreamingPcmFeeder::finish_source`] from the last 4096 samples.
//!
//! Detector quanta are handed out **as they complete**:
//! [`StreamingPcmFeeder::push`]
//! returns the windows whose 128-sample detector intervals end inside the
//! appended chunk, extracted before the ring evicts anything, so a single
//! huge chunk never loses its early quanta. Rejection conditions and float
//! semantics track the batch paths: every selected sample goes through the
//! same views that
//! [`iter_planned_pcm_windows`](crate::preprocessing::windowing::iter_planned_pcm_windows)
//! selects
//! (negative index → prime, source → retained samples, tail → tail
//! prediction).

use std::collections::VecDeque;

use wem_scheduling::FramePlan;

use crate::config::{f32_of, AnalysisError};
use crate::dsp::lpc::{wwise_first_frame_lpc_prime, wwise_lpc_from_data, wwise_lpc_predict};

/// Most recent source samples retained per channel.
///
/// Bound: the detector end-of-stream tail spans 8192 samples past the
/// endpoint, and its detector windows (128 wide on a 64 hop, offset by the
/// 1024-sample prime) can reference source samples up to 8192 + 1024 =
/// 9216 positions before the endpoint; the at-most-2048-sample tail model and
/// 2048-sample frame windows fit inside that.
pub const STREAM_RING_KEEP: i64 = 9216;

/// Samples the end-of-stream tail prediction extends past the source
/// endpoint (matches the batch `select_modes` / `iter_detector_quanta`
/// `terminal_samples`).
pub const STREAM_TAIL_SAMPLES: i64 = 8192;

/// Source samples needed for the first-frame LPC prime (batch invariant).
pub const STREAM_PRIME_BATCH: i64 = 4096;

/// Tail model order / context (batch invariants).
const TAIL_ORDER: i64 = 32;
const TAIL_CONTEXT: i64 = 32;

/// Detector quantum hop / window (batch invariants).
pub const STREAM_DETECTOR_HOP: i64 = 64;
pub const STREAM_DETECTOR_WINDOW: i64 = 128;

/// Bounded incremental PCM feeder over a chunked stream.
///
/// The streaming counterpart of the batch detector/window materializers:
/// it exposes the same detector-stream quanta and frame windows while
/// retaining only [`STREAM_RING_KEEP`] source samples per channel plus the
/// LPC boundary views. Note the batch kernel keeps *two* first-frame
/// primes with different prefills — the detector stream prepends
/// one long half-block (1024), while frame windowing primes with 128;
/// both are cached here. Frame plans and mode decisions stay owned by
/// the analysis session and scheduler; this type only owns sample
/// retention and materialization.
pub struct StreamingPcmFeeder {
    channels: i64,
    blocksizes: [i64; 2],
    /// Most recent `min(total, STREAM_RING_KEEP)` source samples per
    /// channel (ring-buffered).
    ring: Vec<VecDeque<f64>>,
    /// Head samples retained until the first-frame primes are computable.
    prefix: Vec<Vec<f64>>,
    total: i64,
    /// Detector-stream prime (prefill 1024 = one long half-block).
    detector_prime: Option<Vec<Vec<f64>>>,
    /// Frame-windowing prime (prefill 128, the batch default).
    window_prime: Option<Vec<Vec<f64>>>,
    tails: Option<Vec<Vec<f64>>>,
    /// Every quantum below this index has been handed out
    /// (through `push` / `finish_source`).
    quanta_extracted_through: i64,
}

/// A summary, deliberately: the geometry, how far the stream has been
/// consumed, and the sizes of the retained sample buffers.
///
/// The ring, prefix, primes and tail hold thousands of `f64` samples per
/// channel — printing them would bury the diagnostic that asked for the
/// feeder — so they are reported by count and by whether they are populated;
/// `total` and `quanta_extracted_through` are the counters that say where the
/// stream stands.
impl std::fmt::Debug for StreamingPcmFeeder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StreamingPcmFeeder")
            .field("channels", &self.channels)
            .field("blocksizes", &self.blocksizes)
            .field("total_samples", &self.total)
            .field("ring_keep", &self.ring.first().map(VecDeque::len))
            .field("prefix_samples", &self.prefix.first().map(Vec::len))
            .field("detector_prime", &self.detector_prime.is_some())
            .field("window_prime", &self.window_prime.is_some())
            .field("tails", &self.tails.is_some())
            .field("quanta_extracted_through", &self.quanta_extracted_through)
            .finish()
    }
}

impl StreamingPcmFeeder {
    /// Construct an empty feeder for the Wwise block geometry
    /// (mirrors the batch 256/2048 invariant).
    pub fn new(channels: i64, blocksizes: [i64; 2]) -> Result<Self, AnalysisError> {
        use AnalysisError::*;
        if channels <= 0 {
            return Err(PcmFeederEmpty);
        }
        if blocksizes != [256, 2048] {
            return Err(AnalysisError::SessionBlockSizeMismatch {
                got: [blocksizes[0], blocksizes[1]],
            });
        }
        Ok(Self {
            channels,
            blocksizes,
            ring: vec![VecDeque::with_capacity(STREAM_RING_KEEP as usize); channels as usize],
            prefix: vec![Vec::with_capacity(STREAM_PRIME_BATCH as usize); channels as usize],
            total: 0,
            detector_prime: None,
            window_prime: None,
            tails: None,
            quanta_extracted_through: 0,
        })
    }

    /// The profiled channel count.
    pub fn channels(&self) -> i64 {
        self.channels
    }

    /// Total source samples consumed so far (per channel).
    pub fn total_samples(&self) -> i64 {
        self.total
    }

    /// Whether the end-of-stream tail has been computed.
    pub fn eos(&self) -> bool {
        self.tails.is_some()
    }

    /// The next detector quantum index that will be handed out
    /// (all quanta below it have been delivered through `push` /
    /// `finish_source`).
    pub fn quanta_extracted_through(&self) -> i64 {
        self.quanta_extracted_through
    }

    /// Append one chunk of channel-major float rows.
    ///
    /// Both the completed detector quanta and the ring's full pre-append
    /// content stay available until [`settle`](Self::settle): the caller
    /// must first consume the quanta (through
    /// [`for_each_completed_quantum`](Self::for_each_completed_quantum)),
    /// emit every newly ready frame (whose windows may reach back into the
    /// oldest retained samples), and only then call `settle` to evict. A
    /// single huge chunk never loses its early quanta or early frames
    /// this way, and no batch of quanta is ever materialized at once.
    ///
    /// All rows must have equal length; the legacy `f32` rounding
    /// boundary is applied here, exactly where the batch
    /// `detector_pcm_streams` applies it.
    pub fn push(&mut self, rows: &[Vec<f64>]) -> Result<(), AnalysisError> {
        use AnalysisError::*;
        if rows.len() as i64 != self.channels {
            return Err(StreamFeederChannelsMismatch {
                want: self.channels,
                got: rows.len() as i64,
            });
        }
        let chunk_len = rows.first().map(|row| row.len() as i64).unwrap_or(0);
        if rows.iter().any(|row| row.len() as i64 != chunk_len) {
            return Err(PcmChannelsUnequal {
                want: chunk_len,
                got: 0,
            });
        }
        // Append without evicting: the quanta completed by this append may
        // reference the ring's oldest samples and must be extracted while
        // everything is still in hand.
        for (channel_index, channel) in rows.iter().enumerate() {
            let rounded = channel.iter().map(|value| f32_of(*value));
            self.ring[channel_index].extend(rounded.clone());
            if self.detector_prime.is_none() {
                self.prefix[channel_index].extend(rounded);
                // Cap the prefix: the primes only ever read the first
                // STREAM_PRIME_BATCH samples (a huge first chunk must not
                // escape the memory bound).
                if self.prefix[channel_index].len() > STREAM_PRIME_BATCH as usize {
                    self.prefix[channel_index].truncate(STREAM_PRIME_BATCH as usize);
                }
            }
        }
        self.total += chunk_len;
        if self.detector_prime.is_none() && self.total >= STREAM_PRIME_BATCH {
            self.compute_primes()?;
        }
        Ok(())
    }

    /// Consume every detector quantum completed so far, one at a time, in
    /// quantum order, through `consume(quantum_index, window)`.
    ///
    /// Each window is a 128-sample-per-channel slice materialized from the
    /// retained views (prime / ring / EOS tail); the consumer may use it
    /// until the next call. The quantum indices advance monotonically
    /// through the detector stream — the batch path ingests exactly these
    /// quanta in exactly this order.
    pub fn for_each_completed_quantum(
        &mut self,
        mut consume: impl FnMut(i64, &[Vec<f64>]) -> Result<(), AnalysisError>,
    ) -> Result<(), AnalysisError> {
        let stream_len = self.detector_stream_length();
        while (self.quanta_extracted_through * STREAM_DETECTOR_HOP + STREAM_DETECTOR_WINDOW)
            <= stream_len
        {
            if let Some(window) = self.detector_quantum(self.quanta_extracted_through) {
                consume(self.quanta_extracted_through, &window)?;
            }
            self.quanta_extracted_through += 1;
        }
        Ok(())
    }

    /// Materialize every detector quantum completed so far (batch helper;
    /// streaming consumers prefer
    /// [`for_each_completed_quantum`](Self::for_each_completed_quantum)).
    pub fn drain_completed_quanta(&mut self) -> Result<Vec<Vec<Vec<f64>>>, AnalysisError> {
        let mut quanta = Vec::new();
        self.for_each_completed_quantum(|_, window| {
            quanta.push(window.to_vec());
            Ok(())
        })?;
        Ok(quanta)
    }

    /// Evict the ring down to the bounded retention window.
    ///
    /// Call after the quanta from `push` are ingested and every newly
    /// ready frame is emitted: by then each emitted frame's window lies
    /// inside the retained region (a frame becomes mode-ready at
    /// `total >= center + 2048`, far before its samples leave the
    /// 9216-sample ring at `center + 8192`).
    pub fn settle(&mut self) {
        for ring in self.ring.iter_mut() {
            while (ring.len() as i64) > STREAM_RING_KEEP {
                ring.pop_front();
            }
        }
    }

    /// Compute and cache the first-frame LPC primes (the detector stream
    /// prepends one long half-block of 1024; the frame windowing primes
    /// with the batch default 128).
    fn compute_primes(&mut self) -> Result<(), AnalysisError> {
        use AnalysisError::*;
        let detector_prefill = self.blocksizes[1] / 2;
        let window_prefill = 128;
        let mut detector_prime = Vec::with_capacity(self.channels as usize);
        let mut window_prime = Vec::with_capacity(self.channels as usize);
        for channel in &self.prefix {
            if (channel.len() as i64) < STREAM_PRIME_BATCH {
                return Err(StreamFeederSourceShort {
                    frames: channel.len() as i64,
                });
            }
            let batch = &channel[..STREAM_PRIME_BATCH as usize];
            detector_prime.push(wwise_first_frame_lpc_prime(
                batch,
                detector_prefill,
                STREAM_PRIME_BATCH,
                16,
            )?);
            window_prime.push(wwise_first_frame_lpc_prime(
                batch,
                window_prefill,
                STREAM_PRIME_BATCH,
                16,
            )?);
        }
        self.prefix = vec![Vec::new(); self.channels as usize];
        self.detector_prime = Some(detector_prime);
        self.window_prime = Some(window_prime);
        Ok(())
    }

    /// Mark the end of the stream and compute the end-of-stream tail
    /// (8192 samples per channel, 32-tap, trained on the PCM remaining in the
    /// analysis buffer at EOS, capped at one long block). The remaining quanta become
    /// consumable through
    /// [`for_each_completed_quantum`](Self::for_each_completed_quantum).
    pub fn finish_source(&mut self, tail_training: Option<i64>) -> Result<(), AnalysisError> {
        use AnalysisError::*;
        if self.tails.is_some() {
            return Ok(());
        }
        if self.total < STREAM_PRIME_BATCH {
            return Err(StreamFeederSourceShort { frames: self.total });
        }
        let mut tails = Vec::with_capacity(self.channels as usize);
        let tail_training = tail_training.unwrap_or(self.blocksizes[1]);
        if tail_training <= TAIL_CONTEXT || tail_training > self.total {
            return Err(StreamFeederSourceShort { frames: self.total });
        }
        let tail_training = tail_training as usize;
        for ring in &self.ring {
            let len = ring.len();
            if len < tail_training {
                return Err(StreamFeederSourceShort { frames: len as i64 });
            }
            let source: Vec<f64> = ring.iter().copied().collect();
            let coeffs = wwise_lpc_from_data(&source[len - tail_training..], TAIL_ORDER)?;
            tails.push(wwise_lpc_predict(
                &coeffs,
                &source[len - TAIL_CONTEXT as usize..],
                STREAM_TAIL_SAMPLES,
            )?);
        }
        self.tails = Some(tails);
        Ok(())
    }

    /// Detector-stream length (prime + source + tail when EOS)
    /// (batch `detector_pcm_streams` length).
    pub fn detector_stream_length(&self) -> i64 {
        if self.detector_prime.is_none() {
            return 0;
        }
        self.blocksizes[1] / 2 + self.total + if self.eos() { STREAM_TAIL_SAMPLES } else { 0 }
    }

    /// One detector quantum (128 samples per channel) at hop position
    /// `quantum`, exactly the window the batch
    /// [`iter_detector_quanta`](crate::preprocessing::detector_input::iter_detector_quanta)
    /// yields.
    ///
    /// `None` when the quantum window is not materializable from the
    /// retained views (source samples already handed out and evicted).
    pub fn detector_quantum(&self, quantum: i64) -> Option<Vec<Vec<f64>>> {
        let stream_len = self.detector_stream_length();
        if stream_len <= 0 || quantum < 0 {
            return None;
        }
        let start = quantum * STREAM_DETECTOR_HOP;
        if start + STREAM_DETECTOR_WINDOW > stream_len {
            return None;
        }
        let detector_prime = self.detector_prime.as_ref()?;
        let tails = self.tails.as_deref();
        let prime_prefill = self.blocksizes[1] / 2;
        let mut out =
            vec![Vec::with_capacity(STREAM_DETECTOR_WINDOW as usize); self.channels as usize];
        for (channel, row) in out.iter_mut().enumerate() {
            for offset in 0..STREAM_DETECTOR_WINDOW {
                let source_index = start + offset - prime_prefill;
                let value = if source_index < 0 {
                    let prime_index = source_index + prime_prefill;
                    if prime_index >= 0 {
                        detector_prime[channel][prime_index as usize]
                    } else {
                        0.0
                    }
                } else if source_index < self.total {
                    self.retained_source_sample(channel, source_index)?
                } else {
                    let tail = tails.expect("EOS stream length implies tails");
                    let tail_index = source_index - self.total;
                    if tail_index < (tail[channel].len() as i64) {
                        tail[channel][tail_index as usize]
                    } else {
                        0.0
                    }
                };
                row.push(value);
            }
        }
        Some(out)
    }

    /// One frame plan's raw channel-major rows on the PCM timeline,
    /// applying the exact sample views of
    /// [`iter_planned_pcm_windows`](crate::preprocessing::windowing::iter_planned_pcm_windows):
    /// negative indices from the
    /// frame-windowing prime (prefill 128), source indices from the
    /// retained ring, indices past the endpoint from the EOS tail (0.0
    /// where the batch code emits 0.0).
    ///
    /// Errors when a required sample is not retained yet (the caller must
    /// only materialize ready frames); this never happens for streams
    /// driven with the batch lookahead bounds.
    pub fn frame_raw_rows(
        &self,
        plan: &FramePlan,
        blocksizes: &[i64],
    ) -> Result<Vec<Vec<f64>>, AnalysisError> {
        let current = plan.current;
        let n = blocksizes[current as usize];
        let source_origin = blocksizes[1] / 2;
        let start = plan.sample_start - source_origin;
        let end = start + n;
        let primes =
            self.window_prime
                .as_ref()
                .ok_or(AnalysisError::StreamFeederWindowNotReady {
                    want_from: start,
                    want_to: end,
                })?;
        let tails = self.tails.as_deref();
        // The batch windowing tail spans `max block size` samples; mirror
        // its exact 0.0 fallback beyond that.
        let window_tail_len = self.blocksizes[1];
        let prime_prefill = 128;
        let mut out = vec![Vec::with_capacity(n as usize); self.channels as usize];
        for (channel, row) in out.iter_mut().enumerate() {
            for index in start..end {
                let value = if index < 0 {
                    let prime_index = index + prime_prefill;
                    if prime_index >= 0 {
                        primes[channel][prime_index as usize]
                    } else {
                        0.0
                    }
                } else if index < self.total {
                    match self.retained_source_sample(channel, index) {
                        Some(value) => value,
                        None => {
                            return Err(AnalysisError::StreamFeederWindowNotReady {
                                want_from: start,
                                want_to: end,
                            })
                        }
                    }
                } else {
                    match tails {
                        Some(tail) => {
                            let tail_index = index - self.total;
                            if tail_index < window_tail_len {
                                tail[channel][tail_index as usize]
                            } else {
                                0.0
                            }
                        }
                        None => {
                            return Err(AnalysisError::StreamFeederWindowNotReady {
                                want_from: start,
                                want_to: end,
                            })
                        }
                    }
                };
                row.push(value);
            }
        }
        Ok(out)
    }

    /// A retained source sample (the ring holds the most recent
    /// `STREAM_RING_KEEP` samples).
    fn retained_source_sample(&self, channel: usize, index: i64) -> Option<f64> {
        let ring = &self.ring[channel];
        let base = self.total - (ring.len() as i64);
        let offset = index - base;
        if offset >= 0 && (offset as usize) < ring.len() {
            Some(ring[offset as usize])
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_empty_and_bad_geometry() {
        assert!(StreamingPcmFeeder::new(0, [256, 2048]).is_err());
        assert!(StreamingPcmFeeder::new(6, [256, 512]).is_err());
        let mut feeder = StreamingPcmFeeder::new(6, [256, 2048]).expect("feeder");
        assert!(feeder.push(&[vec![1.0; 4], vec![2.0; 4]]).is_err());
        assert!(feeder.push(&[vec![1.0; 4], vec![2.0; 5]]).is_err());
    }

    #[test]
    fn settle_bounds_ring_retention_to_stream_ring_keep() {
        let mut feeder = StreamingPcmFeeder::new(6, [256, 2048]).expect("feeder");
        // Push far more source than the retention bound (ten 5000-sample
        // chunks per channel).
        for pass in 0..10 {
            let rows: Vec<Vec<f64>> = (0..6)
                .map(|channel| {
                    (0..5000)
                        .map(|i| pass as f64 * 1000.0 + channel as f64 + (i as f64) * 0.001)
                        .collect()
                })
                .collect();
            feeder.push(&rows).expect("push");
        }
        feeder.settle();
        // The ring retains exactly the bounded window (the 9216-sample
        // input-memory ceiling of the streaming path).
        for ring in &feeder.ring {
            assert_eq!(ring.len() as i64, STREAM_RING_KEEP);
        }
        assert_eq!(feeder.total_samples(), 50_000);
        // The prefix head is consumed once the primes are cached.
        for prefix in &feeder.prefix {
            assert!(prefix.is_empty());
        }
    }

    #[test]
    fn huge_single_chunk_caps_prefix_growth() {
        // A first chunk far beyond the prime batch must not let the
        // prefix (first-frame head retention) escape the bound.
        let mut feeder = StreamingPcmFeeder::new(6, [256, 2048]).expect("feeder");
        let rows: Vec<Vec<f64>> = vec![vec![0.5f64; 200_000]; 6];
        feeder.push(&rows).expect("push");
        for prefix in &feeder.prefix {
            assert!(
                prefix.is_empty() || (prefix.len() as i64) <= STREAM_PRIME_BATCH,
                "prefix must stay capped at {STREAM_PRIME_BATCH} samples"
            );
        }
    }
}
