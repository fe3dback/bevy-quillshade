//! Core flows on test-only domains (no Material): the registrar (asset kinds and their templates, window parts), the
//! Assets menu, the Project and Game Assets trees, tab keys in the user config, Back / Forward across domains, folders
//! and asset flows through the domain hooks, project dialogs, View toggles, project settings and tools (design §6.1,
//! §6.3, §6.6, §8.1, §14.1).

use std::path::Path;
use std::sync::Arc;

use crate::action::{Action, Dialog, Handled, TabAction, TreeId};
use crate::config::{ProjectTabs, UserConfig};
use crate::doc_kind::{DocCtx, TabDoc};
use crate::dynval::Payload;
use crate::editors::EditorAreas;
use crate::menu::{MainMenu, MenuEntry};
use crate::message::Level;
use crate::tab::{Tab, TabKey, ViewKindId};
use crate::ui::icons::{IconId, IconRef};
use bevy::prelude::*;
use project_core::{AssetRef, Diagnostic, DomainId, Project};

use crate::domain::*;
use crate::state::editor::Editor;
use crate::ui::menu::menu_entries;
use crate::ui::tree::{GameAssetsTree, LeftPanes, ProjectTree, TreeItem, TreeRow, TreeSource, left_panes, row_menu};

const IDS: [DomainId; 2] = [DomainId::new("dummy"), DomainId::new("other")];
const NOTE: &str = "note";

/// A domain of `*.note.ron` files with one editor kind; the first one shows `.txt` game files.
struct Dummy<const N: usize>;

/// What a dummy domain keeps: the notes found by the last rescan.
struct DummyState<const N: usize> {
    notes: Vec<AssetRef>,
    /// What `diagnostics` reports (tests set it).
    problems: Vec<Diagnostic>,
}

/// A note in a tab.
struct NoteDoc(AssetRef);

struct NoteEditor;

impl EditorAreas for NoteEditor {
    const VIEW: ViewKindId = ViewKindId("dummy_note");
    const SIDEBAR_TITLE: &'static str = "Note";
    fn topbar() -> Option<impl Scene> {
        None::<()>
    }
    fn sidebar() -> Option<impl Scene> {
        None::<()>
    }
}

fn note(n: usize, path: &str) -> AssetRef {
    AssetRef { domain: IDS[n], path: path.into(), suffix: NOTE }
}

impl<const N: usize> EditorDomain for Dummy<N> {
    fn id(&self) -> DomainId {
        IDS[N]
    }

    fn info(&self) -> DomainInfo {
        DomainInfo { title: "Dummy", icon: IconId::File.into(), summary: "Dummy — notes for the tests." }
    }

    fn backend(&self) -> Option<Box<dyn project_core::regen::DomainBackend>> {
        None
    }

    fn register(&self, r: &mut DomainRegistrar) {
        // Only the first dummy shows `.txt` game files; each names its template and its Edit item apart.
        if N == 0 {
            r.game_assets(GameAssetsDef {
                extensions: &["txt"],
                open: |e, path| e.info(format!("opened game file {path}")),
                menu: |_, path| vec![MenuEntry::from((None, format!("Inspect {path}"), Action::None))],
            });
        }
        r.state(|| Box::new(DummyState::<N> { notes: Vec::new(), problems: Vec::new() }))
            .menu_items(MainMenu::Edit, |_| vec![MenuEntry::from((None, format!("Tidy {}", IDS[N]), Action::None))])
            .asset_kind(AssetKindDef {
                suffix: NOTE,
                title: "Note",
                icon: IconId::File.into(),
                templates: vec![AssetTemplate {
                    label: if N == 0 { "Note" } else { "Memo" },
                    base_name: "new_note",
                    icon: IconId::File.into(),
                    create: |p, r| std::fs::write(p.root.join(r.source_rel()), "()"),
                }],
                open: |e, r| {
                    e.info(format!("opening {}", r.path));
                    e.project
                        .as_ref()
                        .is_some_and(|p| p.root.join(r.source_rel()).is_file())
                        .then(|| Tab::new(r.domain, NoteDoc(r.clone())))
                },
                row_menu: |_, _| Vec::new(),
            })
            .editor_kind::<NoteEditor>()
            .doc_view(|| Box::new(bsn! { Node }))
            .highlighter("note", |s| vec![(crate::highlight::Style::Keyword, s)])
            .parse_tab(|e, key| {
                let path = key.strip_prefix(&format!("quillshade/{}/", IDS[N]))?.strip_suffix(".note.ron")?;
                let r = note(N, path);
                e.project
                    .as_ref()
                    .is_some_and(|p| p.root.join(r.source_rel()).is_file())
                    .then(|| Tab::new(IDS[N], NoteDoc(r)))
            })
            .tool(ToolDef {
                id: STATS,
                title: "Stats",
                open: |_| Box::new(StatsDoc(0, STATS)),
                menu: Some(MainMenu::File),
            });
    }
}

impl<const N: usize> DomainState for DummyState<N> {
    fn id(&self) -> DomainId {
        IDS[N]
    }

    fn rescan(&mut self, editor: &mut Editor) {
        let Some(p) = &editor.project else { return self.notes.clear() };
        let dir = p.domain_dir(IDS[N]);
        let rels = project_core::files::walk_files(&dir).into_iter().map(|f| p.rel(&f));
        self.notes = rels.filter_map(|rel| project_core::asset_ref::parse(&rel, IDS[N], &[NOTE])).collect();
    }

    /// Any command: a console/logs line.
    fn apply(&mut self, editor: &mut Editor, _action: &crate::dynval::Payload) {
        editor.info("ping");
    }

    fn tree_items(&self, _editor: &Editor) -> Option<Vec<TreeAsset>> {
        let item = |r: &AssetRef| TreeAsset { asset: r.clone(), icon: IconId::File.into(), error: false };
        Some(self.notes.iter().map(item).collect())
    }

    fn generate(&mut self, editor: &mut Editor) {
        editor.info("generated");
    }

    fn diagnostics(&self) -> Vec<Diagnostic> {
        self.problems.clone()
    }

    /// Opens the note the diagnostic names.
    fn focus_diagnostic(&mut self, editor: &mut Editor, diag: &Diagnostic) {
        if let Some(path) = diag.asset.as_deref().and_then(|a| a.strip_prefix(&format!("quillshade/{}/", IDS[N]))) {
            editor.open_ref(&note(N, path.trim_end_matches(".note.ron")));
        }
    }

    fn rename_file(&mut self, editor: &mut Editor, r: &AssetRef, new_path: &str) -> Result<AssetRef, String> {
        let p = editor.project.as_ref().ok_or("no project")?;
        let to = AssetRef { path: new_path.into(), ..r.clone() };
        let target = p.root.join(to.source_rel());
        std::fs::create_dir_all(target.parent().ok_or("no parent")?).map_err(|e| e.to_string())?;
        std::fs::rename(p.root.join(r.source_rel()), p.root.join(to.source_rel())).map_err(|e| e.to_string())?;
        Ok(to)
    }

    fn renamed(&mut self, editor: &mut Editor, from: &AssetRef, to: &AssetRef) {
        let known = self.notes.contains(to) && !self.notes.contains(from);
        editor.info(format!("renamed hook, rescanned first: {known}"));
    }

    fn before_delete(&mut self, editor: &mut Editor, r: &AssetRef) {
        let open = editor.asset_tab(r).is_some();
        editor.info(format!("before delete, tab open: {open}"));
    }

    fn delete_file(&mut self, editor: &mut Editor, r: &AssetRef) -> Result<(), String> {
        let p = editor.project.as_ref().ok_or("no project")?;
        std::fs::remove_file(p.root.join(r.source_rel())).map_err(|e| e.to_string())
    }

    fn deleted(&mut self, editor: &mut Editor, r: &AssetRef) {
        let gone = !self.notes.contains(r) && editor.asset_tab(r).is_none();
        editor.info(format!("deleted hook, rescanned and closed: {gone}"));
    }
}

impl TabDoc for NoteDoc {
    fn asset(&self) -> Option<&AssetRef> {
        Some(&self.0)
    }
    fn view(&self) -> ViewKindId {
        NoteEditor::VIEW
    }
    fn base_title(&self) -> String {
        self.0.source_rel()
    }
    fn icon(&self) -> IconRef {
        IconId::File.into()
    }
    fn watermark(&self) -> Option<&'static str> {
        Some(NOTE)
    }
    fn is_dirty(&self) -> bool {
        false
    }
    fn is_read_only(&self) -> bool {
        false
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
    fn rename(&mut self, _project: &Project, from: &AssetRef, to: &AssetRef) {
        if self.0 == *from {
            self.0 = to.clone();
        }
    }
    fn config_key(&self) -> Option<String> {
        Some(self.0.source_rel())
    }
}

const STATS: ToolId = "stats";

/// The dummy domains' tool (and the test project domain's, under its own id): counts the [`Bump`]s it receives.
struct StatsDoc(usize, ToolId);

/// The stats tool's argument.
#[derive(Debug, PartialEq)]
struct Bump;

impl TabDoc for StatsDoc {
    fn asset(&self) -> Option<&AssetRef> {
        None
    }
    fn view(&self) -> ViewKindId {
        NoteEditor::VIEW
    }
    fn base_title(&self) -> String {
        "Stats".into()
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
        false
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
    fn tool(&self) -> Option<ToolId> {
        Some(self.1)
    }
    fn apply(&mut self, _ctx: &mut DocCtx, action: &TabAction) -> Handled {
        match action {
            TabAction::Doc(p) if p.downcast::<Bump>().is_some() => {
                self.0 += 1;
                Handled::Yes
            }
            _ => Handled::No,
        }
    }
}

fn list(domains: Vec<Box<dyn EditorDomain>>) -> Arc<DomainList> {
    Arc::new(DomainList::new(domains))
}

fn one() -> Arc<DomainList> {
    list(vec![Box::new(Dummy::<0>)])
}

fn two() -> Arc<DomainList> {
    list(vec![Box::new(Dummy::<0>) as Box<dyn EditorDomain>, Box::new(Dummy::<1>)])
}

/// A game crate with `files` (relative to its root, created with `()`).
fn game(files: &[&str]) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("Cargo.toml"), "[package]\nname = \"g\"\n").unwrap();
    std::fs::create_dir_all(dir.path().join("assets")).unwrap();
    for f in files {
        let path = dir.path().join(f);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, "()").unwrap();
    }
    dir
}

