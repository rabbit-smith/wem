//! The worker pool the long-frame channel waves run in.
//!
//! [`crate::psychoacoustics::pipeline::analyze_long_frame`] has two
//! channel-parallel waves — the per-channel transform and the per-channel
//! psychoacoustic map — and one job per channel in each. Those jobs used to run
//! in rayon's process-global pool, whose size belongs to the host and not to the
//! encode: sixteen workers on a development machine against six jobs, so every
//! wave woke more workers than it had work for. Pairing a ten-second
//! six-channel encode against `RAYON_NUM_THREADS=6` on that machine cut its CPU
//! by 45% (1301 ms to 710 ms) with the same bytes out, and the wall optimum sat
//! at the channel count.
//!
//! A library may not fix that by reconfiguring the global pool — not because
//! rayon forbids it, but because that pool is not ours to decide. Rayon's own
//! documentation says calling `build_global` "is not recommended, except in two
//! scenarios" and that the global pool's initialization "happens exactly once",
//! so a library that reaches for it takes a process-wide decision the
//! application may already have made for itself
//! (`docs/findings/internal-parallelism-practice.md`, N4). Reading
//! `RAYON_NUM_THREADS` here to pick a size would take the same decision through
//! a second door. So the analysis session owns a pool of its own instead, sized
//! to the work it schedules: one worker per channel, or fewer when the caller
//! capped it (below). The two waves run inside it, nothing global is touched,
//! and the size is a function of the encode's own geometry and the caller's cap,
//! so the same encode with the same cap builds the same pool on every host.
//!
//! Output does not depend on the pool, and that is the property that makes this
//! shape safe at any size: the jobs stay disjoint — one per channel index, each
//! writing only the slot `iter_mut` handed it — and the results are collected in
//! channel order, so the bytes are the sequential path's bytes whether the pool
//! has one worker or sixteen. A pool smaller than the channel count is safe for
//! the same reason, and it terminates: rayon's scope latch is a stealing latch,
//! so the worker that owns a wave runs its remaining jobs itself while it waits
//! for them (`rayon_core::latch::CountLatch::wait`).
//!
//! # The caller's cap
//!
//! The size is derived — it is the number of jobs the geometry produces, one per
//! channel — and the caller can bound it, down to a single worker, through the
//! encoder's construction options in `wem-core`
//! (`wem_core::encoder::EncoderOptions::max_channel_pool_workers`; the streaming
//! session carries the same option). That cap is the lever every comparable
//! library documents (N2): explicit, stated once at construction on the
//! caller's own surface, never read out of the environment. It is a bound and
//! not a target, so a cap above the channel count leaves the derived size in
//! place — a wave never has more jobs than channels — and `Some(1)` is the off
//! switch. With the `parallel` feature off an encode runs on the calling thread
//! and there is no pool at all, so the cap is accepted and has no effect.
//!
//! What a caller reads back is the size it got
//! ([`AnalysisSession::channel_pool_workers`](crate::session::AnalysisSession::channel_pool_workers)):
//! the derived size after the cap. A reading, not a second knob — the pool is
//! built with the session, so there is nothing to resize afterwards.
//!
//! This is a resource-size detail of one encode, not a scheduling surface: no
//! queue to submit to, and no policy about how work is dispatched — this crate
//! encodes WEM, it does not schedule work.
//!
//! The measurement behind the size — the sweep, the falsification that the cost
//! is the worker count rather than the pool's ownership, and the byte checks at
//! every size — is `docs/findings/pool-sizing.md`.

use std::num::NonZeroUsize;

use crate::config::AnalysisError;

/// The pool a session runs its long-frame channel waves in.
///
/// One worker per channel job, derived from the encode's geometry and built once
/// when the session is built, so a wave never wakes a worker that has no job to
/// take. The pool is the session's own: no global pool is configured, no
/// environment variable is consulted, and two sessions in one process do not
/// share or affect each other's workers.
#[cfg(feature = "parallel")]
pub struct ChannelPool {
    pool: rayon::ThreadPool,
}

/// Without the `parallel` feature there is nothing to size and no threads to
/// own: the waves run one after another on the calling thread. This is the
/// threadless-target path (`wasm32-unknown-unknown`, via `crates/wem-wasm`) and
/// the library's default configuration, which is why the type still exists
/// under this configuration — the long-frame path reaches it the same way at
/// every pool size.
#[cfg(not(feature = "parallel"))]
pub struct ChannelPool;

