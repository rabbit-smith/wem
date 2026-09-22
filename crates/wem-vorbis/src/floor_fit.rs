//! Encoder-side floor1 curve fitting and post quantization
//! (Python: `wwise_wem/vorbis/floor_fit.py`).
//!
//! The live fit path (`floor1_fit_wwise_carriers`) is transcendental-free: curves are
//! already on the Wwise dB scale, and bin quantization uses the profile's
//! dB→quant rule (Python `_wwise_db_quant`).

use crate::floor::{floor1_neighbor_tables, postlist_from_floor, render_point};
use crate::setup::Floor1Setup;

/// Floor fit errors (Python: `ValueError` family).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FloorFitError {
    /// Input curves shorter than the floor range.
    CurvesTooShort,
}

impl std::fmt::Display for FloorFitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FloorFitError::CurvesTooShort => {
                write!(f, "floor1 log curves are shorter than the floor range")
            }
        }
    }
}

impl std::error::Error for FloorFitError {}

/// Encoder-side fit profile parameters.
///
/// Not serialized in the setup packet: the matching libvorbis floor
/// template defines these values for the installed profiles.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FloorFitParams {
    pub maxover: f64,
    pub maxunder: f64,
    pub maxerr: f64,
    pub twofitweight: f64,
    pub twofitatten: f64,
}

/// 128×11 floor profile (libvorbis `60, 30, 500, 1, 18` plus weight 1).
pub const FLOOR1_WWISE_FIT_PARAMS: FloorFitParams = FloorFitParams {
    maxover: 60.0,
    maxunder: 30.0,
    maxerr: 500.0,
    twofitweight: 1.0,
    twofitatten: 18.0,
};

/// 1024×27 floor profile: same error limits, A-bucket weight 3
/// (Python `FLOOR1_WWISE_FIT_PARAMS_LONG`).
pub const FLOOR1_WWISE_FIT_PARAMS_LONG: FloorFitParams = FloorFitParams {
    maxover: 60.0,
    maxunder: 30.0,
    maxerr: 500.0,
    twofitweight: 3.0,
    twofitatten: 18.0,
};

/// Python `_DB_QUANT_SCALE`.
const DB_QUANT_SCALE: f64 = 7.314285755157471;

trait CurveSample: Copy {
    fn reference_value(self) -> f64;
}

impl CurveSample for f32 {
    #[inline]
    fn reference_value(self) -> f64 {
        self as f64
    }
}

impl CurveSample for f64 {
    #[inline]
    fn reference_value(self) -> f64 {
        // Analysis stores f32 values in f64 carriers. Keep the normative f32
        // boundary while avoiding a temporary converted row.
        (self as f32) as f64
    }
}

/// Apply Wwise's dB quantization rule: truncate, then clamp (Python
/// `_wwise_db_quant`).
fn wwise_db_quant(db: f64) -> i64 {
    let q = (db * DB_QUANT_SCALE + 1023.5).trunc();
    if q < 0.0 {
        0
    } else if q > 1023.0 {
        1023
    } else {
        q as i64
    }
}

/// C99 integer division toward zero for the Bresenham fit path.
fn c_trunc_div(num: i64, den: i64) -> i64 {
    // Rust signed division truncates toward zero, matching C99.
    num / den
}

/// Integer sums collected for one sorted floor interval
/// (Python `_FloorFitAcc`).
#[derive(Debug, Clone, Copy)]
struct FloorFitAcc {
    x0: i64,
    x1: i64,
    xa: i64,
    ya: i64,
    x2a: i64,
    xya: i64,
    an: i64,
    xb: i64,
    yb: i64,
    x2b: i64,
    xyb: i64,
    bn: i64,
}

