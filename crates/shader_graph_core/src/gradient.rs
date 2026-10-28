//! Gradient collections (`shaders/**/*.gradients.ron`, gradient design §3–§5): a fixed array of [`SLOTS`] color
//! gradients baked into one `SLOTS`×`SAMPLES` RGBA8 texture (row = slot position) and sampled by the
//! `Sample Gradient` node. Not a graph. Nodes reference a slot by its [`Slot::id`], never by its position, so slots
//! can be reordered without touching graphs. [`Slot::sample`] is the reference math for baking and every editor
//! preview.

use std::collections::HashSet;
use std::hash::{BuildHasher, RandomState};

use serde::{Deserialize, Serialize};

use crate::model::LoadError;

/// Current gradient collection file format version (its own probe; the graph `FORMAT_VERSION` is unrelated).
pub const GRADIENTS_FORMAT_VERSION: u32 = 1;

/// Slots in every collection (rows of the baked texture).
pub const SLOTS: usize = 128;
/// Samples of `t` per slot (columns of the baked texture): pixel `x` is `t = x / (SAMPLES - 1)`.
pub const SAMPLES: usize = 128;
/// Most stops a slot may hold.
pub const MAX_STOPS: usize = 32;

/// Range a segment midpoint is clamped to, so the midpoint remap never divides by zero.
pub const MID_MIN: f32 = 0.01;
pub const MID_MAX: f32 = 0.99;

/// Midpoint of a segment nobody has moved: the colors mix 50/50 halfway.
const DEFAULT_MID: f32 = 0.5;

/// Smallest segment length used as a divisor (stops with equal `t` make a step), as in `curve.rs`.
const MIN_SPAN: f64 = 1e-6;

/// Prefix of the baked texture id of a collection; it can never collide with a Texture asset id (an asset path has
/// no `@`).
pub const GRADIENT_PREFIX: &str = "@gradients/";

/// Baked texture id of collection `collection` (its asset id): `common/terrain` → `@gradients/common/terrain`.
pub fn texture_id(collection: &str) -> String {
    format!("{GRADIENT_PREFIX}{collection}")
}

/// A gradient collection asset. It has no `name` field: the name is the file name, as for a Texture asset.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GradientCollection {
    pub format_version: u32,
    /// Exactly [`SLOTS`] entries; position `r` is row `r` of the baked texture.
    #[serde(default)]
    pub slots: Vec<Slot>,
}

/// One gradient of a collection. Zero stops = an empty slot (baked as opaque white).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Slot {
    /// Stable reference used by nodes: non-zero, at most `i32::MAX`, unique within the collection, never changed.
    pub id: u32,
    /// Editor-only label; empty by default.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub name: String,
    /// At most [`MAX_STOPS`], sorted by `t`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub stops: Vec<Stop>,
}

/// A color stop.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Stop {
    /// Position in `0..1`.
    pub t: f32,
    /// Linear RGBA in `0..1`.
    pub color: [f32; 4],
    /// Midpoint of the segment from this stop to the next, relative to the segment: where the two colors mix
    /// 50/50. In `MID_MIN..=MID_MAX`; ignored on the last stop. It belongs to its stop, so it travels with it when
    /// stops are re-sorted.
    #[serde(default = "default_mid", skip_serializing_if = "is_default_mid")]
    pub mid: f32,
}

fn default_mid() -> f32 {
    DEFAULT_MID
}

fn is_default_mid(mid: &f32) -> bool {
    *mid == DEFAULT_MID
}

#[derive(Deserialize)]
#[serde(rename = "GradientCollection")]
struct VersionProbe {
    format_version: u32,
}

impl GradientCollection {
    /// A collection of [`SLOTS`] empty slots. Ids are drawn from `next_id`; `0`, values above `i32::MAX` and
    /// duplicates are drawn again (so `next_id` must eventually yield enough distinct valid ids).
    pub fn new(mut next_id: impl FnMut() -> u32) -> Self {
        let mut seen = HashSet::with_capacity(SLOTS);
        let slots = (0..SLOTS)
            .map(|_| {
                let id = loop {
                    let id = next_id();
                    if valid_id(id) && seen.insert(id) {
                        break id;
                    }
                };
                Slot { id, name: String::new(), stops: Vec::new() }
            })
            .collect();
        GradientCollection { format_version: GRADIENTS_FORMAT_VERSION, slots }
    }