#[cfg(feature = "parallel")]
impl ChannelPool {
    /// The pool for a session of `channels` channels, holding at most
    /// `max_workers` workers when the caller capped the size.
    ///
    /// The channel count is the geometry the session was built for, and it is
    /// the number of jobs each wave spawns, which is what the measurement puts
    /// the optimum at — so that is the size when the caller states no cap. It is
    /// deliberately not `available_parallelism`: that is the host's capacity,
    /// read from the process's affinity and cgroup limits, and it would make the
    /// pool a different shape on every machine for the same encode. A host with
    /// fewer cores than channels time-slices the runnable workers, exactly as it
    /// time-sliced the same jobs when they ran in the global pool; a host with
    /// more cores leaves the surplus workers parked, which is what the old shape
    /// should have done.
    ///
    /// `max_workers` is the caller's cap, passed down from the encoder's
    /// construction options. It only ever lowers the size: a cap at or above the
    /// channel count leaves the derived size in place, because a wave has one
    /// job per channel and a worker beyond that would have nothing to take.
    ///
    /// Fails only when the host refuses to start a worker ([`AnalysisError::
    /// PoolUnavailable`]); the caller's session construction reports it.
    pub fn for_channels(
        channels: i64,
        max_workers: Option<NonZeroUsize>,
    ) -> Result<Self, AnalysisError> {
        let derived = channels.max(1) as usize;
        let workers = match max_workers {
            Some(cap) => derived.min(cap.get()),
            None => derived,
        };
        Ok(Self {
            pool: build(workers)?,
        })
    }

    /// A pool of an explicitly chosen worker count, for the size sweep in this
    /// module's tests.
    ///
    /// Deliberately not reachable outside a test build: an encode's pool is
    /// sized from its geometry and the caller's cap, and this constructor exists
    /// to sweep pool shapes the cap cannot reach — a pool with more workers than
    /// the wave has jobs — so the collection order is pinned there too. The
    /// parameter is a `NonZeroUsize` so that "let the Rayon runtime decide" —
    /// which would put the host's default back in charge of the shape — is
    /// unrepresentable.
    #[cfg(test)]
    fn for_threads(threads: NonZeroUsize) -> Result<Self, AnalysisError> {
        Ok(Self {
            pool: build(threads.get())?,
        })
    }
}

#[cfg(not(feature = "parallel"))]
impl ChannelPool {
    /// The sequential long-frame path has no workers to size, so the channel
    /// count and the caller's cap are accepted and ignored by construction
    /// rather than silently clamped to a pool of one — and no host call is made
    /// that could fail.
    pub fn for_channels(
        _channels: i64,
        _max_workers: Option<NonZeroUsize>,
    ) -> Result<Self, AnalysisError> {
        Ok(Self)
    }
}

impl ChannelPool {
    /// How many workers a channel wave runs on: the derived one per channel
    /// after the caller's cap for a session's pool, and one — the calling
    /// thread, which is all a threadless build has — without the feature.
    ///
    /// A reading, not a knob: the pool is built with the session, so this
    /// reports the size the encode's geometry and the caller's cap produced and
    /// nothing that can be changed afterwards. It is the same kind of fact as
    /// the frame count on an encode result.
    pub fn workers(&self) -> usize {
        #[cfg(feature = "parallel")]
        {
            self.pool.current_num_threads()
        }
        #[cfg(not(feature = "parallel"))]
        {
            1
        }
    }

    /// Run one job per channel index and collect the results in channel order.
    ///
    /// Order is the sequential order in every configuration: the parallel branch
    /// gives each job its own result slot and reads the slots by index after the
    /// scope has joined them, so an error is still the earliest channel's error
    /// and a success is still the channels in index order. The scope joins every
    /// job it spawned before returning, at any pool size.
    pub fn map_channels<T, F>(&self, count: usize, job: F) -> Result<Vec<T>, AnalysisError>
    where
        F: Fn(usize) -> Result<T, AnalysisError> + Send + Sync,
        T: Send,
    {
        #[cfg(feature = "parallel")]
        {
            // Shared, not cloned: the closure captures the frame's already
            // computed rows by reference, and `F: Sync` is what makes that
            // sound to hand to several workers in one scope.
            let job = &job;
            let mut channels: Vec<Option<Result<T, AnalysisError>>> =
                (0..count).map(|_| None).collect();
            self.pool.scope(|scope| {
                for (channel_index, channel) in channels.iter_mut().enumerate() {
                    scope.spawn(move |_| {
                        *channel = Some(job(channel_index));
                    });
                }
            });
            channels
                .into_iter()
                .map(|channel| {
                    channel.expect("every spawned channel job publishes before the scope returns")
                })
                .collect()
        }
        #[cfg(not(feature = "parallel"))]
        {
            (0..count).map(job).collect()
        }
    }
}

