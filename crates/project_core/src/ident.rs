//! Identifier helpers shared by all domains.

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

/// Shape check only: `[a-z][a-z0-9_]*`, no `__`, no trailing `_`. Keywords are fine because generated code
/// prefixes every user name (decision 0020). Returns a reason on failure.
pub fn check_ident(name: &str) -> Result<(), String> {
    let mut chars = name.chars();
    match chars.next() {
        Some(c) if c.is_ascii_lowercase() => {}
        _ => return Err(format!("`{name}` must start with a lowercase latin letter")),
    }
    if !chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_') {
        return Err(format!("`{name}` must match [a-z][a-z0-9_]* (snake_case)"));
    }
    if name.contains("__") || name.ends_with('_') {
        return Err(format!("`{name}` must not contain `__` or end with `_`"));
    }
    Ok(())
}

/// A path of `/`-separated [`check_ident`] names (an asset path or a folder of a domain's Project tree, the same rule
/// for every domain). Returns a reason on failure.
pub fn check_path(path: &str) -> Result<(), String> {
    if path.is_empty() {
        return Err("empty name".into());
    }
    path.split('/').try_for_each(check_ident)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_are_checked_per_component() {
        assert!(check_path("env/rock_wet").is_ok());
        assert_eq!(check_path(""), Err("empty name".to_string()));
        assert!(check_path("env/Rock").is_err());
        assert!(check_path("env//x").is_err());
    }

    #[test]
    fn snake_case() {
        assert_eq!(to_snake_case("Base Tint"), "base_tint");
        assert_eq!(to_snake_case("albedoMap"), "albedo_map");
        assert_eq!(to_snake_case("a__b"), "a_b");
    }
}
