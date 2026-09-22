//! The C ABI surface: the end-to-end shell paths and the adversarial inputs.
//!
//! One binary, two angles. `end_to_end` drives the FFI the way an external
//! language does — one-shot, handle reuse, concurrent threads, seven uneven
//! chunks, NULL arguments, lifecycle violations — and pins the reference bytes
//! and the stable error codes. `adversarial` hands the same surface the shapes
//! a hostile or confused caller reaches and requires a typed rejection, never a
//! panic and never `WEM_ERR_INTERNAL`. Each module keeps its own test names and
//! assertion messages, so a failure still names the input that was mishandled.

mod common;

mod end_to_end {
    //! C ABI end-to-end tests (wem-capi).
    //!
    //! Drive the FFI surface exactly the way an external language would
    //! (through the rlib symbols, `unsafe` and all) and prove:
    //! * one-shot encode == reference WEM bytes (fixture PCM);
    //! * streaming with seven uneven chunks == the same bytes, seq-ordered
    //!   packets, a terminal finish that carries no summary of its own;
    //! * encoder-handle path matches the one-shot convenience entry;
    //! * one handle shared by several threads encodes concurrently and every
    //!   thread reproduces the reference bytes (the include/wem.h shareability
    //!   promise);
    //! * NULL arguments, unsatisfiable selections, and lifecycle violations
    //!   return the expected stable codes;
    //! * an out-of-table `WemVersion` code is a revision mismatch
    //!   (FORMAT_UNSUPPORTED), never a silent fallback;
    //! * the error-code table and the `WemProfile` layout stay in sync with
    //!   include/wem.h.

    use std::ffi::c_void;
    use std::sync::Barrier;
    use std::thread;

    use wem_capi::{WemEncoder, WemError, WemProfile, WemVersion};

    use crate::common::{fixture_profile, read_fixture_pcm, reference_wem, stereo_profile};

    // ---------------------------------------------------------------------------
    // Test sinks (what a C client would do with its callbacks).
    //
    // Each test owns its sink (passed as `user_data`), so parallel test
    // threads never share a buffer.
    // ---------------------------------------------------------------------------

    struct Sinks {
        out: Vec<u8>,
        packets: Vec<(u32, Vec<u8>)>,
    }

    impl Sinks {
        fn new() -> Box<Self> {
            Box::new(Self {
                out: Vec::new(),
                packets: Vec::new(),
            })
        }

        /// The `user_data` pointer this sink presents to the C callbacks.
        fn user_data(&mut self) -> *mut c_void {
            std::ptr::from_mut(self) as *mut c_void
        }
    }

    /// One `WemEncoder *` that may cross a thread boundary.
    ///
    /// include/wem.h states normatively: *"WemEncoder is shareable: concurrent
    /// encodes may use one handle on different threads"*, and declares the handle
    /// as `/* Profile-resolved shareable encoder (concurrent encodes OK). */`.
    /// This test drives the raw symbol exactly as a C client would, so the only
    /// value that reaches the threads is the raw pointer and there is no Rust type
    /// that could carry the promise — the wrapper carries it instead.
    ///
    /// `unsafe` is the point of this file: an FFI test must exercise the C ABI
    /// on C terms (raw pointers, `unsafe extern "C"` callbacks), and the Rust type
    /// system cannot express a claim the C header makes about a pointer it hands
    /// out. `wem-capi` backs the same promise at compile time with the
    /// `Send + Sync` assertion next to `WemEncoder`.
    #[derive(Clone, Copy)]
    struct SharedHandle(*const WemEncoder);

    impl SharedHandle {
        /// The raw handle one `wem_encoder_encode` call needs. Taking `self` by
        /// value keeps the whole wrapper — and with it the `Send`/`Sync` claim
        /// below — in the closure capture, instead of the bare `!Send` raw pointer
        /// the field holds.
        fn raw(self) -> *const WemEncoder {
            self.0
        }
    }

    // SAFETY: the header promise quoted above — the handle is shareable across
    // threads. The pointer is only ever borrowed by `wem_encoder_encode`
    // (`*const WemEncoder` -> `&WemEncoder`) and is freed after every thread has
    // joined, so no thread can observe a dropped encoder.
    unsafe impl Send for SharedHandle {}
    // SAFETY: `&SharedHandle` only ever yields the raw pointer, and concurrent
    // encodes through it are exactly what the header documents as safe.
    unsafe impl Sync for SharedHandle {}

    unsafe extern "C" fn sink_write(
        data: *const u8,
        len: usize,
        user_data: *mut c_void,
    ) -> WemError {
        if user_data.is_null() {
            return WemError::Internal;
        }
        let sinks = unsafe { &mut *(user_data as *mut Sinks) };
        let chunk = unsafe { std::slice::from_raw_parts(data, len) };
        sinks.out.extend_from_slice(chunk);
        WemError::Ok
    }

    unsafe extern "C" fn sink_packet(
        seq: u32,
        data: *const u8,
        len: usize,
        user_data: *mut c_void,
    ) -> WemError {
        if user_data.is_null() {
            return WemError::Internal;
        }
        let sinks = unsafe { &mut *(user_data as *mut Sinks) };
        let chunk = unsafe { std::slice::from_raw_parts(data, len) };
        sinks.packets.push((seq, chunk.to_vec()));
        WemError::Ok
    }

    // ---------------------------------------------------------------------------
    // Helpers
    // ---------------------------------------------------------------------------

    /// The setup packet inside one assembled WEM container (seq 0).
    fn setup_packet_of(wem_bytes: &[u8]) -> Vec<u8> {
        let parts = wem_container::load_wem_parts_bytes(wem_bytes).expect("wem parts load");
        parts.setup_packet.expect("setup packet present").to_vec()
    }

    // ---------------------------------------------------------------------------
    // 1. One-shot path
    // ---------------------------------------------------------------------------

