//! Integration tests for the generated `quillshade::common` runtime module.

use bevy::color::{Color, ColorToComponents, LinearRgba};
use quillshade::common::{
    CurveData, GradientData, GradientStop, Interp,
    gradient::{DEFAULT_MID, RawGradient, RawStop, linear_to_oklab, oklab_to_linear},
};

#[test]
fn curve_data_sampling_and_ranges() {
    // Linear ramp with Simple interpolation: midpoint = 0.5.
    let ramp_simple = CurveData::new(Interp::Simple, 0.0, 1.0, vec![(0.0, 0.0), (1.0, 1.0)]);
    assert!((ramp_simple.sample(0.5) - 0.5).abs() < 1e-6);

    // Catmull-Rom holds endpoint values outside [first.t, last.t].
    let cr = CurveData::new(Interp::CatmullRom, 0.0, 1.0, vec![(0.2, 0.3), (0.5, 0.8), (0.8, 0.6)]);
    assert_eq!(cr.sample(-1.0), 0.3);
    assert_eq!(cr.sample(0.0), 0.3);
    assert_eq!(cr.sample(0.19), 0.3);
    assert_eq!(cr.sample(0.81), 0.6);
    assert_eq!(cr.sample(1.0), 0.6);
    assert_eq!(cr.sample(2.0), 0.6);

    // 1-point curve is constant everywhere.
    for interp in Interp::ALL {
        let one = CurveData::new(interp, 0.0, 1.0, vec![(0.4, 0.75)]);
        for t in [-1.0, 0.0, 0.4, 1.0, 2.0] {
            assert_eq!(one.sample(t), 0.75);
        }
    }

    // Ranged curve maps normalized [0, 1] into [min, max] real units.
    let ranged = CurveData::new(Interp::Simple, -10.0, 30.0, vec![(0.0, 0.25), (1.0, 0.75)]);
    assert!((ranged.sample(0.0) - 0.0).abs() < 1e-5);
    assert!((ranged.sample(0.5) - 10.0).abs() < 1e-5);
    assert!((ranged.sample(1.0) - 20.0).abs() < 1e-5);
}

#[test]
fn gradient_data_sampling_roundtrip_and_default() {
    let red = Color::LinearRgba(LinearRgba::new(1.0, 0.0, 0.0, 1.0));
    let blue_transparent = Color::LinearRgba(LinearRgba::new(0.0, 0.0, 1.0, 0.0));

    let g = GradientData {
        stops: vec![
            GradientStop { t: 0.0, color: red, mid: None },
            GradientStop { t: 1.0, color: blue_transparent, mid: Some(DEFAULT_MID) },
        ],
    };

    // Out-of-bounds clamps to endpoint stops.
    assert_eq!(g.sample(-1.0), red);
    assert_eq!(g.sample(2.0), blue_transparent);

    // Midpoint (t = 0.5): OKLab RGB mix is distinct from linear RGB mix [0.5, 0.0, 0.5], and alpha lerps linearly.
    let mid = g.sample(0.5).to_linear().to_f32_array();
    assert!((mid[3] - 0.5).abs() < 1e-6, "alpha should lerp linearly to 0.5, got {}", mid[3]);
    let linear_dist = (mid[0] - 0.5).abs() + mid[1].abs() + (mid[2] - 0.5).abs();
    assert!(linear_dist > 0.05, "OKLab mix at t=0.5 should differ from linear RGB mix: {mid:?}");

    let (la, lb) = (linear_to_oklab([1.0, 0.0, 0.0]), linear_to_oklab([0.0, 0.0, 1.0]));
    let expected_rgb = oklab_to_linear([0, 1, 2].map(|i| 0.5 * (la[i] + lb[i]))).map(|c| c.clamp(0.0, 1.0));
    for i in 0..3 {
        assert!((mid[i] - expected_rgb[i]).abs() < 1e-5, "channel {i}: {} vs {}", mid[i], expected_rgb[i]);
    }

    // Equal `t` forms a step where the later stop wins at `t`.
    let blue = Color::LinearRgba(LinearRgba::new(0.0, 0.0, 1.0, 1.0));
    let step = GradientData {
        stops: vec![
            GradientStop { t: 0.0, color: red, mid: None },
            GradientStop { t: 0.5, color: red, mid: None },
            GradientStop { t: 0.5, color: blue, mid: None },
            GradientStop { t: 1.0, color: blue, mid: None },
        ],
    };
    assert_eq!(step.sample(0.49999), red);
    assert_eq!(step.sample(0.5), blue);

    // Color round trip through RawGradient and GradientData.
    let raw = RawGradient {
        stops: vec![
            RawStop { t: 0.0, color: [1.0, 0.25, 0.5, 1.0], mid: 0.3 },
            RawStop { t: 1.0, color: [0.1, 0.8, 0.4, 0.5], mid: DEFAULT_MID },
        ],
    };
    let wrapped = GradientData::from(raw.clone());
    assert_eq!(RawGradient::from(&wrapped), raw);
    assert_eq!(GradientData::from(RawGradient::from(wrapped.clone())), wrapped);

    // Empty default gradient evaluates to opaque white.
    assert_eq!(GradientData::default().sample(0.3), Color::WHITE);
}
