//! Curve modules (decision 0015) with an output range (decision 0038): the range is baked into `POINTS`, so the
//! generated functions return what `Curve::sample` does.

use shader_graph_core::curve::{Curve, Interp};

/// The `(t, value, slope)` triples of a generated module's `POINTS`.
fn points(wesl: &str) -> Vec<[f32; 3]> {
    wesl.lines()
        .filter_map(|l| l.trim().strip_prefix("vec3<f32>(")?.strip_suffix("),"))
        .map(|l| {
            let v: Vec<f32> = l.split(", ").map(|x| x.parse().unwrap()).collect();
            [v[0], v[1], v[2]]
        })
        .collect()
}

/// `sg_curve_linear` / `sg_curve_hermite` of `stdlib/curve.wesl` over the generated points (as `segment` picks).
fn eval(p: &[[f32; 3]], t: f32, interp: Interp) -> f32 {
    let x = t.clamp(p[0][0], p[p.len() - 1][0]);
    let i = (1..p.len() - 1).rfind(|k| x >= p[*k][0]).unwrap_or(0);
    let (a, b) = (p[i], p[i + 1]);
    let h = (b[0] - a[0]).max(1e-6);
    let s = ((x - a[0]) / h).clamp(0.0, 1.0);
    match interp {
        Interp::Simple => a[1] + (b[1] - a[1]) * s,
        Interp::CatmullRom => {
            let (s2, s3) = (s * s, s * s * s);
            (2.0 * s3 - 3.0 * s2 + 1.0) * a[1]
                + (s3 - 2.0 * s2 + s) * h * a[2]
                + (-2.0 * s3 + 3.0 * s2) * b[1]
                + (s3 - s2) * h * b[2]
        }
    }
}

#[test]
fn the_output_range_is_baked_into_the_points() {
    let unit = Curve::new("c", vec![(0.0, 0.0), (0.3, 1.0), (0.7, 0.25), (1.0, 0.5)]);
    let ranged = Curve { min: -2.0, max: 10.0, ..unit.clone() };
    let plain = points(&shader_codegen_wesl::curve(&unit, "fx/c", "quillshade/material/fx/c.curve.ron"));
    let wesl = shader_codegen_wesl::curve(&ranged, "fx/c", "quillshade/material/fx/c.curve.ron");
    let scaled = points(&wesl);
    assert_eq!(scaled.len(), 4, "{wesl}");
    for (u, r) in plain.iter().zip(&scaled) {
        assert_eq!(u[0], r[0], "t is not scaled");
        assert!((r[1] - (-2.0 + 12.0 * u[1])).abs() < 1e-5, "{r:?}");
        assert!((r[2] - 12.0 * u[2]).abs() < 1e-4, "{r:?}");
    }
    for interp in Interp::ALL {
        for k in 0..=20 {
            let t = k as f32 / 20.0;
            let (want, got) = (ranged.sample(t, interp), eval(&scaled, t, interp));
            assert!((want - got).abs() < 1e-4, "{interp:?} t={t}: {want} vs {got}");
        }
    }
    assert!(shader_codegen_wesl::check::parse_error(&wesl).is_none(), "{wesl}");
}

#[test]
fn short_curves_return_real_units() {
    let module = |c: &Curve| shader_codegen_wesl::curve(c, "c", "quillshade/material/c.curve.ron");
    let empty = Curve { min: 3.0, max: 5.0, ..Curve::new("c", vec![]) };
    assert!(module(&empty).contains("    return 3.0f;"), "{}", module(&empty));
    let one = Curve { min: 3.0, max: 5.0, ..Curve::new("c", vec![(0.4, 0.5)]) };
    assert!(module(&one).contains("    return 4.0f;"), "{}", module(&one));
    assert!(module(&Curve::new("c", vec![])).contains("    return 0.0f;"), "the default range keeps its text");
}
