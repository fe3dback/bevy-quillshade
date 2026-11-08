//! Single-source Bevy-free runtime data and math shared between the editor and generated umbrella crates (ADR 0048,
//! design §4.5).
//!
//! Each submodule includes its source template from `crates/project_core/templates/common/` via `include!`, while
//! `project_core::umbrella` embeds the exact same files with `include_str!` into `crates/quillshade/src/common/`.
//!
//! # Template rules (ADR 0048 §2)
//!
//! Every file under `crates/project_core/templates/common/` must obey:
//! - No inner attributes (`//!` doc comments or `#![...]` attributes), which `rustc` rejects inside `include!`.
//! - No `crate::` paths (`super::` only, so generated modules can be mounted at any path).
//! - No `#[cfg(test)]` blocks (unit tests live in host crates outside `templates/`).
//! - Only `std` and `serde` dependencies in `curve.rs` and `gradient.rs` (`bevy::*` appears only in `gradient_bevy.rs`).
//! - Every item `pub` or used internally in both `project_core` and generated umbrella crates (no dead-code warnings).
//! - Formatted with `rustfmt --edition 2024 --config-path rustfmt.toml crates/project_core/templates/common/*.rs`.

/// 1D transfer curve data (`CurveData`, `Interp`) and normalized `Simple` / `CatmullRom` evaluation.
pub mod curve {
    include!("../templates/common/curve.rs");
}

/// 1D OKLab colour gradient data (`RawGradient`, `RawStop`) and sampling.
pub mod gradient {
    include!("../templates/common/gradient.rs");
}

#[cfg(test)]
mod tests {
    use super::curve::{CurveData, Interp, MIN_SPAN, normalize_points, sample_normalized_with, slopes};
    use super::gradient::{DEFAULT_MID, MID_MAX, MID_MIN, RawGradient, RawStop, linear_to_oklab, oklab_to_linear};

    const CURVE_TEMPLATE: &str = include_str!("../templates/common/curve.rs");
    const GRADIENT_TEMPLATE: &str = include_str!("../templates/common/gradient.rs");
    const GRADIENT_BEVY_TEMPLATE: &str = include_str!("../templates/common/gradient_bevy.rs");

    #[test]
    fn common_templates_obey_adr_0048_rules() {
        for (name, src) in [
            ("curve.rs", CURVE_TEMPLATE),
            ("gradient.rs", GRADIENT_TEMPLATE),
            ("gradient_bevy.rs", GRADIENT_BEVY_TEMPLATE),
        ] {
            for (line_no, line) in src.lines().enumerate() {
                let trimmed = line.trim_start();
                assert!(
                    !trimmed.starts_with("//!"),
                    "{name} line {}: inner doc comment forbidden in template",
                    line_no + 1
                );
                assert!(
                    !trimmed.starts_with("#!["),
                    "{name} line {}: inner attribute forbidden in template",
                    line_no + 1
                );
                assert!(!line.contains("crate::"), "{name} line {}: crate:: path forbidden in template", line_no + 1);
                assert!(
                    !trimmed.starts_with("#[cfg(test)]"),
                    "{name} line {}: #[cfg(test)] forbidden in template",
                    line_no + 1
                );
            }
        }
        const { assert!(MIN_SPAN > 0.0) };
    }

    #[test]
    fn interp_labels_and_defaults() {
        assert_eq!(Interp::default(), Interp::CatmullRom);
        assert_eq!(Interp::ALL, [Interp::CatmullRom, Interp::Simple]);
        for i in Interp::ALL {
            assert_eq!(Interp::from_label(i.label()), Some(i));
        }
        assert_eq!(Interp::from_label("Unknown"), None);
    }

    #[test]
    fn empty_and_single_point_curves() {
        for interp in Interp::ALL {
            let empty = CurveData::new(interp, -3.0, 5.0, vec![]);
            assert_eq!(empty.sample_normalized(-1.0), 0.0);
            assert_eq!(empty.sample_normalized(0.5), 0.0);
            assert_eq!(empty.sample_normalized(f32::NAN), 0.0);
            assert_eq!(empty.sample(0.5), -3.0);

            let single = CurveData::new(interp, 2.0, 10.0, vec![(0.4, 0.25)]);
            assert_eq!(single.sample_normalized(0.0), 0.25);
            assert_eq!(single.sample_normalized(0.4), 0.25);
            assert_eq!(single.sample_normalized(1.0), 0.25);
            assert_eq!(single.sample_normalized(f32::NAN), 0.25);
            assert_eq!(single.sample(0.9), 4.0);
        }
    }