    #[test]
    fn one_shot_fixture_rebuilds_reference_wem() {
        let (pcm, frames, _channels) = read_fixture_pcm();
        let mut sinks = Sinks::new();
        let ud = sinks.user_data();

        let code = unsafe {
            wem_capi::wem_encode_pcm16_interleaved(
                &fixture_profile(),
                pcm.as_ptr(),
                frames,
                Some(sink_write),
                ud,
            )
        };
        assert_eq!(code, WemError::Ok, "one-shot encode rejected");

        let out = &sinks.out;
        assert_eq!(
            out,
            reference_wem().as_slice(),
            "one-shot C ABI output differs from reference.wem"
        );
    }

    #[test]
    fn encoder_handle_matches_one_shot_and_reuses() {
        let (pcm, frames, _channels) = read_fixture_pcm();

        let mut handle = std::ptr::null_mut();
        let code = unsafe { wem_capi::wem_encoder_new(&fixture_profile(), &mut handle) };
        assert_eq!(code, WemError::Ok, "encoder_new rejected");
        assert!(!handle.is_null());

        // Two independent encodes through the shared handle: byte-identical.
        for run in 0..2 {
            let mut sinks = Sinks::new();
            let ud = sinks.user_data();
            let code = unsafe {
                wem_capi::wem_encoder_encode(handle, pcm.as_ptr(), frames, Some(sink_write), ud)
            };
            assert_eq!(code, WemError::Ok, "encoder_encode {run} rejected");
            assert_eq!(
                sinks.out,
                reference_wem(),
                "encoder handle run {run} differs"
            );
        }
        unsafe {
            wem_capi::wem_encoder_free(handle);
            // NULL release is a documented no-op.
            wem_capi::wem_encoder_free(std::ptr::null_mut());
        }
    }

    // ---------------------------------------------------------------------------
    // 2. One handle, several threads (the header's shareability promise)
    // ---------------------------------------------------------------------------

    /// include/wem.h: *"WemEncoder is shareable: concurrent encodes may use one
    /// handle on different threads."* This is that sentence executed: one handle,
    /// four threads, each encoding the fixture PCM into its own sink, and each
    /// reproducing the reference container byte for byte.
    #[test]
    fn concurrent_encodes_share_one_handle() {
        const THREADS: usize = 4;

        let (pcm, frames, _channels) = read_fixture_pcm();
        let reference = reference_wem();

        let mut handle = std::ptr::null_mut();
        let code = unsafe { wem_capi::wem_encoder_new(&fixture_profile(), &mut handle) };
        assert_eq!(code, WemError::Ok, "encoder_new rejected");
        assert!(!handle.is_null());
        let shared = SharedHandle(handle.cast_const());
        // Every thread waits here before calling in, so the encodes genuinely
        // overlap instead of merely being permitted to.
        let barrier = Barrier::new(THREADS);

        let outputs: Vec<Vec<u8>> = thread::scope(|scope| {
            let mut joins = Vec::with_capacity(THREADS);
            for index in 0..THREADS {
                let barrier = &barrier;
                let pcm = pcm.as_slice();
                joins.push(scope.spawn(move || {
                    // Each thread passes its own sink: a callback that received a
                    // NULL `user_data` (the bug this guards against) makes
                    // `sink_write` return WEM_ERR_INTERNAL, so the encode returns
                    // that code and the assertion below fails loudly — no thread
                    // can quietly write into another thread's buffer.
                    let mut sinks = Sinks::new();
                    let ud = sinks.user_data();
                    barrier.wait();
                    let code = unsafe {
                        wem_capi::wem_encoder_encode(
                            shared.raw(),
                            pcm.as_ptr(),
                            frames,
                            Some(sink_write),
                            ud,
                        )
                    };
                    assert_eq!(code, WemError::Ok, "thread {index} encode rejected");
                    (index, sinks.out)
                }));
            }
            joins
                .into_iter()
                .map(|join| {
                    let (index, out) = join.join().expect("an encode thread panicked");
                    assert_eq!(
                        out, reference,
                        "thread {index} bytes differ from reference.wem"
                    );
                    out
                })
                .collect()
        });
        assert_eq!(outputs.len(), THREADS);

        unsafe {
            wem_capi::wem_encoder_free(handle);
        }
    }

    // ---------------------------------------------------------------------------
    // 3. Streaming path
    // ---------------------------------------------------------------------------

    #[test]
    fn streaming_seven_uneven_chunks_match_one_shot() {
        let (pcm, _frames, channels) = read_fixture_pcm();

        // Seven uneven, frame-aligned chunks: the last takes the remainder.
        let head_frames: [usize; 6] = [137, 211, 97, 331, 211, 53];
        let mut bounds = vec![0usize];
        for frame_count in head_frames {
            bounds.push(bounds.last().copied().unwrap() + frame_count * channels * 2);
        }
        bounds.push(pcm.len());
        assert!(bounds[6] < pcm.len(), "the seventh chunk must be non-empty");

        let mut sinks = Sinks::new();
        let ud = sinks.user_data();

        let mut session = std::ptr::null_mut();
        let code = unsafe {
            wem_capi::wem_session_new(
                &fixture_profile(),
                Some(sink_write),
                Some(sink_packet),
                ud,
                &mut session,
            )
        };
        assert_eq!(code, WemError::Ok, "session_new rejected");
        assert!(!session.is_null());

        for (index, pair) in bounds.windows(2).enumerate() {
            let (start, end) = (pair[0], pair[1]);
            let code = unsafe {
                wem_capi::wem_session_push(session, pcm[start..end].as_ptr(), end - start)
            };
            assert_eq!(code, WemError::Ok, "push {index} rejected");
        }

        let code = unsafe { wem_capi::wem_session_finish(session) };
        assert_eq!(code, WemError::Ok, "finish rejected");
        unsafe {
            wem_capi::wem_session_free(session);
        }

        // Container bytes: the reference. The container length a client needs is
        // the sum of the `len` values its own write callback just received;
        // finish returns a code and nothing else.
        let out = &sinks.out;
        assert_eq!(
            out,
            reference_wem().as_slice(),
            "streaming bytes differ from reference.wem"
        );

        // Reply stream: seq starts at 0 (the setup packet) and increases by
        // one per emitted packet; the setup packet equals the container's.
        let packets = &sinks.packets;
        assert!(!packets.is_empty(), "no packets delivered");
        for (index, (seq, _data)) in packets.iter().enumerate() {
            assert_eq!(*seq, index as u32, "seq must be contiguous from 0");
        }
        assert_eq!(
            packets[0].1,
            setup_packet_of(out),
            "seq 0 must be the setup packet"
        );
    }