/// Build the session's worker pool.
///
/// `ThreadPoolBuilder::build` fails only when the host refuses to start a
/// thread — the pool is the resource, and it is what the wave runs on, so there
/// is no smaller thing to fall back to and no honest default to substitute. The
/// refusal is reported as [`AnalysisError::PoolUnavailable`] and reaches the
/// caller through the session constructor that asked for the pool: a library
/// does not turn "the host said no" into a process abort.
#[cfg(feature = "parallel")]
fn build(workers: usize) -> Result<rayon::ThreadPool, AnalysisError> {
    rayon::ThreadPoolBuilder::new()
        .num_threads(workers)
        .build()
        .map_err(|error| AnalysisError::PoolUnavailable {
            workers,
            cause: error.to_string(),
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A wave completes and collects in channel order at every pool size,
    /// including one worker and fewer workers than channels — the property the
    /// byte-parity suites read when a host schedules fewer workers than the
    /// encode has channels, checked here directly on the map that produces the
    /// channel rows.
    #[cfg(feature = "parallel")]
    #[test]
    fn map_channels_completes_in_channel_order_at_every_pool_size() {
        for workers in [1usize, 2, 3, 6, 8, 16] {
            let pool = ChannelPool::for_threads(
                std::num::NonZeroUsize::new(workers).expect("a positive worker count"),
            )
            .expect("the host starts the workers");
            let rows: Vec<i64> = pool
                .map_channels(6, |channel_index| Ok(channel_index as i64 * 10))
                .expect("every channel job publishes");
            assert_eq!(rows, vec![0, 10, 20, 30, 40, 50], "{workers} workers");
        }
    }

    /// The size is the work: without a cap a session's pool has one worker per
    /// channel, which is the number of jobs each wave spawns — the row the
    /// measurement puts the wall plateau's bottom at.
    #[cfg(feature = "parallel")]
    #[test]
    fn for_channels_sizes_the_pool_to_the_job_count() {
        for channels in [1i64, 2, 6, 8] {
            let pool =
                ChannelPool::for_channels(channels, None).expect("the host starts the workers");
            assert_eq!(pool.workers(), channels as usize);
        }
    }

    /// The caller's cap bounds the size and never raises it: a cap below the
    /// channel count is the size, a cap above it leaves the derived size in
    /// place, and one worker is representable — the off switch.
    #[cfg(feature = "parallel")]
    #[test]
    fn for_channels_applies_the_cap_without_ever_exceeding_the_job_count() {
        for (channels, cap, expected) in [
            (6i64, 1usize, 1usize),
            (6, 2, 2),
            (6, 3, 3),
            (6, 6, 6),
            (6, 16, 6),
            (2, 1, 1),
            (2, 16, 2),
            (1, 1, 1),
        ] {
            let pool = ChannelPool::for_channels(channels, NonZeroUsize::new(cap))
                .expect("the host starts the workers");
            assert_eq!(
                pool.workers(),
                expected,
                "{channels} channels capped at {cap}"
            );
        }
    }

    /// The first failing channel is the error the caller reads, whichever
    /// worker found it and however many workers there are.
    #[cfg(feature = "parallel")]
    #[test]
    fn map_channels_reports_the_earliest_failing_channel() {
        let pool = ChannelPool::for_threads(
            std::num::NonZeroUsize::new(6).expect("a positive worker count"),
        )
        .expect("the host starts the workers");
        let failed = pool.map_channels(6, |channel_index| {
            if channel_index == 2 {
                Err(AnalysisError::LongAnalysisEmpty)
            } else {
                Ok(channel_index)
            }
        });
        assert_eq!(failed, Err(AnalysisError::LongAnalysisEmpty));
    }

    /// Without the feature the same map is the sequential loop, in the same
    /// order, with no threads involved — one worker in the only sense left, the
    /// calling thread.
    #[cfg(not(feature = "parallel"))]
    #[test]
    fn map_channels_is_sequential_without_the_feature() {
        let pool =
            ChannelPool::for_channels(6, None).expect("the sequential path cannot fail to build");
        assert_eq!(pool.workers(), 1);
        let rows: Vec<i64> = pool
            .map_channels(6, |channel_index| Ok(channel_index as i64 * 10))
            .expect("every channel job publishes");
        assert_eq!(rows, vec![0, 10, 20, 30, 40, 50]);
    }

    /// The cap is inert without the feature: a build with no pool to size
    /// accepts it and still runs on the calling thread, which is the whole
    /// content of "the library imposes no threads on a caller that did not ask".
    #[cfg(not(feature = "parallel"))]
    #[test]
    fn the_cap_is_inert_without_the_feature() {
        let pool = ChannelPool::for_channels(6, NonZeroUsize::new(2))
            .expect("the sequential path cannot fail to build");
        assert_eq!(pool.workers(), 1);
    }

    /// A host refusal reaches the caller as a message naming the observed worker
    /// count and the host's own cause — never as a category name, and never as
    /// the `Debug` rendering.
    #[test]
    fn pool_unavailable_names_the_workers_and_the_cause() {
        let error = AnalysisError::PoolUnavailable {
            workers: 6,
            cause: "resource temporarily unavailable".to_string(),
        };
        let message = error.to_string();
        assert!(message.contains("6 workers"), "{message}");
        assert!(
            message.contains("resource temporarily unavailable"),
            "{message}"
        );
    }
}
