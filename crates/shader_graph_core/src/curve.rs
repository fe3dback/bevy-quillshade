//! Curve assets (`shaders/**/*.curve.ron`, decision 0015): points on the unit square sampled by the
//! `Sample Curve` node. [`Curve::sample`] is the reference for the generated WESL and the editor preview.

use serde::{Deserialize, Serialize};

use crate::model::LoadError;

/// Current curve file format version.
pub const CURVE_FORMAT_VERSION: u32 = 1;

/// Smallest segment length used as a divisor (points with equal `t` make a step).
const MIN_SPAN: f32 = 1e-6;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Curve {
    pub format_version: u32,
    pub name: String,
    /// `(t, value)`, both in `0..1`, sorted by `t`.
    #[serde(default)]
    pub points: Vec<(f32, f32)>,
}

/// How a curve is evaluated between its points.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub enum Interp {
    /// Piecewise linear.
    Simple,
    /// Cubic Hermite with Catmull-Rom tangents (non-uniform knots).
    #[default]
    CatmullRom,
}

impl Interp {
    pub const ALL: [Interp; 2] = [Interp::CatmullRom, Interp::Simple];

    /// Option label of the node's `type` parameter and the editor switch.
    pub fn label(self) -> &'static str {
        match self {
            Interp::Simple => "Simple",
            Interp::CatmullRom => "Catmull-Rom",
        }
    }

    pub fn from_label(s: &str) -> Option<Interp> {
        Interp::ALL.into_iter().find(|i| i.label() == s)
    }

    /// Function of a generated curve module (`sample_curve_simple`, `sample_curve_catmullrom`).
    pub fn wesl_fn(self) -> &'static str {
        match self {
            Interp::Simple => "sample_curve_simple",
            Interp::CatmullRom => "sample_curve_catmullrom",
        }
    }

    /// Suffix of the import alias in shaders using the curve.
    fn alias_suffix(self) -> &'static str {
        match self {
            Interp::Simple => "simple",
            Interp::CatmullRom => "catmullrom",
        }
    }
}

/// Name a shader imports `interp`'s function of curve `path` as: unique per curve, so one shader can sample
/// several curves (`env/falloff` → `sg_curve_env__falloff_simple`).
pub fn curve_fn_alias(path: &str, interp: Interp) -> String {
    format!("sg_curve_{}_{}", path.replace('/', "__"), interp.alias_suffix())
}

#[derive(Deserialize)]
#[serde(rename = "Curve")]
struct VersionProbe {
    format_version: u32,
}

impl Curve {
    pub fn new(name: impl Into<String>, points: Vec<(f32, f32)>) -> Self {
        let mut c = Curve { format_version: CURVE_FORMAT_VERSION, name: name.into(), points };
        c.normalize();
        c
    }

    /// Template of a new curve asset: a straight ramp.
    pub fn ramp(name: impl Into<String>) -> Self {
        Curve::new(name, vec![(0.0, 0.0), (1.0, 1.0)])
    }

    /// Parses a curve file (older format versions would migrate here).
    pub fn from_ron(text: &str) -> Result<Curve, LoadError> {
        let probe: VersionProbe =
            ron::Options::default().from_str(text).map_err(|e| LoadError::Parse(e.to_string()))?;
        match probe.format_version {
            v if v > CURVE_FORMAT_VERSION => Err(LoadError::TooNew { found: v, supported: CURVE_FORMAT_VERSION }),
            CURVE_FORMAT_VERSION => {
                let mut c: Curve =
                    ron::Options::default().from_str(text).map_err(|e| LoadError::Parse(e.to_string()))?;
                c.normalize();
                Ok(c)
            }
            v => Err(LoadError::Unsupported(v)),
        }
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

    /// Drops non-finite points, clamps to the unit square, sorts by `t` (stable: equal `t` keep their order).
    pub fn normalize(&mut self) {
        self.points.retain(|(t, v)| t.is_finite() && v.is_finite());
        for p in &mut self.points {
            *p = (p.0.clamp(0.0, 1.0), p.1.clamp(0.0, 1.0));
        }
        self.points.sort_by(|a, b| a.0.total_cmp(&b.0));
    }

    /// Catmull-Rom slope `dv/dt` at every point.
    pub fn slopes(&self) -> Vec<f32> {
        let p = &self.points;
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

    /// Value at `t` (same math as the generated `sample_curve_*` functions).
    pub fn sample(&self, t: f32, interp: Interp) -> f32 {
        self.sample_with(t, interp, &self.slopes())
    }

    /// [`Curve::sample`] with the [`Curve::slopes`] computed once (drawing samples many `t`).
    pub fn sample_with(&self, t: f32, interp: Interp, slopes: &[f32]) -> f32 {
        let p = &self.points;
        match p.len() {
            0 => return 0.0,
            1 => return p[0].1,
            _ => {}
        }
        let x = t.clamp(p[0].0, p[p.len() - 1].0);
        // Last point whose `t` is <= x, among the segment starts.
        let i = (1..p.len() - 1).rfind(|k| x >= p[*k].0).unwrap_or(0);
        let (a, b) = (p[i], p[i + 1]);
        match interp {
            Interp::Simple => linear(a, b, x),
            Interp::CatmullRom => hermite((a.0, a.1, slopes[i]), (b.0, b.1, slopes[i + 1]), x),
        }
    }
}

/// `sg_curve_linear` of `stdlib/curve.wesl`.
fn linear(a: (f32, f32), b: (f32, f32), t: f32) -> f32 {
    let s = ((t - a.0) / (b.0 - a.0).max(MIN_SPAN)).clamp(0.0, 1.0);
    a.1 + (b.1 - a.1) * s
}

/// `sg_curve_hermite` of `stdlib/curve.wesl`: points are `(t, value, slope)`.
fn hermite(a: (f32, f32, f32), b: (f32, f32, f32), t: f32) -> f32 {
    let h = (b.0 - a.0).max(MIN_SPAN);
    let s = ((t - a.0) / h).clamp(0.0, 1.0);
    let (s2, s3) = (s * s, s * s * s);
    (2.0 * s3 - 3.0 * s2 + 1.0) * a.1
        + (s3 - 2.0 * s2 + s) * h * a.2
        + (-2.0 * s3 + 3.0 * s2) * b.1
        + (s3 - s2) * h * b.2
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_and_normalize() {
        let c = Curve::new("falloff", vec![(1.0, 0.0), (0.25, 2.0), (-1.0, 0.5), (f32::NAN, 0.0)]);
        assert_eq!(c.points, vec![(0.0, 0.5), (0.25, 1.0), (1.0, 0.0)]);
        let text = c.to_ron();
        assert!(text.starts_with("Curve("), "{text}");
        assert_eq!(Curve::from_ron(&text).unwrap(), c);
        let too_new = "Curve(format_version: 9, name: \"x\")";
        assert!(matches!(Curve::from_ron(too_new), Err(LoadError::TooNew { .. })));
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
    fn aliases_are_unique_per_curve_and_mode() {
        assert_eq!(curve_fn_alias("env/falloff", Interp::Simple), "sg_curve_env__falloff_simple");
        assert_eq!(curve_fn_alias("ramp", Interp::CatmullRom), "sg_curve_ramp_catmullrom");
        assert_eq!(Interp::from_label("Catmull-Rom"), Some(Interp::CatmullRom));
    }
}