    #[test]
    fn streaming_null_packet_cb_still_streams_bytes() {
        let (pcm, _frames, _channels) = read_fixture_pcm();
        let mut sinks = Sinks::new();
        let ud = sinks.user_data();

        let mut session = std::ptr::null_mut();
        let code = unsafe {
            wem_capi::wem_session_new(&fixture_profile(), Some(sink_write), None, ud, &mut session)
        };
        assert_eq!(code, WemError::Ok);
        let code = unsafe { wem_capi::wem_session_push(session, pcm.as_ptr(), pcm.len()) };
        assert_eq!(code, WemError::Ok);
        let code = unsafe { wem_capi::wem_session_finish(session) };
        assert_eq!(code, WemError::Ok);
        unsafe {
            wem_capi::wem_session_free(session);
        }
        let out = &sinks.out;
        assert_eq!(out, reference_wem().as_slice());
    }

    // ---------------------------------------------------------------------------
    // 4. Argument validation (NULL / malformed calls)
    // ---------------------------------------------------------------------------

    #[test]
    fn null_arguments_reject_with_state_error() {
        let (pcm, frames, _channels) = read_fixture_pcm();
        let mut sinks = Sinks::new();
        let ud = sinks.user_data();

        let mut handle = std::ptr::null_mut();
        assert_eq!(
            unsafe { wem_capi::wem_encoder_new(std::ptr::null(), &mut handle) },
            WemError::StateError,
            "NULL selection must reject"
        );
        assert_eq!(
            unsafe { wem_capi::wem_encoder_new(&fixture_profile(), std::ptr::null_mut()) },
            WemError::StateError,
            "NULL out handle must reject"
        );
        assert_eq!(
            unsafe {
                wem_capi::wem_encode_pcm16_interleaved(
                    std::ptr::null(),
                    pcm.as_ptr(),
                    frames,
                    Some(sink_write),
                    ud,
                )
            },
            WemError::StateError,
            "NULL selection must reject"
        );
        assert_eq!(
            unsafe {
                wem_capi::wem_encode_pcm16_interleaved(
                    &fixture_profile(),
                    std::ptr::null(),
                    frames,
                    Some(sink_write),
                    ud,
                )
            },
            WemError::StateError,
            "NULL pcm must reject"
        );
        assert_eq!(
            unsafe {
                wem_capi::wem_encode_pcm16_interleaved(
                    &fixture_profile(),
                    pcm.as_ptr(),
                    frames,
                    None,
                    ud,
                )
            },
            WemError::StateError,
            "NULL write callback must reject"
        );

        let mut session = std::ptr::null_mut();
        assert_eq!(
            unsafe {
                wem_capi::wem_session_new(
                    &fixture_profile(),
                    Some(sink_write),
                    None,
                    ud,
                    std::ptr::null_mut(),
                )
            },
            WemError::StateError,
            "NULL out session must reject"
        );
        assert_eq!(
            unsafe { wem_capi::wem_session_new(&fixture_profile(), None, None, ud, &mut session) },
            WemError::StateError,
            "NULL write callback must reject"
        );
        assert_eq!(
            unsafe { wem_capi::wem_session_push(std::ptr::null_mut(), pcm.as_ptr(), 2) },
            WemError::StateError,
            "NULL session push must reject"
        );
        assert_eq!(
            unsafe { wem_capi::wem_session_finish(std::ptr::null_mut()) },
            WemError::StateError,
            "NULL session finish must reject"
        );

        // NULL releases are documented no-ops (no crash, no UB).
        unsafe {
            wem_capi::wem_encoder_free(std::ptr::null_mut());
            wem_capi::wem_session_free(std::ptr::null_mut());
        }
    }

    // ---------------------------------------------------------------------------
    // 5. Profile selection semantics
    // ---------------------------------------------------------------------------

    #[test]
    fn both_installed_selections_resolve() {
        for profile in [fixture_profile(), stereo_profile()] {
            let mut handle = std::ptr::null_mut();
            assert_eq!(
                unsafe { wem_capi::wem_encoder_new(&profile, &mut handle) },
                WemError::Ok,
                "{}ch/{}Hz/2013 must resolve",
                profile.channels,
                profile.sample_rate
            );
            assert!(!handle.is_null());
            unsafe {
                wem_capi::wem_encoder_free(handle);
            }
        }
    }

    #[test]
    fn unsatisfiable_selection_rejects_with_profile_not_found() {
        let (pcm, frames, _channels) = read_fixture_pcm();
        let mut sinks = Sinks::new();
        let ud = sinks.user_data();

        // Geometries no installed configuration provides: an uninstalled rate on
        // an installed channel count, and an uninstalled channel count.
        let unsupported = [
            WemProfile {
                version: WemVersion::Wwise2013,
                channels: 6,
                sample_rate: 48_000,
            },
            WemProfile {
                version: WemVersion::Wwise2013,
                channels: 3,
                sample_rate: 44_100,
            },
            WemProfile {
                version: WemVersion::Wwise2013,
                channels: 2,
                sample_rate: 44_100,
            },
        ];

        for profile in unsupported {
            let mut handle = std::ptr::null_mut();
            assert_eq!(
                unsafe { wem_capi::wem_encoder_new(&profile, &mut handle) },
                WemError::ProfileNotFound,
                "{}ch/{}Hz must not resolve",
                profile.channels,
                profile.sample_rate
            );
            assert!(
                handle.is_null(),
                "a failed encoder_new must leave a NULL handle"
            );

            assert_eq!(
                unsafe {
                    wem_capi::wem_encode_pcm16_interleaved(
                        &profile,
                        pcm.as_ptr(),
                        frames,
                        Some(sink_write),
                        ud,
                    )
                },
                WemError::ProfileNotFound
            );

            let mut session = std::ptr::null_mut();
            assert_eq!(
                unsafe {
                    wem_capi::wem_session_new(&profile, Some(sink_write), None, ud, &mut session)
                },
                WemError::ProfileNotFound
            );
            assert!(
                session.is_null(),
                "a failed session_new must leave a NULL handle"
            );
        }
    }