    /// [`GradientCollection::new`] with random ids (splitmix64 seeded from the std hasher's random keys).
    pub fn random() -> Self {
        let mut state = RandomState::new().hash_one(std::time::SystemTime::now());
        GradientCollection::new(move || {
            // splitmix64; the top 31 bits are always at most `i32::MAX`.
            state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
            let mut z = state;
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
            ((z ^ (z >> 31)) >> 33) as u32
        })
    }

    /// Parses a collection file (older format versions would migrate here), normalizes it and checks its structure:
    /// slot count, ids, stop count (`LoadError::Parse` naming the problem).
    pub fn from_ron(text: &str) -> Result<Self, LoadError> {
        let probe: VersionProbe =
            ron::Options::default().from_str(text).map_err(|e| LoadError::Parse(e.to_string()))?;
        match probe.format_version {
            v if v > GRADIENTS_FORMAT_VERSION => {
                Err(LoadError::TooNew { found: v, supported: GRADIENTS_FORMAT_VERSION })
            }
            GRADIENTS_FORMAT_VERSION => {
                let mut c: GradientCollection =
                    ron::Options::default().from_str(text).map_err(|e| LoadError::Parse(e.to_string()))?;
                c.normalize();
                c.check().map_err(LoadError::Parse)?;
                Ok(c)
            }
            v => Err(LoadError::Unsupported(v)),
        }
    }

    /// Struct names, 2-space indent, one slot per line, defaults (empty name / stops, `mid: 0.5`) omitted.
    pub fn to_ron(&self) -> String {
        let mut c = self.clone();
        c.normalize();
        let cfg = ron::ser::PrettyConfig::new()
            .struct_names(true)
            .indentor("  ".to_string())
            .new_line("\n".to_string())
            .depth_limit(2);
        let mut s = ron::ser::to_string_pretty(&c, cfg).expect("gradient collection serialization");
        s.push('\n');
        s
    }

    /// For every slot: drops stops with a non-finite `t`, clamps `t` and the color channels to `0..1` (NaN → 0) and
    /// `mid` to `MID_MIN..=MID_MAX` (NaN → 0.5), stable-sorts stops by `t` (equal `t` keep their order).
    pub fn normalize(&mut self) {
        for slot in &mut self.slots {
            slot.normalize();
        }
    }

    /// Structural rules a loaded file must satisfy.
    fn check(&self) -> Result<(), String> {
        if self.slots.len() != SLOTS {
            return Err(format!("gradient collection has {} slots, expected {SLOTS}", self.slots.len()));
        }
        let mut seen = HashSet::with_capacity(SLOTS);
        for (pos, slot) in self.slots.iter().enumerate() {
            if !valid_id(slot.id) {
                return Err(format!("slot {pos}: id {} must be in 1..={}", slot.id, i32::MAX));
            }
            if !seen.insert(slot.id) {
                return Err(format!("slot {pos}: duplicate id {}", slot.id));
            }
            if slot.stops.len() > MAX_STOPS {
                return Err(format!("slot {pos}: {} stops, at most {MAX_STOPS} allowed", slot.stops.len()));
            }
        }
        Ok(())
    }

    /// Position (= baked texture row) of the slot with id `id`.
    pub fn position(&self, id: u32) -> Option<usize> {
        self.slots.iter().position(|s| s.id == id)
    }

    /// The baked texture: `SLOTS * SAMPLES * 4` bytes of sRGB-encoded RGBA8, row `r` = slot `r` (see
    /// [`Slot::bake_row`]). Missing slots (an unchecked collection) bake as empty ones.
    pub fn bake_rgba8(&self) -> Vec<u8> {
        let empty = Slot { id: 0, name: String::new(), stops: Vec::new() };
        let mut out = Vec::with_capacity(SLOTS * SAMPLES * 4);
        for r in 0..SLOTS {
            let row = self.slots.get(r).unwrap_or(&empty).bake_row();
            out.extend_from_slice(row.as_flattened());
        }
        out
    }
}

