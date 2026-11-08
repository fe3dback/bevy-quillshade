use bevy::color::{Color, ColorToComponents, LinearRgba};

/// A single colour stop in a [`GradientData`] (with an optional segment midpoint in `[0.01, 0.99]`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GradientStop {
    pub t: f32,
    pub color: Color,
    /// Segment midpoint (`None` uses [`DEFAULT_MID`] = `0.5`; the last stop's midpoint is ignored).
    pub mid: Option<f32>,
}

impl From<RawStop> for GradientStop {
    fn from(s: RawStop) -> Self {
        Self { t: s.t, color: Color::LinearRgba(LinearRgba::from_f32_array(s.color)), mid: Some(s.mid) }
    }
}

impl From<&GradientStop> for RawStop {
    fn from(s: &GradientStop) -> Self {
        Self { t: s.t, color: s.color.to_linear().to_f32_array(), mid: s.mid.unwrap_or(DEFAULT_MID) }
    }
}

impl From<GradientStop> for RawStop {
    fn from(s: GradientStop) -> Self {
        Self::from(&s)
    }
}

/// Baked 1D OKLab colour gradient evaluated at runtime (`sample(t) -> Color`).
// Wire format: serialized as `RawGradient` (never reorder fields of `RawGradient` / `RawStop`).
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(from = "RawGradient", into = "RawGradient")]
pub struct GradientData {
    pub stops: Vec<GradientStop>,
}

impl GradientData {
    /// Evaluates the gradient at `t` in OKLab space, returning [`Color::WHITE`] when `stops` is empty.
    pub fn sample(&self, t: f32) -> Color {
        match sample_stops_f64(
            self.stops.len(),
            |k| {
                let s = &self.stops[k];
                (s.t, s.color.to_linear().to_f32_array(), s.mid.unwrap_or(DEFAULT_MID))
            },
            f64::from(t),
        ) {
            Some(c) => Color::LinearRgba(LinearRgba::from_f32_array(c.map(|v| v as f32))),
            None => Color::WHITE,
        }
    }
}

impl From<RawGradient> for GradientData {
    fn from(raw: RawGradient) -> Self {
        Self { stops: raw.stops.into_iter().map(GradientStop::from).collect() }
    }
}

impl From<&GradientData> for RawGradient {
    fn from(g: &GradientData) -> Self {
        Self { stops: g.stops.iter().map(RawStop::from).collect() }
    }
}

impl From<GradientData> for RawGradient {
    fn from(g: GradientData) -> Self {
        Self::from(&g)
    }
}
