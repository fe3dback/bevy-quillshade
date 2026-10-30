//! Generic diagnostics shared by all domains.

use std::fmt;

use crate::domain::DomainId;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Level {
    Error,
    Warning,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Diagnostic {
    pub domain: Option<DomainId>,
    pub code: String,
    pub level: Level,
    pub message: String,
    /// Project-relative asset path.
    pub asset: Option<String>,
    /// Place inside the asset (`n05.color`).
    pub location: Option<String>,
}

impl Diagnostic {
    fn new(level: Level, code: &str, msg: impl Into<String>) -> Self {
        Diagnostic { domain: None, code: code.to_string(), level, message: msg.into(), asset: None, location: None }
    }

    pub fn error(code: &str, msg: impl Into<String>) -> Self {
        Self::new(Level::Error, code, msg)
    }

    pub fn warning(code: &str, msg: impl Into<String>) -> Self {
        Self::new(Level::Warning, code, msg)
    }

    pub fn in_asset(mut self, asset: impl Into<String>) -> Self {
        self.asset = Some(asset.into());
        self
    }

    pub fn at(mut self, location: impl Into<String>) -> Self {
        self.location = Some(location.into());
        self
    }

    pub fn in_domain(mut self, domain: DomainId) -> Self {
        self.domain = Some(domain);
        self
    }

    pub fn is_error(&self) -> bool {
        self.level == Level::Error
    }
}

impl fmt::Display for Diagnostic {
    /// CI-friendly `asset:location:code: text`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}:{}:{}: {}",
            self.asset.as_deref().unwrap_or("-"),
            self.location.as_deref().unwrap_or("-"),
            self.code,
            self.message
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diagnostic_display_and_level() {
        let d = Diagnostic::error("E003", "bad").in_asset("quillshade/material/a.material.ron").at("n05.color");
        assert_eq!(d.to_string(), "quillshade/material/a.material.ron:n05.color:E003: bad");
        assert!(d.is_error());
        assert_eq!(Diagnostic::warning("W105", "x").to_string(), "-:-:W105: x");
    }
}
