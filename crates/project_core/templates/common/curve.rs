/// Smallest segment length used as a divisor (points with equal `t` make a step).
pub const MIN_SPAN: f32 = 1e-6;

/// How a 1D curve is evaluated between its control points.
// Wire format: never reorder variants (Simple = 0, CatmullRom = 1).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize)]
pub enum Interp {
    /// Piecewise linear interpolation.
    Simple,
    /// Cubic Hermite interpolation with Catmull-Rom tangents (non-uniform knots).
    #[default]
    CatmullRom,
}

impl Interp {
    pub const ALL: [Self; 2] = [Self::CatmullRom, Self::Simple];

    /// Option label of the node's `type` parameter and the editor switch.
    pub const fn label(self) -> &'static str {
        match self {
            Self::Simple => "Simple",
            Self::CatmullRom => "Catmull-Rom",
        }
    }

    pub fn from_label(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|i| i.label() == s)
    }
}

fn default_max() -> f32 {
    1.0
}

/// Baked 1D transfer curve (`0..1` input mapped to `[min, max]` output).
// Wire format: never reorder fields.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct CurveData {
    #[serde(default)]
    pub interp: Interp,
    #[serde(default)]
    pub min: f32,
    #[serde(default = "default_max")]
    pub max: f32,
    #[serde(default)]
    pub points: Vec<(f32, f32)>,
}

impl Default for CurveData {
    fn default() -> Self {
        Self::linear()
    }
}

impl CurveData {
    /// Creates a curve with normalized control points.
    pub fn new(interp: Interp, min: f32, max: f32, mut points: Vec<(f32, f32)>) -> Self {
        normalize_points(&mut points);
        Self { interp, min, max, points }
    }

    /// Straight `0..1` identity ramp evaluated with [`Interp::CatmullRom`].
    pub fn linear() -> Self {
        Self { interp: Interp::CatmullRom, min: 0.0, max: 1.0, points: vec![(0.0, 0.0), (1.0, 1.0)] }
    }

    /// Catmull-Rom slope `dv/dt` at every point in normalized space.
    pub fn slopes(&self) -> Vec<f32> {
        slopes(&self.points)
    }

    /// Value at `t` in normalized `[0, 1]` space before `[min, max]` scaling (`CatmullRom` may overshoot).
    pub fn sample_normalized(&self, t: f32) -> f32 {
        sample_normalized_with(&self.points, &self.slopes(), self.interp, t)
    }

    /// Maps a normalized `[0, 1]` value into `[min, max]` real units.
    pub fn to_real(&self, norm_v: f32) -> f32 {
        self.min + norm_v * (self.max - self.min)
    }

    /// Value at `t` in real units (`min..max`, unclamped so `CatmullRom` overshoot is preserved).
    pub fn sample(&self, t: f32) -> f32 {
        self.to_real(self.sample_normalized(t))
    }
}

/// Drops non-finite points, clamps both coordinates to `0.0..=1.0`, and sorts by `t` (stable: equal `t` keep their
/// relative order).
pub fn normalize_points(points: &mut Vec<(f32, f32)>) {
    points.retain(|(t, v)| t.is_finite() && v.is_finite());
    for p in points.iter_mut() {
        *p = (p.0.clamp(0.0, 1.0), p.1.clamp(0.0, 1.0));
    }
    points.sort_by(|a, b| a.0.total_cmp(&b.0));
}

/// Catmull-Rom slope `dv/dt` at every point in normalized space.
pub fn slopes(points: &[(f32, f32)]) -> Vec<f32> {
    let n = points.len();
    let diff = |a: usize, b: usize| {
        let dt = points[b].0 - points[a].0;
        if dt.abs() < MIN_SPAN { 0.0 } else { (points[b].1 - points[a].1) / dt }
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

/// Evaluates normalized `points` at `t` with precomputed Catmull-Rom `slopes`.
///
/// Panic-free on any input: out-of-range or `NaN` `t` is clamped via `t.max(first.t).min(last.t)` (mapping `NaN` to
/// the first point's value when bounds are finite).
pub fn sample_normalized_with(points: &[(f32, f32)], slopes: &[f32], interp: Interp, t: f32) -> f32 {
    match points.len() {
        0 => return 0.0,
        1 => return points[0].1,
        _ => {}
    }
    let x = t.max(points[0].0).min(points[points.len() - 1].0);
    // Last point whose `t` is <= x, among the segment starts.
    let i = (1..points.len() - 1).rfind(|k| x >= points[*k].0).unwrap_or(0);
    let (a, b) = (points[i], points[i + 1]);
    match interp {
        Interp::Simple => linear(a, b, x),
        Interp::CatmullRom => {
            let sa = slopes.get(i).copied().unwrap_or(0.0);
            let sb = slopes.get(i + 1).copied().unwrap_or(0.0);
            hermite((a.0, a.1, sa), (b.0, b.1, sb), x)
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
