//! Float32-compatible MDCT, analysis windows, and block extraction.
//!
//! Mirrors Python `wwise_wem/analysis/dsp/transform.py`. Every Python
//! `_f32(...)` boundary is one `f32_of(...)` call here; operation and
//! butterfly ordering is preserved verbatim. The runtime path never computes
//! analytic transcendental values: trigonometry comes from the profile's
//! static `MdctLook` bank and windows come from `FrozenMathTables`.

use crate::config::{f32_of, AnalysisError, MdctLook, TransientDetectorTables};

/// The analysis path reads a float32 window table. The analytic expression
/// reproduces its 256-point block except the endpoint, where the reference
/// encoder's stored value is seven float32 ULPs below a host `sin` result.
/// Kept as a documented bit constant; never used by the exact-profile path.
#[allow(dead_code)]
const WWISE_SHORT_WINDOW_ENDPOINT_F32: f32 = f32::from_bits(0x050c_7838);

/// Build one MDCT look (Python `make_mdct_look`).
///
/// `static_trig` supplies the reference encoder's stored f32 trig bank. When
/// `None`, the libvorbis analytic construction is used for geometries the
/// Wwise binary has no static bank for (non-profile fallback path).
pub fn make_mdct_look(n: i64, static_trig: Option<&[f32]>) -> Result<MdctLook, AnalysisError> {
    if n < 64 || (n & (n - 1)) != 0 {
        return Err(AnalysisError::MalformedField {
            reason: "MDCT size must be a power of two >= 64",
        });
    }
    let log2n = n.trailing_zeros() as i64;

    let trig: Vec<f32> = match static_trig {
        Some(values) => {
            if values.len() as i64 != n + n / 4 {
                return Err(AnalysisError::MalformedField {
                    reason: "static MDCT trig bank has the wrong length",
                });
            }
            values.to_vec()
        }
        None => {
            // Retain the reference libvorbis construction.
            let mut trig = vec![0.0f32; (n + n / 4) as usize];
            for i in 0..(n / 4) {
                trig[2 * i as usize] =
                    (std::f64::consts::PI / n as f64 * (4 * i) as f64).cos() as f32;
                trig[2 * i as usize + 1] =
                    (-(std::f64::consts::PI / n as f64 * (4 * i) as f64)).sin() as f32;
                trig[n as usize / 2 + 2 * i as usize] =
                    (std::f64::consts::PI / (2.0 * n as f64) * ((2 * i) as f64 + 1.0)).cos() as f32;
                trig[n as usize / 2 + 2 * i as usize + 1] =
                    (std::f64::consts::PI / (2.0 * n as f64) * ((2 * i) as f64 + 1.0)).sin() as f32;
            }
            for i in 0..(n / 8) {
                trig[n as usize + 2 * i as usize] =
                    (std::f64::consts::PI / n as f64 * ((4 * i) as f64 + 2.0)).cos() as f32 * 0.5;
                trig[n as usize + 2 * i as usize + 1] =
                    (-(std::f64::consts::PI / n as f64 * ((4 * i) as f64 + 2.0))).sin() as f32
                        * 0.5;
            }
            trig
        }
    };

    let mask = (1i64 << (log2n - 1)) - 1;
    let msb = 1i64 << (log2n - 2);
    let mut bitrev = Vec::with_capacity((n / 4) as usize);
    for i in 0..(n / 8) {
        let mut acc = 0i64;
        for j in 0..log2n {
            if (msb >> j) & i != 0 {
                acc |= 1i64 << j;
            }
        }
        bitrev.push(((!acc) & mask) - 1);
        bitrev.push(acc);
    }
    Ok(MdctLook {
        n,
        log2n,
        trig,
        bitrev,
        scale: (4.0 / n as f64) as f32,
    })
}

// ---------------------------------------------------------------------------
// MDCT butterflies and bitreverse (Python `_b8` .. `_bitreverse`)
// ---------------------------------------------------------------------------