fn open(domains: Arc<DomainList>, root: &Path) -> Editor {
    let mut e = Editor::new(domains);
    e.open_project(root).unwrap();
    e
}

/// A domain whose editor kind reuses the dummy note kind's view id.
struct Clash;

struct ClashEditor;

impl EditorAreas for ClashEditor {
    const VIEW: ViewKindId = NoteEditor::VIEW;
    const SIDEBAR_TITLE: &'static str = "Clash";
    fn topbar() -> Option<impl Scene> {
        None::<()>
    }
    fn sidebar() -> Option<impl Scene> {
        None::<()>
    }
}

impl EditorDomain for Clash {
    fn id(&self) -> DomainId {
        DomainId::new("clash")
    }

    fn info(&self) -> DomainInfo {
        DomainInfo { title: "Clash", icon: IconId::File.into(), summary: "" }
    }

    fn backend(&self) -> Option<Box<dyn project_core::regen::DomainBackend>> {
        None
    }

    fn register(&self, r: &mut DomainRegistrar) {
        r.editor_kind::<ClashEditor>();
    }
}

#[test]
#[should_panic(expected = "share the view id `dummy_note`")]
fn two_kinds_with_one_view_id_panic() {
    list(vec![Box::new(Dummy::<0>) as Box<dyn EditorDomain>, Box::new(Clash)]);
}

#[test]
fn registrar_collects_kinds() {
    let one = one();
    assert_eq!(one.ids().collect::<Vec<_>>(), [IDS[0], DomainId::PROJECT], "the project domain last");
    assert_eq!(one.domains()[0].info.title, "Dummy");
    let kinds: Vec<&str> = one.asset_kinds(IDS[0]).iter().map(|k| k.suffix).collect();
    assert_eq!(kinds, [NOTE]);
    assert!(one.asset_kind(IDS[0], NOTE).is_some_and(|k| k.templates.len() == 1));
    assert!(one.asset_kinds(IDS[1]).is_empty(), "an unknown domain has no kinds");
    let views: Vec<ViewKindId> = one.kind_defs().0.iter().map(|k| k.view).collect();
    let settings = crate::settings::SETTINGS_VIEW;
    assert_eq!(views, [crate::editors::text::TEXT, NoteEditor::VIEW, settings], "core's text viewer first");
    assert!(one.crate_contributions().is_empty(), "no backend, no crate module");
    assert!(one.is_game_file("a/b.TXT") && !one.is_game_file("a/b.png") && !one.is_game_file(".txt"));
    // Two domains registering one editor kind (a kit's) list it once; each gets its own state.
    assert_eq!(two().kind_defs().0.len(), 3, "the text viewer, the shared kind and Project Settings");
    assert_eq!(Editor::new(two()).domains.ids().collect::<Vec<_>>(), IDS);
}

#[test]
fn assets_menu_lists_templates() {
    let dir = game(&[]);
    let mut e = open(one(), dir.path());
    let entries = menu_entries(MainMenu::Assets, &e);
    let labels: Vec<&str> = entries.iter().map(|m| m.label.as_str()).collect();
    assert_eq!(labels, ["New Note...", "New folder...", "", "Generate"]);
    let new = Action::NewAssetDialog { kind: (IDS[0], NOTE), template: 0 };
    assert_eq!(entries[0].action, new);
    e.apply(new);
    let Some(Dialog::Input { title, initial, .. }) = &e.dialog else { panic!("no dialog: {:?}", e.dialog) };
    assert_eq!((title.as_str(), initial.as_str()), ("New Note", "new_note"));
    e.apply(Action::Submit("first".into()));
    assert!(dir.path().join("quillshade/dummy/first.note.ron").is_file());
    assert_eq!(e.tab().and_then(|t| t.doc().asset()), Some(&note(0, "first")), "the new asset opens");
}

#[test]
fn tree_shows_domain_folder_only() {
    let dir = game(&["quillshade/dummy/notes/a.note.ron", "quillshade/material/env/rock.material.ron"]);
    std::fs::create_dir_all(dir.path().join("quillshade/dummy/empty")).unwrap();
    let e = open(one(), dir.path());
    let tree = ProjectTree(IDS[0]);
    assert_eq!(tree.id(), TreeId::Project(IDS[0]));
    assert_eq!(
        tree.rows(&e),
        [
            (0, TreeItem::Folder(String::new())),
            (1, TreeItem::Folder("empty".into())),
            (1, TreeItem::Folder("notes".into())),
            (2, TreeItem::Asset(note(0, "notes/a"))),
        ]
    );
    assert_eq!(crate::ui::tree::row_label(tree.id(), &TreeItem::Folder(String::new())), "dummy");
}

#[test]
fn game_assets_filter_uses_extensions() {
    let dir = game(&["assets/a.txt", "assets/b.png", "assets/sub/c.TXT", "assets/quillshade/dummy/x.txt"]);
    let e = open(one(), dir.path());
    assert_eq!(e.game_files, ["a.txt", "sub/c.TXT"]);
    let rows: Vec<TreeItem> = GameAssetsTree.rows(&e).into_iter().map(|(_, i)| i).collect();
    assert_eq!(
        rows,
        [
            TreeItem::Folder(String::new()),
            TreeItem::Folder("sub".into()),
            TreeItem::GameFile("sub/c.TXT".into()),
            TreeItem::GameFile("a.txt".into()),
        ]
    );
}

/// Review Focus 4: open-tab keys of an unknown domain, or that no domain parses, are skipped without a dialog; the
/// others restore. The saved active index is a position among the saved entries: it activates the tab its own entry
/// restored, also with skipped entries before it; when the active entry itself is skipped, the last restored tab
/// stays active.
#[test]
fn unknown_tab_keys_are_skipped() {
    let dir = game(&["quillshade/dummy/a.note.ron", "quillshade/dummy/b.note.ron"]);
    let root = Project::open(dir.path()).unwrap().root;
    let (a, b) = ("dummy:quillshade/dummy/a.note.ron", "dummy:quillshade/dummy/b.note.ron");
    let open = ["nope:x", "dummy:missing", a, "no-prefix", b];
    let restore = |active| {
        let mut e = Editor::new(one());
        e.config
            .tabs
            .insert(root.clone(), ProjectTabs { open: open.map(String::from).to_vec(), active, ..Default::default() });
        e.open_project(dir.path()).unwrap();
        e
    };
    let mut e = restore(Some(2));
    assert_eq!(e.tabs.len(), 2);
    assert!(e.dialog.is_none(), "{:?}", e.dialog);
    assert_eq!(e.active, Some(0), "entry 2 (a) is restored tab 0");
    assert_eq!(e.active_key(), Some(TabKey::new(IDS[0], "quillshade/dummy/a.note.ron")));
    assert_eq!(restore(Some(4)).active, Some(1), "entry 4 (b) is restored tab 1");
    assert_eq!(restore(Some(0)).active, Some(1), "a skipped active entry: the last restored tab");
    assert_eq!(restore(None).active, Some(1));
    e.save_tabs_to_config();
    assert_eq!(e.config.tabs[&root].open, [a, b]);
    assert_eq!(e.config.tabs[&root].active, Some(0));
}

#[test]
fn back_forward_across_tabs_of_two_domains() {
    let dir = game(&["quillshade/dummy/a.note.ron", "quillshade/other/b.note.ron"]);
    let mut e = open(two(), dir.path());
    let (a, b) = (note(0, "a"), note(1, "b"));
    e.apply(Action::OpenAsset(a.clone()));
    e.apply(Action::OpenAsset(b.clone()));
    assert_eq!(e.tabs.iter().map(|t| t.domain).collect::<Vec<_>>(), IDS);
    let active = |e: &Editor| e.tab().and_then(|t| t.doc().asset()).cloned();
    e.apply(Action::NavBack);
    assert_eq!(active(&e), Some(a.clone()));
    e.apply(Action::NavForward);
    assert_eq!(active(&e), Some(b.clone()));
    // A closed tab of the other domain reopens through its domain.
    e.apply(Action::ActivateTab(0));
    e.apply(Action::CloseTab(1));
    assert_eq!(active(&e), Some(a));
    e.apply(Action::NavBack);
    assert_eq!(active(&e), Some(b));
    assert_eq!(e.tabs.len(), 2);
}

/// Design §6.4: lines a domain's hook or registered callback (here its asset kind's `open`) writes carry its domain,
/// the editor's own lines none.
#[test]
fn log_lines_carry_their_domain() {
    #[derive(Debug, PartialEq)]
    struct Ping;
    let dir = game(&["quillshade/other/b.note.ron"]);
    let mut e = open(two(), dir.path());
    let tags = |e: &Editor| e.messages.iter().map(|m| (m.domain, m.text.clone())).collect::<Vec<_>>();
    e.apply(Action::Domain(IDS[1], crate::dynval::Payload::new(Ping)));
    assert_eq!(tags(&e).last(), Some(&(Some(IDS[1]), "ping".to_string())));
    assert!(tags(&e).iter().any(|(d, t)| d.is_none() && t.starts_with("Opened project")), "{:?}", tags(&e));
    e.apply(Action::OpenAsset(note(1, "b")));
    assert_eq!(e.tabs.len(), 1);
    assert_eq!(tags(&e).last(), Some(&(Some(IDS[1]), "opening b".to_string())));
}