    #[test]
    fn holds_endpoint_values_outside_point_span() {
        for interp in Interp::ALL {
            let c = CurveData::new(interp, 0.0, 1.0, vec![(0.2, 0.3), (0.8, 0.7)]);
            assert_eq!(c.sample(-10.0), 0.3);
            assert_eq!(c.sample(0.0), 0.3);
            assert_eq!(c.sample(0.19), 0.3);
            assert_eq!(c.sample(0.81), 0.7);
            assert_eq!(c.sample(1.0), 0.7);
            assert_eq!(c.sample(10.0), 0.7);
        }
    }

    #[test]
    fn duplicate_t_forms_step_without_division_by_zero() {
        let step_simple =
            CurveData::new(Interp::Simple, 0.0, 1.0, vec![(0.0, 0.0), (0.5, 0.0), (0.5, 1.0), (1.0, 1.0)]);
        assert_eq!(step_simple.sample(0.25), 0.0);
        assert_eq!(step_simple.sample(0.5), 1.0);
        assert_eq!(step_simple.sample(0.75), 1.0);

        let step_cr =
            CurveData::new(Interp::CatmullRom, 0.0, 1.0, vec![(0.0, 0.0), (0.5, 0.0), (0.5, 1.0), (1.0, 1.0)]);
        for t in [0.0, 0.25, 0.5, 0.75, 1.0] {
            assert!(step_cr.sample(t).is_finite());
        }

        // Two coincident knots: zero span -> zero slope, no division by zero.
        let zero_span = CurveData::new(Interp::CatmullRom, 0.0, 1.0, vec![(0.5, 0.2), (0.5, 0.8)]);
        assert_eq!(zero_span.slopes(), vec![0.0, 0.0]);
        assert!(zero_span.sample(0.5).is_finite());
    }

    #[test]
    fn new_normalizes_unsorted_out_of_bounds_and_nan_points() {
        let c = CurveData::new(
            Interp::Simple,
            0.0,
            1.0,
            vec![(1.0, -0.5), (0.25, 2.0), (-1.0, 0.5), (f32::NAN, 0.3), (0.5, f32::INFINITY)],
        );
        assert_eq!(c.points, vec![(0.0, 0.5), (0.25, 1.0), (1.0, 0.0)]);
        let mut pts = vec![(0.8, 0.2), (0.1, 0.9)];
        normalize_points(&mut pts);
        assert_eq!(pts, vec![(0.1, 0.9), (0.8, 0.2)]);
    }

    #[test]
    fn unnormalized_struct_literal_and_non_finite_t_never_panic() {
        for interp in Interp::ALL {
            let raw = CurveData { interp, min: -1.0, max: 3.0, points: vec![(0.9, 0.2), (0.1, 0.8), (0.5, 0.4)] };
            for t in [-1.0, 0.0, 0.3, 0.5, 1.0, 2.0, f32::NEG_INFINITY, f32::INFINITY, f32::NAN] {
                let v = raw.sample(t);
                assert!(v.is_finite(), "non-finite sample {v} for interp={interp:?}, t={t}");
            }
            // Short slopes slice fallback also never panics.
            let v = sample_normalized_with(&raw.points, &[], interp, 0.5);
            assert!(v.is_finite());
        }

        // Ordered curve with NaN / +-inf t: NaN maps to the first point's value, +-inf clamp to endpoints.
        for interp in Interp::ALL {
            let ordered = CurveData::new(interp, 10.0, 20.0, vec![(0.2, 0.3), (0.8, 0.9)]);
            assert_eq!(ordered.sample_normalized(f32::NAN), 0.3);
            assert_eq!(ordered.sample(f32::NAN), 13.0);
            assert_eq!(ordered.sample(f32::NEG_INFINITY), 13.0);
            assert_eq!(ordered.sample(f32::INFINITY), 19.0);
        }
    }

