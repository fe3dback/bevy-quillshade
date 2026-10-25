//! Module paths of autogen files and relative WESL imports between them.
//!
//! Bevy maps `assets/shaders/autogen/dir_env/mat_rock.wesl` to module
//! `package::shaders::autogen::dir_env::mat_rock`; `super::` drops the last component of the importing module
//! (spec §10.2, verified in M0 spike). User path components are prefixed (`dir_`, `mat_`, `fun_`, `crv_`;
//! decision 0020), so keywords are valid names and generated modules never clash with each other.

use shader_graph_core::ident::{curve_file_ident, dir_ident, function_ident, material_file_ident};

/// Asset path `env/rock_wet` → raw components (`["env", "rock_wet"]`).
pub fn split(path: &str) -> Vec<String> {
    path.split('/').filter(|s| !s.is_empty()).map(str::to_string).collect()
}

/// Directory components `dir_*`, last component through `leaf`.
fn prefixed(path: &str, leaf: fn(&str) -> String) -> Vec<String> {
    let parts = split(path);
    let Some((name, dirs)) = parts.split_last() else { return Vec::new() };
    dirs.iter().map(|d| dir_ident(d)).chain([leaf(name)]).collect()
}

/// Module components of a material below `autogen/` and `src/materials/`: `env/rock_wet` → `["dir_env",
/// "mat_rock_wet"]`. Shared by the WESL and Rust generators.
pub fn material_components(path: &str) -> Vec<String> {
    prefixed(path, material_file_ident)
}

pub const AUTOGEN: [&str; 2] = ["shaders", "autogen"];

pub fn material_module(path: &str) -> Vec<String> {
    let mut m: Vec<String> = AUTOGEN.iter().map(|s| s.to_string()).collect();
    m.extend(material_components(path));
    m
}

pub fn function_module(path: &str) -> Vec<String> {
    let mut m: Vec<String> = AUTOGEN.iter().map(|s| s.to_string()).collect();
    m.push("functions".into());
    m.extend(prefixed(path, function_ident));
    m
}

pub fn curve_module(path: &str) -> Vec<String> {
    let mut m: Vec<String> = AUTOGEN.iter().map(|s| s.to_string()).collect();
    m.push("curves".into());
    m.extend(prefixed(path, curve_file_ident));
    m
}

pub fn stdlib_module(name: &str) -> Vec<String> {
    let mut m: Vec<String> = AUTOGEN.iter().map(|s| s.to_string()).collect();
    m.push("stdlib".into());
    m.push(name.into());
    m
}

/// Asset-relative file of a module (`shaders/autogen/dir_env/mat_rock.wesl`).
pub fn module_file(module: &[String]) -> String {
    format!("{}.wesl", module.join("/"))
}

/// Import prefix that reaches module `to` from module `from` (`super::super::stdlib::math`).
pub fn relative(from: &[String], to: &[String]) -> String {
    let mut p = from.iter().zip(to).take_while(|(a, b)| a == b).count().min(from.len().saturating_sub(1));
    while p > 0 && to.len() <= p {
        p -= 1;
    }
    let k = from.len() - p;
    if k == 0 || p == 0 {
        return format!("package::{}", to.join("::"));
    }
    let mut parts: Vec<String> = std::iter::repeat_n("super".to_string(), k).collect();
    parts.extend(to[p..].iter().cloned());
    parts.join("::")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(s: &str) -> Vec<String> {
        s.split("::").map(str::to_string).collect()
    }

    #[test]
    fn prefixed_modules() {
        assert_eq!(material_components("env/rock_wet"), ["dir_env", "mat_rock_wet"]);
        assert_eq!(material_module("loop/type"), v("shaders::autogen::dir_loop::mat_type"));
        assert_eq!(function_module("common/fn"), v("shaders::autogen::functions::dir_common::fun_fn"));
        assert_eq!(curve_module("pulse"), v("shaders::autogen::curves::crv_pulse"));
        assert_eq!(curve_module("common/pulse"), v("shaders::autogen::curves::dir_common::crv_pulse"));
        assert_eq!(stdlib_module("math"), v("shaders::autogen::stdlib::math"));
    }

    #[test]
    fn relative_imports() {
        // The spec example (autogen/env/rock_wet.wesl → autogen/stdlib/math.wesl), with prefixed components.
        let rock = material_module("env/rock_wet");
        assert_eq!(rock, v("shaders::autogen::dir_env::mat_rock_wet"));
        assert_eq!(relative(&rock, &stdlib_module("math")), "super::super::stdlib::math");
        assert_eq!(
            relative(&rock, &function_module("common/wet_mask")),
            "super::super::functions::dir_common::fun_wet_mask"
        );
        assert_eq!(relative(&material_module("rock"), &stdlib_module("math")), "super::stdlib::math");
        // Sibling.
        assert_eq!(relative(&function_module("a"), &function_module("b")), "super::fun_b");
        assert_eq!(
            relative(&material_module("kw/loop/type"), &curve_module("kw/pulse")),
            "super::super::super::curves::dir_kw::crv_pulse"
        );
        // Module named like an ancestor directory (cannot happen with prefixes, but `relative` handles it).
        assert_eq!(relative(&v("shaders::autogen::env::rock"), &v("shaders::autogen::env")), "super::super::env");
    }
}