/// Design §6.3: each domain's Project tree has its own selected folder; folders are created and deleted in the domain's
/// directory, under the project-wide identifier rule; the folder menu carries the tree's domain.
#[test]
fn folders_are_per_domain() {
    let dir = game(&[]);
    let mut e = open(two(), dir.path());
    let input = |e: &Editor| match &e.dialog {
        Some(Dialog::Input { label, initial, .. }) => (label.clone(), initial.clone()),
        d => panic!("no input dialog: {d:?}"),
    };
    e.apply(Action::NewFolderDialog(Some(IDS[1])));
    assert_eq!(input(&e), ("Folder path inside quillshade/other/".to_string(), "new_folder".to_string()));
    e.apply(Action::Submit("sub".into()));
    assert!(dir.path().join("quillshade/other/sub").is_dir());
    assert!(!dir.path().join("quillshade/dummy/sub").exists());
    assert_eq!((e.current_folder(IDS[0]), e.current_folder(IDS[1])), ("", "sub"));
    // The selected folder of the template's domain is proposed.
    e.apply(Action::NewAssetDialog { kind: (IDS[1], NOTE), template: 0 });
    assert_eq!(input(&e).1, "sub/new_note");
    e.apply(Action::NewAssetDialog { kind: (IDS[0], NOTE), template: 0 });
    assert_eq!(input(&e).1, "new_note");
    e.apply(Action::CloseDialog);
    // No domain given and no tab: the first domain.
    e.apply(Action::NewFolderDialog(None));
    assert_eq!(input(&e).0, "Folder path inside quillshade/dummy/");
    e.apply(Action::Submit("Bad".into()));
    assert!(matches!(&e.dialog, Some(Dialog::Message { .. })), "{:?}", e.dialog);
    assert!(!dir.path().join("quillshade/dummy/Bad").exists());
    e.apply(Action::CloseDialog);
    // The folder row's menu acts on its tree's domain.
    let row = crate::ui::tree::TreeRow { tree: TreeId::Project(IDS[1]), item: TreeItem::Folder("sub".into()) };
    let actions: Vec<Action> = crate::ui::tree::row_menu(&e, &row).into_iter().map(|m| m.action).collect();
    assert!(actions.contains(&Action::NewFolderDialog(Some(IDS[1]))), "{actions:?}");
    assert!(actions.contains(&Action::DeleteFolderDialog(IDS[1], "sub".into())), "{actions:?}");
    assert_eq!(crate::ui::tree::click_actions(&row, 1)[1], Action::SelectFolder(IDS[1], "sub".into()));
    e.apply(Action::SelectFolder(IDS[0], String::new()));
    assert_eq!(e.current_folder(IDS[1]), "sub", "another domain's selection stays");
    e.apply(Action::DeleteFolder(IDS[1], "sub".into()));
    assert!(!dir.path().join("quillshade/other/sub").exists());
}

/// Design §6.3: rename, move and delete are core flows around the domain's hooks; Generate reaches every domain.
#[test]
fn asset_flows_call_the_domain_hooks() {
    let dir = game(&["quillshade/other/a.note.ron"]);
    let mut e = open(two(), dir.path());
    let lines = |e: &Editor, n: usize| {
        let all: Vec<_> = e.messages.iter().map(|m| (m.domain, m.text.clone())).collect();
        all[all.len().saturating_sub(n)..].to_vec()
    };
    e.apply(Action::OpenAsset(note(1, "a")));
    e.apply(Action::RenameDialog(note(1, "a")));
    e.apply(Action::Submit("b".into()));
    assert!(dir.path().join("quillshade/other/b.note.ron").is_file());
    assert_eq!(
        lines(&e, 2),
        [
            (None, "Renamed quillshade/other/a.note.ron -> quillshade/other/b.note.ron".to_string()),
            (Some(IDS[1]), "renamed hook, rescanned first: true".to_string()),
        ]
    );
    e.apply(Action::MoveTo(note(1, "b"), "sub".into()));
    assert!(dir.path().join("quillshade/other/sub/b.note.ron").is_file(), "{:?}", e.dialog);
    e.apply(Action::OpenAsset(note(1, "sub/b")));
    e.apply(Action::DeleteDialog(note(1, "sub/b")));
    let Some(Dialog::Confirm { message, .. }) = &e.dialog else { panic!("{:?}", e.dialog) };
    assert_eq!(message, "Delete quillshade/other/sub/b.note.ron?");
    e.apply(Action::Delete(note(1, "sub/b")));
    assert!(!dir.path().join("quillshade/other/sub/b.note.ron").exists());
    assert!(e.asset_tab(&note(1, "sub/b")).is_none(), "its tab closed");
    assert_eq!(
        lines(&e, 3),
        [
            (Some(IDS[1]), "before delete, tab open: true".to_string()),
            (None, "Deleted quillshade/other/sub/b.note.ron".to_string()),
            (Some(IDS[1]), "deleted hook, rescanned and closed: true".to_string()),
        ]
    );
    e.apply(Action::RegenAll);
    assert_eq!(lines(&e, 2), [(Some(IDS[0]), "generated".to_string()), (Some(IDS[1]), "generated".to_string())]);
}

/// Window parts a domain registers (design §6.1, §8): editor content views, highlighters, Quick Start steps; the
/// first domain registering a language provides it, every domain's Quick Start steps are listed.
#[test]
fn registrar_collects_window_parts() {
    let two = two();
    assert_eq!(two.domains().iter().map(|d| d.doc_views.len()).collect::<Vec<_>>(), [1, 1, 1], "and the project's");
    let note = two.highlighter("note").expect("the dummy's highlighter");
    assert_eq!(note("x"), [(crate::highlight::Style::Keyword, "x")]);
    assert!(two.highlighter("rust").is_some(), "core's own");
    assert!(two.highlighter("wesl").is_none());
    assert_eq!(two.quick_starts().count(), 0, "no domain registered one");
    const STEPS: [(&str, bool); 1] = [("Only", false)];
    struct Help;
    impl EditorDomain for Help {
        fn id(&self) -> DomainId {
            DomainId::new("help")
        }
        fn info(&self) -> DomainInfo {
            DomainInfo { title: "Help", icon: IconId::File.into(), summary: "" }
        }
        fn backend(&self) -> Option<Box<dyn project_core::regen::DomainBackend>> {
            None
        }
        fn register(&self, r: &mut DomainRegistrar) {
            r.quick_start(QuickStartDef { steps: &STEPS, body: |_| Vec::new() });
        }
    }
    let with_help = list(vec![Box::new(Dummy::<0>) as Box<dyn EditorDomain>, Box::new(Help)]);
    let quick: Vec<_> = with_help.quick_starts().map(|(id, q)| (id, q.steps)).collect();
    assert_eq!(quick, [(DomainId::new("help"), &STEPS[..])]);
}

// ---- core flows moved from the Material tests (they need no domain of their own) ----

#[test]
fn picked_folder_submits_the_project_dialog() {
    let dir = game(&[]);
    let mut e = Editor::new(one());
    e.apply(Action::OpenProjectDialog);
    assert!(!e.pick_folder, "only our dialog opens: the native picker waits for Browse");
    e.apply(Action::CloseDialog);
    e.apply(Action::FolderPicked(dir.path().into()));
    assert!(e.project.is_none(), "dialog closed meanwhile: ignored");
    e.apply(Action::OpenProjectDialog);
    e.apply(Action::BrowseFolder);
    e.apply(Action::FolderPicked(dir.path().into()));
    assert!(e.project.is_some() && e.dialog.is_none() && !e.pick_folder);
}

#[test]
fn recent_project_closes_the_open_dialog() {
    let dir = game(&[]);
    let mut e = open(one(), dir.path());
    e.apply(Action::OpenProjectDialog);
    e.apply(Action::OpenProject(dir.path().into()));
    assert!(e.project.is_some() && e.dialog.is_none(), "{:?}", e.dialog);
    e.apply(Action::OpenProjectDialog);
    e.apply(Action::OpenProject(dir.path().join("missing")));
    assert!(matches!(e.dialog, Some(Dialog::Message { .. })), "the error replaces it: {:?}", e.dialog);
}

#[test]
fn forget_recent_keeps_the_project_open() {
    let dir = game(&[]);
    let mut e = open(one(), dir.path());
    let root = e.project.as_ref().unwrap().root.clone();
    assert_eq!(e.config.recent_projects, std::slice::from_ref(&root));
    e.apply(Action::ForgetRecent(root));
    assert!(e.config.recent_projects.is_empty());
    assert!(e.project.is_some(), "only the list entry goes");
}

#[test]
fn new_project_sets_up_a_game_crate() {
    let dir = tempfile::tempdir().unwrap();
    let mut e = Editor::new(one());
    e.apply(Action::NewProjectDialog);
    e.apply(Action::Submit(dir.path().display().to_string()));
    assert!(e.project.is_none(), "no Cargo.toml: refused");
    assert!(matches!(e.dialog, Some(Dialog::Message { .. })));
    std::fs::write(dir.path().join("Cargo.toml"), "[package]\nname = \"game\"\n").unwrap();
    e.apply(Action::NewProject(dir.path().into()));
    let root = e.project.as_ref().unwrap().root.clone();
    assert!(root.join("assets").is_dir() && root.join("quillshade/dummy").is_dir());
    assert!(root.join("quillshade/project.ron").is_file());
    assert_eq!(e.config.recent_projects, [root]);
}