    #[test]
    fn simple_vs_catmull_rom_on_peak_and_ranged_overshoot() {
        let peak_pts = vec![(0.2, 0.0), (0.5, 1.0), (0.8, 0.0)];
        let simple = CurveData::new(Interp::Simple, 0.0, 1.0, peak_pts.clone());
        let cr = CurveData::new(Interp::CatmullRom, 0.0, 1.0, peak_pts);
        assert_eq!(slopes(&cr.points)[1], 0.0, "zero tangent at symmetric peak");
        assert!((simple.sample(0.35) - 0.5).abs() < 1e-6);
        assert!(cr.sample(0.45) > simple.sample(0.45), "Catmull-Rom arches above chord near peak");

        // Ranged output in real units; Catmull-Rom overshoot above 1.0 is preserved (not clamped to min..max).
        let overshoot =
            CurveData::new(Interp::CatmullRom, -2.0, 10.0, vec![(0.0, 0.0), (0.1, 1.0), (0.9, 1.0), (1.0, 0.0)]);
        let mid_norm = overshoot.sample_normalized(0.5);
        assert!(mid_norm > 1.0, "expected Catmull-Rom overshoot > 1.0, got {mid_norm}");
        let mid_real = overshoot.sample(0.5);
        assert!(mid_real > 10.0, "expected unclamped real value > max (10.0), got {mid_real}");
        assert!((mid_real - (-2.0 + 12.0 * mid_norm)).abs() < 1e-5);
    }

    #[test]
    fn default_is_linear_ramp_and_ron_roundtrips_with_light_defaults() {
        let def = CurveData::default();
        assert_eq!(def, CurveData::linear());
        assert_eq!(def.interp, Interp::CatmullRom);
        assert!((def.sample(0.35) - 0.35).abs() < 1e-6);

        let custom = CurveData::new(Interp::Simple, -4.0, 8.0, vec![(0.0, 0.25), (1.0, 0.75)]);
        let text = ron::to_string(&custom).expect("serialize CurveData");
        let back: CurveData = ron::from_str(&text).expect("deserialize CurveData");
        assert_eq!(back, custom);

        // Light's `baked.ron` curve entry format omits `interp` -> defaults to `Interp::CatmullRom`.
        let light_entry = "(min: 0.0, max: 1.0, points: [(0.0, 0.0), (0.25, 0.15), (0.75, 0.85), (1.0, 1.0)])";
        let parsed: CurveData = ron::from_str(light_entry).expect("parse Light baked.ron curve entry");
        assert_eq!(parsed.interp, Interp::CatmullRom);
        assert_eq!(parsed.min, 0.0);
        assert_eq!(parsed.max, 1.0);
        assert_eq!(parsed.points.len(), 4);
    }

    const RED: [f32; 4] = [1.0, 0.0, 0.0, 1.0];
    const GREEN: [f32; 4] = [0.0, 1.0, 0.0, 1.0];
    const BLUE: [f32; 4] = [0.0, 0.0, 1.0, 1.0];

    fn raw_grad(stops: &[(f32, [f32; 4], f32)]) -> RawGradient {
        RawGradient { stops: stops.iter().map(|&(t, color, mid)| RawStop { t, color, mid }).collect() }
    }

    fn close4(a: [f32; 4], b: [f32; 4], eps: f32) -> bool {
        a.iter().zip(b).all(|(x, y)| (x - y).abs() < eps)
    }

    #[test]
    fn gradient_empty_single_bounds_and_non_finite_t() {
        let empty = RawGradient::default();
        assert_eq!(empty.sample(0.5), None);
        assert_eq!(empty.sample_f64(0.5), None);

        let one = raw_grad(&[(0.4, GREEN, DEFAULT_MID)]);
        for t in [-1.0, 0.0, 0.4, 1.0, f32::NEG_INFINITY, f32::INFINITY, f32::NAN] {
            assert_eq!(one.sample(t), Some(GREEN));
        }

        let two = raw_grad(&[(0.2, RED, DEFAULT_MID), (0.8, BLUE, DEFAULT_MID)]);
        assert_eq!(two.sample(-10.0), Some(RED));
        assert_eq!(two.sample(0.19), Some(RED));
        assert_eq!(two.sample(0.2), Some(RED));
        assert_eq!(two.sample(0.8), Some(BLUE));
        assert_eq!(two.sample(0.81), Some(BLUE));
        assert_eq!(two.sample(10.0), Some(BLUE));
        assert_eq!(two.sample(f32::NAN), Some(RED));
        assert_eq!(two.sample(f32::NEG_INFINITY), Some(RED));
        assert_eq!(two.sample(f32::INFINITY), Some(BLUE));
    }

