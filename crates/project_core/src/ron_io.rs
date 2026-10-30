//! RON loading helpers shared by all asset formats.

use serde::Deserialize;

#[derive(Debug, thiserror::Error)]
pub enum LoadError {
    #[error("parse error: {0}")]
    Parse(String),
    #[error("format_version {found} is newer than supported {supported}; update the editor")]
    TooNew { found: u32, supported: u32 },
    #[error("unsupported format_version {0}")]
    Unsupported(u32),
}

/// Struct-name-agnostic probe: a map visitor that keeps `format_version` and skips every other field.
struct VersionProbe(u32);

impl<'de> Deserialize<'de> for VersionProbe {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V;
        impl<'de> serde::de::Visitor<'de> for V {
            type Value = VersionProbe;

            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("a struct with `format_version`")
            }

            fn visit_map<A: serde::de::MapAccess<'de>>(self, mut map: A) -> Result<VersionProbe, A::Error> {
                let mut version = None;
                while let Some(key) = map.next_key::<String>()? {
                    if key == "format_version" {
                        version = Some(map.next_value::<u32>()?);
                    } else {
                        map.next_value::<serde::de::IgnoredAny>()?;
                    }
                }
                version.map(VersionProbe).ok_or_else(|| serde::de::Error::missing_field("format_version"))
            }
        }
        d.deserialize_any(V)
    }
}

/// Reads `format_version` of any struct-named RON document.
pub fn probe_version(text: &str) -> Result<u32, LoadError> {
    let probe: VersionProbe = ron::Options::default().from_str(text).map_err(|e| LoadError::Parse(e.to_string()))?;
    Ok(probe.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn probes_version_of_any_struct_name() {
        assert_eq!(probe_version("Asset(format_version: 3, kind: Function)").unwrap(), 3);
        assert_eq!(probe_version("Curve(format_version: 1)").unwrap(), 1);
        assert!(matches!(probe_version("garbage"), Err(LoadError::Parse(_))));
    }
}
