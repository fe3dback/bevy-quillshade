//! Text editor kind (core, design §6.2): a read-only generated file ([`TextDoc`]) with syntax highlighting. Which
//! files exist and what colours them is up to the domains.

use bevy::prelude::*;
use bevy_enhanced_input::prelude::Actions;
use project_core::{AssetRef, Project};

use super::{EditorAreas, EditorInput, SidebarKey, fill_sidebar_with};
use crate::action::{Handled, TabAction};
use crate::doc_kind::{DocCtx, StatusCtx, StatusInfo, TabDoc};
use crate::main_input::{EditCopy, add_kind, ctrl, hotkey};
use crate::state::editor::Editor;
use crate::tab::{TextDoc, ViewKindId, format_highlights};
use crate::theme::{TEXT as TEXT_COLOR, TEXT_DIM, text};
use crate::ui::icons::{IconId, IconRef};
use crate::ui::text_view::TextViewBar;
use crate::ui::widgets::subpane_box;
use crate::util::text::plural;

/// The text editor's window areas.
pub struct TextEditor;

/// Read-only generated file with syntax highlighting.
pub const TEXT: ViewKindId = ViewKindId("text");

impl EditorAreas for TextEditor {
    const VIEW: ViewKindId = TEXT;
    const SIDEBAR_TITLE: &'static str = "Generated File";

    /// The bar row, filled by `ui::text_view::redraw`.
    fn topbar() -> Option<impl Scene> {
        Some(bsn! { @super::bar_row() TextViewBar })
    }

    /// The file: its path, language, source asset and size.
    fn sidebar() -> Option<impl Scene> {
        Some(bsn! { @super::sidebar_body() TextSidebarBody })
    }

    fn build(app: &mut App) {
        app.add_systems(Update, redraw_sidebar);
    }

    fn input(app: &mut App) {
        add_kind::<Self>(app);
    }
}

/// The text editor's input context: Ctrl+C copies the whole file.
#[derive(Component, Default, Clone)]
pub struct TextCtx;

impl EditorInput for TextEditor {
    type Ctx = TextCtx;

    fn bindings() -> impl Scene {
        bsn! {
            Actions<TextCtx> [
                @{hotkey::<EditCopy>(ctrl(KeyCode::KeyC))}
            ]
        }
    }
}

/// The text editor's right sidebar body.
#[derive(Component, Default, Clone)]
struct TextSidebarBody;

fn redraw_sidebar(
    editor: Res<Editor>,
    body: Query<Entity, With<TextSidebarBody>>,
    mut key: Local<SidebarKey>,
    mut commands: Commands,
) {
    let Some(t) = editor.tab().and_then(|t| t.get::<TextDoc>()) else { return key.forget() };
    fill_sidebar_with(&mut commands, &body, &mut key, SidebarKey::now(&editor), || text_rows(t));
}

/// A generated file: where it comes from (Source / Open externally are buttons of the editor topbar).
fn text_rows(t: &TextDoc) -> Vec<Box<dyn Scene>> {
    let mut out: Vec<Box<dyn Scene>> = Vec::new();
    let lang = language_name(t.lang);
    out.push(Box::new(text(t.rel.clone(), TEXT_COLOR, 15.0)));
    out.push(Box::new(text(format!("Generated {lang}, read-only"), TEXT_DIM, 12.0)));
    out.push(Box::new(text(format!("Source: {}", t.source.source_rel()), TEXT_DIM, 12.0)));
    if let Some(src) = &t.text {
        out.push(Box::new(text(plural(src.lines().count(), "line", "lines"), TEXT_DIM, 12.0)));
    }
    if !t.highlights.is_empty() {
        out.push(Box::new(text(format_highlights(&t.highlights), TEXT_DIM, 12.0)));
    }
    vec![subpane_box("File", out)]
}

/// How the sidebar names a language: `Rust`, and the other (shader / config) languages in capitals (`WESL`).
fn language_name(lang: &str) -> String {
    match lang {
        "rust" => "Rust".into(),
        other => other.to_uppercase(),
    }
}

impl TabDoc for TextDoc {
    /// Copy takes the whole file (the view has no selection).
    fn apply(&mut self, ctx: &mut DocCtx, action: &TabAction) -> Handled {
        if *action != TabAction::Copy {
            return Handled::No;
        }
        ctx.copy_text = self.text.clone();
        Handled::Yes
    }

    fn asset(&self) -> Option<&AssetRef> {
        None
    }

    /// The source asset: the tab closes with it on delete and highlights it in the project tree.
    fn owner(&self) -> Option<&AssetRef> {
        Some(&self.source)
    }

    fn view(&self) -> ViewKindId {
        TEXT
    }

    fn base_title(&self) -> String {
        self.rel.clone()
    }

    fn icon(&self) -> IconRef {
        IconId::Code.into()
    }

