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
// Inverse MDCT, synthesis window and overlap-add
// ---------------------------------------------------------------------------

/// Return the `n` samples one `n/2` spectrum synthesizes (Python
/// `mdct_backward`, libvorbis `mdct.c`), at unit scale.
///
/// The pair of [`mdct_forward`]: the same frozen `look.trig` bank read in the
/// other direction, the same butterfly and bit-reverse order, one
/// `f32_of(...)` per reference assignment. It carries no normalization of its
/// own — the forward's baked-in `4/n` is what makes the lapped round trip
/// close at unit scale, so a scale correction here would break the pair.
///
/// The result is the *unwindowed* block. Synthesis is
/// [`apply_synthesis_window_in_place`] followed by the overlap-add in
/// [`SynthesisOla`] (or the whole-sequence form, [`synthesize_sequence`]).
pub fn mdct_backward(look: &MdctLook, spectrum: &[f64]) -> Result<Vec<f64>, AnalysisError> {
    let n = look.n as usize;
    let n2 = n >> 1;
    if spectrum.len() < n2 {
        return Err(AnalysisError::SamplesShort {
            need: n2 as i64,
            got: spectrum.len() as i64,
        });
    }
    let n4 = n >> 2;
    let mut out = vec![0.0f64; n];

    // Rotate: the coefficient row folds into the butterfly's input, walked
    // backwards in two interleaved passes — the first reads the odd offsets
    // of each eight-sample group, the second the even ones.
    let mut idx = n2 as i64 - 7;
    let mut o = n2 + n4;
    let mut t = n4;
    while idx >= 0 {
        o -= 4;
        let t0 = look.trig[t] as f64;
        let t1 = look.trig[t + 1] as f64;
        let t2 = look.trig[t + 2] as f64;
        let t3 = look.trig[t + 3] as f64;
        let i = idx as usize;
        out[o] = f32_of(-spectrum[i + 2] * t3 - spectrum[i] * t2);
        out[o + 1] = f32_of(spectrum[i] * t3 - spectrum[i + 2] * t2);
        out[o + 2] = f32_of(-spectrum[i + 6] * t1 - spectrum[i + 4] * t0);
        out[o + 3] = f32_of(spectrum[i + 4] * t1 - spectrum[i + 6] * t0);
        idx -= 8;
        t += 4;
    }

    let mut idx = n2 as i64 - 8;
    let mut o = n2 + n4;
    let mut t = n4;
    while idx >= 0 {
        t -= 4;
        let t0 = look.trig[t] as f64;
        let t1 = look.trig[t + 1] as f64;
        let t2 = look.trig[t + 2] as f64;
        let t3 = look.trig[t + 3] as f64;
        let i = idx as usize;
        out[o] = f32_of(spectrum[i + 4] * t3 + spectrum[i + 6] * t2);
        out[o + 1] = f32_of(spectrum[i + 4] * t2 - spectrum[i + 6] * t3);
        out[o + 2] = f32_of(spectrum[i] * t1 + spectrum[i + 2] * t0);
        out[o + 3] = f32_of(spectrum[i] * t0 - spectrum[i + 2] * t1);
        idx -= 8;
        o += 4;
    }

    // Same butterfly and bit-reverse stages the forward runs, on the lower
    // half (Python: `mdct_butterflies(init, out[n2:], n2)`).
    {
        let tail = &mut out[n2..];
        mdct_butterflies(look, tail, n2 as i64);
    }
    mdct_bitreverse(look, &mut out);

    // Rotate: the butterfly's two halves mirror out into the block (the
    // reference labels this pass "rotate + window"; the window it names is the
    // caller's, applied separately by `apply_synthesis_window_in_place`). The
    // eight inputs are this iteration's source region; snapshotting them
    // preserves the reference's read-then-write order, which the disjoint
    // regions of a power-of-two look would hide.
    let mut o1 = n2 + n4;
    let mut o2 = n2 + n4;
    let mut i = 0usize;
    let mut t = n2;
    while i < o1 {
        o1 -= 4;
        let t0 = look.trig[t] as f64;
        let t1 = look.trig[t + 1] as f64;
        let t2 = look.trig[t + 2] as f64;
        let t3 = look.trig[t + 3] as f64;
        let t4 = look.trig[t + 4] as f64;
        let t5 = look.trig[t + 5] as f64;
        let t6 = look.trig[t + 6] as f64;
        let t7 = look.trig[t + 7] as f64;
        let r0 = out[i];
        let r1 = out[i + 1];
        let r2 = out[i + 2];
        let r3 = out[i + 3];
        let r4 = out[i + 4];
        let r5 = out[i + 5];
        let r6 = out[i + 6];
        let r7 = out[i + 7];
        out[o1 + 3] = f32_of(r0 * t1 - r1 * t0);
        out[o2] = -f32_of(r0 * t0 + r1 * t1);
        out[o1 + 2] = f32_of(r2 * t3 - r3 * t2);
        out[o2 + 1] = -f32_of(r2 * t2 + r3 * t3);
        out[o1 + 1] = f32_of(r4 * t5 - r5 * t4);
        out[o2 + 2] = -f32_of(r4 * t4 + r5 * t5);
        out[o1] = f32_of(r6 * t7 - r7 * t6);
        out[o2 + 3] = -f32_of(r6 * t6 + r7 * t7);
        o2 += 4;
        i += 8;
        t += 8;
    }

    // Mirror: the front quarter folds into the two middle quarters.
    let mut i = n2 + n4;
    let mut o1 = n4;
    let mut o2 = n4;
    while o2 < i {
        o1 -= 4;
        i -= 4;
        out[o1 + 3] = out[i + 3];
        out[o2] = -out[i + 3];
        out[o1 + 2] = out[i + 2];
        out[o2 + 1] = -out[i + 2];
        out[o1 + 1] = out[i + 1];
        out[o2 + 2] = -out[i + 1];
        out[o1] = out[i];
        out[o2 + 3] = -out[i];
        o2 += 4;
    }

    // Mirror: the top quarter reverses into the third quarter.
    let mut i = n2 + n4;
    let mut o1 = n2 + n4;
    let o2 = n2;
    while o1 > o2 {
        o1 -= 4;
        out[o1] = out[i + 3];
        out[o1 + 1] = out[i + 2];
        out[o1 + 2] = out[i + 1];
        out[o1 + 3] = out[i];
        i += 4;
    }
    Ok(out)
}

