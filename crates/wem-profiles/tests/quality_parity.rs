//! Quality mechanism: shared parity pins.
//!
//! The expected doubles below are the shared test vectors: the Python
//! reference (reference/wwise_wem_reference/profiles/quality.py,
//! tests/unit/profiles/test_quality_curves.py) and this module must return
//! identical values bit for bit (same f64 arithmetic, same two-step
//! fractional-index form, same normalization entry).
//!
//! The end-to-end wiring — repackaging the installed 6ch profile bytes with a
//! synthetic quality-curves resource and proving quality=None keeps the
//! historical surface, a quality value applies the interpolated overrides
//! (through the f32 boundary), and a quality request from a profile without
//! curves is a configuration error — drives the crate-private in-memory
//! loader, so it lives in `src/bytes_loader_tests.rs`.

#![allow(clippy::excessive_precision)]

use wem_profiles::{linear_frac, normalize_quality_factor};

// ---------------------------------------------------------------------------
// Shared parity pins (identical vectors on the Python side)
// ---------------------------------------------------------------------------

const TWO_CHANNEL_BP: [f64; 13] = [
    -0.2, -0.1, 0.0, 0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8, 0.9, 1.0,
];
const TWO_CHANNEL_DESC3: [f64; 13] = [
    1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0,
];
const TWO_CHANNEL_DESC29: [f64; 13] = [
    -100.0, -100.0, -100.0, -100.0, -100.0, -100.0, -100.0, -105.0, -105.0, -105.0, -105.0, -110.0,
    -120.0,
];
const TWO_CHANNEL_DESC30: [f64; 13] = [
    -130.0, -130.0, -130.0, -130.0, -130.0, -135.0, -140.0, -140.0, -140.0, -140.0, -140.0, -140.0,
    -150.0,
];
const TWO_CHANNEL_DESC31: [f64; 13] = [
    12.9, 13.8, 14.7, 15.6, 16.5, 17.1, 18.0, 19.5, 48.0, 999.0, 999.0, 999.0, 999.0,
];

#[test]
fn normalization_pins_match_the_python_reference() {
    assert_eq!(normalize_quality_factor(0.0), 1e-7);
    assert_eq!(normalize_quality_factor(4.0), 0.4000001);
    assert_eq!(normalize_quality_factor(9.0), 0.9000001);
    assert_eq!(normalize_quality_factor(10.0), 0.9998999834060669);
    assert_eq!(normalize_quality_factor(100.0), 0.9998999834060669);
}

#[test]
fn kernel_pins_match_the_python_reference() {
    // (bp=[0.5,0.9], samples=[0,1]).
    let bp = [0.5, 0.9];
    let s = [0.0, 1.0];
    assert_eq!(linear_frac(&bp, &s, 0.1), (0.0, true));
    assert_eq!(linear_frac(&bp, &s, 0.7), (0.4999999999999999, false));
    // At the last breakpoint the N - 0.001 clamp formula applies (not the
    // stored value).
    assert_eq!(linear_frac(&bp, &s, 0.9), (0.999, false));
    assert_eq!(linear_frac(&bp, &s, 1.0), (0.999, true));
    assert_eq!(linear_frac(&bp, &s, 2.0), (0.999, true));

    // (bp=[0,4,8], samples=[10,20,30]).
    let bp = [0.0, 4.0, 8.0];
    let s = [10.0, 20.0, 30.0];
    assert_eq!(linear_frac(&bp, &s, 2.0), (15.0, false));
    assert_eq!(linear_frac(&bp, &s, 6.0), (25.0, false));
    assert_eq!(linear_frac(&bp, &s, 9.0), (29.990000000000002, true));
    assert_eq!(linear_frac(&bp, &s, 8.0), (29.990000000000002, false));
    assert_eq!(linear_frac(&bp, &s, 4.0), (20.0, false));
    assert_eq!(linear_frac(&bp, &s, 0.0), (10.0, false));

    // Below-domain clamp.
    let bp = [0.5, 0.9, 0.95];
    let s = [10.0, 20.0, 30.0];
    assert_eq!(linear_frac(&bp, &s, 0.0100001), (10.0, true));
}

#[test]
fn two_channel_13bp_curves_match_the_python_reference() {
    // q = 4.0 -> qnorm = 0.4000001 -> the two-step fraction at i = 4.
    {
        let qnorm = normalize_quality_factor(4.0);
        assert_eq!(qnorm, 0.4000001);
        let (d3, e3) = linear_frac(&TWO_CHANNEL_BP, &TWO_CHANNEL_DESC3, qnorm);
        let (d29, e29) = linear_frac(&TWO_CHANNEL_BP, &TWO_CHANNEL_DESC29, qnorm);
        let (d30, e30) = linear_frac(&TWO_CHANNEL_BP, &TWO_CHANNEL_DESC30, qnorm);
        let (d31, e31) = linear_frac(&TWO_CHANNEL_BP, &TWO_CHANNEL_DESC31, qnorm);
        assert!(!e3 && !e29 && !e30 && !e31);
        assert_eq!(d3, 1.0);
        assert_eq!(d29, -100.000005);
        assert_eq!(d30, -140.0);
        assert_eq!(d31, 18.0000015);
    }
    // q = 10.0 -> clamped qnorm, in-domain segment (11, 12).
    {
        let qnorm = normalize_quality_factor(10.0);
        assert_eq!(qnorm, 0.9998999834060669);
        let (d29, e29) = linear_frac(&TWO_CHANNEL_BP, &TWO_CHANNEL_DESC29, qnorm);
        let (d30, e30) = linear_frac(&TWO_CHANNEL_BP, &TWO_CHANNEL_DESC30, qnorm);
        let (d31, e31) = linear_frac(&TWO_CHANNEL_BP, &TWO_CHANNEL_DESC31, qnorm);
        assert!(!e29 && !e30 && !e31);
        assert_eq!(d29, -119.98999834060669);
        assert_eq!(d30, -149.9899983406067);
        assert_eq!(d31, 999.0);
    }
    // q = -3.0 -> below-domain clamp, extrapolated.
    {
        let qnorm = normalize_quality_factor(-3.0);
        let (d31, e31) = linear_frac(&TWO_CHANNEL_BP, &TWO_CHANNEL_DESC31, qnorm);
        assert!(e31);
        assert_eq!(d31, 12.9);
    }
}
