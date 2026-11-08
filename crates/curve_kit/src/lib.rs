//! Curve assets (`*.curve.ron`, decision 0015; design §7.1): points on the unit square sampled by the
//! `Sample Curve` node, scaled to the curve's output range `min..max` (decision 0038). [`Curve::sample`] is the
//! reference for the generated WESL; the editor draws the normalized shape ([`Curve::sample_normalized`]).
//!
//! With the `editor` feature (default), [`editor`] is the curve editor every domain with curves composes
//! ([`editor::CurveDomain`]); the model above stays Bevy-free for generator backends.

use serde::{Deserialize, Serialize};

use project_core::common::curve::{CurveData, normalize_points, sample_normalized_with, slopes};
use project_core::{AssetUuid, LoadError, check_version, probe_version};

pub use project_core::common::curve::{Interp, MIN_SPAN};

#[cfg(feature = "editor")]
pub mod editor;

/// Current curve file format version: 2 carries the asset `id` (ADR 0047 §2, written by the v3 migration); older
/// files are upgraded by the Migration Tool (`shader-editor migrate`), never by the loader.
pub const CURVE_FORMAT_VERSION: u32 = 2;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Curve {
    pub format_version: u32,
    /// The asset's UUID (ADR 0047 §2); nil (omitted from the file) until the asset is first saved.
    #[serde(default, skip_serializing_if = "AssetUuid::is_nil")]
    pub id: AssetUuid,
    pub name: String,
    /// Output of a point value of 0 (decision 0038); omitted from the file at its default.
    #[serde(default, skip_serializing_if = "is_default_min")]
    pub min: f32,
    /// Output of a point value of 1, `> min`; omitted from the file at its default.
    #[serde(default = "default_max", skip_serializing_if = "is_default_max")]
    pub max: f32,
    /// `(t, value)`, both in `0..1`, sorted by `t`; a value `v` outputs `min + v * (max - min)`.
    #[serde(default)]
    pub points: Vec<(f32, f32)>,
}

pub const DEFAULT_MIN: f32 = 0.0;
pub const DEFAULT_MAX: f32 = 1.0;

fn default_max() -> f32 {
    DEFAULT_MAX
}

fn is_default_min(x: &f32) -> bool {
    *x == DEFAULT_MIN
}

fn is_default_max(x: &f32) -> bool {
    *x == DEFAULT_MAX
}

/// Why `min..max` is not a valid output range (`None`: it is).
pub fn range_error(min: f32, max: f32) -> Option<String> {
    if !min.is_finite() || !max.is_finite() {
        Some(format!("curve range min {min} / max {max} must be finite"))
    } else if min >= max {
        Some(format!("curve range min {min} must be less than max {max}"))
    } else {
        None
    }
}

impl Curve {
    pub fn new(name: impl Into<String>, points: Vec<(f32, f32)>) -> Self {
        let mut c = Curve {
            format_version: CURVE_FORMAT_VERSION,
            id: AssetUuid::NIL,
            name: name.into(),
            min: DEFAULT_MIN,
            max: DEFAULT_MAX,
            points,
        };
        c.normalize();
        c
    }

    /// Template of a new curve asset: a straight ramp.
    pub fn ramp(name: impl Into<String>) -> Self {
        Curve::new(name, vec![(0.0, 0.0), (1.0, 1.0)])
    }

    /// Parses a curve file of the current format ([`CURVE_FORMAT_VERSION`]); any other is refused.
    pub fn from_ron(text: &str) -> Result<Curve, LoadError> {
        check_version(probe_version(text)?, CURVE_FORMAT_VERSION)?;
        let mut c: Curve = ron::Options::default().from_str(text).map_err(|e| LoadError::Parse(e.to_string()))?;
        if let Some(e) = range_error(c.min, c.max) {
            return Err(LoadError::Parse(e));
        }
        c.normalize();
        Ok(c)
    }

    /// Mints the asset's UUID when it has none (ADR 0047 §6); returns it.
    pub fn ensure_id(&mut self) -> AssetUuid {
        self.id.ensure()
    }

    pub fn to_ron(&self) -> String {
        let mut c = self.clone();
        c.normalize();
        let cfg =
            ron::ser::PrettyConfig::new().struct_names(true).indentor("  ".to_string()).new_line("\n".to_string());
        let mut s = ron::ser::to_string_pretty(&c, cfg).expect("curve serialization");
        s.push('\n');
        s
    }

