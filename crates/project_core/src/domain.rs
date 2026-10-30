//! Identifier of an asset domain (`material`, `light`, …).

use std::fmt;

#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
pub struct DomainId(&'static str);

impl DomainId {
    /// The editor's built-in project domain (design §14.2): reserved, no domain may use it; it has no folder, no
    /// backend and is unknown to the CLI.
    pub const PROJECT: DomainId = DomainId::new("project");

    pub const fn new(id: &'static str) -> DomainId {
        DomainId(id)
    }

    pub fn as_str(self) -> &'static str {
        self.0
    }
}

impl fmt::Display for DomainId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0)
    }
}

/// Valid ids match `[a-z][a-z0-9_]*`.
pub fn is_valid_id(s: &str) -> bool {
    let mut chars = s.chars();
    chars.next().is_some_and(|c| c.is_ascii_lowercase())
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn domain_ids() {
        assert!(
            is_valid_id("material")
                && is_valid_id("light2")
                && !is_valid_id("Light")
                && !is_valid_id("2d")
                && !is_valid_id("")
        );
        assert_eq!(DomainId::new("light").to_string(), "light");
        assert_eq!(DomainId::new("light").as_str(), "light");
    }
}