fn b8(x: &mut [f64], base: usize) {
    let b = base;
    let r0 = f32_of(x[b + 6] + x[b + 2]);
    let r1 = f32_of(x[b + 6] - x[b + 2]);
    let r2 = f32_of(x[b + 4] + x[b]);
    let r3 = f32_of(x[b + 4] - x[b]);
    x[b + 6] = f32_of(r0 + r2);
    x[b + 4] = f32_of(r0 - r2);
    let r0 = f32_of(x[b + 5] - x[b + 1]);
    let r2 = f32_of(x[b + 7] - x[b + 3]);
    x[b] = f32_of(r1 + r0);
    x[b + 2] = f32_of(r1 - r0);
    let r0 = f32_of(x[b + 5] + x[b + 1]);
    let r1 = f32_of(x[b + 7] + x[b + 3]);
    x[b + 3] = f32_of(r2 + r3);
    x[b + 1] = f32_of(r2 - r3);
    x[b + 7] = f32_of(r1 + r0);
    x[b + 5] = f32_of(r1 - r0);
}

fn b16(x: &mut [f64], base: usize) {
    let b = base;
    let r0 = f32_of(x[b + 1] - x[b + 9]);
    let r1 = f32_of(x[b] - x[b + 8]);
    x[b + 8] = f32_of(x[b + 8] + x[b]);
    x[b + 9] = f32_of(x[b + 9] + x[b + 1]);
    x[b] = f32_of((r0 + r1) * 0.7071067690849304);
    x[b + 1] = f32_of((r0 - r1) * 0.7071067690849304);
    let r0 = f32_of(x[b + 3] - x[b + 11]);
    let r1 = f32_of(x[b + 10] - x[b + 2]);
    x[b + 10] = f32_of(x[b + 10] + x[b + 2]);
    x[b + 11] = f32_of(x[b + 11] + x[b + 3]);
    x[b + 2] = f32_of(r0);
    x[b + 3] = f32_of(r1);
    let r0 = f32_of(x[b + 12] - x[b + 4]);
    let r1 = f32_of(x[b + 13] - x[b + 5]);
    x[b + 12] = f32_of(x[b + 12] + x[b + 4]);
    x[b + 13] = f32_of(x[b + 13] + x[b + 5]);
    x[b + 4] = f32_of((r0 - r1) * 0.7071067690849304);
    x[b + 5] = f32_of((r0 + r1) * 0.7071067690849304);
    let r0 = f32_of(x[b + 14] - x[b + 6]);
    let r1 = f32_of(x[b + 15] - x[b + 7]);
    x[b + 14] = f32_of(x[b + 14] + x[b + 6]);
    x[b + 15] = f32_of(x[b + 15] + x[b + 7]);
    x[b + 6] = f32_of(r0);
    x[b + 7] = f32_of(r1);
    b8(x, b);
    b8(x, b + 8);
}

