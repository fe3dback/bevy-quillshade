//! Open tabs (spec §9 "State", design §6.2). A tab holds one document of some domain, shown by the view of its kind
//! ([`TabDoc::view`]); every view fills the editor content (dark background, file type watermark in the
//! bottom-right corner). See decision 0014.

use std::any::Any;
use std::fmt;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use project_core::{AssetRef, DomainId, Project};

use crate::doc_kind::TabDoc;
use crate::domain::{TOOL_KEY_PREFIX, ToolId};

/// The view that shows a tab's content: one per editor kind (`ViewKindId("my_domain.notes")`), defined next to the kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ViewKindId(pub &'static str);

/// A tab's identity across sessions and in the navigation history (design §6.2, §6.6): its domain and the key its
/// document gives ([`TabDoc::config_key`]), which only that domain parses back; for a tool tab `"@tool/<id>"`
/// (design §14.1, [`Tab::key`]), which core opens itself. Stored in the user config as
/// `"<domain>:<key>"` ([`fmt::Display`]); reading one back needs the registered domains (`DomainList`).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TabKey {
    pub domain: DomainId,
    pub key: String,
}

impl TabKey {
    pub fn new(domain: DomainId, key: impl Into<String>) -> TabKey {
        TabKey { domain, key: key.into() }
    }

    /// The key of tool `id`'s tab (design §14.1): `"@tool/<id>"`.
    pub fn tool(domain: DomainId, id: ToolId) -> TabKey {
        TabKey::new(domain, format!("{TOOL_KEY_PREFIX}{id}"))
    }

    /// This key after the tab `from` became `to` (an asset was renamed, design §14.4): `to` for `from` itself, and
    /// `<to>#<view>` for a view derived from it (`<from>#<view>`, Material's generated files); `None` for any other
    /// key.
    pub fn renamed(&self, from: &TabKey, to: &TabKey) -> Option<TabKey> {
        if self.domain != from.domain {
            return None;
        }
        if self.key == from.key {
            return Some(to.clone());
        }
        let view = self.key.strip_prefix(from.key.as_str())?.strip_prefix('#')?;
        Some(TabKey::new(to.domain, format!("{}#{view}", to.key)))
    }

    /// Splits a config entry into its domain prefix and key: `None` when there is no `<id>:` prefix with a valid
    /// domain id (an entry written before domains existed, design §6.6).
    pub fn split(entry: &str) -> Option<(&str, &str)> {
        entry.split_once(':').filter(|(id, _)| project_core::is_valid_id(id))
    }
}

impl fmt::Display for TabKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.domain, self.key)
    }
}

/// A hook a domain gives a [`TextDoc`]: called with the domain's state after the file was re-read.
pub type TextReloaded = fn(&mut dyn Any, &TextDoc);

/// Read-only view of a text file generated from an asset ("Show generated WESL / Rust"), highlighted by the
/// highlighter registered for [`TextDoc::lang`] (design §6.2). The domain says which file it is.
#[derive(Debug, Clone)]
pub struct TextDoc {
    /// The asset the file is generated from (the tab's owner).
    pub source: AssetRef,
    /// The language (and the key's suffix): `"rust"`, `"wesl"`, …
    pub lang: &'static str,
    pub path: PathBuf,
    /// Project-relative path (tab title).
    pub rel: String,
    /// File contents; `None` while the file does not exist.
    pub text: Option<String>,
    /// Set to a fresh value whenever `text` changes; the view rebuilds when it differs.
    pub revision: u64,
    on_reload: Option<TextReloaded>,
}

impl TextDoc {
    /// A process-wide unique revision: the view caches on `(rel, revision)`, so a tab closed and reopened on a
    /// regenerated file (a preview toggle, decision 0022) must not reuse the closed tab's revision.
    fn next_revision() -> u64 {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        NEXT.fetch_add(1, Ordering::Relaxed)
    }

    /// The tab of `path` (generated from `source`, in language `lang`), read now.
    pub fn open(project: &Project, source: AssetRef, lang: &'static str, path: PathBuf) -> TextDoc {
        let mut doc = TextDoc { source, lang, rel: project.rel(&path), path, text: None, revision: 0, on_reload: None };
        doc.reload();
        doc
    }