    /// Drops non-finite points, clamps to the unit square, sorts by `t` (stable: equal `t` keep their order). The
    /// output range is left alone.
    pub fn normalize(&mut self) {
        normalize_points(&mut self.points);
    }

    /// Runtime [`CurveData`] representation with the given [`Interp`].
    pub fn data(&self, interp: Interp) -> CurveData {
        CurveData { interp, min: self.min, max: self.max, points: self.points.clone() }
    }

    /// `max - min`: the scale from normalized values (and slopes) to real units.
    pub fn span(&self) -> f32 {
        self.max - self.min
    }

    /// A normalized value (`0..1` = `min..max`) in real units.
    pub fn to_real(&self, v: f32) -> f32 {
        self.min + v * self.span()
    }

    /// A real value as a normalized one (inverse of [`Curve::to_real`]).
    pub fn to_normalized(&self, x: f32) -> f32 {
        (x - self.min) / self.span()
    }

    /// Catmull-Rom slope `dv/dt` at every point, normalized (real units: times [`Curve::span`]).
    pub fn slopes(&self) -> Vec<f32> {
        slopes(&self.points)
    }

    /// Value at `t` in real units (same math as the generated `sample_curve_*` functions, whose constants hold the
    /// points already scaled to `min..max`; Hermite and linear interpolation are linear in values and slopes).
    pub fn sample(&self, t: f32, interp: Interp) -> f32 {
        self.to_real(self.sample_normalized(t, interp))
    }

    /// Value at `t` before the output range is applied (`0..1`, Catmull-Rom may overshoot): the drawn shape.
    pub fn sample_normalized(&self, t: f32, interp: Interp) -> f32 {
        self.sample_normalized_with(t, interp, &self.slopes())
    }

    /// [`Curve::sample_normalized`] with the [`Curve::slopes`] computed once (drawing samples many `t`).
    pub fn sample_normalized_with(&self, t: f32, interp: Interp, slopes: &[f32]) -> f32 {
        sample_normalized_with(&self.points, slopes, interp, t)
    }
}