    fn watermark(&self) -> Option<&'static str> {
        Some(self.lang)
    }

    fn is_dirty(&self) -> bool {
        false
    }

    fn is_read_only(&self) -> bool {
        true
    }

    fn revision(&self) -> u64 {
        self.revision
    }

    fn save(&mut self, _ctx: &mut DocCtx) -> std::io::Result<()> {
        Ok(())
    }

    fn changed_on_disk(&self, _project: &Project) -> bool {
        false
    }

    /// Re-reads the file and tells the domain (Material: a WESL file's line count for the graph's editor statusbar).
    fn reload_from_disk(&mut self, ctx: &mut DocCtx) -> Result<(), String> {
        self.reload();
        self.reloaded(&mut *ctx.domain);
        match self.text {
            Some(_) => Ok(()),
            None => Err(format!("{} does not exist", self.rel)),
        }
    }

    fn keep_mine(&mut self, _project: &Project) {}

    /// The generated file moves with its source: the domain's rename follow-up retargets the tab ([`TextDoc::retarget`])
    /// once it knows the new file.
    fn rename(&mut self, _project: &Project, _from: &AssetRef, _to: &AssetRef) {}

    /// `AssetRef::source_rel` of the source plus `#<lang>` (`…ron#wesl`).
    fn config_key(&self) -> Option<String> {
        Some(format!("{}#{}", self.source.source_rel(), self.lang))
    }

    fn status(&self, ctx: &StatusCtx) -> Option<StatusInfo> {
        if self.highlights.is_empty() {
            return None;
        }
        let mut counts = Vec::new();
        if let Some(src) = &self.text {
            counts.push(plural(src.lines().count(), "line", "lines"));
        }
        counts.push(format_highlights(&self.highlights));
        Some(StatusInfo::new(false, ctx, counts))
    }
}

#[cfg(test)]
mod tests {
    use project_core::DomainId;

    use super::*;
    use crate::tab::Tab;

    fn project() -> (tempfile::TempDir, Project) {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]\nname = \"g\"\n").unwrap();
        std::fs::create_dir_all(dir.path().join("assets")).unwrap();
        let p = Project::open(dir.path()).unwrap();
        (dir, p)
    }

    fn source() -> AssetRef {
        AssetRef { domain: DomainId::new("dummy"), path: "a/x".into(), suffix: "note" }
    }

    /// The domain names the file; the tab is read-only, owned by the source asset and remembered as
    /// `<source key>#<lang>` (the form saved configs already use).
    #[test]
    fn text_tab_identity() {
        let (_dir, p) = project();
        let file = p.root.join("out.txt");
        std::fs::write(&file, "one\ntwo\n").unwrap();
        let doc = TextDoc::open(&p, source(), "note", file);
        assert_eq!(doc.text.as_deref(), Some("one\ntwo\n"));
        let tab = Tab::new(DomainId::new("dummy"), doc);
        let d = tab.doc();
        assert_eq!(d.owner(), Some(&source()));
        assert!(d.asset().is_none() && d.is_read_only() && !d.is_dirty());
        assert_eq!(d.config_key().as_deref(), Some("quillshade/dummy/a/x.note.ron#note"));
        assert_eq!(d.watermark(), Some("note"));
        assert_eq!(d.base_title(), "out.txt");
    }

    /// A reopened file gets a fresh revision, an unchanged reload keeps it, and the domain's hook sees every reload.
    #[test]
    fn reload_bumps_the_revision_and_calls_the_hook() {
        let (_dir, p) = project();
        let file = p.root.join("out.txt");
        std::fs::write(&file, "a\n").unwrap();
        let mut doc = TextDoc::open(&p, source(), "note", file.clone()).with_reload_hook(|state, t| {
            if let Some(lines) = state.downcast_mut::<usize>() {
                *lines = t.text.as_deref().map_or(0, |s| s.lines().count());
            }
        });
        let first = doc.revision;
        assert!(!doc.reload(), "unchanged");
        assert_eq!(doc.revision, first);
        std::fs::write(&file, "a\nb\nc\n").unwrap();
        let (mut lines, mut messages) = (0usize, Vec::new());
        let mut ctx = DocCtx::new(Some(&p), &mut lines, &mut messages);
        assert_eq!(doc.reload_from_disk(&mut ctx), Ok(()));
        assert_ne!(doc.revision, first);
        assert_eq!(lines, 3, "the hook got the domain state");
        assert_ne!(TextDoc::open(&p, source(), "note", file).revision, doc.revision, "a reopened tab is fresh");
    }

    /// The domain retargets the tab after its asset was renamed; core's own rename leaves it alone.
    #[test]
    fn retarget_follows_the_source() {
        let (_dir, p) = project();
        let (old, new) = (p.root.join("old.txt"), p.root.join("new.txt"));
        std::fs::write(&old, "x").unwrap();
        let mut doc = TextDoc::open(&p, source(), "note", old);
        let moved = AssetRef { path: "a/y".into(), ..source() };
        doc.rename(&p, &source(), &moved);
        assert_eq!(doc.rel, "old.txt", "core cannot know the new file");
        doc.retarget(&p, moved.clone(), new);
        assert_eq!((doc.source.clone(), doc.rel.as_str(), doc.text.clone()), (moved, "new.txt", None));
    }
}