fn valid_id(id: u32) -> bool {
    id != 0 && id <= i32::MAX as u32
}

impl Slot {
    /// An empty slot has no stops.
    pub fn is_empty(&self) -> bool {
        self.stops.is_empty()
    }

    fn normalize(&mut self) {
        self.stops.retain(|s| s.t.is_finite());
        for s in &mut self.stops {
            s.t = s.t.clamp(0.0, 1.0);
            for c in &mut s.color {
                *c = if c.is_nan() { 0.0 } else { c.clamp(0.0, 1.0) };
            }
            s.mid = if s.mid.is_nan() { DEFAULT_MID } else { s.mid.clamp(MID_MIN, MID_MAX) };
        }
        self.stops.sort_by(|a, b| a.t.total_cmp(&b.t));
    }

    /// Linear RGBA at `t` (gradient design §4); `None` for an empty slot. Before the first stop: its color; after
    /// the last: its color; equal `t`: a step where the later stop wins at exactly that `t`. Between stops the
    /// midpoint-remapped fraction mixes RGB in OKLab and alpha linearly. A NaN `t` counts as before the first stop.
    pub fn sample(&self, t: f32) -> Option<[f32; 4]> {
        self.sample_f64(f64::from(t)).map(|c| c.map(|v| v as f32))
    }