/// Accumulate floor-fit statistics over an inclusive interval
/// (Python `_accumulate_fit`).
fn accumulate_fit<Q: CurveSample, F: CurveSample>(
    quantized_curve: &[Q],
    floor_curve: &[F],
    x0: i64,
    x1: i64,
    n: usize,
    params: &FloorFitParams,
) -> FloorFitAcc {
    let mut acc = FloorFitAcc {
        x0,
        x1,
        xa: 0,
        ya: 0,
        x2a: 0,
        xya: 0,
        an: 0,
        xb: 0,
        yb: 0,
        x2b: 0,
        xyb: 0,
        bn: 0,
    };
    let end = x1.min(n as i64 - 1);
    if x0 > end {
        return acc;
    }
    let atten = params.twofitatten;
    for x in x0..=end {
        let quantized = wwise_db_quant(quantized_curve[x as usize].reference_value());
        if quantized == 0 {
            continue;
        }
        // The primary bucket contains fitted-floor values at or below the raw
        // MDCT curve plus attenuation. The complementary bucket is secondary.
        if floor_curve[x as usize].reference_value() + atten
            >= quantized_curve[x as usize].reference_value()
        {
            acc.xa += x;
            acc.ya += quantized;
            acc.x2a += x * x;
            acc.xya += x * quantized;
            acc.an += 1;
        } else {
            acc.xb += x;
            acc.yb += quantized;
            acc.x2b += x * x;
            acc.xyb += x * quantized;
            acc.bn += 1;
        }
    }
    acc
}

/// Fit a line using the exact weighted least-squares accumulator
/// (Python `_fit_line`).
fn fit_line(fits: &[FloorFitAcc], y0: i64, y1: i64, params: &FloorFitParams) -> (i64, i64, bool) {
    if fits.is_empty() {
        return (0, 0, true);
    }
    let mut xb = 0.0f64;
    let mut yb = 0.0f64;
    let mut x2b = 0.0f64;
    let mut xyb = 0.0f64;
    let mut bn = 0.0f64;
    for acc in fits {
        // Weight the primary bucket using its sample-count denominator.
        let weight = ((acc.bn + acc.an) as f64) * params.twofitweight / ((acc.an + 1) as f64) + 1.0;
        xb += acc.xb as f64 + acc.xa as f64 * weight;
        yb += acc.yb as f64 + acc.ya as f64 * weight;
        x2b += acc.x2b as f64 + acc.x2a as f64 * weight;
        xyb += acc.xyb as f64 + acc.xya as f64 * weight;
        bn += acc.bn as f64 + acc.an as f64 * weight;
    }

    let x0 = fits[0].x0;
    let x1 = fits[fits.len() - 1].x1;
    if y0 >= 0 {
        xb += x0 as f64;
        yb += y0 as f64;
        x2b += x0 as f64 * x0 as f64;
        xyb += y0 as f64 * x0 as f64;
        bn += 1.0;
    }
    if y1 >= 0 {
        xb += x1 as f64;
        yb += y1 as f64;
        x2b += x1 as f64 * x1 as f64;
        xyb += y1 as f64 * x1 as f64;
        bn += 1.0;
    }
    let denom = bn * x2b - xb * xb;
    if denom <= 0.0 {
        return (0, 0, true);
    }
    let intercept = (yb * x2b - xyb * xb) / denom;
    let slope = (bn * xyb - xb * yb) / denom;
    // Add 0.5 before flooring. This differs from nearest-even rounding at
    // half-integer ties, including one long-profile endpoint.
    let mut out0 = (intercept + slope * x0 as f64 + 0.5).floor();
    let mut out1 = (intercept + slope * x1 as f64 + 0.5).floor();
    out0 = out0.clamp(0.0, 1023.0);
    out1 = out1.clamp(0.0, 1023.0);
    (out0 as i64, out1 as i64, false)
}

/// Return whether the fitted line exceeds the local error bounds
/// (Python `_inspect_error`).
fn inspect_error<Q: CurveSample, F: CurveSample>(
    x0: i64,
    x1: i64,
    y0: i64,
    y1: i64,
    quantized_curve: &[Q],
    floor_curve: &[F],
    params: &FloorFitParams,
) -> bool {
    let adx = x1 - x0;
    if adx <= 0 {
        return true;
    }
    let dy = y1 - y0;
    let base = c_trunc_div(dy, adx);
    let sy = if dy < 0 { base - 1 } else { base + 1 };
    let ady = dy.abs() - (base * adx).abs();
    let mut x = x0;
    let mut y = y0;
    let mut err = 0i64;
    let val = wwise_db_quant(quantized_curve[x as usize].reference_value());
    let d0 = y - val;
    let mut mse = d0 * d0;
    let mut count = 1i64;
    if quantized_curve[x as usize].reference_value()
        <= floor_curve[x as usize].reference_value() + params.twofitatten
        && ((y as f64) + params.maxover < val as f64 || (y as f64) - params.maxunder > val as f64)
    {
        return true;
    }

    while x + 1 < x1 {
        x += 1;
        err += ady;
        if err >= adx {
            err -= adx;
            y += sy;
        } else {
            y += base;
        }
        let val = wwise_db_quant(quantized_curve[x as usize].reference_value());
        let d = y - val;
        mse += d * d;
        count += 1;
        if quantized_curve[x as usize].reference_value()
            <= floor_curve[x as usize].reference_value() + params.twofitatten
            && val != 0
            && ((y as f64) + params.maxover < val as f64
                || (y as f64) - params.maxunder > val as f64)
        {
            return true;
        }
    }

    if params.maxover * params.maxover / count as f64 > params.maxerr {
        return false;
    }
    if params.maxunder * params.maxunder / count as f64 > params.maxerr {
        return false;
    }
    // Use integer division for accumulated MSE before comparing with maxerr.
    // With maxerr=0 this intentionally
    // tolerates a total error smaller than the number of checked bins.
    (mse / count) as f64 > params.maxerr
}