    #[test]
    fn malformed_selection_geometry_rejects_with_state_error() {
        for (channels, sample_rate) in [(0, 44_100), (6, 0), (-6, 44_100)] {
            let profile = WemProfile {
                version: WemVersion::Wwise2013,
                channels,
                sample_rate,
            };
            let mut handle = std::ptr::null_mut();
            assert_eq!(
                unsafe { wem_capi::wem_encoder_new(&profile, &mut handle) },
                WemError::StateError,
                "{channels}ch/{sample_rate}Hz is malformed, not merely uninstalled"
            );
            assert!(handle.is_null());
        }
    }

    /// A selection struct as a client built against a *newer* header would pass
    /// it: same layout, a version code this revision does not implement.
    #[repr(C)]
    struct ForeignSelection {
        version: u32,
        channels: i32,
        sample_rate: i32,
    }

    #[test]
    fn out_of_table_version_code_rejects_with_format_unsupported() {
        let foreign = ForeignSelection {
            version: 7,
            channels: 6,
            sample_rate: 44_100,
        };
        let mut handle = std::ptr::null_mut();
        let code = unsafe {
            wem_capi::wem_encoder_new(
                std::ptr::from_ref(&foreign).cast::<WemProfile>(),
                &mut handle,
            )
        };
        assert_eq!(
            code,
            WemError::FormatUnsupported,
            "an unimplemented WemVersion code must not fall back to a default"
        );
        assert!(handle.is_null());
    }

    // ---------------------------------------------------------------------------
    // 6. Lifecycle violations
    // ---------------------------------------------------------------------------

    #[test]
    fn lifecycle_violations_reject_with_state_error() {
        let (pcm, _frames, _channels) = read_fixture_pcm();
        let mut sinks = Sinks::new();
        let ud = sinks.user_data();

        let mut session = std::ptr::null_mut();
        assert_eq!(
            unsafe {
                wem_capi::wem_session_new(
                    &fixture_profile(),
                    Some(sink_write),
                    None,
                    ud,
                    &mut session,
                )
            },
            WemError::Ok
        );
        // Full stream in one chunk, then the terminal calls.
        assert_eq!(
            unsafe { wem_capi::wem_session_push(session, pcm.as_ptr(), pcm.len()) },
            WemError::Ok,
            "push rejected"
        );

        // A second finish (after a successful one) is a lifecycle violation.
        assert_eq!(
            unsafe { wem_capi::wem_session_finish(session) },
            WemError::Ok,
            "finish on a full stream"
        );
        assert_eq!(
            unsafe { wem_capi::wem_session_push(session, pcm.as_ptr(), 2) },
            WemError::StateError,
            "push after finish must reject"
        );
        assert_eq!(
            unsafe { wem_capi::wem_session_finish(session) },
            WemError::StateError,
            "double finish must reject"
        );
        unsafe {
            wem_capi::wem_session_free(session);
        }
    }

    #[test]
    fn short_stream_rejects_with_input_too_short() {
        let (pcm, frames, channels) = read_fixture_pcm();
        assert!(frames > 4096, "fixture is longer than the minimum");
        let short_frames = 100usize;
        let short_len = short_frames * channels * 2;
        let mut sinks = Sinks::new();
        let ud = sinks.user_data();

        let mut session = std::ptr::null_mut();
        assert_eq!(
            unsafe {
                wem_capi::wem_session_new(
                    &fixture_profile(),
                    Some(sink_write),
                    None,
                    ud,
                    &mut session,
                )
            },
            WemError::Ok
        );
        assert_eq!(
            unsafe { wem_capi::wem_session_push(session, pcm.as_ptr(), short_len) },
            WemError::Ok,
            "pushing a short stream is legal until finish"
        );
        assert_eq!(
            unsafe { wem_capi::wem_session_finish(session) },
            WemError::InputTooShort,
            "a stream under 4096 frames must reject at finish"
        );
        unsafe {
            wem_capi::wem_session_free(session);
        }
    }

    // ---------------------------------------------------------------------------
    // 7. Out-parameters on every exit (include/wem.h, "MEMORY OWNERSHIP")
    //
    // A C caller cannot tell an unwritten out-parameter from one holding
    // garbage, so the ones that remain are driven here on the exits that do
    // *not* write the interesting value: the handle out-pointers on every
    // failure. `wem_session_finish` has no out-parameter to drive (ABI
    // revision 3), so what is left of it — the return code as the whole result,
    // and the bytes the write callback did or did not receive — is driven below.
    // ---------------------------------------------------------------------------

    /// A non-NULL out-pointer a failing call must overwrite with NULL. Never
    /// dereferenced: the point of the sentinel is that a caller can see the
    /// difference between "set to NULL" and "left alone".
    fn sentinel_session() -> *mut wem_capi::WemSession {
        std::ptr::without_provenance_mut::<wem_capi::WemSession>(0x5E55_10A1)
    }

    fn sentinel_encoder() -> *mut WemEncoder {
        std::ptr::without_provenance_mut::<WemEncoder>(0x5E55_10A2)
    }

    #[test]
    fn encoder_new_writes_its_out_pointer_on_every_failure() {
        let (pcm, frames, _channels) = read_fixture_pcm();
        let mut sinks = Sinks::new();
        let ud = sinks.user_data();

        let uninstalled = WemProfile {
            version: WemVersion::Wwise2013,
            channels: 2,
            sample_rate: 44_100,
        };
        let malformed = WemProfile {
            version: WemVersion::Wwise2013,
            channels: 0,
            sample_rate: 44_100,
        };

        for (label, profile) in [
            ("NULL profile", std::ptr::null()),
            ("uninstalled selection", &uninstalled as *const WemProfile),
            ("malformed geometry", &malformed as *const WemProfile),
        ] {
            let mut handle = sentinel_encoder();
            let code = unsafe { wem_capi::wem_encoder_new(profile, &mut handle) };
            assert_ne!(code, WemError::Ok, "{label} must not succeed");
            assert!(
                handle.is_null(),
                "{label}: *out_encoder must be NULL after a failure, not the caller's own value"
            );
            // The NULL the caller now holds is a documented no-op to free.
            unsafe { wem_capi::wem_encoder_free(handle) };
        }

        // The same entry through its one-shot sibling, for the record: the
        // NULL handle it would have produced is what a later call rejects.
        assert_eq!(
            unsafe {
                wem_capi::wem_encode_pcm16_interleaved(
                    std::ptr::null(),
                    pcm.as_ptr(),
                    frames,
                    Some(sink_write),
                    ud,
                )
            },
            WemError::StateError
        );
    }