/// The window mode triple one block's hybrid window actually reads.
///
/// Short blocks do not use pending long-transition flags; their effective
/// window is always the short one, so a long neighbour cannot stretch a short
/// block's halves. Long blocks take the size of each block they overlap. This
/// is the rule the analysis materializer applies before it calls
/// [`apply_vorbis_window_in_place`]; synthesis mirrors it here so neither
/// direction can be given the other's reading.
fn effective_window_modes(current: i64, previous: i64, following: i64) -> (i64, i64, i64) {
    if current == 0 {
        (0, 0, 0)
    } else {
        (previous, current, following)
    }
}

/// The validated spans of one block's effective synthesis window.
fn synthesis_window_spans(
    samples_len: usize,
    blocksizes: &[i64],
    previous: i64,
    current: i64,
    following: i64,
) -> Result<WindowSpans, AnalysisError> {
    let (left, current, right) = effective_window_modes(current, previous, following);
    frame_window_spans(samples_len, blocksizes, left, current, right)
}

/// Apply the synthesis window to one inverse-transformed block in place
/// (Python `apply_vorbis_window`, synthesis direction).
///
/// The synthesis window is the analysis window: the same frozen halves
/// multiplied into the same offsets with the same rounding, which is what
/// makes `w² + w'² = 1` cancel the two overlapping blocks' aliases. Only the
/// mode reading differs, and it differs the same way in both directions: a
/// short block always uses the short window (see `effective_window_modes`).
pub fn apply_synthesis_window_in_place(
    out: &mut [f64],
    blocksizes: &[i64],
    previous: i64,
    current: i64,
    following: i64,
    frozen_windows: Option<&std::collections::HashMap<i64, Vec<f32>>>,
) -> Result<(), AnalysisError> {
    let spans = synthesis_window_spans(out.len(), blocksizes, previous, current, following)?;
    window_frame(out, &spans, frozen_windows)
}

