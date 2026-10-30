//! Asset identity across domains (design §5): domain + domain-relative path + file suffix.

use std::fmt;

use crate::domain::DomainId;

/// An asset of any domain. `path` is domain-relative and has no suffix (`env/rock_wet`).
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct AssetRef {
    pub domain: DomainId,
    pub path: String,
    pub suffix: &'static str,
}

impl AssetRef {
    /// `quillshade/<domain>/<path>.<suffix>.ron`, relative to the project root.
    pub fn source_rel(&self) -> String {
        format!("quillshade/{}/{}.{}.ron", self.domain, self.path, self.suffix)
    }
}

impl fmt::Display for AssetRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.source_rel())
    }
}

/// Inverse of [`AssetRef::source_rel`] for `domain`: `None` for another domain's folder or a suffix not in
/// `suffixes`.
pub fn parse(rel: &str, domain: DomainId, suffixes: &[&'static str]) -> Option<AssetRef> {
    let rel = rel.replace('\\', "/");
    let rest = rel.strip_prefix("quillshade/")?.strip_prefix(domain.as_str())?.strip_prefix('/')?;
    let stem = rest.strip_suffix(".ron")?;
    suffixes.iter().find_map(|&suffix| {
        let path = stem.strip_suffix(suffix)?.strip_suffix('.')?;
        Some(AssetRef { domain, path: path.to_string(), suffix })
    })
}
#[cfg(test)]
mod tests {
    use super::*;

    const SUFFIXES: &[&str] = &["material", "curve"];

    #[test]
    fn round_trip() {
        let d = DomainId::new("material");
        let r = AssetRef { domain: d, path: "env/rock_wet".into(), suffix: "material" };
        assert_eq!(r.source_rel(), "quillshade/material/env/rock_wet.material.ron");
        assert_eq!(r.to_string(), r.source_rel());
        assert_eq!(parse(&r.source_rel(), d, SUFFIXES), Some(r));
        assert_eq!(parse("quillshade/light/a.curve.ron", d, SUFFIXES), None);
        assert_eq!(parse("quillshade/material/a.unknown.ron", d, SUFFIXES), None);
        assert_eq!(parse("quillshade/materialx/a.curve.ron", d, SUFFIXES), None);
    }
}