    #[test]
    fn gradient_equal_t_step_and_midpoints() {
        let step = raw_grad(&[(0.0, RED, 0.5), (0.5, RED, 0.5), (0.5, BLUE, 0.5), (1.0, BLUE, 0.5)]);
        assert_eq!(step.sample(0.49999), Some(RED));
        assert_eq!(step.sample(0.5), Some(BLUE));

        let g_mid50 = raw_grad(&[(0.0, RED, 0.5), (1.0, BLUE, 0.5)]);
        let half = g_mid50.sample(0.5).unwrap();
        // OKLab mid-mix of red and blue is distinct from linear RGB mix [0.5, 0.0, 0.5, 1.0].
        assert!(!close4(half, [0.5, 0.0, 0.5, 1.0], 1e-3), "unexpected linear mix: {half:?}");
        let (la, lb) = (linear_to_oklab([1.0, 0.0, 0.0]), linear_to_oklab([0.0, 0.0, 1.0]));
        let expected_rgb = oklab_to_linear([0, 1, 2].map(|i| 0.5 * (la[i] + lb[i]))).map(|c| c.clamp(0.0, 1.0));
        assert!(close4(half, [expected_rgb[0], expected_rgb[1], expected_rgb[2], 1.0], 1e-5));

        // Midpoint 0.25 reaches the 50/50 mix at t = 0.25.
        let g_mid25 = raw_grad(&[(0.0, RED, 0.25), (1.0, BLUE, 0.5)]);
        assert!(close4(g_mid25.sample(0.25).unwrap(), half, 1e-6));

        // Raw path clamps out-of-range midpoints (0.0 -> MID_MIN = 0.01, 1.0 -> MID_MAX = 0.99, NaN -> DEFAULT_MID = 0.5).
        let g_mid_zero = raw_grad(&[(0.0, RED, 0.0), (1.0, BLUE, 0.5)]);
        let g_mid_min = raw_grad(&[(0.0, RED, MID_MIN), (1.0, BLUE, 0.5)]);
        let g_mid_one = raw_grad(&[(0.0, RED, 1.0), (1.0, BLUE, 0.5)]);
        let g_mid_max = raw_grad(&[(0.0, RED, MID_MAX), (1.0, BLUE, 0.5)]);
        let g_mid_nan = raw_grad(&[(0.0, RED, f32::NAN), (1.0, BLUE, 0.5)]);
        for t in [0.01, 0.25, 0.5, 0.75, 0.99] {
            assert_eq!(g_mid_zero.sample(t), g_mid_min.sample(t));
            assert_eq!(g_mid_one.sample(t), g_mid_max.sample(t));
            assert_eq!(g_mid_nan.sample(t), g_mid50.sample(t));
        }

        // Last stop's `mid` is ignored.
        let g_last_mid_a = raw_grad(&[(0.0, RED, 0.5), (0.5, GREEN, 0.5), (1.0, BLUE, 0.05)]);
        let g_last_mid_b = raw_grad(&[(0.0, RED, 0.5), (0.5, GREEN, 0.5), (1.0, BLUE, 0.95)]);
        for t in [0.0, 0.25, 0.5, 0.75, 1.0] {
            assert_eq!(g_last_mid_a.sample(t), g_last_mid_b.sample(t));
        }
    }

    #[test]
    fn gradient_identical_rgb_short_circuit_and_alpha_lerp() {
        // Identical RGB with varying alpha: RGB stays exact without OKLab roundtrip drift, alpha lerps linearly.
        let rgb = [0.123_456_79, 0.654_321_1, 0.333_333_34];
        let g = raw_grad(&[(0.0, [rgb[0], rgb[1], rgb[2], 1.0], 0.5), (1.0, [rgb[0], rgb[1], rgb[2], 0.0], 0.5)]);
        let mid = g.sample(0.5).unwrap();
        assert_eq!([mid[0], mid[1], mid[2]], rgb);
        assert!((mid[3] - 0.5).abs() < 1e-6);
    }

    #[test]
    fn gradient_ron_roundtrips_and_defaults_mid() {
        let g = raw_grad(&[(0.0, RED, 0.25), (1.0, BLUE, DEFAULT_MID)]);
        let text = ron::to_string(&g).expect("serialize RawGradient");
        let back: RawGradient = ron::from_str(&text).expect("deserialize RawGradient");
        assert_eq!(back, g);

        let omitted_mid = "(stops: [(t: 0.0, color: (1.0, 0.0, 0.0, 1.0)), (t: 1.0, color: (0.0, 0.0, 1.0, 1.0))])";
        let parsed: RawGradient = ron::from_str(omitted_mid).expect("parse RawGradient with omitted mid");
        assert_eq!(parsed.stops.len(), 2);
        assert_eq!(parsed.stops[0].mid, DEFAULT_MID);
        assert_eq!(parsed.stops[1].mid, DEFAULT_MID);
    }
}