fn b32(x: &mut [f64], base: usize) {
    // Constants mirror libvorbis butterfly32; its s3 branch is unused here.
    let c1 = 0.9238795042037964;
    let s1 = 0.3826834261417389;
    let c3 = 0.3826834261417389;
    let s2 = 0.7071067690849304;
    let b = base;
    let r0 = f32_of(x[b + 30] - x[b + 14]);
    let r1 = f32_of(x[b + 31] - x[b + 15]);
    x[b + 30] = f32_of(x[b + 30] + x[b + 14]);
    x[b + 31] = f32_of(x[b + 31] + x[b + 15]);
    x[b + 14] = f32_of(r0);
    x[b + 15] = f32_of(r1);
    let r0 = f32_of(x[b + 28] - x[b + 12]);
    let r1 = f32_of(x[b + 29] - x[b + 13]);
    x[b + 28] = f32_of(x[b + 28] + x[b + 12]);
    x[b + 29] = f32_of(x[b + 29] + x[b + 13]);
    x[b + 12] = f32_of(r0 * c1 - r1 * s1);
    x[b + 13] = f32_of(r0 * s1 + r1 * c1);
    let r0 = f32_of(x[b + 26] - x[b + 10]);
    let r1 = f32_of(x[b + 27] - x[b + 11]);
    x[b + 26] = f32_of(x[b + 26] + x[b + 10]);
    x[b + 27] = f32_of(x[b + 27] + x[b + 11]);
    x[b + 10] = f32_of((r0 - r1) * s2);
    x[b + 11] = f32_of((r0 + r1) * s2);
    let r0 = f32_of(x[b + 24] - x[b + 8]);
    let r1 = f32_of(x[b + 25] - x[b + 9]);
    x[b + 24] = f32_of(x[b + 24] + x[b + 8]);
    x[b + 25] = f32_of(x[b + 25] + x[b + 9]);
    x[b + 8] = f32_of(r0 * s1 - r1 * c1);
    x[b + 9] = f32_of(r1 * s1 + r0 * c1);
    let r0 = f32_of(x[b + 22] - x[b + 6]);
    let r1 = f32_of(x[b + 7] - x[b + 23]);
    x[b + 22] = f32_of(x[b + 22] + x[b + 6]);
    x[b + 23] = f32_of(x[b + 23] + x[b + 7]);
    x[b + 6] = f32_of(r1);
    x[b + 7] = f32_of(r0);
    let r0 = f32_of(x[b + 4] - x[b + 20]);
    let r1 = f32_of(x[b + 5] - x[b + 21]);
    x[b + 20] = f32_of(x[b + 20] + x[b + 4]);
    x[b + 21] = f32_of(x[b + 21] + x[b + 5]);
    x[b + 4] = f32_of(r1 * c1 + r0 * s1);
    x[b + 5] = f32_of(r1 * s1 - r0 * c1);
    let r0 = f32_of(x[b + 2] - x[b + 18]);
    let r1 = f32_of(x[b + 3] - x[b + 19]);
    x[b + 18] = f32_of(x[b + 18] + x[b + 2]);
    x[b + 19] = f32_of(x[b + 19] + x[b + 3]);
    x[b + 2] = f32_of((r1 + r0) * s2);
    x[b + 3] = f32_of((r1 - r0) * s2);
    let r0 = f32_of(x[b] - x[b + 16]);
    let r1 = f32_of(x[b + 1] - x[b + 17]);
    x[b + 16] = f32_of(x[b + 16] + x[b]);
    x[b + 17] = f32_of(x[b + 17] + x[b + 1]);
    x[b] = f32_of(r1 * c3 + r0 * c1);
    x[b + 1] = f32_of(r1 * c1 - r0 * c3);
    b16(x, b);
    b16(x, b + 16);
}

fn mdct_first(trig: &[f32], x: &mut [f64], points: i64) {
    let mut x1 = points - 8;
    let mut x2 = (points >> 1) - 8;
    let mut t = 0i64;
    while x2 >= 0 {
        for off in [6i64, 4, 2, 0] {
            // This stage spills both differences before loading the trig pair.
            let a = (x1 + off) as usize;
            let b = (x1 + off + 1) as usize;
            let c = (x2 + off) as usize;
            let d = (x2 + off + 1) as usize;
            let r0 = f32_of(x[a] - x[c]);
            let r1 = f32_of(x[b] - x[d]);
            x[a] = f32_of(x[a] + x[c]);
            x[b] = f32_of(x[b] + x[d]);
            x[c] = f32_of(r1 * (trig[(t + 1) as usize] as f64) + r0 * (trig[t as usize] as f64));
            x[d] = f32_of(r1 * (trig[t as usize] as f64) - r0 * (trig[(t + 1) as usize] as f64));
            t += 4;
        }
        x1 -= 8;
        x2 -= 8;
    }
}

