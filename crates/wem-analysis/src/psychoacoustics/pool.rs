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
//! A library may not fix that by reconfiguring the global pool. That pool is
//! ambient state the caller cannot see, and `docs/reference/standards.md`
//! (Caller streams and ambient state) forbids deciding anything behind the
//! caller's back — which is the same reason `RAYON_NUM_THREADS` must not be read
//! here to pick a size. So the analysis session owns a pool of its own instead,
//! sized to the work it schedules: one worker per channel. The two waves run
//! inside it, nothing global is touched, and the pool shape is a function of the
//! encode's own geometry, so the same encode builds the same pool on every host.
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
//! This is a resource-size detail of one encode, not a scheduling surface. The
//! size is derived — it is the number of jobs the geometry produces — and there
//! is no way for a caller to choose it, no queue to submit to, and no policy
//! about how work is dispatched: this crate encodes WEM, it does not schedule
//! work. What a caller *can* do is read the size the encode produced
//! ([`AnalysisSession::channel_pool_workers`](crate::session::AnalysisSession::channel_pool_workers)),
//! which is transparency rather than a setter.
//!
//! The measurement behind the size — the sweep, the falsification that the cost
//! is the worker count rather than the pool's ownership, and the byte checks at
//! every size — is `docs/findings/pool-sizing.md`.

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
/// threadless-target path (`wasm32-unknown-unknown`, via `crates/wem-wasm`),
/// which is why the type still exists under this configuration — the long-frame
/// path reaches it the same way at every pool size.
#[cfg(not(feature = "parallel"))]
pub struct ChannelPool;

#[cfg(feature = "parallel")]
impl ChannelPool {
    /// The pool for a session of `channels` channels: one worker per channel,
    /// so every job of a wave has a worker of its own.
    ///
    /// The channel count is the geometry the session was built for, and it is
    /// the number of jobs each wave spawns, which is what the measurement puts
    /// the optimum at. It is deliberately not `available_parallelism`: that is
    /// the host's capacity, read from the process's affinity and cgroup limits,
    /// and it would make the pool a different shape on every machine for the
    /// same encode. A host with fewer cores than channels time-slices the
    /// runnable workers, exactly as it time-sliced the same jobs when they ran
    /// in the global pool; a host with more cores leaves the surplus workers
    /// parked, which is what the old shape should have done.
    ///
    /// Fails only when the host refuses to start a worker ([`AnalysisError::
    /// PoolUnavailable`]); the caller's session construction reports it.
    pub fn for_channels(channels: i64) -> Result<Self, AnalysisError> {
        Ok(Self {
            pool: build(channels.max(1) as usize)?,
        })
    }

    /// A pool of an explicitly chosen worker count, for the size sweep in this
    /// module's tests.
    ///
    /// Deliberately not reachable outside a test build: the shipped size is a
    /// property of the encode's geometry, and a caller-settable worker count is
    /// the beginning of a scheduling surface this crate does not have. The
    /// parameter is a `NonZeroUsize` so that "let the Rayon runtime decide" —
    /// which would put the host's default back in charge of the shape — is
    /// unrepresentable.
    #[cfg(test)]
    fn for_threads(threads: std::num::NonZeroUsize) -> Result<Self, AnalysisError> {
        Ok(Self {
            pool: build(threads.get())?,
        })
    }
}

#[cfg(not(feature = "parallel"))]
impl ChannelPool {
    /// The sequential long-frame path has no workers to size, so the channel
    /// count is accepted and ignored by construction rather than silently
    /// clamped to a pool of one — and no host call is made that could fail.
    pub fn for_channels(_channels: i64) -> Result<Self, AnalysisError> {
        Ok(Self)
    }
}

impl ChannelPool {
    /// How many workers a channel wave runs on: one per channel for a session's
    /// pool, and one — the calling thread, which is all a threadless target has
    /// — without the feature.
    ///
    /// A reading, not a knob: the size is derived, so this reports what the
    /// encode's geometry produced and nothing a caller can set. It is the same
    /// kind of fact as the frame count on an encode result.
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

    /// The size is the work: a session's pool has one worker per channel, which
    /// is the number of jobs each wave spawns — the row the measurement puts the
    /// wall plateau's bottom at. There is no other size a caller can ask for.
    #[cfg(feature = "parallel")]
    #[test]
    fn for_channels_sizes_the_pool_to_the_job_count() {
        for channels in [1i64, 2, 6, 8] {
            let pool = ChannelPool::for_channels(channels).expect("the host starts the workers");
            assert_eq!(pool.workers(), channels as usize);
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
        let pool = ChannelPool::for_channels(6).expect("the sequential path cannot fail to build");
        assert_eq!(pool.workers(), 1);
        let rows: Vec<i64> = pool
            .map_channels(6, |channel_index| Ok(channel_index as i64 * 10))
            .expect("every channel job publishes");
        assert_eq!(rows, vec![0, 10, 20, 30, 40, 50]);
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