    #[test]
    fn session_new_writes_its_out_pointer_on_every_failure() {
        let mut sinks = Sinks::new();
        let ud = sinks.user_data();

        let uninstalled = WemProfile {
            version: WemVersion::Wwise2013,
            channels: 2,
            sample_rate: 44_100,
        };

        // A NULL profile, an unsatisfiable selection, and — the branch that used
        // to return without writing — a missing required write callback.
        for (label, profile, write_cb) in [
            ("NULL profile", std::ptr::null(), Some(sink_write as _)),
            (
                "uninstalled selection",
                &uninstalled as *const WemProfile,
                Some(sink_write as _),
            ),
            (
                "missing write callback",
                std::ptr::from_ref(&fixture_profile()),
                None,
            ),
        ] {
            let mut session = sentinel_session();
            let code =
                unsafe { wem_capi::wem_session_new(profile, write_cb, None, ud, &mut session) };
            assert_ne!(code, WemError::Ok, "{label} must not succeed");
            assert!(
                session.is_null(),
                "{label}: *out_session must be NULL after a failure, not the caller's own value"
            );
            unsafe { wem_capi::wem_session_free(session) };
        }
    }

    /// `wem_session_finish` returns a code and nothing else (ABI revision 3):
    /// the whole outcome is that code plus what the client's own write callback
    /// already received, and a rejected finish delivers no bytes at all.
    #[test]
    fn finish_reports_its_outcome_and_delivers_bytes_only_on_success() {
        let (pcm, _frames, channels) = read_fixture_pcm();

        // 1. A stream under the 4096-frame minimum: the kernel's own rejection,
        //    and no container bytes reach the sink.
        let mut sinks = Sinks::new();
        let ud = sinks.user_data();
        let mut session = std::ptr::null_mut();
        assert_eq!(
            unsafe {
                wem_capi::wem_session_new(
                    &fixture_profile(),
                    Some(sink_write),
                    None,
                    ud,
                    &mut session,
                )
            },
            WemError::Ok
        );
        let short_len = 100 * channels * 2;
        assert_eq!(
            unsafe { wem_capi::wem_session_push(session, pcm.as_ptr(), short_len) },
            WemError::Ok
        );
        assert_eq!(
            unsafe { wem_capi::wem_session_finish(session) },
            WemError::InputTooShort
        );
        assert!(
            sinks.out.is_empty(),
            "a rejected finish must not deliver a container it did not produce"
        );

        // 2. A second finish: the session is terminal, so this is a refusal.
        assert_eq!(
            unsafe { wem_capi::wem_session_finish(session) },
            WemError::StateError,
            "finish is terminal whatever it returned"
        );
        assert!(sinks.out.is_empty());
        unsafe { wem_capi::wem_session_free(session) };

        // 3. A write callback that aborts the encode: its own code comes back and
        //    the bytes never left.
        let mut refusing = Sinks::new();
        let refusing_ud = refusing.user_data();
        let mut session = std::ptr::null_mut();
        assert_eq!(
            unsafe {
                wem_capi::wem_session_new(
                    &fixture_profile(),
                    Some(sink_write_refuses),
                    None,
                    refusing_ud,
                    &mut session,
                )
            },
            WemError::Ok
        );
        assert_eq!(
            unsafe { wem_capi::wem_session_push(session, pcm.as_ptr(), pcm.len()) },
            WemError::Ok
        );
        assert_eq!(
            unsafe { wem_capi::wem_session_finish(session) },
            WemError::StateError,
            "the callback's own code comes back"
        );
        assert!(refusing.out.is_empty());
        unsafe { wem_capi::wem_session_free(session) };

        // 4. The success path: WEM_OK, and the sink holds the reference container
        //    — the length it needs is what its own callback counted.
        let mut sinks = Sinks::new();
        let ud = sinks.user_data();
        let mut session = std::ptr::null_mut();
        assert_eq!(
            unsafe {
                wem_capi::wem_session_new(
                    &fixture_profile(),
                    Some(sink_write),
                    None,
                    ud,
                    &mut session,
                )
            },
            WemError::Ok
        );
        assert_eq!(
            unsafe { wem_capi::wem_session_push(session, pcm.as_ptr(), pcm.len()) },
            WemError::Ok
        );
        assert_eq!(
            unsafe { wem_capi::wem_session_finish(session) },
            WemError::Ok
        );
        assert_eq!(sinks.out, reference_wem());
        unsafe { wem_capi::wem_session_free(session) };
    }

    /// A write callback that refuses the first block with the kernel's own
    /// "malformed call" code, so the finish path returns before delivering.
    unsafe extern "C" fn sink_write_refuses(
        _data: *const u8,
        _len: usize,
        _user_data: *mut c_void,
    ) -> WemError {
        WemError::StateError
    }
}

mod adversarial {
    //! Adversarial input at the C ABI boundary: every case is a typed rejection.
    //!
    //! `WEM_ERR_INTERNAL` is the defect code (include/wem.h, "Panics"): it says
    //! this library broke an invariant, so no input a caller can hand this surface
    //! may produce it — and none may panic. The cases below are the input-derived
    //! paths a hostile or merely confused caller reaches: a truncated RIFF header,
    //! a `fmt ` chunk of size 0 or 1, a zero-frame stream, absurd channel counts
    //! and geometries, a non-finite quality factor, PCM whose shape cannot be
    //! represented at all.
    //!
    //! Every case names itself in its assertion message, so a failure says which
    //! input was accepted, panicked, or was reported as a defect.
    //!
    //! Two of the listed shapes have no C entry point of their own and are driven
    //! where they do enter the kernel, which is what keeps the claim honest:
    //!
    //! * a WAV is parsed by `wem_core::usecases::wav::parse_pcm16` — the wasm
    //!   shell's `wem_parse_wav` and the CLI's input path;
    //! * a quality factor is bound by
    //!   `wem_core::encoder::Encoder::new_with_quality` — PyO3's `quality=`.
    //!
    //! The float-domain forms of non-finite and out-of-range PCM exist only in the
    //! Python shell (a `float64` memoryview, out-of-range `int`s), which drives
    //! them in its own case; on this surface PCM is raw `int16` bytes, where every
    //! bit pattern is a sample and the adversarial shapes are lengths and channel
    //! counts instead.