fn mdct_generic(trig: &[f32], x: &mut [f64], base: i64, points: i64, stride: i64) {
    let mut x1 = base + points - 8;
    let mut x2 = base + (points >> 1) - 8;
    let mut t = 0i64;
    while x2 >= base {
        for off in [6i64, 4, 2, 0] {
            let a = (x1 + off) as usize;
            let b = (x1 + off + 1) as usize;
            let c = (x2 + off) as usize;
            let d = (x2 + off + 1) as usize;
            let r0 = f32_of(x[a] - x[c]);
            let r1 = f32_of(x[b] - x[d]);
            x[a] = f32_of(x[a] + x[c]);
            x[b] = f32_of(x[b] + x[d]);
            x[c] = f32_of(r1 * (trig[(t + 1) as usize] as f64) + r0 * (trig[t as usize] as f64));
            x[d] = f32_of(r1 * (trig[t as usize] as f64) - r0 * (trig[(t + 1) as usize] as f64));
            t += stride;
        }
        x1 -= 8;
        x2 -= 8;
    }
}

fn mdct_butterflies(look: &MdctLook, x: &mut [f64], points: i64) {
    let mut stages = look.log2n - 5;
    // Python: `if (stages := stages - 1) > 0: _first(...)`
    stages -= 1;
    if stages > 0 {
        mdct_first(&look.trig, x, points);
    }
    let mut i = 1i64;
    loop {
        // Python: `while (stages := stages - 1) > 0:` decrements before checking.
        stages -= 1;
        if stages <= 0 {
            break;
        }
        for j in 0..(1i64 << i) {
            mdct_generic(&look.trig, x, (points >> i) * j, points >> i, 4 << i);
        }
        i += 1;
    }
    for base in (0..points).step_by(32) {
        b32(x, base as usize);
    }
}

fn mdct_bitreverse(look: &MdctLook, x: &mut [f64]) {
    let n = look.n as usize;
    let half = n >> 1;
    let mut w0 = 0usize;
    let mut w1 = half;
    let mut t = n;
    let mut bit = 0;
    while w0 < w1 {
        let ia = half + look.bitrev[bit] as usize;
        let ib = half + look.bitrev[bit + 1] as usize;
        // reference routine spills both butterfly inputs to float stack;
        // subtraction/sum as host doubles is usually invisible, but it
        // changes the low-MDCT cancellation bins by whole dB after the
        // bit-domain log curve.
        let r0 = f32_of(x[ia + 1] - x[ib + 1]);
        let r1 = f32_of(x[ia] + x[ib]);
        let r2 = f32_of(r1 * (look.trig[t] as f64) + r0 * (look.trig[t + 1] as f64));
        let r3 = f32_of(r1 * (look.trig[t + 1] as f64) - r0 * (look.trig[t] as f64));
        w1 -= 4;
        let h0 = f32_of(0.5 * (x[ia + 1] + x[ib + 1]));
        let h1 = f32_of(0.5 * (x[ia] - x[ib]));
        x[w0] = f32_of(h0 + r2);
        x[w1 + 2] = f32_of(h0 - r2);
        x[w0 + 1] = f32_of(h1 + r3);
        x[w1 + 3] = f32_of(r3 - h1);

        let ia = half + look.bitrev[bit + 2] as usize;
        let ib = half + look.bitrev[bit + 3] as usize;
        let r0 = f32_of(x[ia + 1] - x[ib + 1]);
        let r1 = f32_of(x[ia] + x[ib]);
        let r2 = f32_of(r1 * (look.trig[t + 2] as f64) + r0 * (look.trig[t + 3] as f64));
        let r3 = f32_of(r1 * (look.trig[t + 3] as f64) - r0 * (look.trig[t + 2] as f64));
        let h0 = f32_of(0.5 * (x[ia + 1] + x[ib + 1]));
        let h1 = f32_of(0.5 * (x[ia] - x[ib]));
        x[w0 + 2] = f32_of(h0 + r2);
        x[w1] = f32_of(h0 - r2);
        x[w0 + 3] = f32_of(h1 + r3);
        x[w1 + 1] = f32_of(r3 - h1);
        w0 += 4;
        bit += 4;
        t += 4;
    }
}