fn post_y(values_a: &[i64], values_b: &[i64], pos: usize) -> i64 {
    if values_a[pos] < 0 {
        return values_b[pos];
    }
    if values_b[pos] < 0 {
        return values_a[pos];
    }
    (values_a[pos] + values_b[pos]) >> 1
}

/// Update high-neighbor links after inserting a split post
/// (Python `_propagate_high_neighbor`).
fn propagate_high_neighbor(local_hi: &mut [usize], sortpos: usize, high: usize, post_index: usize) {
    let mut cursor: i64 = sortpos as i64 - 1;
    while cursor >= 0 && local_hi[cursor as usize] == high {
        local_hi[cursor as usize] = post_index;
        cursor -= 1;
    }
}

/// Fit analysis curves stored as f32-valued f64 carriers without allocating
/// converted rows. Every sample is still rounded through f32 at the boundary.
pub fn floor1_fit_wwise_carriers(
    fitted_floor_curve: &[f64],
    raw_mdct_curve: &[f64],
    floor: &Floor1Setup,
    n: Option<usize>,
) -> Result<Option<Vec<i64>>, FloorFitError> {
    floor1_fit_wwise_impl(fitted_floor_curve, raw_mdct_curve, floor, n)
}

fn floor1_fit_wwise_impl<Q: CurveSample, F: CurveSample>(
    fitted_floor_curve: &[Q],
    raw_mdct_curve: &[F],
    floor: &Floor1Setup,
    n: Option<usize>,
) -> Result<Option<Vec<i64>>, FloorFitError> {
    let postlist = postlist_from_floor(floor);
    let posts = postlist.len();
    if posts < 2 {
        return Ok(None);
    }
    let spectrum_n = n.unwrap_or(1usize << floor.rangebits);
    let required = spectrum_n.min(postlist[1] as usize);
    if spectrum_n == 0 || raw_mdct_curve.len() < required || fitted_floor_curve.len() < required {
        return Err(FloorFitError::CurvesTooShort);
    }
    let params = if (spectrum_n, posts) == (1024, 29) {
        FLOOR1_WWISE_FIT_PARAMS_LONG
    } else {
        FLOOR1_WWISE_FIT_PARAMS
    };

    let mut order: Vec<usize> = (0..posts).collect();
    order.sort_by_key(|&index| postlist[index]);
    let sorted_x: Vec<i64> = order.iter().map(|&index| postlist[index]).collect();
    let mut reverse_index = vec![0usize; posts];
    for (sorted_pos, &index) in order.iter().enumerate() {
        reverse_index[index] = sorted_pos;
    }

    let fits: Vec<FloorFitAcc> = (0..posts - 1)
        .map(|i| {
            accumulate_fit(
                fitted_floor_curve,
                raw_mdct_curve,
                sorted_x[i],
                sorted_x[i + 1],
                spectrum_n,
                &params,
            )
        })
        .collect();
    // A non-empty primary bucket is the floor's nonzero-signal test.
    if fits.iter().map(|acc| acc.an).sum::<i64>() == 0 {
        return Ok(None);
    }

    let mut fit_a = vec![-200i64; posts];
    let mut fit_b = vec![-200i64; posts];
    let mut local_lo = vec![0usize; posts];
    let mut local_hi = vec![1usize; posts];
    let mut memo = vec![-1i64; posts];

    let (y0, y1, _) = fit_line(&fits, -200, -200, &params);
    fit_a[0] = y0;
    fit_b[0] = y0;
    fit_a[1] = y1;
    fit_b[1] = y1;

    for post_index in 2..posts {
        let sortpos = reverse_index[post_index];
        let ln = local_lo[sortpos];
        let hn = local_hi[sortpos];
        if memo[ln] == hn as i64 {
            continue;
        }
        let lsortpos = reverse_index[ln];
        let hsortpos = reverse_index[hn];
        memo[ln] = hn as i64;
        let lx = postlist[ln];
        let hx = postlist[hn];
        let ly = post_y(&fit_a, &fit_b, ln);
        let hy = post_y(&fit_a, &fit_b, hn);
        if inspect_error(lx, hx, ly, hy, fitted_floor_curve, raw_mdct_curve, &params) {
            let (mut ly0, mut ly1, ret0) = fit_line(&fits[lsortpos..sortpos], -200, -200, &params);
            let (mut hy0, mut hy1, ret1) = fit_line(&fits[sortpos..hsortpos], -200, -200, &params);
            if ret0 {
                ly0 = ly;
                ly1 = hy0;
            }
            if ret1 {
                hy0 = ly1;
                hy1 = hy;
            }
            if ret0 && ret1 {
                fit_a[post_index] = -200;
                fit_b[post_index] = -200;
            } else {
                fit_b[ln] = ly0;
                if ln == 0 {
                    fit_a[ln] = ly0;
                }
                fit_a[post_index] = ly1;
                fit_b[post_index] = hy0;
                fit_a[hn] = hy1;
                if hn == 1 {
                    fit_b[hn] = hy1;
                }
                if ly1 >= 0 || hy0 >= 0 {
                    propagate_high_neighbor(&mut local_hi, sortpos, hn, post_index);
                    let mut j = sortpos + 1;
                    while j < posts && local_lo[j] == ln {
                        local_lo[j] = post_index;
                        j += 1;
                    }
                }
            }
        } else {
            fit_a[post_index] = -200;
            fit_b[post_index] = -200;
        }
    }

    let mut output = vec![0i64; posts];
    output[0] = post_y(&fit_a, &fit_b, 0);
    output[1] = post_y(&fit_a, &fit_b, 1);
    let (loneighbor, hineighbor) = floor1_neighbor_tables(&postlist);
    for post_index in 2..posts {
        let ln = loneighbor[post_index - 2] as usize;
        let hn = hineighbor[post_index - 2] as usize;
        let predicted = render_point(
            postlist[ln],
            postlist[hn],
            output[ln],
            output[hn],
            postlist[post_index],
        );
        let vx = post_y(&fit_a, &fit_b, post_index);
        output[post_index] = if vx >= 0 && predicted != vx {
            vx
        } else {
            predicted | 0x8000
        };
    }
    Ok(Some(output))
}