impl From<&Curve> for CurveData {
    fn from(c: &Curve) -> Self {
        c.data(Interp::default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use project_core::LoadError;

    fn reference_slopes(p: &[(f32, f32)]) -> Vec<f32> {
        let n = p.len();
        let diff = |a: usize, b: usize| {
            let dt = p[b].0 - p[a].0;
            if dt.abs() < MIN_SPAN { 0.0 } else { (p[b].1 - p[a].1) / dt }
        };
        (0..n)
            .map(|i| match (i, n) {
                (_, 0 | 1) => 0.0,
                (0, _) => diff(0, 1),
                (i, n) if i == n - 1 => diff(n - 2, n - 1),
                (i, _) => diff(i - 1, i + 1),
            })
            .collect()
    }

    fn reference_linear(a: (f32, f32), b: (f32, f32), t: f32) -> f32 {
        let s = ((t - a.0) / (b.0 - a.0).max(MIN_SPAN)).clamp(0.0, 1.0);
        a.1 + (b.1 - a.1) * s
    }

    fn reference_hermite(a: (f32, f32, f32), b: (f32, f32, f32), t: f32) -> f32 {
        let h = (b.0 - a.0).max(MIN_SPAN);
        let s = ((t - a.0) / h).clamp(0.0, 1.0);
        let (s2, s3) = (s * s, s * s * s);
        (2.0 * s3 - 3.0 * s2 + 1.0) * a.1
            + (s3 - 2.0 * s2 + s) * h * a.2
            + (-2.0 * s3 + 3.0 * s2) * b.1
            + (s3 - s2) * h * b.2
    }

    fn reference_sample(curve: &Curve, t: f32, interp: Interp) -> f32 {
        let p = &curve.points;
        let norm = match p.len() {
            0 => 0.0,
            1 => p[0].1,
            _ => {
                let sl = reference_slopes(p);
                let x = t.clamp(p[0].0, p[p.len() - 1].0);
                let i = (1..p.len() - 1).rfind(|k| x >= p[*k].0).unwrap_or(0);
                let (a, b) = (p[i], p[i + 1]);
                match interp {
                    Interp::Simple => reference_linear(a, b, x),
                    Interp::CatmullRom => reference_hermite((a.0, a.1, sl[i]), (b.0, b.1, sl[i + 1]), x),
                }
            }
        };
        curve.min + norm * (curve.max - curve.min)
    }

    #[test]
    fn bitwise_parity_with_curve_data_and_pre_move_reference() {
        let pulse = Curve::new("pulse", vec![(0.0, 0.0), (0.25, 1.0), (0.5, 0.3), (0.75, 0.1), (1.0, 0.0)]);
        let steps = Curve::new("steps", vec![(0.0, 0.2), (0.33, 0.2), (0.34, 1.0), (0.66, 1.0), (0.67, 0.5)]);
        let smooth = Curve::new("smooth", vec![(0.0, 0.0), (0.25, 0.15), (0.75, 0.85), (1.0, 1.0)]);
        let ramp = Curve::ramp("ramp");
        let one_pt = Curve::new("one", vec![(0.35, 0.7)]);
        let dup_t = Curve::new("dup", vec![(0.0, 0.0), (0.5, 0.1), (0.5, 0.9), (1.0, 1.0)]);
        let ranged = Curve { min: -2.0, max: 5.0, ..pulse.clone() };
        let curves = [pulse, steps, smooth, ramp, one_pt, dup_t, ranged];

        let mut ts = vec![-1.0_f32, -0.01, 0.0, 0.001];
        for k in 1..=20 {
            ts.push(k as f32 * 0.05);
        }
        ts.extend([0.999, 1.0, 1.5, f32::NEG_INFINITY, f32::INFINITY]);

        for curve in &curves {
            assert_eq!(CurveData::from(curve), curve.data(Interp::CatmullRom));
            for interp in Interp::ALL {
                let data = curve.data(interp);
                for &t in &ts {
                    let from_curve = curve.sample(t, interp);
                    let from_data = data.sample(t);
                    let from_ref = reference_sample(curve, t, interp);
                    assert_eq!(
                        from_curve.to_bits(),
                        from_data.to_bits(),
                        "Curve vs CurveData mismatch on {} ({interp:?}, t={t}): {from_curve} vs {from_data}",
                        curve.name
                    );
                    assert_eq!(
                        from_curve.to_bits(),
                        from_ref.to_bits(),
                        "Curve vs reference mismatch on {} ({interp:?}, t={t}): {from_curve} vs {from_ref}",
                        curve.name
                    );
                }
            }
        }
    }

    #[test]
    fn roundtrip_and_normalize() {
        let c = Curve::new("falloff", vec![(1.0, 0.0), (0.25, 2.0), (-1.0, 0.5), (f32::NAN, 0.0)]);
        assert_eq!(c.points, vec![(0.0, 0.5), (0.25, 1.0), (1.0, 0.0)]);
        let text = c.to_ron();
        assert!(text.starts_with("Curve("), "{text}");
        assert_eq!(Curve::from_ron(&text).unwrap(), c);
        let too_new = "Curve(format_version: 9, name: \"x\")";
        assert!(matches!(Curve::from_ron(too_new), Err(LoadError::TooNew { .. })));
        let older = "Curve(format_version: 1, name: \"x\")";
        assert!(matches!(Curve::from_ron(older), Err(LoadError::TooOld { found: 1, current: 2 })));
    }

    #[test]
    fn saved_curve_keeps_its_id() {
        let mut c = Curve::ramp("r");
        assert!(!c.to_ron().contains("id:"), "a nil id is not written");
        let id = c.ensure_id();
        assert!(id.is_v7());
        assert_eq!(c.ensure_id(), id, "an id is minted once");
        let text = c.to_ron();
        assert!(text.starts_with(&format!("Curve(\n  format_version: 2,\n  id: \"{id}\",\n")), "{text}");
        assert_eq!(Curve::from_ron(&text).unwrap(), c);
    }

    #[test]
    fn sampling() {
        let ramp = Curve::ramp("r");
        for interp in Interp::ALL {
            // A straight line stays straight, ends hold outside the points.
            assert!((ramp.sample(0.3, interp) - 0.3).abs() < 1e-6, "{interp:?}");
        }
        let c = Curve::new("c", vec![(0.2, 0.0), (0.5, 1.0), (0.8, 0.0)]);
        for interp in Interp::ALL {
            assert_eq!(c.sample(0.0, interp), 0.0);
            assert_eq!(c.sample(1.0, interp), 0.0);
            assert!((c.sample(0.5, interp) - 1.0).abs() < 1e-6);
        }
        assert!((c.sample(0.35, Interp::Simple) - 0.5).abs() < 1e-6);
        // Catmull-Rom is smooth: zero slope at the peak, above the line near it.
        assert_eq!(c.slopes()[1], 0.0);
        assert!(c.sample(0.45, Interp::CatmullRom) > c.sample(0.45, Interp::Simple));
        assert_eq!(Curve::new("e", vec![]).sample(0.5, Interp::CatmullRom), 0.0);
        assert_eq!(Curve::new("o", vec![(0.3, 0.7)]).sample(0.9, Interp::Simple), 0.7);
        // Equal t: a step, no division by zero.
        let step = Curve::new("s", vec![(0.0, 0.0), (0.5, 0.0), (0.5, 1.0), (1.0, 1.0)]);
        assert_eq!(step.sample(0.75, Interp::Simple), 1.0);
        assert!(step.sample(0.25, Interp::CatmullRom).is_finite());
    }

    #[test]
    fn range_is_stored_only_when_set() {
        let c = Curve::new("r", vec![(0.0, 0.25), (1.0, 1.0)]);
        assert_eq!((c.min, c.max), (0.0, 1.0));
        let text = c.to_ron();
        assert!(!text.contains("min") && !text.contains("max"), "{text}");
        let old =
            "Curve(\n  format_version: 2,\n  name: \"r\",\n  points: [\n    (0.0, 0.25),\n    (1.0, 1.0),\n  ],\n)\n";
        assert_eq!(text, old, "a file without a range re-saves byte-identically");
        assert_eq!(Curve::from_ron(old).unwrap(), c);

        let ranged = Curve { min: -2.0, max: 10.0, ..c.clone() };
        let text = ranged.to_ron();
        assert!(text.contains("min: -2.0") && text.contains("max: 10.0"), "{text}");
        let back = Curve::from_ron(&text).unwrap();
        assert_eq!(back, ranged);
        assert_eq!(back.points, c.points, "points stay normalized");
    }

    #[test]
    fn invalid_ranges_fail_to_load() {
        let file = |range: &str| format!("Curve(format_version: 2, name: \"r\", {range} points: [(0.0, 0.5)])");
        for range in ["min: 1.0,", "min: 2.0, max: 2.0,", "min: 3.0, max: -1.0,", "max: NaN,", "min: -inf,"] {
            match Curve::from_ron(&file(range)) {
                Err(LoadError::Parse(e)) => assert!(e.starts_with("curve range"), "{range}: {e}"),
                other => panic!("{range}: {other:?}"),
            }
        }
        assert!(Curve::from_ron(&file("min: -1.0, max: 0.0,")).is_ok());
    }

    #[test]
    fn sampling_returns_real_units() {
        let c = Curve { min: -2.0, max: 10.0, ..Curve::new("c", vec![(0.2, 0.0), (0.5, 1.0), (0.8, 0.25)]) };
        for interp in Interp::ALL {
            assert_eq!(c.sample(0.0, interp), -2.0);
            assert_eq!(c.sample(0.5, interp), 10.0);
            assert_eq!(c.sample(1.0, interp), 1.0);
            let n = c.sample_normalized(0.4, interp);
            assert!((c.sample(0.4, interp) - (-2.0 + 12.0 * n)).abs() < 1e-5, "{interp:?}");
        }
        assert!((c.sample(0.35, Interp::Simple) - 4.0).abs() < 1e-5);
        assert_eq!((c.to_real(0.5), c.to_normalized(4.0)), (4.0, 0.5));
        assert_eq!(Curve { min: 3.0, max: 5.0, ..Curve::new("e", vec![]) }.sample(0.5, Interp::Simple), 3.0);
        assert_eq!(Curve { min: 3.0, max: 5.0, ..Curve::new("o", vec![(0.1, 0.5)]) }.sample(0.9, Interp::Simple), 4.0);
    }
}