/// View → Show console / Show properties (design §3): each toggle flips its flag and saves the user config, which the
/// next launch reads (Review Focus 5).
#[test]
fn toggle_log_and_properties_persist() {
    let dir = game(&[]);
    let mut e = open(one(), dir.path());
    let config = dir.path().join("config/config.ron");
    e.config_path = Some(config.clone());
    assert!(e.config.show_log && e.config.show_properties, "shown by default");
    e.apply(Action::ToggleLog);
    assert!(!e.config.show_log && e.config.show_properties);
    assert_eq!(UserConfig::load(&config), e.config, "saved");
    e.apply(Action::ToggleProperties);
    assert!(!e.config.show_properties);
    let saved = UserConfig::load(&config);
    assert!(!saved.show_log && !saved.show_properties);
    e.apply(Action::ToggleLog);
    assert!(UserConfig::load(&config).show_log, "toggled back");
    let next = Editor::new(one()).with_config(UserConfig::load(&config), Some(config));
    assert!(next.config.show_log && !next.config.show_properties);
}

#[test]
fn quick_start_switches_steps() {
    let dir = game(&[]);
    let mut e = open(one(), dir.path());
    e.apply(Action::QuickStart(0));
    assert_eq!(e.dialog, Some(Dialog::QuickStart(0)));
    e.apply(Action::QuickStart(5));
    assert_eq!(e.dialog, Some(Dialog::QuickStart(5)), "a step tab switches the open dialog");
    e.apply(Action::CloseDialog);
    assert_eq!(e.dialog, None);
}

#[test]
fn external_project_ron_edit_reloads_settings() {
    let dir = game(&[]);
    let mut e = open(one(), dir.path());
    let path = e.project.as_ref().unwrap().settings_path();
    std::fs::write(&path, "ProjectSettings(format_version: 1, header_date: true)\n").unwrap();
    e.files_changed(vec![path.clone()]);
    assert!(e.project.as_ref().unwrap().settings.header_date);
    // A broken edit keeps the last good settings and says so once.
    std::fs::write(&path, "ProjectSettings(format_version: 1, header_date: ").unwrap();
    let before = e.messages.len();
    e.files_changed(vec![path.clone()]);
    assert!(e.project.as_ref().unwrap().settings.header_date);
    assert_eq!(e.messages.len(), before + 1);
    let last = e.messages.last().unwrap();
    assert_eq!(last.level, Level::Warning);
    assert!(last.text.contains("project.ron") && last.text.contains("keeping the previous settings"), "{}", last.text);
}

#[test]
fn malformed_project_ron_refuses_to_open() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("Cargo.toml"), "[package]\nname = \"g\"\n").unwrap();
    std::fs::create_dir_all(dir.path().join("assets")).unwrap();
    std::fs::create_dir_all(dir.path().join("quillshade")).unwrap();
    std::fs::write(dir.path().join("quillshade/project.ron"), "(format_version: 1, header_date: tru)").unwrap();
    let mut e = Editor::new(one());
    e.apply(Action::OpenProject(dir.path().to_path_buf()));
    assert!(e.project.is_none());
    let Some(Dialog::Message { message, .. }) = &e.dialog else { panic!("{:?}", e.dialog) };
    assert!(message.starts_with("Cannot open project: cannot read project settings `"), "{message}");
    assert!(message.contains("project.ron"), "{message}");
}