    /// Calls `hook` with the domain's state whenever the editor re-reads the file ([`TabDoc::reload_from_disk`]).
    pub fn with_reload_hook(mut self, hook: TextReloaded) -> TextDoc {
        self.on_reload = Some(hook);
        self
    }

    /// Re-reads the file; returns whether the contents changed.
    pub fn reload(&mut self) -> bool {
        let text = std::fs::read_to_string(&self.path).ok();
        if text == self.text && self.revision > 0 {
            return false;
        }
        self.text = text;
        self.revision = Self::next_revision();
        true
    }

    /// Follows the source asset to a new name: the generated file is `path` now (the domain computes it; the
    /// regeneration fills it).
    pub fn retarget(&mut self, project: &Project, source: AssetRef, path: PathBuf) {
        self.source = source;
        self.rel = project.rel(&path);
        self.path = path;
        self.text = None;
        self.reload();
    }

    pub(crate) fn reloaded(&self, domain: &mut dyn Any) {
        if let Some(hook) = self.on_reload {
            hook(domain, self);
        }
    }
}

/// One open tab: the domain it belongs to and its document.
pub struct Tab {
    pub domain: DomainId,
    pub doc: Box<dyn TabDoc>,
}

impl fmt::Debug for Tab {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Tab").field("domain", &self.domain).field("title", &self.title()).finish()
    }
}

impl Tab {
    pub fn new(domain: DomainId, doc: impl TabDoc) -> Tab {
        Tab { domain, doc: Box::new(doc) }
    }

    /// The content as a [`TabDoc`], whatever its kind.
    pub fn doc(&self) -> &dyn TabDoc {
        self.doc.as_ref()
    }

    pub fn doc_mut(&mut self) -> &mut dyn TabDoc {
        self.doc.as_mut()
    }

    /// The document as `T`; `None` for a tab of another kind.
    pub fn get<T: TabDoc>(&self) -> Option<&T> {
        (self.doc.as_ref() as &dyn Any).downcast_ref()
    }

    pub fn get_mut<T: TabDoc>(&mut self) -> Option<&mut T> {
        (self.doc.as_mut() as &mut dyn Any).downcast_mut()
    }

    /// The tab's identity (Back / Forward, finding an open tab): `"@tool/<id>"` for a tool tab (design §14.1), else
    /// its document's [`TabDoc::config_key`].
    pub fn key(&self) -> Option<TabKey> {
        match self.doc.tool() {
            Some(id) => Some(TabKey::tool(self.domain, id)),
            None => self.doc.config_key().map(|k| TabKey::new(self.domain, k)),
        }
    }

    /// The asset the tab belongs to ([`TabDoc::owner`]).
    pub fn owner(&self) -> Option<&AssetRef> {
        self.doc.owner()
    }

    pub fn title(&self) -> String {
        format!("{}{}", self.doc.base_title(), self.doc.title_suffix())
    }

    /// File name without directories and extensions (`rock_wet`); the secondbar falls back to [`Tab::title`] when
    /// several tabs share one.
    pub fn short_title(&self) -> String {
        format!("{}{}", self.short_name(), self.doc.title_suffix())
    }

    /// [`Tab::short_title`] without the suffix: what tab labels compare to find shared names.
    fn short_name(&self) -> String {
        let title = self.doc.base_title();
        let name = title.rsplit('/').next().unwrap_or(&title);
        name.split('.').next().unwrap_or(name).to_string()
    }
}

/// The secondbar's tab labels: [`Tab::short_title`], or the full [`Tab::title`] for every tab whose short name is
/// shared (a suffix such as ` (preview)` does not tell two `rock` tabs apart).
pub fn labels(tabs: &[Tab]) -> Vec<String> {
    let short: Vec<String> = tabs.iter().map(Tab::short_name).collect();
    tabs.iter()
        .zip(&short)
        .map(|(t, s)| if short.iter().filter(|o| *o == s).count() > 1 { t.title() } else { t.short_title() })
        .collect()
}

#[cfg(test)]
pub(crate) mod tests {
    use project_core::Project;

    use super::*;
    use crate::doc_kind::DocCtx;
    use crate::ui::icons::{IconId, IconRef};