/// The synthesis overlap-add state: one block at a time, on the encoder's own
/// scheduling timeline.
///
/// A block lands where the scheduler planned it
/// (`wem_scheduling::plan_mode_sequence`): the planner's `buffer_base`
/// advances by `(current + following block size) / 4` per block, and a block's
/// own start is `base + blocksizes[1] / 2 - n/2` — so a long block following a
/// short one starts *before* it, and every block's centre is `base +
/// blocksizes[1] / 2`. PCM sample `i` is timeline position
/// `i + blocksizes[1] / 2`, so the timeline's leading `blocksizes[1] / 2`
/// samples are lead-in and never appear in [`SynthesisOla::pcm`] — the offset
/// is the planner's, not a normalization, and `positions_match_the_scheduler`
/// pins every position against the scheduler's own plans.
///
/// The window needs the *following* block's size, so a streaming caller pushes
/// block `k` once packet `k + 1` has been parsed, and the last block at
/// [`SynthesisOla::finish`].
pub struct SynthesisOla {
    blocksizes: [i64; 2],
    /// Timeline accumulator: `samples[i]` is timeline position `i`.
    samples: Vec<f64>,
    /// Lead-in length, `blocksizes[1] / 2`. A block's *centre* is
    /// `base + origin` whatever its size, which is why the planner's cursor
    /// sits there.
    origin: usize,
    /// Mode of the block before the next one (the planner's `previous`).
    previous: i64,
    /// The planner's `buffer_base`: the timeline position the next block's
    /// extent is measured from. Its own start is `base + origin - n/2`, so a
    /// long block following a short one starts *before* it.
    base: usize,
    /// Final PCM samples handed out so far.
    completed: usize,
    /// Timeline position just past the last block's window support.
    support_end: usize,
}

impl SynthesisOla {
    /// Start a synthesis stream over the profile's short/long block-size pair
    /// (mode 0 = short, 1 = long).
    pub fn new(blocksizes: &[i64]) -> Result<Self, AnalysisError> {
        if blocksizes.len() != 2 || blocksizes.iter().any(|size| *size < 2 || *size & 1 != 0) {
            return Err(AnalysisError::WindowBlockSizeInvalid);
        }
        if blocksizes[0] >= blocksizes[1] {
            // The pair is `[short, long]` — the container's own geometry and
            // the meaning of every span this module computes. Anything else
            // makes the timeline origin (`blocksizes[1] / 2`) meaningless.
            return Err(AnalysisError::WindowIntervalsIncompatible);
        }
        let origin = (blocksizes[1] / 2) as usize;
        Ok(Self {
            blocksizes: [blocksizes[0], blocksizes[1]],
            samples: vec![0.0f64; origin],
            origin,
            previous: 0,
            base: 0,
            completed: 0,
            support_end: 0,
        })
    }

    /// Timeline position (the encoder's own coordinate) a block of mode
    /// `current` lands on. PCM sample `i` is timeline position
    /// `i + blocksizes[1] / 2`.
    pub fn next_position(&self, current: i64) -> Result<i64, AnalysisError> {
        let n = self.block_size(current)?;
        Ok((self.base + self.origin) as i64 - n / 2)
    }

    /// The PCM samples synthesized so far that are final: every block that
    /// reaches them has been pushed. Index `i` is PCM sample `i`.
    pub fn pcm(&self) -> &[f64] {
        &self.samples[self.origin..self.origin + self.completed]
    }

