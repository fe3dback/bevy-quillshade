//! Module paths of autogen files and relative WESL imports between them.
//!
//! Bevy maps `assets/shaders/autogen/env/rock.wesl` to module `package::shaders::autogen::env::rock`;
//! `super::` drops the last component of the importing module (spec §10.2, verified in M0 spike).

/// Asset path `env/rock_wet` → components.
pub fn split(path: &str) -> Vec<String> {
    path.split('/').filter(|s| !s.is_empty()).map(str::to_string).collect()
}

pub const AUTOGEN: [&str; 2] = ["shaders", "autogen"];

pub fn material_module(path: &str) -> Vec<String> {
    let mut m: Vec<String> = AUTOGEN.iter().map(|s| s.to_string()).collect();
    m.extend(split(path));
    m
}

pub fn function_module(path: &str) -> Vec<String> {
    let mut m: Vec<String> = AUTOGEN.iter().map(|s| s.to_string()).collect();
    m.push("functions".into());
    m.extend(split(path));
    m
}

pub fn stdlib_module(name: &str) -> Vec<String> {
    let mut m: Vec<String> = AUTOGEN.iter().map(|s| s.to_string()).collect();
    m.push("stdlib".into());
    m.push(name.into());
    m
}

/// Asset-relative file of a module (`shaders/autogen/env/rock.wesl`).
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
    fn relative_imports() {
        // Matches the spec example: autogen/env/rock_wet.wesl → autogen/stdlib/math.wesl.
        assert_eq!(
            relative(&v("shaders::autogen::env::rock_wet"), &v("shaders::autogen::stdlib::math")),
            "super::super::stdlib::math"
        );
        assert_eq!(
            relative(&v("shaders::autogen::env::rock_wet"), &v("shaders::autogen::functions::common::wet_mask")),
            "super::super::functions::common::wet_mask"
        );
        assert_eq!(relative(&v("shaders::autogen::rock"), &v("shaders::autogen::stdlib::math")), "super::stdlib::math");
        // Sibling.
        assert_eq!(relative(&v("shaders::autogen::functions::a"), &v("shaders::autogen::functions::b")), "super::b");
        // File named like an ancestor directory.
        assert_eq!(relative(&v("shaders::autogen::env::rock"), &v("shaders::autogen::env")), "super::super::env");
    }
}
