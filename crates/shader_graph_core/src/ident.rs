//! Identifier rules shared by asset names and parameter names (spec §4, 5, 11.3; decision 0020).
//! Names are shape-checked only; generated code prefixes them (`inp_`, `out_`, `fun_`, `dir_`, `mat_`, `crv_`).

/// Rust keywords (strict + reserved) and WGSL keywords / reserved words that could clash.
const RESERVED: &[&str] = &[
    // Rust
    "as",
    "break",
    "const",
    "continue",
    "crate",
    "else",
    "enum",
    "extern",
    "false",
    "fn",
    "for",
    "if",
    "impl",
    "in",
    "let",
    "loop",
    "match",
    "mod",
    "move",
    "mut",
    "pub",
    "ref",
    "return",
    "self",
    "static",
    "struct",
    "super",
    "trait",
    "true",
    "type",
    "unsafe",
    "use",
    "where",
    "while",
    "async",
    "await",
    "dyn",
    "abstract",
    "become",
    "box",
    "do",
    "final",
    "macro",
    "override",
    "priv",
    "typeof",
    "unsized",
    "virtual",
    "yield",
    "try",
    "gen",
    // WGSL / WESL
    "alias",
    "case",
    "const_assert",
    "default",
    "diagnostic",
    "discard",
    "enable",
    "loop",
    "requires",
    "switch",
    "var",
    "bool",
    "f16",
    "f32",
    "i32",
    "u32",
    "vec2",
    "vec3",
    "vec4",
    "mat2x2",
    "mat3x3",
    "mat4x4",
    "array",
    "atomic",
    "sampler",
    "sampler_comparison",
    "texture_2d",
    "ptr",
    "function",
    "private",
    "storage",
    "uniform",
    "workgroup",
    "import",
    "package",
    "attribute",
    "varying",
    "precision",
    "mesh",
    "export",
    "module",
    "this",
    "null",
    // Names used by generated code
    "params",
    "out",
    "pbr_input",
    "fragment",
    "vertex",
    "globals",
    "view",
    "constants",
    "default",
    "new",
    "material",
    "register",
];

/// Prefixes reserved for generated identifiers.
const RESERVED_PREFIXES: &[&str] = &["n_", "b_", "sg_", "_"];

/// `check_ident` plus keyword/reserved-name and reserved-prefix rules. For names embedded verbatim in user
/// WGSL (custom-node inputs), which no prefix protects.
pub fn check_code_ident(name: &str) -> Result<(), String> {
    check_ident(name)?;
    if RESERVED.contains(&name) {
        return Err(format!("`{name}` is a reserved word in Rust/WGSL or generated code"));
    }
    if let Some(p) = RESERVED_PREFIXES.iter().find(|p| name.starts_with(*p)) {
        return Err(format!("`{name}` uses reserved prefix `{p}`"));
    }
    Ok(())
}

/// WGSL name of a user input / parameter (`inp_{name}`).
pub fn input_ident(name: &str) -> String {
    format!("inp_{name}")
}

/// WGSL name of a user output (`out_{name}`).
pub fn output_ident(name: &str) -> String {
    format!("out_{name}")
}

/// WGSL name of a function asset (`fun_{name}`).
pub fn function_ident(name: &str) -> String {
    format!("fun_{name}")
}

/// Module name of a directory (`dir_{name}`).
pub fn dir_ident(name: &str) -> String {
    format!("dir_{name}")
}

/// Module name of a material file (`mat_{name}`).
pub fn material_file_ident(name: &str) -> String {
    format!("mat_{name}")
}

/// Module name of a curve file (`crv_{name}`).
pub fn curve_file_ident(name: &str) -> String {
    format!("crv_{name}")
}

pub use project_core::ident::{check_ident, to_snake_case};

/// `rock_wet` → `RockWet`.
pub fn to_pascal_case(name: &str) -> String {
    name.split('_')
        .filter(|s| !s.is_empty())
        .map(|s| {
            let mut c = s.chars();
            match c.next() {
                Some(f) => f.to_ascii_uppercase().to_string() + c.as_str(),
                None => String::new(),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn idents() {
        for ok in ["rock_wet", "a1", "fn", "type", "loop", "self", "vec3", "n_foo", "params"] {
            assert!(check_ident(ok).is_ok(), "{ok}");
        }
        for bad in ["Rock", "1a", "a-b", "a__b", "a_", ""] {
            assert!(check_ident(bad).is_err(), "{bad}");
        }
        for bad in ["fn", "vec3", "loop", "n_foo", "_x"] {
            assert!(check_code_ident(bad).is_err(), "{bad}");
        }
        assert!(check_code_ident("strength").is_ok());
        assert_eq!(input_ident("type"), "inp_type");
        assert_eq!(output_ident("out"), "out_out");
        assert_eq!(function_ident("fn"), "fun_fn");
        assert_eq!(
            (dir_ident("loop"), material_file_ident("type"), curve_file_ident("pulse")),
            ("dir_loop".into(), "mat_type".into(), "crv_pulse".into())
        );
        assert_eq!(to_snake_case("Base Tint"), "base_tint");
        assert_eq!(to_snake_case("albedoMap"), "albedo_map");
        assert_eq!(to_pascal_case("rock_wet"), "RockWet");
    }
}