    /// Inverse-transform, window and overlap-add one block, and return the
    /// PCM samples this block completed.
    ///
    /// `current` is the mode of the block being pushed, `following` the mode
    /// of the block after it; `look` is the frozen MDCT look for
    /// `blocksizes[current]` and `spectrum` its `n/2` coefficients.
    ///
    /// Every later block's first touched sample is at or after this block's
    /// centre — a block zeroes everything before its left span, and that span
    /// begins exactly at the previous block's centre — so the samples below
    /// that centre are final once this returns. The returned slice is a prefix
    /// of [`SynthesisOla::pcm`] that no later push rewrites.
    pub fn push(
        &mut self,
        look: &MdctLook,
        frozen_windows: &std::collections::HashMap<i64, Vec<f32>>,
        current: i64,
        following: i64,
        spectrum: &[f64],
    ) -> Result<&[f64], AnalysisError> {
        let n = self.block_size(current)?;
        let following_size = self.block_size(following)?;
        if look.n != n {
            return Err(if current == 0 {
                AnalysisError::ShortMdctLookGeometry { want: n }
            } else {
                AnalysisError::LongMdctLookGeometry { want: n }
            });
        }
        let spans = synthesis_window_spans(
            n as usize,
            &self.blocksizes,
            self.previous,
            current,
            following,
        )?;
        let mut block = mdct_backward(look, spectrum)?;
        window_frame(&mut block, &spans, Some(frozen_windows))?;

        let position = self.base + self.origin - (n / 2) as usize;
        let end = position + n as usize;
        if self.samples.len() < end {
            self.samples.resize(end, 0.0f64);
        }
        for (offset, value) in block.iter().enumerate() {
            let slot = position + offset;
            self.samples[slot] = f32_of(self.samples[slot] + *value);
        }

        let before = self.completed;
        // The block's centre is `base + origin`. A later block zeroes
        // everything before its left span, which begins exactly there, so
        // everything below the centre is final now.
        self.completed = self.completed.max(self.base);
        self.support_end = position + spans.right_end as usize;
        self.previous = current;
        self.base += ((n + following_size) / 4) as usize;
        Ok(&self.pcm()[before..])
    }

    /// Release the last block's tail: with no further block to overlap, the
    /// window support of the last block pushed is final. Returns the PCM
    /// samples that this releases, empty when nothing is pending.
    pub fn finish(&mut self) -> &[f64] {
        let before = self.completed;
        self.completed = self
            .completed
            .max(self.support_end.saturating_sub(self.origin));
        &self.pcm()[before..]
    }

    /// The block size one mode names, rejecting a mode the pair does not have.
    fn block_size(&self, mode: i64) -> Result<i64, AnalysisError> {
        if mode < 0 || mode >= self.blocksizes.len() as i64 {
            return Err(AnalysisError::WindowStateIndexOutOfRange { index: mode });
        }
        Ok(self.blocksizes[mode as usize])
    }
}