/// A kit's commands and deferred steps (design §6.2): a domain's step handlers get its payloads first, with its
/// state in place; what none handles goes to the state's `apply`; a domain without a state still runs its handlers.
#[test]
fn step_handlers_run_before_the_state() {
    use crate::action::Handled;
    use crate::dynval::Payload;

    /// A step only the handler knows.
    #[derive(Debug, PartialEq)]
    struct Kit(&'static str);

    fn handle(e: &mut Editor, p: &Payload) -> Handled {
        let Some(Kit(text)) = p.downcast::<Kit>() else { return Handled::No };
        let state = if e.domains.get::<DummyState<0>>().is_some() { "state in place" } else { "no state" };
        e.info(format!("kit {text}: {state}"));
        Handled::Yes
    }

    /// Dummy 0 with the kit's handler.
    struct WithKit;
    impl EditorDomain for WithKit {
        fn id(&self) -> DomainId {
            IDS[0]
        }
        fn info(&self) -> DomainInfo {
            Dummy::<0>.info()
        }
        fn backend(&self) -> Option<Box<dyn project_core::regen::DomainBackend>> {
            None
        }
        fn register(&self, r: &mut DomainRegistrar) {
            Dummy::<0>.register(r);
            r.steps(handle);
        }
    }

    /// Only the kit's handler: no state.
    struct KitOnly;
    impl EditorDomain for KitOnly {
        fn id(&self) -> DomainId {
            IDS[1]
        }
        fn info(&self) -> DomainInfo {
            Dummy::<1>.info()
        }
        fn backend(&self) -> Option<Box<dyn project_core::regen::DomainBackend>> {
            None
        }
        fn register(&self, r: &mut DomainRegistrar) {
            r.steps(handle);
        }
    }

    let dir = game(&[]);
    let mut e = open(list(vec![Box::new(WithKit) as Box<dyn EditorDomain>, Box::new(KitOnly)]), dir.path());
    let last = |e: &Editor| e.messages.last().map(|m| (m.domain, m.text.clone()));
    e.apply(Action::Domain(IDS[0], Payload::new(Kit("a"))));
    assert_eq!(last(&e), Some((Some(IDS[0]), "kit a: state in place".into())));
    e.apply(Action::Domain(IDS[0], Payload::new(7u8)));
    assert_eq!(last(&e), Some((Some(IDS[0]), "ping".into())), "unhandled: the state's apply");
    e.apply(Action::Domain(IDS[1], Payload::new(Kit("b"))));
    assert_eq!(last(&e), Some((Some(IDS[1]), "kit b: state in place".into())), "the other domain's state is there");
    let n = e.messages.len();
    e.apply(Action::Domain(IDS[1], Payload::new(7u8)));
    assert_eq!(e.messages.len(), n, "nobody handles it: dropped");
}

// ---- tools (design §14.1) ----

fn open_stats(n: usize, arg: Option<Payload>) -> Action {
    Action::OpenTool { domain: IDS[n], id: STATS, arg }
}

/// The open tool tabs' domains and counts.
fn stats(e: &Editor) -> Vec<(DomainId, usize)> {
    e.tabs.iter().filter_map(|t| t.get::<StatsDoc>().map(|d| (t.domain, d.0))).collect()
}

/// A tool is unique: opening it again activates the open tab.
#[test]
fn tool_opens_once() {
    let dir = game(&["quillshade/dummy/a.note.ron"]);
    let mut e = open(one(), dir.path());
    e.apply(open_stats(0, None));
    e.apply(Action::OpenAsset(note(0, "a")));
    e.apply(open_stats(0, None));
    assert_eq!(e.tabs.len(), 2);
    assert_eq!(e.active, Some(0), "the tool tab");
    assert_eq!(e.active_key(), Some(TabKey::new(IDS[0], "@tool/stats")));
    assert_eq!(e.tab().and_then(|t| t.doc().tool()), Some(STATS));
    // An unknown tool opens nothing.
    e.apply(Action::OpenTool { domain: IDS[0], id: "nope", arg: None });
    e.apply(Action::OpenTool { domain: DomainId::new("nope"), id: STATS, arg: None });
    assert_eq!((e.tabs.len(), e.active), (2, Some(0)));
}

/// Review Focus 2: an argument for an open tool reaches the open instance; no second tab.
#[test]
fn tool_argument_reaches_open_instance() {
    let dir = game(&[]);
    let mut e = open(one(), dir.path());
    e.apply(open_stats(0, None));
    e.apply(open_stats(0, Some(Payload::new(Bump))));
    e.apply(open_stats(0, Some(Payload::new(Bump))));
    assert_eq!(stats(&e), [(IDS[0], 2)]);
    assert_eq!(e.tabs.len(), 1);
}

/// Uniqueness is per `(domain, tool id)`: one tool registered by two domains gives a tab each.
#[test]
fn tool_tabs_are_per_domain() {
    let dir = game(&[]);
    let mut e = open(two(), dir.path());
    e.apply(open_stats(0, None));
    e.apply(open_stats(1, Some(Payload::new(Bump))));
    e.apply(open_stats(0, None));
    assert_eq!(stats(&e), [(IDS[0], 0), (IDS[1], 1)]);
    assert_eq!(e.active, Some(0));
}

/// Tool tabs are never written to the user config.
#[test]
fn tool_tabs_are_not_saved() {
    let dir = game(&["quillshade/dummy/a.note.ron"]);
    let mut e = open(one(), dir.path());
    let root = e.project.as_ref().unwrap().root.clone();
    e.apply(open_stats(0, None));
    e.apply(Action::OpenAsset(note(0, "a")));
    e.apply(Action::ActivateTab(0));
    e.save_config();
    assert_eq!(e.config.tabs[&root].open, ["dummy:quillshade/dummy/a.note.ron"]);
    assert_eq!(e.config.tabs[&root].active, None, "the active tool is not remembered");
}

/// Review Focus 2: Back / Forward reach a closed tool, which reopens.
#[test]
fn back_reopens_a_closed_tool() {
    let dir = game(&["quillshade/dummy/a.note.ron"]);
    let mut e = open(one(), dir.path());
    e.apply(Action::OpenAsset(note(0, "a")));
    e.apply(open_stats(0, None));
    e.apply(Action::CloseTab(1));
    assert!(stats(&e).is_empty());
    e.apply(Action::NavBack);
    assert_eq!(stats(&e), [(IDS[0], 0)], "reopened");
    assert_eq!(e.active_key(), Some(TabKey::new(IDS[0], "@tool/stats")));
    e.apply(Action::NavForward);
    assert_eq!(e.tab().and_then(|t| t.doc().asset()), Some(&note(0, "a")));
    assert_eq!(e.tabs.len(), 2);
}

/// A tool registered with a main menu has an item `"{title}..."` there; File's follow "Close Project" (design §8.1:
/// where Project Settings... goes).
#[test]
fn tool_menu_item() {
    let dir = game(&[]);
    let mut e = open(one(), dir.path());
    assert_eq!(e.active_domain, Some(IDS[0]), "the tool's domain is the active one");
    assert_eq!(e.domain_list.tools(IDS[0]).len(), 1);
    assert!(e.domain_list.tool(IDS[0], STATS).is_some() && e.domain_list.tool(IDS[1], STATS).is_none());
    let entries = menu_entries(MainMenu::File, &e);
    let labels: Vec<&str> = entries.iter().map(|m| m.label.as_str()).collect();
    assert_eq!(labels[2..6], ["Close Project", "Stats...", "Project Settings...", ""], "right after Close Project");
    let item = &entries[3];
    assert_eq!(item.action, open_stats(0, None));
    e.apply(item.action.clone());
    assert_eq!(stats(&e), [(IDS[0], 0)]);
    assert!(menu_entries(MainMenu::Assets, &e).iter().all(|m| m.label != "Stats..."));
}

// ---- the project domain, the active domain (design §14.2, §14.4) ----

const PROJECT_TOOL: ToolId = "x";

/// The built-in project domain (Project Settings) with one more tool.
struct ProjectWithTool;

impl EditorDomain for ProjectWithTool {
    fn id(&self) -> DomainId {
        DomainId::PROJECT
    }
    fn info(&self) -> DomainInfo {
        ProjectDomain.info()
    }
    fn backend(&self) -> Option<Box<dyn project_core::regen::DomainBackend>> {
        None
    }
    fn register(&self, r: &mut DomainRegistrar) {
        ProjectDomain.register(r);
        let open = |_: &mut Editor| -> Box<dyn TabDoc> { Box::new(StatsDoc(0, PROJECT_TOOL)) };
        r.tool(ToolDef { id: PROJECT_TOOL, title: "X", open, menu: Some(MainMenu::File) });
    }
}

/// The two dummy domains, then a project domain with tool `x`.
fn two_with_project_tool() -> Arc<DomainList> {
    let domains = vec![Box::new(Dummy::<0>) as Box<dyn EditorDomain>, Box::new(Dummy::<1>)];
    Arc::new(DomainList::with_project_domain(domains, &ProjectWithTool))
}

/// A domain claiming the reserved id.
struct Squatter;

impl EditorDomain for Squatter {
    fn id(&self) -> DomainId {
        DomainId::PROJECT
    }
    fn info(&self) -> DomainInfo {
        DomainInfo { title: "Squatter", icon: IconId::File.into(), summary: "" }
    }
    fn backend(&self) -> Option<Box<dyn project_core::regen::DomainBackend>> {
        None
    }
    fn register(&self, _r: &mut DomainRegistrar) {}
}

#[test]
fn project_domain_is_last_and_reserved() {
    let two = two();
    assert_eq!(two.ids().collect::<Vec<_>>(), [IDS[0], IDS[1], DomainId::PROJECT]);
    assert_eq!(two.real_ids().collect::<Vec<_>>(), IDS);
    assert_eq!(two.get(DomainId::PROJECT).map(|d| d.info.title), Some("Project"));
    assert!(two.crate_contributions().iter().all(|c| c.domain != DomainId::PROJECT));
    let dir = game(&[]);
    let e = open(two, dir.path());
    assert!(!dir.path().join("quillshade/project").exists(), "no folder of its own");
    assert!(!e.folders.contains_key(&DomainId::PROJECT));
    let reserved = std::panic::catch_unwind(|| DomainList::new(vec![Box::new(Squatter)]));
    assert!(reserved.is_err(), "a domain may not use the reserved id");
}

/// Review Focus 1: a project tool's tab overlays the active domain without changing it; closing it goes back to a tab
/// of that domain, never to another domain's.
#[test]
fn overlay_keeps_active_domain() {
    let dir = game(&["quillshade/dummy/a.note.ron", "quillshade/other/b.note.ron"]);
    let mut e = open(two_with_project_tool(), dir.path());
    e.apply(Action::OpenAsset(note(1, "b")));
    e.apply(Action::OpenAsset(note(0, "a")));
    e.apply(Action::OpenTool { domain: DomainId::PROJECT, id: PROJECT_TOOL, arg: None });
    assert_eq!(e.active, Some(2), "the tool tab");
    assert_eq!(e.active_domain, Some(IDS[0]));
    assert!(!e.is_greyed(1) && !e.is_greyed(2), "the dummy tab and the tool");
    assert!(e.is_greyed(0), "the other domain's tab");
    let labels: Vec<String> = menu_entries(MainMenu::Assets, &e).into_iter().map(|m| m.label).collect();
    assert!(labels.contains(&"New Note...".to_string()), "the active domain's templates: {labels:?}");
    assert!(!labels.contains(&"New Memo...".to_string()), "never the other domain's: {labels:?}");
    assert_eq!(left_panes(&e), LeftPanes::Domain { tree: IDS[0], game_assets: true }, "its left sidebar");
    e.apply(Action::CloseTab(2));
    assert_eq!(e.active, Some(1));
    assert_eq!(e.tab().and_then(|t| t.doc().asset()), Some(&note(0, "a")), "never the other domain's tab");
    assert_eq!(e.active_domain, Some(IDS[0]));
    // With no tab of the active domain left, closing the tool leaves no tab active.
    e.apply(Action::OpenTool { domain: DomainId::PROJECT, id: PROJECT_TOOL, arg: None });
    e.apply(Action::CloseTab(1));
    assert_eq!(e.active_key(), Some(TabKey::tool(DomainId::PROJECT, PROJECT_TOOL)));
    e.apply(Action::CloseTab(1));
    assert_eq!((e.active, e.active_domain), (None, Some(IDS[0])));
}

#[test]
fn switch_domain_restores_last_tab() {
    let dir = game(&["quillshade/dummy/a.note.ron", "quillshade/other/b.note.ron"]);
    let mut e = open(two(), dir.path());
    e.apply(Action::OpenAsset(note(0, "a")));
    e.apply(Action::OpenAsset(note(1, "b")));
    assert_eq!(e.active_domain, Some(IDS[1]), "a real domain's tab activates its domain");
    e.apply(Action::SwitchDomain(IDS[0]));
    assert_eq!(e.active, Some(0));
    assert_eq!(e.active_domain, Some(IDS[0]));
    assert!(e.is_greyed(1) && !e.is_greyed(0));
    e.apply(Action::SwitchDomain(IDS[0]));
    assert_eq!(e.active, Some(0), "the active domain: nothing happens");
    e.apply(Action::SwitchDomain(IDS[1]));
    assert_eq!(e.active, Some(1));
    // Back goes to the other domain's tab and with it to its domain.
    e.apply(Action::NavBack);
    assert_eq!((e.active, e.active_domain), (Some(0), Some(IDS[0])));
}

#[test]
fn switch_to_empty_domain_shows_hint() {
    let dir = game(&["quillshade/dummy/a.note.ron"]);
    let mut e = open(two(), dir.path());
    e.apply(Action::OpenAsset(note(0, "a")));
    e.apply(Action::SwitchDomain(IDS[1]));
    assert_eq!((e.active, e.active_domain), (None, Some(IDS[1])));
    assert!(e.is_greyed(0));
    e.apply(Action::SwitchDomain(IDS[0]));
    assert_eq!((e.active, e.active_domain), (Some(0), Some(IDS[0])));
}

/// Review Focus 5: user configs written before the active domain existed (with `maximized`, no `active_domain` or
/// `last_tabs`), or naming a domain that is gone, open without a panic or a dialog; a tool key or a key of an unknown
/// domain in `open` is skipped; the first real domain is active.
#[test]
fn old_config_opens_cleanly() {
    let dir = game(&["quillshade/dummy/a.note.ron"]);
    let root = Project::open(dir.path()).unwrap().root;
    let root_ron = format!("{:?}", root.display().to_string());
    let open = r#"open: ["project:@tool/x", "nope:a", "dummy:quillshade/dummy/a.note.ron"]"#;
    let configs = [
        format!("(window: (800, 600), maximized: true, tabs: {{ {root_ron}: (active: Some(2), {open}) }})"),
        format!(
            "(window: (800, 600), tabs: {{ {root_ron}: (active: Some(2), {open}, active_domain: Some(\"gone\")) }})"
        ),
    ];
    for text in configs {
        let config: UserConfig = ron::from_str(&text).unwrap_or_else(|err| panic!("{err}: {text}"));
        assert_eq!(config.window, (800, 600));
        let mut e = Editor::new(two_with_project_tool()).with_config(config, None);
        e.open_project(dir.path()).unwrap();
        assert_eq!(e.tabs.len(), 1, "{text}");
        assert!(e.dialog.is_none(), "{:?}", e.dialog);
        assert_eq!(e.active_domain, Some(IDS[0]));
        assert_eq!(e.active, Some(0));
    }
}

#[test]
fn active_domain_and_last_tabs_round_trip() {
    let dir = game(&["quillshade/dummy/a.note.ron", "quillshade/other/b.note.ron", "quillshade/other/c.note.ron"]);
    let config = dir.path().join("config/config.ron");
    let mut e = open(two_with_project_tool(), dir.path());
    e.config_path = Some(config.clone());
    let root = e.project.as_ref().unwrap().root.clone();
    e.apply(Action::OpenAsset(note(1, "b")));
    e.apply(Action::OpenAsset(note(1, "c")));
    e.apply(Action::ActivateTab(0));
    e.apply(Action::OpenAsset(note(0, "a")));
    e.apply(Action::OpenTool { domain: DomainId::PROJECT, id: PROJECT_TOOL, arg: None });
    e.save_config();
    let saved = UserConfig::load(&config);
    let tabs = &saved.tabs[&root];
    assert_eq!(tabs.active_domain.as_deref(), Some("dummy"));
    let last: Vec<(&str, &str)> = tabs.last_tabs.iter().map(|(d, k)| (d.as_str(), k.as_str())).collect();
    assert_eq!(last, [("dummy", "quillshade/dummy/a.note.ron"), ("other", "quillshade/other/b.note.ron")]);
    assert!(!std::fs::read_to_string(&config).unwrap().contains("@tool"), "tool keys are never written");
    let mut next = Editor::new(two_with_project_tool()).with_config(saved, Some(config));
    next.open_project(dir.path()).unwrap();
    assert_eq!(next.tabs.len(), 3);
    assert_eq!(next.active_domain, Some(IDS[0]));
    assert_eq!(next.active_key(), Some(TabKey::new(IDS[0], "quillshade/dummy/a.note.ron")));
    next.apply(Action::SwitchDomain(IDS[1]));
    assert_eq!(next.active_key(), Some(TabKey::new(IDS[1], "quillshade/other/b.note.ron")), "the remembered tab");
}

// ---- the left sidebar and the main menus follow the active domain (design §8.1, §14.4) ----

/// The Project tree's asset rows the left sidebar shows.
fn sidebar_assets(e: &Editor) -> Vec<TreeItem> {
    let LeftPanes::Domain { tree, .. } = left_panes(e) else { panic!("no Project tree: {:?}", left_panes(e)) };
    let rows = ProjectTree(tree).rows(e).into_iter().map(|(_, i)| i);
    rows.filter(|i| matches!(i, TreeItem::Asset(_))).collect()
}

#[test]
fn sidebar_shows_active_domain_only() {
    let dir = game(&["quillshade/dummy/a.note.ron", "quillshade/other/b.note.ron"]);
    let mut e = open(two_with_project_tool(), dir.path());
    assert_eq!(e.active_domain, Some(IDS[0]));
    assert_eq!(sidebar_assets(&e), [TreeItem::Asset(note(0, "a"))]);
    e.apply(Action::SwitchDomain(IDS[1]));
    assert_eq!(sidebar_assets(&e), [TreeItem::Asset(note(1, "b"))]);
    assert_eq!(crate::ui::shell::empty_hint(&e), crate::ui::shell::ASSET_HINT);
    // The project domain: its tools in place of both panes, a click opens one.
    e.apply(Action::SwitchDomain(DomainId::PROJECT));
    assert_eq!(left_panes(&e), LeftPanes::Tools);
    assert_eq!(crate::ui::shell::empty_hint(&e), crate::ui::shell::TOOL_HINT);
    let open_x = Action::OpenTool { domain: DomainId::PROJECT, id: PROJECT_TOOL, arg: None };
    assert_eq!(
        crate::ui::tree::project_tools(&e),
        [("Project Settings".to_string(), open_settings()), ("X".into(), open_x)]
    );
}

#[test]
fn game_assets_follow_active_domain() {
    let dir = game(&["assets/a.txt", "quillshade/other/b.note.ron"]);
    let mut e = open(two(), dir.path());
    let file = TreeItem::GameFile("a.txt".into());
    let menu = |e: &Editor| {
        let row = TreeRow { tree: TreeId::GameAssets, item: file.clone() };
        row_menu(e, &row).into_iter().map(|m| m.label).collect::<Vec<_>>()
    };
    assert_eq!(left_panes(&e), LeftPanes::Domain { tree: IDS[0], game_assets: true });
    assert!(GameAssetsTree.rows(&e).into_iter().any(|(_, i)| i == file));
    assert_eq!(menu(&e), ["Inspect a.txt"]);
    e.apply(Action::OpenGameImage("a.txt".into()));
    assert_eq!(e.messages.last().map(|m| (m.domain, m.text.as_str())), Some((Some(IDS[0]), "opened game file a.txt")));

    e.apply(Action::SwitchDomain(IDS[1]));
    assert_eq!(left_panes(&e), LeftPanes::Domain { tree: IDS[1], game_assets: false }, "Game Assets hidden");
    assert!(GameAssetsTree.rows(&e).into_iter().all(|(_, i)| i != file));
    assert!(menu(&e).is_empty(), "no menu of another domain");
    let n = e.messages.len();
    e.apply(Action::OpenGameImage("a.txt".into()));
    assert_eq!(e.messages.len(), n, "a double click opens nothing");
}

#[test]
fn assets_menu_is_per_domain() {
    let dir = game(&[]);
    let mut e = open(two_with_project_tool(), dir.path());
    let labels = |e: &Editor| menu_entries(MainMenu::Assets, e).into_iter().map(|m| m.label).collect::<Vec<_>>();
    assert_eq!(labels(&e), ["New Note...", "New folder...", "", "Generate"]);
    let entries = menu_entries(MainMenu::Assets, &e);
    assert_eq!(entries[1].action, Action::NewFolderDialog(Some(IDS[0])));
    assert_eq!((entries[3].action.clone(), entries[3].icon), (Action::RegenAll, Some(IconId::Regenerate.into())));
    e.apply(Action::SwitchDomain(IDS[1]));
    assert_eq!(labels(&e), ["New Memo...", "New folder...", "", "Generate"]);
    assert_eq!(menu_entries(MainMenu::Assets, &e)[1].action, Action::NewFolderDialog(Some(IDS[1])));
    e.apply(Action::SwitchDomain(DomainId::PROJECT));
    assert_eq!(labels(&e), ["Generate"], "the project domain has no asset kinds");
}

/// A domain's main menu items and tool items show only while it is active; the project domain's always (design
/// §14.2).
#[test]
fn domain_menu_items_follow_active_domain() {
    let dir = game(&[]);
    let mut e = open(two_with_project_tool(), dir.path());
    let labels = |e: &Editor, menu| menu_entries(menu, e).into_iter().map(|m| m.label).collect::<Vec<_>>();
    let edit = labels(&e, MainMenu::Edit);
    assert!(edit.contains(&"Tidy dummy".to_string()) && !edit.contains(&"Tidy other".to_string()), "{edit:?}");
    let open_x = Action::OpenTool { domain: DomainId::PROJECT, id: PROJECT_TOOL, arg: None };
    let stats = |e: &Editor| {
        let tools = menu_entries(MainMenu::File, e).into_iter().filter(|m| m.label.ends_with("..."));
        let tools = tools.filter(|m| matches!(m.action, Action::OpenTool { .. }));
        tools.map(|m| m.action).collect::<Vec<_>>()
    };
    let project_tools = [open_settings(), open_x];
    assert_eq!(stats(&e)[..], [[open_stats(0, None)].as_slice(), &project_tools].concat(), "the project's last");
    e.apply(Action::SwitchDomain(IDS[1]));
    let edit = labels(&e, MainMenu::Edit);
    assert!(edit.contains(&"Tidy other".to_string()) && !edit.contains(&"Tidy dummy".to_string()), "{edit:?}");
    assert_eq!(stats(&e)[..], [[open_stats(1, None)].as_slice(), &project_tools].concat());
    e.apply(Action::SwitchDomain(DomainId::PROJECT));
    assert_eq!(stats(&e), project_tools, "only the project's tools");
    assert!(!labels(&e, MainMenu::Edit).iter().any(|l| l.starts_with("Tidy")));
}

/// Domain `dummy` has 1 error, `other` has 2 (each against its own note).
fn with_problems(e: &mut Editor) {
    let err = |n: usize, p: &str| {
        let mut d = Diagnostic::error("E1", "bad").in_asset(format!("quillshade/{}/{p}.note.ron", IDS[n]));
        d.domain = Some(IDS[n]);
        d
    };
    e.domains.get_mut::<DummyState<0>>().unwrap().problems = vec![err(0, "a")];
    e.domains.get_mut::<DummyState<1>>().unwrap().problems = vec![err(1, "b"), err(1, "c")];
}

/// Design §6.4: the console shows the active domain's diagnostics unless "All domains" is on; the mainbar counts all.
#[test]
fn console_filters_by_active_domain() {
    let dir = game(&["quillshade/dummy/a.note.ron", "quillshade/other/b.note.ron"]);
    let mut e = open(two(), dir.path());
    with_problems(&mut e);
    e.apply(Action::SwitchDomain(IDS[0]));
    assert_eq!((e.console_diagnostics().len(), e.error_count()), (1, 3));
    e.apply(Action::SwitchDomain(IDS[1]));
    assert_eq!((e.console_diagnostics().len(), e.error_count()), (2, 3));
    e.apply(Action::SwitchDomain(IDS[0]));
    e.apply(Action::ToggleConsoleAll);
    assert!(e.console_all);
    assert_eq!((e.console_diagnostics().len(), e.error_count()), (3, 3));
    e.apply(Action::SwitchDomain(DomainId::PROJECT));
    e.apply(Action::ToggleConsoleAll);
    assert_eq!(e.console_diagnostics().len(), 3, "the project domain active: every domain");
}

#[test]
fn global_messages_always_show() {
    let dir = game(&["quillshade/dummy/a.note.ron"]);
    let mut e = open(two(), dir.path());
    e.messages.clear();
    e.domain_msg(None, Level::Info, "global");
    e.domain_msg(Some(IDS[0]), Level::Info, "mine");
    e.domain_msg(Some(IDS[1]), Level::Info, "theirs");
    // The project domain's lines (Project Settings) are about every domain.
    e.domain_msg(Some(DomainId::PROJECT), Level::Warning, "project");
    let shown = |e: &Editor| e.console_messages().map(|m| m.text.clone()).collect::<Vec<_>>();
    e.apply(Action::SwitchDomain(IDS[0]));
    assert_eq!(shown(&e), ["global", "mine", "project"]);
    e.apply(Action::SwitchDomain(IDS[1]));
    assert_eq!(shown(&e), ["global", "theirs", "project"]);
    e.apply(Action::ToggleConsoleAll);
    assert_eq!(shown(&e).len(), 4);
}

#[test]
fn focus_other_domain_diagnostic_switches() {
    let dir = game(&["quillshade/dummy/a.note.ron", "quillshade/other/b.note.ron"]);
    let mut e = open(two(), dir.path());
    with_problems(&mut e);
    e.apply(Action::SwitchDomain(IDS[0]));
    e.apply(Action::ToggleConsoleAll);
    let theirs = e.console_diagnostics().into_iter().find(|d| d.domain == Some(IDS[1])).unwrap();
    e.apply(Action::FocusDiagnostic(theirs));
    assert_eq!(e.active_domain, Some(IDS[1]));
}

// ---- Project Settings (design §14.2) ----

use crate::dynval::{PropKey, PropValue};
use crate::settings::{SETTINGS_TOOL, SettingsDoc, SettingsProp};

/// The first dummy domain with a generator, so the umbrella crate has its module.
struct Backed;

/// [`Backed`]'s generator: nothing but its module of the umbrella crate.
struct DummyBackend;

impl project_core::regen::DomainBackend for DummyBackend {
    fn id(&self) -> DomainId {
        IDS[0]
    }
    fn crate_contribution(&self) -> project_core::umbrella::CrateContribution {
        project_core::umbrella::CrateContribution {
            domain: IDS[0],
            plugin: "DummyPlugin",
            bevy_features: &[],
            dependencies: &[],
        }
    }
    fn plan(&self, _project: &Project, _compile: bool) -> project_core::regen::DomainPlan {
        unreachable!("the editor tests never run the CLI path")
    }
}

impl EditorDomain for Backed {
    fn id(&self) -> DomainId {
        IDS[0]
    }
    fn info(&self) -> DomainInfo {
        Dummy::<0>.info()
    }
    fn backend(&self) -> Option<Box<dyn project_core::regen::DomainBackend>> {
        Some(Box::new(DummyBackend))
    }
    fn register(&self, r: &mut DomainRegistrar) {
        Dummy::<0>.register(r);
    }
}

fn open_settings() -> Action {
    Action::OpenTool { domain: DomainId::PROJECT, id: SETTINGS_TOOL, arg: None }
}

fn set(prop: SettingsProp, on: bool) -> Action {
    Action::Tab(TabAction::SetValue { key: PropKey::new(prop), value: PropValue::Bool(on), merge: false })
}

fn shown(e: &Editor, prop: SettingsProp) -> Option<PropValue> {
    e.current_value(&PropKey::new(prop))
}

/// Toggling "Used in game" off and saving writes `project.ron` and the umbrella crate (the module stays, the plugin
/// goes); Undo and Save bring both back.
#[test]
fn settings_tool_saves_and_regenerates_umbrella() {
    let dir = game(&[]);
    let mut e = open(list(vec![Box::new(Backed)]), dir.path());
    let p = e.project.clone().unwrap();
    let lib = || std::fs::read_to_string(p.crate_dir().join("src/lib.rs")).unwrap();
    let ron = || std::fs::read_to_string(p.settings_path()).unwrap();
    e.apply(open_settings());
    assert_eq!(e.active_key(), Some(TabKey::tool(DomainId::PROJECT, SETTINGS_TOOL)));
    let doc = e.active_doc::<SettingsDoc>().expect("the settings tab");
    assert!(doc.asset().is_none() && doc.config_key().is_none() && doc.tool() == Some(SETTINGS_TOOL));
    assert_eq!(shown(&e, SettingsProp::Enabled(IDS[0])), Some(PropValue::Bool(true)));

    e.apply(set(SettingsProp::Enabled(IDS[0]), false));
    assert!(e.is_dirty());
    assert_eq!(shown(&e, SettingsProp::Enabled(IDS[0])), Some(PropValue::Bool(false)));
    e.apply(Action::Save);
    assert!(!e.is_dirty() && e.dialog.is_none(), "{:?}", e.dialog);
    assert!(ron().contains("enabled: false"), "{}", ron());
    assert!(!e.project.as_ref().unwrap().domain_enabled(IDS[0]), "the editor's project follows");
    assert!(lib().contains("pub mod dummy;") && !lib().contains("dummy::DummyPlugin"), "{}", lib());

    e.apply(TabAction::Undo);
    assert!(e.is_dirty());
    assert_eq!(shown(&e, SettingsProp::Enabled(IDS[0])), Some(PropValue::Bool(true)));
    e.apply(Action::Save);
    assert!(!e.is_dirty());
    assert!(!ron().contains("enabled"), "{}", ron());
    assert!(lib().contains("pub mod dummy;") && lib().contains("dummy::DummyPlugin"), "{}", lib());
    assert!(e.project.as_ref().unwrap().domain_enabled(IDS[0]));
}

/// Review Focus 3: an outside edit of `project.ron` reloads a clean Settings tab, asks Keep mine / Reload for a dirty
/// one, and an unparsable file keeps the settings and the tab with a warning.
#[test]
fn settings_outside_edit() {
    let dir = game(&[]);
    let mut e = open(one(), dir.path());
    let path = e.project.as_ref().unwrap().settings_path();
    let key = TabKey::tool(DomainId::PROJECT, SETTINGS_TOOL);
    e.apply(open_settings());

    // Clean: reloaded.
    std::fs::write(&path, "ProjectSettings(format_version: 1, header_date: true)\n").unwrap();
    e.files_changed(vec![path.clone()]);
    assert_eq!(shown(&e, SettingsProp::HeaderDate), Some(PropValue::Bool(true)));
    assert!(!e.is_dirty() && e.dialog.is_none(), "{:?}", e.dialog);

    // Dirty: asks.
    e.apply(set(SettingsProp::HeaderDate, false));
    std::fs::write(
        &path,
        "ProjectSettings(format_version: 1, header_date: true, domains: {\"dummy\": (enabled: false)})",
    )
    .unwrap();
    e.files_changed(vec![path.clone()]);
    let Some(Dialog::Confirm { buttons, .. }) = e.dialog.clone() else { panic!("no dialog: {:?}", e.dialog) };
    let expected = [("Keep mine".to_string(), Action::KeepMine(key.clone())), ("Reload".into(), Action::Reload(key))];
    assert_eq!(buttons, expected);
    assert!(!e.project.as_ref().unwrap().domain_enabled(IDS[0]), "the project follows the file anyway");
    e.apply(buttons[0].1.clone());
    assert!(e.dialog.is_none() && e.is_dirty(), "Keep mine: the edits stay");
    assert_eq!(shown(&e, SettingsProp::HeaderDate), Some(PropValue::Bool(false)));
    assert_eq!(shown(&e, SettingsProp::Enabled(IDS[0])), Some(PropValue::Bool(true)));

    // Unparsable: a warning, nothing else changes.
    let before = e.project.as_ref().unwrap().settings.clone();
    let messages = e.messages.len();
    std::fs::write(&path, "ProjectSettings(format_version: 1, header_date: ").unwrap();
    e.files_changed(vec![path.clone()]);
    assert_eq!(e.project.as_ref().unwrap().settings, before);
    assert!(e.dialog.is_none(), "{:?}", e.dialog);
    assert!(e.is_dirty());
    assert_eq!(shown(&e, SettingsProp::HeaderDate), Some(PropValue::Bool(false)));
    assert_eq!(e.messages.len(), messages + 1);
    let last = e.messages.last().unwrap();
    assert_eq!(last.level, Level::Warning);
    assert!(last.text.contains("project.ron") && last.text.contains("keeping the previous settings"), "{}", last.text);

    // Dirty again, Reload: the file's version.
    std::fs::write(&path, "ProjectSettings(format_version: 1, header_date: true)").unwrap();
    e.files_changed(vec![path.clone()]);
    let Some(Dialog::Confirm { buttons, .. }) = e.dialog.clone() else { panic!("no dialog: {:?}", e.dialog) };
    e.apply(buttons[1].1.clone());
    assert!(e.dialog.is_none() && !e.is_dirty());
    assert_eq!(shown(&e, SettingsProp::HeaderDate), Some(PropValue::Bool(true)));
    assert_eq!(shown(&e, SettingsProp::Enabled(IDS[0])), Some(PropValue::Bool(true)));
}

/// Design §14.2: the project domain with no tab of its own opens Project Settings; switching back to it later
/// activates that tab, and after it closed, opens it again. File → Project Settings... opens it too.
#[test]
fn switch_to_project_opens_settings() {
    let dir = game(&["quillshade/dummy/a.note.ron"]);
    let mut e = open(one(), dir.path());
    let settings = Some(TabKey::tool(DomainId::PROJECT, SETTINGS_TOOL));
    e.apply(Action::OpenAsset(note(0, "a")));
    e.apply(Action::SwitchDomain(DomainId::PROJECT));
    assert_eq!((e.active_key(), e.active_domain), (settings.clone(), Some(DomainId::PROJECT)));
    assert!(e.active_doc::<SettingsDoc>().is_some());
    e.apply(Action::SwitchDomain(IDS[0]));
    assert_eq!(e.active, Some(0));
    e.apply(Action::SwitchDomain(DomainId::PROJECT));
    assert_eq!((e.tabs.len(), e.active), (2, Some(1)), "the open tab, not a second one");
    e.apply(Action::CloseTab(1));
    e.apply(Action::SwitchDomain(IDS[0]));
    e.apply(Action::SwitchDomain(DomainId::PROJECT));
    assert_eq!((e.tabs.len(), e.active_key()), (2, settings));
    let item = menu_entries(MainMenu::File, &e).into_iter().find(|m| m.label == "Project Settings...");
    assert_eq!(item.map(|m| m.action), Some(open_settings()));
}

// ---- core defaults of the asset kinds (design §14.4) ----

/// A domain of `*.note.ron` files that relies on core's defaults: no `tree_items`, `parse_tab`, `tab_exists`,
/// `rename_file` or `delete_file` of its own (design §14.4). Its id is the first dummy's.
struct Plain;

/// What the plain domain keeps: only the diagnostics tests set.
struct PlainState {
    problems: Vec<Diagnostic>,
}

/// The plain domain's note kind icon, distinct from the generic file icon of other rows.
const PLAIN_ICON: IconId = IconId::Comment;

impl EditorDomain for Plain {
    fn id(&self) -> DomainId {
        IDS[0]
    }

    fn info(&self) -> DomainInfo {
        DomainInfo { title: "Plain", icon: IconId::File.into(), summary: "Plain — notes through core's defaults." }
    }

    fn backend(&self) -> Option<Box<dyn project_core::regen::DomainBackend>> {
        None
    }

    fn register(&self, r: &mut DomainRegistrar) {
        r.state(|| Box::new(PlainState { problems: Vec::new() }))
            .asset_kind(AssetKindDef {
                suffix: NOTE,
                title: "Note",
                icon: PLAIN_ICON.into(),
                templates: Vec::new(),
                open: |e, r| {
                    let exists = e.project.as_ref().is_some_and(|p| p.root.join(r.source_rel()).is_file());
                    exists.then(|| Tab::new(r.domain, NoteDoc(r.clone())))
                },
                row_menu: |_, _| Vec::new(),
            })
            .editor_kind::<NoteEditor>();
    }
}

impl DomainState for PlainState {
    fn id(&self) -> DomainId {
        IDS[0]
    }

    fn diagnostics(&self) -> Vec<Diagnostic> {
        self.problems.clone()
    }
}

fn plain() -> Arc<DomainList> {
    list(vec![Box::new(Plain)])
}

/// Design §14.4: a domain without `tree_items` gets the files of its registered kinds as Project tree rows, each with
/// its kind's icon and the error colour while the domain has an error against it; other files are not listed.
#[test]
fn default_tree_lists_registered_kinds() {
    let dir = game(&["quillshade/dummy/x.note.ron", "quillshade/dummy/y.other.ron", "quillshade/dummy/sub/z.note.ron"]);
    let mut e = open(plain(), dir.path());
    let tree = ProjectTree(IDS[0]);
    assert_eq!(
        tree.rows(&e),
        [
            (0, TreeItem::Folder(String::new())),
            (1, TreeItem::Folder("sub".into())),
            (2, TreeItem::Asset(note(0, "sub/z"))),
            (1, TreeItem::Asset(note(0, "x"))),
        ]
    );
    let items = tree.items(&e);
    assert!(items.iter().all(|i| i.icon == IconRef::from(PLAIN_ICON) && !i.error), "{items:?}");
    let mut d = Diagnostic::error("E1", "bad").in_asset("quillshade/dummy/x.note.ron");
    d.domain = Some(IDS[0]);
    e.domains.get_mut::<PlainState>().unwrap().problems =
        vec![d, Diagnostic::warning("W1", "meh").in_asset("quillshade/dummy/sub/z.note.ron")];
    let errors: Vec<(AssetRef, bool)> = tree.items(&e).into_iter().map(|i| (i.asset, i.error)).collect();
    assert_eq!(errors, [(note(0, "sub/z"), false), (note(0, "x"), true)], "a warning is no error");
    // A file written outside the editor shows after the rescan.
    std::fs::write(dir.path().join("quillshade/dummy/w.note.ron"), "()").unwrap();
    e.rescan();
    assert!(tree.items(&e).iter().any(|i| i.asset == note(0, "w")));
}

/// Design §14.4: rename / move and delete of a domain without file hooks: the file moves (its folders created), the
/// open tab follows, Back lands on the renamed key; delete removes the file and closes the tab.
#[test]
fn default_rename_and_delete() {
    let dir = game(&["quillshade/dummy/x.note.ron", "quillshade/dummy/w.note.ron"]);
    let mut e = open(plain(), dir.path());
    e.apply(Action::OpenAsset(note(0, "x")));
    e.apply(Action::OpenAsset(note(0, "w")));
    e.apply(Action::RenameDialog(note(0, "x")));
    e.apply(Action::Submit("sub/x2".into()));
    assert!(e.dialog.is_none(), "{:?}", e.dialog);
    assert!(!dir.path().join("quillshade/dummy/x.note.ron").exists());
    assert!(dir.path().join("quillshade/dummy/sub/x2.note.ron").is_file());
    let moved = note(0, "sub/x2");
    assert!(e.asset_tab(&moved).is_some(), "the open tab follows");
    assert!(ProjectTree(IDS[0]).items(&e).iter().any(|i| i.asset == moved), "rescanned");
    e.apply(Action::NavBack);
    assert_eq!(e.active_key(), Some(TabKey::new(IDS[0], moved.source_rel())));
    // A taken name is refused.
    e.apply(Action::MoveTo(note(0, "w"), "sub".into()));
    e.apply(Action::RenameDialog(note(0, "sub/w")));
    e.apply(Action::Submit("sub/x2".into()));
    assert!(matches!(e.dialog, Some(Dialog::Message { .. })), "{:?}", e.dialog);
    assert!(dir.path().join("quillshade/dummy/sub/w.note.ron").is_file());
    e.apply(Action::CloseDialog);

    e.apply(Action::Delete(moved.clone()));
    assert!(!dir.path().join(moved.source_rel()).exists());
    assert!(e.asset_tab(&moved).is_none(), "its tab closed");
    assert!(!ProjectTree(IDS[0]).items(&e).iter().any(|i| i.asset == moved));
}

/// Design §14.4: a saved tab key that is a registered kind's source path restores without a `parse_tab`; one whose
/// file is gone, or of no registered kind, is skipped.
#[test]
fn default_tab_key_restores() {
    let dir = game(&["quillshade/dummy/x.note.ron", "quillshade/dummy/y.other.ron"]);
    let root = Project::open(dir.path()).unwrap().root;
    let open = [
        "dummy:quillshade/dummy/gone.note.ron",
        "dummy:quillshade/dummy/x.note.ron",
        "dummy:quillshade/dummy/y.other.ron",
    ];
    let mut e = Editor::new(plain());
    e.config
        .tabs
        .insert(root, ProjectTabs { open: open.map(String::from).to_vec(), active: Some(1), ..Default::default() });
    e.open_project(dir.path()).unwrap();
    assert!(e.dialog.is_none(), "{:?}", e.dialog);
    assert_eq!(e.tabs.len(), 1);
    assert_eq!(e.active_key(), Some(TabKey::new(IDS[0], "quillshade/dummy/x.note.ron")));
    // Back / Forward treats a vanished file as gone.
    let key = TabKey::new(IDS[0], "quillshade/dummy/gone.note.ron");
    assert!(!e.key_exists(&key));
    assert!(e.key_exists(&TabKey::new(IDS[0], "quillshade/dummy/x.note.ron")));
}

const BUILT: [DomainId; 2] = [DomainId::new("alpha"), DomainId::new("beta")];

/// A domain with a module in the umbrella crate whose state logs its `generate`.
struct Built<const N: usize>;

struct BuiltState<const N: usize>;

impl<const N: usize> project_core::regen::DomainBackend for Built<N> {
    fn id(&self) -> DomainId {
        BUILT[N]
    }
    fn crate_contribution(&self) -> project_core::umbrella::CrateContribution {
        project_core::umbrella::CrateContribution {
            domain: BUILT[N],
            plugin: "Plugin",
            bevy_features: &[],
            dependencies: &[],
        }
    }
    fn plan(&self, _project: &Project, _compile: bool) -> project_core::regen::DomainPlan {
        unreachable!("the editor runs the state's generate")
    }
}

impl<const N: usize> EditorDomain for Built<N> {
    fn id(&self) -> DomainId {
        BUILT[N]
    }
    fn info(&self) -> DomainInfo {
        DomainInfo { title: "Built", icon: IconId::File.into(), summary: "" }
    }
    fn backend(&self) -> Option<Box<dyn project_core::regen::DomainBackend>> {
        Some(Box::new(Built::<N>))
    }
    fn register(&self, r: &mut DomainRegistrar) {
        r.state(|| Box::new(BuiltState::<N>));
    }
}

impl<const N: usize> DomainState for BuiltState<N> {
    fn id(&self) -> DomainId {
        BUILT[N]
    }
    fn generate(&mut self, editor: &mut Editor) {
        editor.info(format!("generated {}", BUILT[N]));
    }
}

/// Design §6.5: opening a project generates each domain whose module of the umbrella crate is missing, so a save of
/// another domain (which rewrites `src/lib.rs` naming every module) never leaves the crate without it; a domain with
/// its module, or without a backend, does not generate.
#[test]
fn open_generates_missing_modules() {
    let dir = game(&["crates/quillshade/src/alpha/mod.rs"]);
    let domains = vec![Box::new(Built::<0>) as Box<dyn EditorDomain>, Box::new(Built::<1>), Box::new(Dummy::<0>)];
    let e = open(list(domains), dir.path());
    let generated: Vec<&str> =
        e.messages.iter().filter(|m| m.text.starts_with("generated")).map(|m| m.text.as_str()).collect();
    assert_eq!(generated, ["generated beta"]);
    let line = e.messages.iter().find(|m| m.text == "generated beta").unwrap();
    assert_eq!(line.domain, Some(BUILT[1]), "the domain's own line");
}