    /// A minimal document named `title`, shown by the view `test`.
    pub struct TestDoc(pub &'static str);
    struct OtherDoc;

    macro_rules! doc {
        ($t:ty, $title:expr) => {
            impl TabDoc for $t {
                fn asset(&self) -> Option<&AssetRef> {
                    None
                }
                fn view(&self) -> ViewKindId {
                    ViewKindId("test")
                }
                fn base_title(&self) -> String {
                    $title(self)
                }
                fn icon(&self) -> IconRef {
                    IconId::File.into()
                }
                fn watermark(&self) -> Option<&'static str> {
                    None
                }
                fn is_dirty(&self) -> bool {
                    false
                }
                fn is_read_only(&self) -> bool {
                    true
                }
                fn revision(&self) -> u64 {
                    0
                }
                fn save(&mut self, _ctx: &mut DocCtx) -> std::io::Result<()> {
                    Ok(())
                }
                fn changed_on_disk(&self, _project: &Project) -> bool {
                    false
                }
                fn reload_from_disk(&mut self, _ctx: &mut DocCtx) -> Result<(), String> {
                    Ok(())
                }
                fn keep_mine(&mut self, _project: &Project) {}
                fn rename(&mut self, _project: &Project, _from: &AssetRef, _to: &AssetRef) {}
                fn config_key(&self) -> Option<String> {
                    None
                }
            }
        };
    }
    doc!(TestDoc, |d: &TestDoc| d.0.to_string());
    doc!(OtherDoc, |_: &OtherDoc| "other".to_string());

    #[test]
    fn downcast_to_concrete_doc() {
        let mut tab = Tab::new(DomainId::new("test"), TestDoc("a/b.x"));
        assert!(tab.get::<TestDoc>().is_some());
        assert!(tab.get::<OtherDoc>().is_none());
        assert!(tab.get_mut::<TestDoc>().is_some());
        assert!(tab.get_mut::<OtherDoc>().is_none());
    }

    #[test]
    fn tab_keys_split_on_a_domain_prefix() {
        let k = TabKey::new(DomainId::new("material"), "quillshade/material/a.curve.ron#wesl");
        assert_eq!(k.to_string(), "material:quillshade/material/a.curve.ron#wesl");
        assert_eq!(TabKey::split(&k.to_string()), Some(("material", "quillshade/material/a.curve.ron#wesl")));
        // Entries from before domains existed have no valid prefix.
        assert_eq!(TabKey::split("quillshade/material/a.curve.ron"), None);
        assert_eq!(TabKey::split("@game/a:b.png"), None);
        assert_eq!(TabKey::split("@internal/default"), None);
    }

    /// Design §14.4: a rename rewrites the key itself and the views derived from it (`#…`), in its domain only.
    #[test]
    fn tab_keys_follow_a_rename() {
        let (d, o) = (DomainId::new("dummy"), DomainId::new("other"));
        let (from, to) = (TabKey::new(d, "quillshade/dummy/a.note.ron"), TabKey::new(d, "quillshade/dummy/b.note.ron"));
        assert_eq!(from.renamed(&from, &to), Some(to.clone()));
        let view = TabKey::new(d, "quillshade/dummy/a.note.ron#wesl");
        assert_eq!(view.renamed(&from, &to), Some(TabKey::new(d, "quillshade/dummy/b.note.ron#wesl")));
        assert_eq!(TabKey::new(d, "quillshade/dummy/a.note.ronx").renamed(&from, &to), None);
        assert_eq!(TabKey::new(o, "quillshade/dummy/a.note.ron").renamed(&from, &to), None);
        let mut nav = crate::nav::NavHistory::default();
        for k in [&from, &view, &TabKey::new(o, "x")] {
            nav.visit(k.clone());
        }
        nav.rename(&from, &to);
        let keys: Vec<String> = nav.entries().iter().map(|e| e.tab.to_string()).collect();
        assert_eq!(keys, ["dummy:quillshade/dummy/b.note.ron", "dummy:quillshade/dummy/b.note.ron#wesl", "other:x"]);
    }

    #[test]
    fn labels_spell_out_shared_names() {
        let tabs = [
            Tab::new(DomainId::new("test"), TestDoc("env/rock.material")),
            Tab::new(DomainId::new("test"), TestDoc("props/rock.material")),
            Tab::new(DomainId::new("test"), TestDoc("env/grass.material")),
        ];
        assert_eq!(labels(&tabs), ["env/rock.material", "props/rock.material", "grass"]);
    }
}