/// Synthesize a whole mode sequence: inverse MDCT, hybrid synthesis window and
/// overlap-add, in one pass (the entry the decode surface is promised).
///
/// `looks` and `blocksizes` are indexed by mode (0 = short, 1 = long), `modes`
/// is the sequence the packet headers carry, `terminal_following` is the mode
/// of the block after the last one (the encoder plans its own tail with a long
/// block) and `spectra` holds one `n/2`-long coefficient row per mode, in the
/// same order.
///
/// Returns the PCM samples the sequence covers, sample `i` being the encoder's
/// input sample `i`: the scheduler timeline's leading `blocksizes[1] / 2`
/// samples are lead-in and are dropped here rather than handed to the caller.
/// A caller that knows the stream's frame count takes the first
/// `dw_total_pcm_frames` samples; the samples past them are the encoder's own
/// tail look-ahead.
///
/// A mode transition does not narrow this. The hybrid window zeroes only the
/// regions outside a block's two spans, so where a long block overlaps a short
/// one the samples between the two blocks' centres that neither half covers are
/// carried *unwindowed*, at weight one. The lapped round trip is the identity
/// there as well; what it does not close over is the incomplete lead-in and
/// tail window halves, which is why synthesis starts at PCM sample zero and why
/// a caller reads its frame count out of the container rather than off this
/// length.
pub fn synthesize_sequence(
    looks: [&MdctLook; 2],
    frozen_windows: &std::collections::HashMap<i64, Vec<f32>>,
    blocksizes: &[i64],
    modes: &[i64],
    terminal_following: i64,
    spectra: &[Vec<f64>],
) -> Result<Vec<f64>, AnalysisError> {
    if spectra.len() != modes.len() {
        // One coefficient row per block; a short or surplus row set is the
        // same class of input defect as a short sample block.
        return Err(AnalysisError::SamplesShort {
            need: modes.len() as i64,
            got: spectra.len() as i64,
        });
    }
    let mut ola = SynthesisOla::new(blocksizes)?;
    let mut pcm: Vec<f64> = Vec::new();
    for (index, &current) in modes.iter().enumerate() {
        let following = if index + 1 < modes.len() {
            modes[index + 1]
        } else {
            terminal_following
        };
        let look = looks
            .get(current as usize)
            .ok_or(AnalysisError::WindowStateIndexOutOfRange { index: current })?;
        pcm.extend_from_slice(ola.push(
            look,
            frozen_windows,
            current,
            following,
            &spectra[index],
        )?);
    }
    pcm.extend_from_slice(ola.finish());
    Ok(pcm)
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

    // -----------------------------------------------------------------------
    // Inverse MDCT and overlap-add
    // -----------------------------------------------------------------------

    /// The analytic Vorbis window half the carrier's frozen tables store
    /// (Python `vorbis_window`'s unfrozen branch): the same construction,
    /// differing from the stored words only in the two documented patched
    /// regions — the 256 endpoint and the 2048 head — where a few f32 ULPs at
    /// the smallest window entries cannot move a reconstruction that is
    /// already at the float32 noise floor. Using it keeps this test inside
    /// `wem-analysis`, which imports no profile resources.
    fn analytic_window_half(n: i64) -> Vec<f32> {
        (0..n / 2)
            .map(|i| {
                let inner = (std::f64::consts::PI * (i as f64 + 0.5) / n as f64).sin();
                f32_of((std::f64::consts::PI / 2.0 * (inner * inner)).sin()) as f32
            })
            .collect()
    }

    fn frozen_window_halves(blocksizes: &[i64]) -> std::collections::HashMap<i64, Vec<f32>> {
        blocksizes
            .iter()
            .map(|size| (*size, analytic_window_half(*size)))
            .collect()
    }

    /// A deterministic full-band source on the scheduler timeline: a fixed
    /// xorshift state, so every run compares a live round trip rather than a
    /// recorded waveform.
    fn source_stream(len: usize) -> Vec<f64> {
        let mut state = 0x2545_f491_4f6c_dd1du64;
        (0..len)
            .map(|_| {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                f32_of((state >> 40) as f64 / 8_388_608.0 - 1.0)
            })
            .collect()
    }

    /// One round trip's measurements, taken where the lapped geometry carries a
    /// sample at total weight one — the region where the transform must be the
    /// identity.
    struct RoundTrip {
        blocks: usize,
        /// PCM samples compared, i.e. carried at weight one.
        compared: usize,
        /// PCM samples the geometry does not carry at weight one: the
        /// incomplete first and last window halves, and nothing else.
        skipped: usize,
        max_abs: f64,
        peak: f64,
        rms: f64,
        /// `argmin_c |x - c·x'|`, the scale a normalization mismatch would show
        /// up in. The transform claims unit scale, so this must be 1.
        fitted_scale: f64,
    }

    impl RoundTrip {
        fn relative(&self) -> f64 {
            self.max_abs / self.peak
        }

        fn rms_relative(&self) -> f64 {
            self.rms / self.peak
        }
    }

    /// Window, forward-transform, synthesize and overlap-add one mode
    /// sequence — the whole chain with no measurement in it, so the same
    /// function can be run on a probe source.
    fn round_trip_chain(
        blocksizes: &[i64],
        looks: &[MdctLook],
        frozen: &std::collections::HashMap<i64, Vec<f32>>,
        plans: &[wem_scheduling::FramePlan],
        modes: &[i64],
        terminal_following: i64,
        source: &[f64],
    ) -> Vec<f64> {
        // The forward half is the existing analysis path: gather each plan's
        // block, apply the encoder's hybrid window, transform. The short-block
        // window rule is the materializer's, applied here as it does there.
        let mut spectra: Vec<Vec<f64>> = Vec::with_capacity(plans.len());
        for plan in plans {
            let n = blocksizes[plan.current as usize] as usize;
            let start = plan.sample_start as usize;
            let modes = if plan.current == 0 {
                (0, 0, 0)
            } else {
                (plan.previous, plan.current, plan.following)
            };
            let mut row = source[start..start + n].to_vec();
            apply_vorbis_window_in_place(
                &mut row,
                blocksizes,
                modes.0,
                modes.1,
                modes.2,
                Some(frozen),
            )
            .expect("analysis window");
            spectra.push(mdct_forward(&looks[plan.current as usize], &row).expect("forward MDCT"));
        }
        synthesize_sequence(
            [&looks[0], &looks[1]],
            frozen,
            blocksizes,
            modes,
            terminal_following,
            &spectra,
        )
        .expect("synthesis")
    }

    /// Measure one round trip: reconstruct a deterministic source, and read
    /// off where the chain carries a sample at unit weight by running the same
    /// chain on an all-ones source. Nothing here is a recorded expectation —
    /// the reference is the definition of the chain, computed live.
    fn round_trip(blocksizes: &[i64], modes: &[i64], terminal_following: i64) -> RoundTrip {
        let looks: Vec<MdctLook> = blocksizes.iter().map(|n| synthetic_look(*n)).collect();
        let frozen = frozen_window_halves(blocksizes);
        let origin = (blocksizes[1] / 2) as usize;
        let plans = wem_scheduling::plan_mode_sequence(modes, blocksizes, terminal_following)
            .expect("the scheduler plans the mode sequence");
        let source_len = plans.last().expect("nonempty plans").sample_end as usize + 64;
        let source = source_stream(source_len);
        let chain = |input: &[f64]| {
            round_trip_chain(
                blocksizes,
                &looks,
                &frozen,
                &plans,
                modes,
                terminal_following,
                input,
            )
        };
        let pcm = chain(&source);
        let weight = chain(&vec![1.0f64; source_len]);

        let mut measured = RoundTrip {
            blocks: plans.len(),
            compared: 0,
            skipped: 0,
            max_abs: 0.0,
            peak: 0.0,
            rms: 0.0,
            fitted_scale: 0.0,
        };
        let mut sum_error_sq = 0.0f64;
        let mut sum_xy = 0.0f64;
        let mut sum_xx = 0.0f64;
        for (i, value) in pcm.iter().enumerate() {
            let reference = source[origin + i];
            // Only samples the geometry carries at weight one: a window tail
            // outside it carries `w² ≠ 1` and would report its own deficit,
            // which is the geometry's lead-in, not the transform's error.
            if (weight[i] - 1.0).abs() > 1.0e-7 {
                measured.skipped += 1;
                continue;
            }
            measured.compared += 1;
            let error = value - reference;
            measured.max_abs = measured.max_abs.max(error.abs());
            measured.peak = measured.peak.max(reference.abs());
            sum_error_sq += error * error;
            sum_xy += reference * value;
            sum_xx += value * value;
        }
        measured.rms = (sum_error_sq / measured.compared.max(1) as f64).sqrt();
        measured.fitted_scale = sum_xy / sum_xx;
        measured
    }

    /// The claim the decode contract rests on: the forward and the inverse,
    /// windowed and overlapped, compose to the identity on the covered
    /// interior, at the float32 noise floor and with no scale correction.
    #[test]
    fn inverse_mdct_round_trip_closes() {
        let blocksizes = [256, 2048];
        let cases: [(&str, Vec<i64>); 3] = [
            ("long blocks", vec![1; 9]),
            ("short blocks", vec![0; 40]),
            ("transitions", vec![1, 1, 1, 1, 0, 0, 0, 0, 1, 1, 1, 1]),
        ];
        for (name, modes) in cases {
            assert_round_trip(name, &round_trip(&blocksizes, &modes, 1));
        }
    }

    /// The same claim at a second geometry, which is the only way the smaller
    /// butterfly stage (`log2n == 6`) and its short-window pair are exercised.
    #[test]
    fn inverse_mdct_round_trip_closes_at_smaller_geometry() {
        let blocksizes = [64, 512];
        for modes in [vec![1; 5], vec![0; 40], vec![1, 1, 0, 0, 0, 0, 1, 1]] {
            assert_round_trip(
                &format!("n=64/512 modes {modes:?}"),
                &round_trip(&blocksizes, &modes, 1),
            );
        }
    }

    /// State one round trip's claim, and refuse to state it vacuously: the
    /// unit-weight mask must carry a fifth of the stream before the comparison
    /// counts as evidence.
    ///
    /// The mask is deliberately strict — a sample counts only when the chain
    /// carries it at weight one to within a f32 ULP — so that the comparison
    /// measures the transform rather than the frozen window's own rounding
    /// (`w² + w'²` reaches 1 only to 7–8e-08) or the incomplete lead-in and
    /// tail window halves. It excludes a large minority for exactly that
    /// reason.
    fn assert_round_trip(name: &str, measured: &RoundTrip) {
        let carried = measured.compared + measured.skipped;
        println!(
            "{name}: {} blocks, {} of {carried} samples carried at unit weight, max|e| {:.3e}, \
             peak {:.3e}, relative {:.3e}, rms {:.3e}, fitted scale {:.9}",
            measured.blocks,
            measured.compared,
            measured.max_abs,
            measured.peak,
            measured.relative(),
            measured.rms_relative(),
            measured.fitted_scale,
        );
        assert!(
            measured.compared * 5 >= carried,
            "{name}: only {} of {carried} samples are carried at unit weight — the comparison \
             would be vacuous",
            measured.compared
        );
        assert!(
            measured.relative() < 1.0e-6,
            "{name}: relative reconstruction error {:.3e} exceeds the float32 noise floor",
            measured.relative()
        );
        assert!(
            measured.rms_relative() < 1.0e-6,
            "{name}: relative rms reconstruction error {:.3e} exceeds the float32 noise floor",
            measured.rms_relative()
        );
        assert!(
            (measured.fitted_scale - 1.0).abs() < 1.0e-6,
            "{name}: fitted scale {:.9} is not unity — the pair needs a normalization correction",
            measured.fitted_scale
        );
    }

    /// The inverse MDCT from its mathematical definition, `x[i] = Σ_k
    /// X[k]·cos(π/M·(i + ½ + M/2)·(k + ½))` with `M = n/2`, evaluated live in
    /// f64. This is the transform the frozen bank's fast routine must compute;
    /// what it evaluates is the definition, so a run compares against the
    /// definition and never against a recorded number.
    fn imdct_definition(n: i64, spectrum: &[f64]) -> Vec<f64> {
        let m = n / 2;
        (0..n)
            .map(|i| {
                (0..m)
                    .map(|k| {
                        spectrum[k as usize]
                            * (std::f64::consts::PI / m as f64
                                * (i as f64 + 0.5 + m as f64 / 2.0)
                                * (k as f64 + 0.5))
                                .cos()
                    })
                    .sum()
            })
            .collect()
    }

    /// The fast inverse against the definition, for both block sizes: an
    /// angle that isolates the reverse routine, with no forward and no
    /// overlap-add in it.
    #[test]
    fn mdct_backward_matches_the_transform_definition() {
        for n in [256i64, 2048] {
            let look = synthetic_look(n);
            let spectrum = source_stream(n as usize / 2);
            let fast = mdct_backward(&look, &spectrum).expect("fast inverse");
            let reference = imdct_definition(n, &spectrum);
            assert_eq!(fast.len(), reference.len());
            let peak = reference.iter().fold(0.0f64, |acc, v| acc.max(v.abs()));
            let max_abs = fast
                .iter()
                .zip(reference.iter())
                .fold(0.0f64, |acc, (a, b)| acc.max((a - b).abs()));
            let relative = max_abs / peak;
            println!("n={n}: fast vs definition max|e| {max_abs:.3e}, peak {peak:.3e}, relative {relative:.3e}");
            assert!(
                relative < 1.0e-5,
                "n={n}: the fast inverse differs from the definition by {relative:.3e} relative"
            );
        }
    }

    /// Every position and advance the overlap-add state uses is the
    /// scheduler's own, compared here against the plans it produces rather
    /// than against a recorded number.
    #[test]
    fn positions_match_the_scheduler() {
        let blocksizes = [256, 2048];
        let frozen = frozen_window_halves(&blocksizes);
        let looks: Vec<MdctLook> = blocksizes.iter().map(|n| synthetic_look(*n)).collect();
        let sequences: [Vec<i64>; 6] = [
            vec![1; 6],
            vec![0; 20],
            vec![1, 1, 0, 0, 0, 0, 1, 1],
            vec![0, 0, 1],
            vec![1, 0, 1],
            vec![0, 1],
        ];
        for modes in sequences {
            let plans = wem_scheduling::plan_mode_sequence(&modes, &blocksizes, 1).expect("plans");
            let mut ola = SynthesisOla::new(&blocksizes).expect("synthesis state");
            for plan in &plans {
                assert_eq!(
                    ola.next_position(plan.current).expect("position"),
                    plan.sample_start,
                    "modes {modes:?} block {} landed on the wrong position",
                    plan.index
                );
                let spectrum = vec![0.0f64; (blocksizes[plan.current as usize] / 2) as usize];
                ola.push(
                    &looks[plan.current as usize],
                    &frozen,
                    plan.current,
                    plan.following,
                    &spectrum,
                )
                .expect("push");
            }
        }
    }

    /// The synthesis surface rejects what a caller can get wrong, with the
    /// values it observed rather than a panic.
    #[test]
    fn synthesis_rejects_bad_input() {
        let blocksizes = [256, 2048];
        let frozen = frozen_window_halves(&blocksizes);
        let short_look = synthetic_look(256);
        let short_spectrum = vec![0.0f64; 128];
        let long_spectrum = vec![0.0f64; 1024];
        let truncated = vec![0.0f64; 64];

        let too_short = vec![0.0f64; 127];
        assert!(matches!(
            mdct_backward(&short_look, &too_short),
            Err(AnalysisError::SamplesShort {
                need: 128,
                got: 127
            })
        ));
        assert!(matches!(
            SynthesisOla::new(&[256]),
            Err(AnalysisError::WindowBlockSizeInvalid)
        ));
        assert!(matches!(
            SynthesisOla::new(&[2048, 256]),
            Err(AnalysisError::WindowIntervalsIncompatible)
        ));

        let mut ola = SynthesisOla::new(&blocksizes).expect("synthesis state");
        assert!(matches!(
            ola.next_position(2),
            Err(AnalysisError::WindowStateIndexOutOfRange { index: 2 })
        ));
        assert!(matches!(
            ola.push(&short_look, &frozen, 3, 1, &short_spectrum),
            Err(AnalysisError::WindowStateIndexOutOfRange { index: 3 })
        ));
        assert!(matches!(
            ola.push(&short_look, &frozen, 1, 9, &long_spectrum),
            Err(AnalysisError::WindowStateIndexOutOfRange { index: 9 })
        ));
        assert!(matches!(
            ola.push(&short_look, &frozen, 1, 1, &long_spectrum),
            Err(AnalysisError::LongMdctLookGeometry { want: 2048 })
        ));
        assert!(matches!(
            ola.push(&short_look, &frozen, 0, 1, &truncated),
            Err(AnalysisError::SamplesShort { need: 128, got: 64 })
        ));

        let short = synthetic_look(256);
        let long = synthetic_look(2048);
        let one_row = vec![long_spectrum.clone()];
        assert!(matches!(
            synthesize_sequence([&short, &long], &frozen, &blocksizes, &[1, 1], 1, &one_row),
            Err(AnalysisError::SamplesShort { need: 2, got: 1 })
        ));
        // The first block's left window is the short one (the planner's
        // initial `previous` is short), so that is the half it needs first.
        let no_windows = std::collections::HashMap::new();
        let rows = vec![long_spectrum.clone()];
        assert!(matches!(
            synthesize_sequence([&short, &long], &no_windows, &blocksizes, &[1], 1, &rows),
            Err(AnalysisError::FrozenWindowDomainMiss { size: 256 })
        ));
        let empty: Vec<Vec<f64>> = Vec::new();
        assert!(
            synthesize_sequence([&short, &long], &frozen, &blocksizes, &[], 1, &empty)
                .expect("an empty sequence synthesizes nothing")
                .is_empty()
        );
    }
}