    use std::ffi::c_void;

    use wem_capi::{
        wem_encode_pcm16_interleaved, wem_encoder_new, wem_session_new, WemEncoder, WemError,
        WemProfile, WemSession, WemVersion,
    };
    use wem_core::encoder::{Encoder, Pcm16};
    use wem_core::error::EncoderError;
    use wem_core::stream::StreamSession;
    use wem_core::usecases::wav::parse_pcm16;
    use wem_core::{WwiseProfile, WwiseVersion};

    // ---------------------------------------------------------------------------
    // Case drivers: a panic is never an answer, so it is caught and named.
    // ---------------------------------------------------------------------------

    /// Run one case. The library must return a value: a panic is an invariant
    /// violation, not a rejection of bad input, and the message says which input
    /// caused it.
    fn drive<T>(case: &str, work: impl FnOnce() -> T) -> T {
        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(work)) {
            Ok(value) => value,
            Err(_) => panic!(
                "adversarial case {case}: the library panicked; a panic is an invariant \
             violation, never an answer to input"
            ),
        }
    }

    /// The ABI call must refuse the case with the documented code.
    fn assert_abi(case: &str, expected: WemError, call: impl FnOnce() -> WemError) {
        let code = drive(case, call);
        assert_eq!(
            code, expected,
            "adversarial case {case}: expected the documented {expected:?}, got {code:?}"
        );
    }

    /// The kernel call must refuse the case, and the error class is returned for
    /// the caller to pin.
    fn rejected<T>(case: &str, call: impl FnOnce() -> Result<T, EncoderError>) -> EncoderError {
        match drive(case, call) {
            Ok(_) => panic!("adversarial case {case}: the kernel accepted input it must refuse"),
            Err(error) => error,
        }
    }

    /// The stable class name of one kernel error (the include/wem.h table).
    fn class_of(error: &EncoderError) -> &'static str {
        match error {
            EncoderError::ProfileNotFound { .. } => "PROFILE_NOT_FOUND",
            EncoderError::StateError { .. } => "STATE_ERROR",
            EncoderError::GeometryMismatch { .. } => "GEOMETRY_MISMATCH",
            EncoderError::InputTooShort { .. } => "INPUT_TOO_SHORT",
            EncoderError::FormatUnsupported { .. } => "FORMAT_UNSUPPORTED",
            EncoderError::Internal(_) => "INTERNAL",
        }
    }

    /// The case was refused as caller input, not reported as a defect.
    fn assert_refused(case: &str, error: &EncoderError, expected: &str) {
        assert_ne!(
            class_of(error),
            "INTERNAL",
            "adversarial case {case}: reported as a defect ({error}); INTERNAL means this \
         library broke an invariant, not that the caller passed something bad"
        );
        assert_eq!(
            class_of(error),
            expected,
            "adversarial case {case}: wrong rejection class for {error}"
        );
    }

    // ---------------------------------------------------------------------------
    // Adversarial RIFF/WAVE construction
    // ---------------------------------------------------------------------------

    /// A RIFF/WAVE file whose `WAVE` form is `chunks` verbatim, with the RIFF size
    /// field computed from them.
    fn wav(chunks: &[u8]) -> Vec<u8> {
        let mut out = Vec::with_capacity(chunks.len() + 12);
        out.extend_from_slice(b"RIFF");
        out.extend_from_slice(&(chunks.len() as u32 + 4).to_le_bytes());
        out.extend_from_slice(b"WAVE");
        out.extend_from_slice(chunks);
        out
    }

    /// One chunk with an explicit size field, so a case can declare a size its
    /// payload does not have.
    fn chunk(id: &[u8; 4], declared_size: u32, payload: &[u8]) -> Vec<u8> {
        let mut out = id.to_vec();
        out.extend_from_slice(&declared_size.to_le_bytes());
        out.extend_from_slice(payload);
        if declared_size % 2 == 1 {
            out.push(0); // word alignment
        }
        out
    }

    /// The 16-byte PCM `fmt ` payload the parser requires. The byte-rate field is
    /// sink data for the parser, so it saturates instead of overflowing on the
    /// absurd geometries built below (a test that panics building its own input
    /// would prove nothing about the library).
    fn fmt_payload(channels: u16, sample_rate: u32, bits_per_sample: u16) -> Vec<u8> {
        let mut out = Vec::with_capacity(16);
        out.extend_from_slice(&1u16.to_le_bytes()); // PCM
        out.extend_from_slice(&channels.to_le_bytes());
        out.extend_from_slice(&sample_rate.to_le_bytes());
        out.extend_from_slice(
            &sample_rate
                .saturating_mul(u32::from(channels))
                .saturating_mul(2)
                .to_le_bytes(),
        );
        out.extend_from_slice(&channels.saturating_mul(2).to_le_bytes());
        out.extend_from_slice(&bits_per_sample.to_le_bytes());
        out
    }

    /// A well-formed signed-16 PCM WAV: `frames` frames of silence.
    fn silence_wav(channels: u16, sample_rate: u32, frames: usize) -> Vec<u8> {
        let bytes = frames * usize::from(channels) * 2;
        let mut chunks = chunk(b"fmt ", 16, &fmt_payload(channels, sample_rate, 16));
        chunks.extend_from_slice(&chunk(b"data", bytes as u32, &vec![0u8; bytes]));
        wav(&chunks)
    }

    // ---------------------------------------------------------------------------
    // Cases
    // ---------------------------------------------------------------------------

    /// A RIFF header that stops short of its own form: the parser must refuse the
    /// file, never index past what it received.
    #[test]
    fn truncated_riff_headers_are_rejected_not_panics() {
        // Every prefix of a valid file that does not yet carry a whole header.
        let complete = silence_wav(1, 44_100, 1);
        for length in 0..12 {
            let case = format!("RIFF header truncated to {length} bytes");
            let error = rejected(&case, || parse_pcm16(&complete[..length]));
            assert_refused(&case, &error, "FORMAT_UNSUPPORTED");
        }

        // A header that lies about its own form: a wrong RIFF size field is
        // accepted (the chunk walk is the authority), a wrong form is not.
        let mut wrong_form = complete.clone();
        wrong_form[8..12].copy_from_slice(b"AVI ");
        let case = "RIFF header declaring a form other than WAVE";
        let error = rejected(case, || parse_pcm16(&wrong_form));
        assert_refused(case, &error, "FORMAT_UNSUPPORTED");

        // Cut inside the first chunk, and inside the data chunk.
        for cut in [12, 20, 28] {
            let case = format!("valid WAV truncated at {cut} bytes");
            let error = rejected(&case, || parse_pcm16(&complete[..cut]));
            assert_refused(&case, &error, "FORMAT_UNSUPPORTED");
        }
    }

    /// A `fmt ` chunk whose declared size is 0 or 1: the size field is part of the
    /// input, so it is validated like the payload.
    #[test]
    fn fmt_chunks_of_size_zero_and_one_are_rejected_not_panics() {
        for declared in [0u32, 1] {
            let case = format!("fmt chunk declaring size {declared}");
            let mut chunks = chunk(b"fmt ", declared, &[0u8; 1][..declared as usize]);
            chunks.extend_from_slice(&chunk(b"data", 2, &[0, 0]));
            let error = rejected(&case, || parse_pcm16(&wav(&chunks)));
            assert_refused(&case, &error, "FORMAT_UNSUPPORTED");
        }

        // A size field no file can hold: larger than any buffer the parser reads,
        // and large enough that a 32-bit `offset + size` would wrap.
        for declared in [u32::MAX, u32::MAX - 15, 0x8000_0000] {
            let case = format!("fmt chunk declaring size {declared}");
            let mut chunks = chunk(b"fmt ", declared, &fmt_payload(1, 44_100, 16));
            chunks.extend_from_slice(&chunk(b"data", 2, &[0, 0]));
            let error = rejected(&case, || parse_pcm16(&wav(&chunks)));
            assert_refused(&case, &error, "FORMAT_UNSUPPORTED");
        }

        // A `fmt ` chunk that promises 16 bytes and delivers fewer.
        for delivered in 0..16usize {
            let case = format!("fmt chunk promising 16 bytes, delivering {delivered}");
            let mut chunks = chunk(b"fmt ", 16, &fmt_payload(1, 44_100, 16)[..delivered]);
            chunks.extend_from_slice(&chunk(b"data", 2, &[0, 0]));
            let error = rejected(&case, || parse_pcm16(&wav(&chunks)));
            assert_refused(&case, &error, "FORMAT_UNSUPPORTED");
        }
    }

    /// Zero frames: an empty stream is refused wherever it arrives, and never
    /// reaches the analysis pipeline.
    #[test]
    fn zero_frame_streams_are_rejected_not_panics() {
        // A data chunk with no samples at all.
        let mut chunks = chunk(b"fmt ", 16, &fmt_payload(1, 44_100, 16));
        chunks.extend_from_slice(&chunk(b"data", 0, &[]));
        let case = "WAV with an empty data chunk";
        let error = rejected(case, || parse_pcm16(&wav(&chunks)));
        assert_refused(case, &error, "FORMAT_UNSUPPORTED");

        // The ABI: zero frames is a malformed call, not encodable input.
        let pcm = [0i16; 64];
        let profile = profile(6, 44_100);
        assert_abi(
            "one-shot encode of zero frames",
            WemError::StateError,
            || unsafe {
                wem_encode_pcm16_interleaved(
                    &profile,
                    pcm.as_ptr().cast::<u8>(),
                    0,
                    Some(discard_write),
                    std::ptr::null_mut(),
                )
            },
        );

        // The kernel's streaming session: zero frames accumulated, then Finish.
        let case = "streaming session finished with zero frames";
        let error = rejected(case, || {
            StreamSession::for_selection(selection(6, 44_100))?.finish()
        });
        assert_refused(case, &error, "INPUT_TOO_SHORT");

        // The kernel's encoder: a stream below the 4096-frame minimum.
        let case = "one-shot encode of 4095 frames";
        let error = rejected(case, || {
            let encoder = Encoder::new(selection(6, 44_100))?;
            let pcm = Pcm16::from_interleaved_le(44_100, 6, vec![0u8; 4095 * 6 * 2])?;
            encoder.encode_pcm(&pcm)
        });
        assert_refused(case, &error, "INPUT_TOO_SHORT");
    }

    /// Absurd channel counts and geometries: non-positive ones are malformed
    /// arguments, positive ones that no compiled configuration satisfies are
    /// unsatisfiable selections — never a defect, and never an allocation sized by
    /// the caller's number.
    #[test]
    fn absurd_geometries_are_rejected_not_panics() {
        for (channels, sample_rate, expected) in [
            (0, 44_100, WemError::StateError),
            (-1, 44_100, WemError::StateError),
            (i32::MIN, 44_100, WemError::StateError),
            (6, 0, WemError::StateError),
            (6, -44_100, WemError::StateError),
            (6, i32::MIN, WemError::StateError),
            (i32::MAX, 44_100, WemError::ProfileNotFound),
            (i32::MAX, i32::MAX, WemError::ProfileNotFound),
            (6, i32::MAX, WemError::ProfileNotFound),
        ] {
            let case = format!("{channels}ch/{sample_rate}Hz selection");
            let profile = profile(channels, sample_rate);
            let mut handle: *mut WemEncoder = std::ptr::null_mut();
            assert_abi(&case, expected, || unsafe {
                wem_encoder_new(&profile, &mut handle)
            });
            assert!(
                handle.is_null(),
                "adversarial case {case}: a refused selection must hand out no handle"
            );

            let mut session: *mut WemSession = std::ptr::null_mut();
            assert_abi(&case, expected, || unsafe {
                wem_session_new(
                    &profile,
                    Some(discard_write),
                    None,
                    std::ptr::null_mut(),
                    &mut session,
                )
            });
            assert!(
                session.is_null(),
                "adversarial case {case}: a refused selection must hand out no session"
            );
        }

        // A selection the ABI resolves, driven with a frame count whose byte
        // length cannot be represented: the multiplication is checked, never
        // wrapped into a short slice.
        let pcm = [0i16; 64];
        let profile = profile(6, 44_100);
        assert_abi(
            "one-shot encode of usize::MAX frames",
            WemError::StateError,
            || unsafe {
                wem_encode_pcm16_interleaved(
                    &profile,
                    pcm.as_ptr().cast::<u8>(),
                    usize::MAX,
                    Some(discard_write),
                    std::ptr::null_mut(),
                )
            },
        );

        // The kernel's PCM constructors: a channel count that cannot be
        // multiplied into a byte length, and a channel count from a WAV header
        // that no data chunk can fill.
        let case = "interleaved PCM claiming usize::MAX channels";
        let error = rejected(case, || {
            Pcm16::from_interleaved_le(44_100, usize::MAX, [0, 0])
        });
        assert_refused(case, &error, "STATE_ERROR");

        let case = "channel-major PCM claiming usize::MAX channels";
        let error = rejected(case, || {
            Pcm16::from_channel_major_le(44_100, usize::MAX, vec![0, 0])
        });
        assert_refused(case, &error, "STATE_ERROR");

        let mut chunks = chunk(b"fmt ", 16, &fmt_payload(0, 44_100, 16));
        chunks.extend_from_slice(&chunk(b"data", 2, &[0, 0]));
        let case = "WAV declaring zero channels";
        let error = rejected(case, || parse_pcm16(&wav(&chunks)));
        assert_refused(case, &error, "FORMAT_UNSUPPORTED");

        let mut chunks = chunk(b"fmt ", 16, &fmt_payload(u16::MAX, 44_100, 16));
        chunks.extend_from_slice(&chunk(b"data", 2, &[0, 0]));
        let case = "WAV declaring 65535 channels with a two-byte data chunk";
        let error = rejected(case, || parse_pcm16(&wav(&chunks)));
        assert_refused(case, &error, "FORMAT_UNSUPPORTED");
    }

    /// A quality factor is caller input: NaN and the infinities are refused by the
    /// profile resolution, never carried into the analysis resources.
    #[test]
    fn non_finite_quality_is_rejected_not_a_panic() {
        for quality in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            let case = format!("quality {quality}");
            let error = rejected(&case, || {
                Encoder::new_with_quality(selection(6, 44_100), Some(quality)).map(|_| ())
            });
            assert_refused(&case, &error, "STATE_ERROR");

            let error = rejected(&case, || {
                StreamSession::for_selection_quality(selection(6, 44_100), Some(quality))
                    .map(|_| ())
            });
            assert_refused(&case, &error, "STATE_ERROR");
        }
    }

    /// PCM shapes that cannot be represented: a byte length that is not a whole
    /// number of frames, a channel count no allocation can serve, a chunk that
    /// carries half a sample.
    #[test]
    fn unrepresentable_pcm_shapes_are_rejected_not_panics() {
        // Interleaved bytes that stop inside a frame.
        for bytes in [1usize, 11, 13, 6 * 2 * 4096 - 1] {
            let case = format!("interleaved PCM of {bytes} bytes for 6 channels");
            let error = rejected(&case, || {
                Pcm16::from_interleaved_le(44_100, 6, vec![0u8; bytes]).map(|_| ())
            });
            assert_refused(&case, &error, "GEOMETRY_MISMATCH");
        }

        // A stream chunk that carries a trailing partial frame reaches the ABI
        // through the session, where the same rule holds.
        let case = "session chunk with a trailing partial frame";
        let mut session: *mut WemSession = std::ptr::null_mut();
        let profile = profile(6, 44_100);
        let mut sink: Vec<u8> = Vec::new();
        let user_data = std::ptr::from_mut(&mut sink).cast::<c_void>();
        assert_eq!(
            drive(case, || unsafe {
                wem_session_new(&profile, Some(discard_write), None, user_data, &mut session)
            }),
            WemError::Ok,
            "adversarial case {case}: the fixture selection must open"
        );
        let odd = [0u8; 13];
        assert_abi(case, WemError::GeometryMismatch, || unsafe {
            wem_capi::wem_session_push(session, odd.as_ptr(), odd.len())
        });

        // The rejected chunk was a refusal, not a defect: the session is still the
        // live session it was, and the stream it carries is unaffected. A push of
        // whole frames still succeeds (the byte suites own what it produces).
        let whole = [0u8; 12 * 16];
        assert_abi(
            &format!("{case}: the session after a refused chunk"),
            WemError::Ok,
            || unsafe { wem_capi::wem_session_push(session, whole.as_ptr(), whole.len()) },
        );
        unsafe {
            wem_capi::wem_session_free(session);
        }
    }

    // ---------------------------------------------------------------------------
    // Fixtures (no digest constants: the byte suites own the bytes)
    // ---------------------------------------------------------------------------

    /// A write callback that accepts everything: no case here produces container
    /// bytes worth keeping.
    unsafe extern "C" fn discard_write(
        _data: *const u8,
        _len: usize,
        _user_data: *mut c_void,
    ) -> WemError {
        WemError::Ok
    }

    /// One ABI profile selection.
    fn profile(channels: i32, sample_rate: i32) -> WemProfile {
        WemProfile {
            version: WemVersion::Wwise2013,
            channels,
            sample_rate,
        }
    }

    /// The same selection in the kernel's own type.
    fn selection(channels: i64, sample_rate: i64) -> WwiseProfile {
        WwiseProfile::new(WwiseVersion::Wwise2013, channels, sample_rate)
            .expect("the case's geometry is a valid selection")
    }
}