    /// [`Slot::sample`] in `f64`: baking rounds to 8 bits, and the wider math keeps platform `libm` differences far
    /// from a rounding boundary (the baked PNG must be byte-identical on every OS).
    fn sample_f64(&self, t: f64) -> Option<[f64; 4]> {
        let s = &self.stops;
        let color = |k: usize| s[k].color.map(|c| f64::from(c).clamp(0.0, 1.0));
        let (first, last) = (s.first()?, s.len() - 1);
        if s.len() == 1 || t.is_nan() || t < f64::from(first.t) {
            return Some(color(0));
        }
        if t >= f64::from(s[last].t) {
            return Some(color(last));
        }
        // Last segment start whose `t` is <= t (a later stop wins at equal `t`).
        let i = (0..last).rfind(|k| t >= f64::from(s[*k].t)).unwrap_or(0);
        let (a, b) = (f64::from(s[i].t), f64::from(s[i + 1].t));
        if b - a < MIN_SPAN {
            return Some(color(i + 1));
        }
        let x = ((t - a) / (b - a)).clamp(0.0, 1.0);
        let m = f64::from(if s[i].mid.is_nan() { DEFAULT_MID } else { s[i].mid.clamp(MID_MIN, MID_MAX) });
        let f = if x < m { 0.5 * x / m } else { 0.5 + 0.5 * (x - m) / (1.0 - m) };
        let (ca, cb) = (color(i), color(i + 1));
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

    /// One baked row: [`SAMPLES`] pixels, pixel `x` = `sample(x / (SAMPLES - 1))`, RGB sRGB-encoded, alpha stored
    /// as is (8 bits each, rounded). An empty slot is opaque white.
    pub fn bake_row(&self) -> [[u8; 4]; SAMPLES] {
        let byte = |v: f64| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
        std::array::from_fn(|x| {
            let t = x as f64 / (SAMPLES - 1) as f64;
            let c = self.sample_f64(t).unwrap_or([1.0; 4]);
            [byte(srgb_encode(c[0])), byte(srgb_encode(c[1])), byte(srgb_encode(c[2])), byte(c[3])]
        })
    }
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

// sRGB transfer function (IEC 61966-2-1), as `sg_linear_to_srgb` / `sg_srgb_to_linear`: negative input clamps to 0.

fn srgb_encode(c: f64) -> f64 {
    let x = c.max(0.0);
    if x <= 0.0031308 { x * 12.92 } else { 1.055 * x.powf(1.0 / 2.4) - 0.055 }
}

fn srgb_decode(c: f64) -> f64 {
    let x = c.max(0.0);
    if x <= 0.04045 { x / 12.92 } else { ((x + 0.055) / 1.055).powf(2.4) }
}

/// Linear → sRGB-encoded channel.
pub fn linear_to_srgb(c: f32) -> f32 {
    srgb_encode(f64::from(c)) as f32
}

/// sRGB-encoded → linear channel.
pub fn srgb_to_linear(c: f32) -> f32 {
    srgb_decode(f64::from(c)) as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    const RED: [f32; 4] = [1.0, 0.0, 0.0, 1.0];
    const GREEN: [f32; 4] = [0.0, 1.0, 0.0, 1.0];
    const BLUE: [f32; 4] = [0.0, 0.0, 1.0, 1.0];

    fn slot(stops: &[(f32, [f32; 4], f32)]) -> Slot {
        Slot {
            id: 1,
            name: String::new(),
            stops: stops.iter().map(|&(t, color, mid)| Stop { t, color, mid }).collect(),
        }
    }

    fn close(a: [f32; 4], b: [f32; 4]) -> bool {
        a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-5)
    }

    fn close3(a: [f32; 3], b: [f32; 3], eps: f32) -> bool {
        a.iter().zip(b).all(|(x, y)| (x - y).abs() < eps)
    }

    /// RGB mixed in OKLab, alpha mixed linearly.
    fn oklab_mix(a: [f32; 4], b: [f32; 4], f: f32) -> [f32; 4] {
        let (la, lb) = (linear_to_oklab([a[0], a[1], a[2]]), linear_to_oklab([b[0], b[1], b[2]]));
        let rgb = oklab_to_linear([0, 1, 2].map(|i| la[i] + (lb[i] - la[i]) * f));
        let [r, g, b2] = rgb.map(|c| c.clamp(0.0, 1.0));
        [r, g, b2, a[3] + (b[3] - a[3]) * f]
    }

    fn numbered() -> impl FnMut() -> u32 {
        let mut n = 0;
        move || {
            n += 1;
            n
        }
    }

    #[test]
    fn new_has_128_unique_valid_ids() {
        let mut seq = [0u32, 7, 7, u32::MAX].into_iter().chain(1..);
        let c = GradientCollection::new(|| seq.next().unwrap());
        assert_eq!(c.slots.len(), SLOTS);
        assert_eq!(c.slots[0].id, 7); // 0 re-drawn, duplicate 7 and u32::MAX re-drawn
        let ids: std::collections::BTreeSet<u32> = c.slots.iter().map(|s| s.id).collect();
        assert_eq!(ids.len(), SLOTS);
        assert!(ids.iter().all(|i| *i != 0 && *i <= i32::MAX as u32));
        assert!(c.slots.iter().all(Slot::is_empty));
        assert_eq!(c.format_version, GRADIENTS_FORMAT_VERSION);
        assert_eq!(c.position(c.slots[5].id), Some(5));
        assert_eq!(c.position(0), None);
    }

    #[test]
    fn random_ids_are_unique_and_valid() {
        let c = GradientCollection::random();
        let ids: std::collections::BTreeSet<u32> = c.slots.iter().map(|s| s.id).collect();
        assert_eq!(ids.len(), SLOTS);
        assert!(ids.iter().all(|i| *i != 0 && *i <= i32::MAX as u32));
    }

    #[test]
    fn roundtrip_and_normalize() {
        let mut c = GradientCollection::new(numbered());
        c.slots[0].name = "Terrain height".into();
        c.slots[0].stops = vec![
            Stop { t: 1.0, color: [2.0, -1.0, f32::NAN, 0.5], mid: 0.0 },
            Stop { t: 0.25, color: GREEN, mid: 0.3 },
            Stop { t: f32::NAN, color: RED, mid: 0.5 },
            Stop { t: 0.25, color: BLUE, mid: f32::NAN },
            Stop { t: -1.0, color: RED, mid: 2.0 },
            Stop { t: f32::INFINITY, color: RED, mid: 0.5 },
        ];
        c.normalize();
        assert_eq!(
            c.slots[0].stops,
            vec![
                Stop { t: 0.0, color: RED, mid: MID_MAX },
                Stop { t: 0.25, color: GREEN, mid: 0.3 }, // stable: GREEN stays before BLUE
                Stop { t: 0.25, color: BLUE, mid: 0.5 },
                Stop { t: 1.0, color: [1.0, 0.0, 0.0, 0.5], mid: MID_MIN },
            ]
        );
        let text = c.to_ron();
        assert!(text.starts_with("GradientCollection("), "{text}");
        assert!(!text.contains("mid: 0.5"), "{text}");
        assert!(!text.contains("name: \"\""), "{text}");
        assert!(!text.contains("stops: []"), "{text}");
        assert!(text.contains("Slot(id: 2)"), "{text}");
        assert_eq!(GradientCollection::from_ron(&text).unwrap(), c);
        // Loading normalizes too.
        let messy = text.replace("t: 0.25", "t: 7.0");
        let loaded = GradientCollection::from_ron(&messy).unwrap();
        assert!(loaded.slots[0].stops.iter().all(|s| (0.0..=1.0).contains(&s.t)));
        assert!(loaded.slots[0].stops.is_sorted_by(|a, b| a.t <= b.t));
    }

    #[test]
    fn load_rejects_structural_errors() {
        let base = GradientCollection::new(numbered());
        let parse_msg = |c: &GradientCollection| match GradientCollection::from_ron(&c.to_ron()) {
            Err(LoadError::Parse(msg)) => msg,
            other => panic!("expected a parse error, got {other:?}"),
        };

        let mut short = base.clone();
        short.slots.pop();
        assert!(parse_msg(&short).contains("128"), "{}", parse_msg(&short));

        let mut dup = base.clone();
        dup.slots[3].id = dup.slots[1].id;
        assert!(parse_msg(&dup).contains("duplicate"), "{}", parse_msg(&dup));

        let mut zero = base.clone();
        zero.slots[0].id = 0;
        assert!(parse_msg(&zero).contains("id"), "{}", parse_msg(&zero));

        let mut big = base.clone();
        big.slots[0].id = i32::MAX as u32 + 1;
        assert!(parse_msg(&big).contains("id"), "{}", parse_msg(&big));

        let mut many = base.clone();
        many.slots[2].stops = (0..=MAX_STOPS).map(|i| Stop { t: i as f32 / 40.0, color: RED, mid: 0.5 }).collect();
        assert!(parse_msg(&many).contains("32"), "{}", parse_msg(&many));
        many.slots[2].stops.pop();
        assert!(GradientCollection::from_ron(&many.to_ron()).is_ok());

        let too_new = "GradientCollection(format_version: 9, slots: [])";
        assert!(matches!(GradientCollection::from_ron(too_new), Err(LoadError::TooNew { .. })));
        assert!(matches!(GradientCollection::from_ron("garbage"), Err(LoadError::Parse(_))));
    }

    #[test]
    fn sampling_midpoint_and_ends() {
        let s = slot(&[(0.0, RED, 0.5), (1.0, BLUE, 0.5)]);
        assert_eq!(s.sample(-1.0), Some(RED)); // before first stop
        assert_eq!(s.sample(2.0), Some(BLUE));
        // Non-finite `t`: NaN and -inf give the first stop's color, +inf the last one's.
        assert_eq!(s.sample(f32::NAN), Some(RED));
        assert_eq!(s.sample(f32::NEG_INFINITY), Some(RED));
        assert_eq!(s.sample(f32::INFINITY), Some(BLUE));
        // mid 0.5 at t=0.5 equals the OKLab midpoint of RED and BLUE (alpha 1)
        let half = s.sample(0.5).unwrap();
        assert!(close(half, oklab_mix(RED, BLUE, 0.5)), "{half:?}");
        assert!(!close(half, [0.5, 0.0, 0.5, 1.0]), "not a plain linear mix: {half:?}");
        // mid 0.25: the 50/50 mix is reached at t=0.25
        let m = slot(&[(0.0, RED, 0.25), (1.0, BLUE, 0.5)]);
        assert!(close(m.sample(0.25).unwrap(), half));
        // ... and the remap is piecewise linear on both sides of the midpoint
        assert!(close(m.sample(0.625).unwrap(), oklab_mix(RED, BLUE, 0.75)));
        // alpha is a plain lerp: (0,a=1)→(1,a=0) at mid 0.5 gives a=0.5 at t=0.5
        let fade = slot(&[(0.0, [1.0, 1.0, 1.0, 1.0], 0.5), (1.0, [1.0, 1.0, 1.0, 0.0], 0.5)]);
        assert!((fade.sample(0.5).unwrap()[3] - 0.5).abs() < 1e-6);
        assert!(slot(&[]).sample(0.3).is_none());
        assert_eq!(slot(&[(0.4, GREEN, 0.5)]).sample(0.9), Some(GREEN));
        // Stops are hit exactly; the last stop's `mid` is ignored.
        let three = slot(&[(0.0, RED, 0.5), (0.5, GREEN, 0.5), (1.0, BLUE, 0.1)]);
        assert_eq!(three.sample(0.5), Some(GREEN));
        assert_eq!(three.sample(1.0), Some(BLUE));
    }

    #[test]
    fn equal_t_is_a_step_later_stop_wins() {
        let s = slot(&[(0.0, RED, 0.5), (0.5, RED, 0.5), (0.5, BLUE, 0.5), (1.0, BLUE, 0.5)]);
        assert_eq!(s.sample(0.49999), Some(RED));
        assert_eq!(s.sample(0.5), Some(BLUE));
    }

    #[test]
    fn oklab_roundtrip() {
        for c in [[0.0, 0.0, 0.0], [1.0, 1.0, 1.0], [0.2, 0.5, 0.9]] {
            assert!(close3(oklab_to_linear(linear_to_oklab(c)), c, 1e-4));
        }
        // White is L = 1, a = b = 0.
        assert!(close3(linear_to_oklab([1.0, 1.0, 1.0]), [1.0, 0.0, 0.0], 1e-4));
    }

    #[test]
    fn srgb_transfer() {
        assert_eq!(linear_to_srgb(0.0), 0.0);
        assert!((linear_to_srgb(1.0) - 1.0).abs() < 1e-6);
        assert!((linear_to_srgb(0.214_041_14) - 0.5).abs() < 1e-5);
        for c in [0.001, 0.02, 0.5, 0.9] {
            assert!((srgb_to_linear(linear_to_srgb(c)) - c).abs() < 1e-5);
        }
    }

    #[test]
    fn bake_layout() {
        let mut c = GradientCollection::new(numbered());
        c.slots[1].stops =
            vec![Stop { t: 0.0, color: [0.0, 0.0, 0.0, 1.0], mid: 0.5 }, Stop { t: 1.0, color: [1.0; 4], mid: 0.5 }];
        let bytes = c.bake_rgba8();
        assert_eq!(bytes.len(), 128 * 128 * 4);
        assert_eq!(&bytes[0..4], &[255, 255, 255, 255]); // empty slot 0 → white
        let row1 = 128 * 4;
        assert_eq!(&bytes[row1..row1 + 4], &[0, 0, 0, 255]); // pixel 0 = t 0
        assert_eq!(&bytes[row1 + 127 * 4..row1 + 128 * 4], &[255, 255, 255, 255]); // pixel 127 = t 1
        assert_eq!(c.bake_rgba8(), bytes); // deterministic
        assert_eq!(&bytes[row1..row1 + 128 * 4], c.slots[1].bake_row().as_flattened());
        // sRGB-encoded: the OKLab mid-gray of black → white is L = 0.5, i.e. linear 0.125, sRGB ≈ 0.39.
        let mid = c.slots[1].sample(0.5).unwrap();
        assert!((mid[0] - 0.125).abs() < 1e-3, "{mid:?}");
        // Alpha is stored as is, not sRGB-encoded.
        c.slots[2].stops = vec![Stop { t: 0.0, color: [1.0, 1.0, 1.0, 0.5], mid: 0.5 }];
        assert_eq!(c.slots[2].bake_row()[0][3], 128);
    }

    #[test]
    fn texture_id_is_prefixed() {
        assert_eq!(texture_id("common/terrain"), "@gradients/common/terrain");
    }
}