/// Apply the 1024→multiplier quantization done by floor1_encode
/// (Python `floor1_quantize_posts`).
pub fn floor1_quantize_posts(
    fit_posts: &[i64],
    multiplier: u64,
) -> Result<Vec<i64>, crate::floor::Floor1Error> {
    let mut out = Vec::with_capacity(fit_posts.len());
    for &post in fit_posts {
        let mut value = post & 0x7FFF;
        match multiplier {
            1 => value >>= 2,
            2 => value >>= 3,
            3 => value /= 12,
            4 => value >>= 4,
            _ => return Err(crate::floor::Floor1Error::InvalidMultiplier { multiplier }),
        }
        out.push(value | (post & 0x8000));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wwise_db_quant_matches_python() {
        // Oracle from Python _wwise_db_quant.
        assert_eq!(wwise_db_quant(0.0), 1023);
        assert_eq!(wwise_db_quant(-140.0), 0);
        assert_eq!(wwise_db_quant(-70.0), 511);
        assert_eq!(wwise_db_quant(10.0), 1023); // clamps above range
        assert_eq!(wwise_db_quant(-200.0), 0); // clamps below range
    }

    #[test]
    fn c_trunc_div_toward_zero() {
        assert_eq!(c_trunc_div(7, 2), 3);
        assert_eq!(c_trunc_div(-7, 2), -3);
        assert_eq!(c_trunc_div(7, -2), -3);
        assert_eq!(c_trunc_div(-7, -2), 3);
    }

    #[test]
    fn quantize_posts_mult2() {
        // 10-bit → mult-2 domain (>>3), preserving the 0x8000 flag.
        let posts = vec![0x7FFF, 0x8000, 120];
        assert_eq!(
            floor1_quantize_posts(&posts, 2).unwrap(),
            vec![0x7FFF >> 3, 0x8000, 120 >> 3]
        );
    }
}