/// Return the normalized `n/2` MDCT spectrum for one `n`-sample block
/// (Python `mdct_forward`).
pub fn mdct_forward(look: &MdctLook, samples: &[f64]) -> Result<Vec<f64>, AnalysisError> {
    let n = look.n as usize;
    if samples.len() < n {
        return Err(AnalysisError::SamplesShort {
            need: look.n,
            got: samples.len() as i64,
        });
    }
    let n2 = n >> 1;
    let n4 = n >> 2;
    let n8 = n >> 3;
    let mut w = vec![0.0f64; n];
    let w2 = n2;
    let mut x0 = n2 + n4;
    let mut x1 = x0 + 1;
    let mut t = n2;
    let mut i = 0usize;
    while i < n8 {
        x0 -= 4;
        t -= 2;
        // spectrum transform's var_8/var_C are float stack slots. Keeping
        // these sums in host double lets a low bit leak into every later
        // butterfly stage.
        let r0 = f32_of(samples[x0 + 2] + samples[x1]);
        let r1 = f32_of(samples[x0] + samples[x1 + 2]);
        w[w2 + i] = f32_of(r1 * (look.trig[t + 1] as f64) + r0 * (look.trig[t] as f64));
        w[w2 + i + 1] = f32_of(r1 * (look.trig[t] as f64) - r0 * (look.trig[t + 1] as f64));
        x1 += 4;
        i += 2;
    }

    let mut x1 = 1usize;
    while i < n2 - n8 {
        t -= 2;
        x0 -= 4;
        let r0 = f32_of(samples[x0 + 2] - samples[x1]);
        let r1 = f32_of(samples[x0] - samples[x1 + 2]);
        w[w2 + i] = f32_of(r1 * (look.trig[t + 1] as f64) + r0 * (look.trig[t] as f64));
        w[w2 + i + 1] = f32_of(r1 * (look.trig[t] as f64) - r0 * (look.trig[t + 1] as f64));
        x1 += 4;
        i += 2;
    }

    let mut x0 = n;
    while i < n2 {
        t -= 2;
        x0 -= 4;
        let r0 = f32_of(-samples[x0 + 2] - samples[x1]);
        let r1 = f32_of(-samples[x0] - samples[x1 + 2]);
        w[w2 + i] = f32_of(r1 * (look.trig[t + 1] as f64) + r0 * (look.trig[t] as f64));
        w[w2 + i + 1] = f32_of(r1 * (look.trig[t] as f64) - r0 * (look.trig[t + 1] as f64));
        x1 += 4;
        i += 2;
    }

    // Python: `butterfly_buf = w[n2:]` is a view; the butterflies mutate it
    // in place, then `w[n2:] = butterfly_buf` writes the same values back.
    {
        let tail = &mut w[n2..];
        mdct_butterflies(look, tail, n2 as i64);
    }
    mdct_bitreverse(look, &mut w);

    let mut out = vec![0.0f64; n2];
    let mut t = n2;
    let mut x0 = n2;
    for i in 0..n4 {
        x0 -= 1;
        out[i] = f32_of(
            (w[2 * i] * (look.trig[t] as f64) + w[2 * i + 1] * (look.trig[t + 1] as f64))
                * (look.scale as f64),
        );
        out[x0] = f32_of(
            (w[2 * i] * (look.trig[t + 1] as f64) - w[2 * i + 1] * (look.trig[t] as f64))
                * (look.scale as f64),
        );
        t += 2;
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// Windows
// ---------------------------------------------------------------------------

/// Return the psychoacoustic analysis window (Python `wwise_psy_window`).
/// The transient detector is locked to n=128 and reads the stored float
/// words at look+36; other sizes have no runtime analytic path.
pub fn wwise_psy_window(
    n: i64,
    tables: &TransientDetectorTables,
) -> Result<Vec<f64>, AnalysisError> {
    if n < 2 {
        return Err(AnalysisError::PsyWindowSize { n });
    }
    if n == 128 {
        if tables.n != n {
            return Err(AnalysisError::TransientWindowGeometry {
                tables_n: tables.n,
                n,
            });
        }
        return Ok(tables.window.iter().map(|v| *v as f64).collect());
    }
    Err(AnalysisError::PsyWindowSize { n })
}

/// Run the transient -> spectrum psycho front-end (Python `wwise_psy_mdct`).
pub fn wwise_psy_mdct(
    samples: &[f64],
    look: &MdctLook,
    tables: &TransientDetectorTables,
) -> Result<Vec<f64>, AnalysisError> {
    let n = samples.len() as i64;
    if look.n != n {
        return Err(AnalysisError::TransientMdctGeometry { look_n: look.n, n });
    }
    let window = wwise_psy_window(n, tables)?;
    let windowed: Vec<f64> = samples
        .iter()
        .zip(window.iter())
        .map(|(s, w)| f32_of(*s * *w))
        .collect();
    mdct_forward(look, &windowed)
}

/// Apply the encoder-side Vorbis hybrid window to one block
/// (Python `apply_vorbis_window`).
pub fn apply_vorbis_window(
    samples: &[f64],
    blocksizes: &[i64],
    previous: i64,
    current: i64,
    following: i64,
    frozen_windows: Option<&std::collections::HashMap<i64, Vec<f32>>>,
) -> Result<Vec<f64>, AnalysisError> {
    let spans = frame_window_spans(samples.len(), blocksizes, previous, current, following)?;
    let mut out: Vec<f64> = samples[..spans.n as usize].to_vec();
    window_frame(&mut out, &spans, frozen_windows)?;
    Ok(out)
}

/// Apply the same hybrid window in place, to the rows the caller already
/// gathered (the analysis-input materializer fills a frame's own row
/// buffers and windows them there — no gather buffer, no copy).
///
/// `out` holds the frame's `n` raw values, exactly as the copying form's
/// `samples[..n]` does; the arithmetic and its order are the copying
/// form's, statement for statement.
pub fn apply_vorbis_window_in_place(
    out: &mut [f64],
    blocksizes: &[i64],
    previous: i64,
    current: i64,
    following: i64,
    frozen_windows: Option<&std::collections::HashMap<i64, Vec<f32>>>,
) -> Result<(), AnalysisError> {
    let spans = frame_window_spans(out.len(), blocksizes, previous, current, following)?;
    window_frame(out, &spans, frozen_windows)
}

/// The validated hybrid-window spans of one frame's block sizes.
struct WindowSpans {
    n: i64,
    left_n: i64,
    right_n: i64,
    left_begin: i64,
    left_end: i64,
    right_begin: i64,
    right_end: i64,
}

/// Validate one frame's block-size triple and derive its window spans:
/// the checks `apply_vorbis_window` has always made, in the same order.
fn frame_window_spans(
    samples_len: usize,
    blocksizes: &[i64],
    previous: i64,
    current: i64,
    following: i64,
) -> Result<WindowSpans, AnalysisError> {
    let n = blocksizes[current as usize];
    let left_n = blocksizes[previous as usize];
    let right_n = blocksizes[following as usize];
    if samples_len < n as usize {
        return Err(AnalysisError::SamplesShort {
            need: n,
            got: samples_len as i64,
        });
    }
    if blocksizes.iter().any(|size| *size < 2 || *size & 1 != 0) {
        return Err(AnalysisError::WindowBlockSizeInvalid);
    }
    for index in [previous, current, following] {
        if !(0 <= index && index < blocksizes.len() as i64) {
            return Err(AnalysisError::WindowStateIndexOutOfRange { index });
        }
    }

    let left_begin = n / 4 - left_n / 4;
    let left_end = left_begin + left_n / 2;
    let right_begin = n / 2 + n / 4 - right_n / 4;
    let right_end = right_begin + right_n / 2;
    if !(0 <= left_begin
        && left_begin <= left_end
        && left_end <= right_begin
        && right_begin <= right_end
        && right_end <= n)
    {
        return Err(AnalysisError::WindowIntervalsIncompatible);
    }
    Ok(WindowSpans {
        n,
        left_n,
        right_n,
        left_begin,
        left_end,
        right_begin,
        right_end,
    })
}

/// Apply the hybrid window to a frame's raw rows in place.
///
/// Both spans read only the frozen half of their window: the left span
/// walks offsets `0..left_n / 2` forwards, the right span walks
/// `right_n / 2 - 1` down to `0`. In the window's
/// `half ++ reverse(half)` layout those indices are all in the first half,
/// where the value is the frozen entry itself, so materializing the full
/// window per frame would build a second half no index here reaches.
fn window_frame(
    out: &mut [f64],
    spans: &WindowSpans,
    frozen_windows: Option<&std::collections::HashMap<i64, Vec<f32>>>,
) -> Result<(), AnalysisError> {
    let frozen = frozen_windows.ok_or(AnalysisError::FrozenWindowDomainMiss { size: spans.n })?;
    let left_half = frozen_window_half(frozen, spans.left_n)?;
    let right_half = frozen_window_half(frozen, spans.right_n)?;

    for i in 0..spans.left_begin {
        out[i as usize] = 0.0;
    }
    for i in spans.left_begin..spans.left_end {
        out[i as usize] =
            f32_of(out[i as usize] * (left_half[(i - spans.left_begin) as usize] as f64));
    }
    for i in spans.right_begin..spans.right_end {
        out[i as usize] = f32_of(
            out[i as usize]
                * (right_half[(spans.right_n / 2 - 1 - (i - spans.right_begin)) as usize] as f64),
        );
    }
    for i in spans.right_end..spans.n {
        out[i as usize] = 0.0;
    }
    Ok(())
}

/// The frozen window half for `size`, validated exactly as the full
/// window builder validated it (domain miss, then size, then length).
fn frozen_window_half(
    frozen: &std::collections::HashMap<i64, Vec<f32>>,
    size: i64,
) -> Result<&[f32], AnalysisError> {
    let half = frozen
        .get(&size)
        .ok_or(AnalysisError::FrozenWindowDomainMiss { size })?;
    if size < 2 || size & 1 != 0 {
        return Err(AnalysisError::WindowSizeInvalid { n: size });
    }
    if half.len() as i64 != size / 2 {
        return Err(AnalysisError::FrozenWindowHalfMismatch {
            size,
            want: size / 2,
            got: half.len() as i64,
        });
    }
    Ok(half)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Synthetic f32 trig bank for a small power-of-two size (test-only; the
    /// exact-profile path always uses the checksummed bank from
    /// wem-profiles).
    fn synthetic_look(n: i64) -> MdctLook {
        let static_trig = None;
        make_mdct_look(n, static_trig).expect("synthetic look builds")
    }

    #[test]
    fn mdct_forward_rejects_short_input() {
        let look = synthetic_look(64);
        let samples = vec![0.0f64; 63];
        assert!(matches!(
            mdct_forward(&look, &samples),
            Err(AnalysisError::SamplesShort { .. })
        ));
    }

    #[test]
    fn hybrid_window_rejects_missing_frozen() {
        // The claim the deleted `vorbis_window` test made, moved to the path
        // actually used: a missing frozen half is a domain error, not an
        // analytic fallback.
        let mut row = vec![0.0f64; 2048];
        assert!(matches!(
            apply_vorbis_window_in_place(&mut row, &[256, 2048], 0, 1, 1, None),
            Err(AnalysisError::FrozenWindowDomainMiss { size: 2048 })
        ));
    }

    use crate::config::u32_to_f32;

    #[test]
    fn u32_f32_helper_roundtrip() {
        assert_eq!(u32_to_f32(0x3F80_0000), 1.0f32);
        assert_eq!(u32_to_f32(0x0000_0000), 0.0f32);
    }
}
