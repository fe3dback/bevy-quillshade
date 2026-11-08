/// Range a segment midpoint is clamped to, so the midpoint remap never divides by zero.
pub const MID_MIN: f32 = 0.01;
pub const MID_MAX: f32 = 0.99;

/// Midpoint of a segment nobody has moved: the colors mix 50/50 halfway.
pub const DEFAULT_MID: f32 = 0.5;

/// Smallest segment length used as a divisor (stops with equal `t` make a step), as in `curve.rs`.
const MIN_SPAN: f64 = 1e-6;

fn default_mid() -> f32 {
    DEFAULT_MID
}

/// A single colour stop in linear RGBA space (`0..1`) with a segment midpoint in `[0.01, 0.99]`.
// Wire format: never reorder fields.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct RawStop {
    pub t: f32,
    pub color: [f32; 4],
    #[serde(default = "default_mid")]
    pub mid: f32,
}

/// Baked Bevy-free 1D OKLab colour gradient (`stops` sorted by `t`).
// Wire format: never reorder fields.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct RawGradient {
    #[serde(default)]
    pub stops: Vec<RawStop>,
}

impl RawGradient {
    /// Evaluates the gradient at `t` in `f64` (`None` when `stops` is empty).
    pub fn sample_f64(&self, t: f64) -> Option<[f64; 4]> {
        sample_stops_f64(
            self.stops.len(),
            |k| {
                let s = self.stops[k];
                (s.t, s.color, s.mid)
            },
            t,
        )
    }

    /// Linear RGBA at `t` (`None` when `stops` is empty).
    pub fn sample(&self, t: f32) -> Option<[f32; 4]> {
        self.sample_f64(f64::from(t)).map(|c| c.map(|v| v as f32))
    }
}

/// Evaluates `n` stops `(t, linear_rgba, mid)` at `t` in `f64`.
///
/// Returns `None` when `n == 0`. Before the first stop (or when `n == 1` or `t` is `NaN`): the first stop's
/// color; at or after the last stop: the last stop's color; equal `t`: a step where the later stop wins at
/// exactly `t`. Between stops the midpoint-remapped fraction mixes RGB in OKLab and alpha linearly.
pub fn sample_stops_f64(n: usize, stop_at: impl Fn(usize) -> (f32, [f32; 4], f32), t: f64) -> Option<[f64; 4]> {
    if n == 0 {
        return None;
    }
    let color = |c: [f32; 4]| c.map(|v| f64::from(v).clamp(0.0, 1.0));
    let last = n - 1;
    let (first_t, first_c, _) = stop_at(0);
    if n == 1 || t.is_nan() || t < f64::from(first_t) {
        return Some(color(first_c));
    }
    let (last_t, last_c, _) = stop_at(last);
    if t >= f64::from(last_t) {
        return Some(color(last_c));
    }
    // Last segment start whose `t` is <= t (a later stop wins at equal `t`).
    let i = (0..last).rfind(|&k| t >= f64::from(stop_at(k).0)).unwrap_or(0);
    let (a_t, a_c, a_mid) = stop_at(i);
    let (b_t, b_c, _) = stop_at(i + 1);
    let (a, b) = (f64::from(a_t), f64::from(b_t));
    if b - a < MIN_SPAN {
        return Some(color(b_c));
    }
    let x = ((t - a) / (b - a)).clamp(0.0, 1.0);
    let m = f64::from(if a_mid.is_nan() { DEFAULT_MID } else { a_mid.clamp(MID_MIN, MID_MAX) });
    let f = if x < m { 0.5 * x / m } else { 0.5 + 0.5 * (x - m) / (1.0 - m) };
    let (ca, cb) = (color(a_c), color(b_c));
    if f <= 0.0 {
        return Some(ca);
    }
    if f >= 1.0 {
        return Some(cb);
    }
    let rgb = if ca[..3] == cb[..3] {
        [ca[0], ca[1], ca[2]]
    } else {
        let (la, lb) = (oklab_f64([ca[0], ca[1], ca[2]]), oklab_f64([cb[0], cb[1], cb[2]]));
        oklab_to_linear_f64([0, 1, 2].map(|k| la[k] + (lb[k] - la[k]) * f)).map(|c| c.clamp(0.0, 1.0))
    };
    Some([rgb[0], rgb[1], rgb[2], ca[3] + (cb[3] - ca[3]) * f])
}

// OKLab (Björn Ottosson, 2020) over linear sRGB: the matrices of `stdlib/color.wesl` (decision 0036).

const RGB_TO_LMS: [[f64; 3]; 3] = [
    [0.4122214708, 0.5363325363, 0.0514459929],
    [0.2119034982, 0.6806995451, 0.1073969566],
    [0.0883024619, 0.2817188376, 0.6299787005],
];
const LMS_TO_LAB: [[f64; 3]; 3] = [
    [0.2104542553, 0.7936177850, -0.0040720468],
    [1.9779984951, -2.4285922050, 0.4505937099],
    [0.0259040371, 0.7827717662, -0.8086757660],
];
const LAB_TO_LMS: [[f64; 3]; 3] =
    [[1.0, 0.3963377774, 0.2158037573], [1.0, -0.1055613458, -0.0638541728], [1.0, -0.0894841775, -1.2914855480]];
const LMS_TO_RGB: [[f64; 3]; 3] = [
    [4.0767416621, -3.3077115913, 0.2309699292],
    [-1.2684380046, 2.6097574011, -0.3413193965],
    [-0.0041960863, -0.7034186147, 1.7076147010],
];

fn mul(m: &[[f64; 3]; 3], v: [f64; 3]) -> [f64; 3] {
    m.map(|row| row[0] * v[0] + row[1] * v[1] + row[2] * v[2])
}

fn oklab_f64(c: [f64; 3]) -> [f64; 3] {
    mul(&LMS_TO_LAB, mul(&RGB_TO_LMS, c).map(f64::cbrt))
}

fn oklab_to_linear_f64(lab: [f64; 3]) -> [f64; 3] {
    mul(&LMS_TO_RGB, mul(&LAB_TO_LMS, lab).map(|t| t * t * t))
}

/// Linear sRGB → OKLab (`sg_rgb_to_oklab`).
pub fn linear_to_oklab(c: [f32; 3]) -> [f32; 3] {
    oklab_f64(c.map(f64::from)).map(|v| v as f32)
}

/// OKLab → linear sRGB (`sg_oklab_to_rgb` without its clamp: out-of-gamut results stay out of `0..1`).
pub fn oklab_to_linear(c: [f32; 3]) -> [f32; 3] {
    oklab_to_linear_f64(c.map(f64::from)).map(|v| v as f32)
}
