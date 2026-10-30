//! WESL side of curve assets (`quillshade/material/**/*.curve.ron`, decision 0015): the model and math live in
//! `curve_kit` (design §7.1) and are re-exported here.

pub use curve_kit::*;

/// WESL function names of a curve interpolation mode.
pub trait InterpWesl {
    /// Function of a generated curve module (`sample_curve_simple`, `sample_curve_catmullrom`).
    fn wesl_fn(self) -> &'static str;
}

impl InterpWesl for Interp {
    fn wesl_fn(self) -> &'static str {
        match self {
            Interp::Simple => "sample_curve_simple",
            Interp::CatmullRom => "sample_curve_catmullrom",
        }
    }
}

/// Suffix of the import alias in shaders using the curve.
fn alias_suffix(interp: Interp) -> &'static str {
    match interp {
        Interp::Simple => "simple",
        Interp::CatmullRom => "catmullrom",
    }
}

/// Name a shader imports `interp`'s function of curve `path` as: unique per curve, so one shader can sample
/// several curves (`env/falloff` → `sg_curve_env__falloff_simple`).
pub fn curve_fn_alias(path: &str, interp: Interp) -> String {
    format!("sg_curve_{}_{}", path.replace('/', "__"), alias_suffix(interp))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aliases_are_unique_per_curve_and_mode() {
        assert_eq!(curve_fn_alias("env/falloff", Interp::Simple), "sg_curve_env__falloff_simple");
        assert_eq!(curve_fn_alias("ramp", Interp::CatmullRom), "sg_curve_ramp_catmullrom");
        assert_eq!(Interp::from_label("Catmull-Rom"), Some(Interp::CatmullRom));
    }
}
