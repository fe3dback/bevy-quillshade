//! Identifier rules shared by asset names and parameter names (spec §5, 11.3).

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

/// Checks `[a-z][a-z0-9_]*` and keyword/reserved-name rules. Returns a reason on failure.
pub fn check_ident(name: &str) -> Result<(), String> {
    let mut chars = name.chars();
    match chars.next() {
        Some(c) if c.is_ascii_lowercase() => {}
        _ => return Err(format!("`{name}` must start with a lowercase latin letter")),
    }
    if !chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_') {
        return Err(format!("`{name}` must match [a-z][a-z0-9_]* (snake_case)"));
    }
    if RESERVED.contains(&name) {
        return Err(format!("`{name}` is a reserved word in Rust/WGSL or generated code"));
    }
    if let Some(p) = RESERVED_PREFIXES.iter().find(|p| name.starts_with(*p)) {
        return Err(format!("`{name}` uses reserved prefix `{p}`"));
    }
    if name.contains("__") || name.ends_with('_') {
        return Err(format!("`{name}` must not contain `__` or end with `_`"));
    }
    Ok(())
}

/// Converts a free-form display name ("Base Tint", "albedoMap") into snake_case.
pub fn to_snake_case(name: &str) -> String {
    let mut out = String::new();
    let mut prev_lower = false;
    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            if c.is_ascii_uppercase() && prev_lower && !out.ends_with('_') {
                out.push('_');
            }
            out.push(c.to_ascii_lowercase());
            prev_lower = c.is_ascii_lowercase() || c.is_ascii_digit();
        } else {
            if !out.is_empty() && !out.ends_with('_') {
                out.push('_');
            }
            prev_lower = false;
        }
    }
    out.trim_matches('_').to_string()
}

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
        assert!(check_ident("rock_wet").is_ok());
        assert!(check_ident("a1").is_ok());
        assert!(check_ident("Rock").is_err());
        assert!(check_ident("1a").is_err());
        assert!(check_ident("fn").is_err());
        assert!(check_ident("vec3").is_err());
        assert!(check_ident("n_foo").is_err());
        assert!(check_ident("a-b").is_err());
        assert_eq!(to_snake_case("Base Tint"), "base_tint");
        assert_eq!(to_snake_case("albedoMap"), "albedo_map");
        assert_eq!(to_pascal_case("rock_wet"), "RockWet");
    }
}
