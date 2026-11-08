//! The editor's asset index (ADR 0047 §10, §11, §16): built on open and rescan, refreshed after saves, its
//! diagnostics in the console/logs, the editor statusbar and the mainbar's count, and the UUID quick fixes (dialogs,
//! tree menu, core's default `set_asset_uuid`) on a test domain whose backend indexes a tiny text format, and the
//! cross-domain follow-up of a save (`regenerate_affected`, techdoc §4.3) with a second such domain.

use project_core::asset_index::{IndexedAsset, IndexedRef};
use project_core::{AssetUuid, DomainIndex};

use super::*;

const IDX: DomainId = DomainId::new("idx");
const CARD: &str = "card";

/// A domain of `*.card.ron` text files, indexed by [`CardBackend`].
struct Indexed;

/// Indexes a domain's files of one suffix (`*.card.ron` of [`IDX`]): a file whose first line is `v2` is in the
/// current format, its `id: "<uuid>"` line is its id (none: nil, so `AE004`) and every `ref: "<uuid>"` line a
/// reference (location `ref`). Other files are legacy.
struct CardBackend(DomainId, &'static str);

/// The domain's state: every hook at its default (core's `set_asset_uuid` included).
struct IdxState;

/// A card in a tab: its text as edited and as last saved or loaded.
struct CardDoc {
    r: AssetRef,
    text: String,
    saved: String,
}

/// An edit of a [`CardDoc`]: appends a line.
#[derive(Debug, PartialEq)]
struct Append(&'static str);

fn card(path: &str) -> AssetRef {
    AssetRef { domain: IDX, path: path.into(), suffix: CARD }
}

fn read(p: &Project, r: &AssetRef) -> String {
    std::fs::read_to_string(p.root.join(r.source_rel())).unwrap()
}

impl project_core::regen::DomainBackend for CardBackend {
    fn id(&self) -> DomainId {
        self.0
    }
    fn crate_contribution(&self) -> project_core::umbrella::CrateContribution {
        project_core::umbrella::CrateContribution {
            domain: self.0,
            plugin: "IdxPlugin",
            loader: None,
            bevy_features: &[],
            dependencies: &[],
        }
    }
    fn index(&self, project: &Project) -> DomainIndex {
        let files = project_core::files::walk_files(&project.domain_dir(self.0));
        let assets = files.into_iter().filter_map(|f| {
            let asset = project_core::asset_ref::parse(&project.rel(&f), self.0, &[self.1])?;
            let text = std::fs::read_to_string(&f).ok()?;
            if text.lines().next() != Some("v2") {
                return None;
            }
            let id = match text.lines().find_map(|l| value(l, "id")) {
                Some(s) => AssetUuid::parse(s).ok_or_else(|| "has a malformed `id`".to_string()),
                None => Ok(AssetUuid::NIL),
            };
            let refs = text.lines().filter_map(|l| AssetUuid::parse(value(l, "ref")?));
            let refs = refs.map(|to| IndexedRef { to, sub: None, location: Some("ref".into()) }).collect();
            Some(IndexedAsset { asset, id, sub_items: Vec::new(), refs })
        });
        DomainIndex { domain: self.0, assets: assets.collect() }
    }
    fn plan(
        &self,
        _project: &Project,
        _index: &project_core::AssetIndex,
        _compile: bool,
    ) -> project_core::regen::DomainPlan {
        unreachable!("the editor tests never run the CLI path")
    }
}

impl EditorDomain for Indexed {
    fn id(&self) -> DomainId {
        IDX
    }
    fn info(&self) -> DomainInfo {
        DomainInfo { title: "Indexed", icon: IconId::File.into(), summary: "Cards with asset UUIDs, for the tests." }
    }
    fn backend(&self) -> Option<Box<dyn project_core::regen::DomainBackend>> {
        Some(Box::new(CardBackend(IDX, CARD)))
    }
    fn register(&self, r: &mut DomainRegistrar) {
        r.state(|| Box::new(IdxState))
            .asset_kind(AssetKindDef {
                suffix: CARD,
                title: "Card",
                icon: IconId::File.into(),
                templates: vec![AssetTemplate {
                    label: "Card",
                    base_name: "new_card",
                    icon: IconId::File.into(),
                    create: |p, r| std::fs::write(p.root.join(r.source_rel()), "v2\n"),
                }],
                open: |e, r| {
                    let text = std::fs::read_to_string(e.project.as_ref()?.root.join(r.source_rel())).ok()?;
                    Some(Tab::new(IDX, CardDoc { r: r.clone(), saved: text.clone(), text }))
                },
                row_menu: |_, _| Vec::new(),
            })
            .editor_kind::<NoteEditor>()
            .doc_view(|| Box::new(bsn! { Node }));
    }
}

impl DomainState for IdxState {
    fn id(&self) -> DomainId {
        IDX
    }
}

impl TabDoc for CardDoc {
    fn asset(&self) -> Option<&AssetRef> {
        Some(&self.r)
    }
    fn view(&self) -> ViewKindId {
        NoteEditor::VIEW
    }
    fn base_title(&self) -> String {
        self.r.source_rel()
    }
    fn icon(&self) -> IconRef {
        IconId::File.into()
    }
    fn watermark(&self) -> Option<&'static str> {
        Some(CARD)
    }
    fn is_dirty(&self) -> bool {
        self.text != self.saved
    }
    fn is_read_only(&self) -> bool {
        false
    }
    fn revision(&self) -> u64 {
        0
    }
    fn save(&mut self, ctx: &mut DocCtx) -> std::io::Result<()> {
        let p = ctx.project.ok_or_else(|| std::io::Error::other("no project"))?;
        std::fs::write(p.root.join(self.r.source_rel()), &self.text)?;
        self.saved = self.text.clone();
        Ok(())
    }
    fn changed_on_disk(&self, project: &Project) -> bool {
        std::fs::read_to_string(project.root.join(self.r.source_rel())).is_ok_and(|t| t != self.saved)
    }
    fn reload_from_disk(&mut self, ctx: &mut DocCtx) -> Result<(), String> {
        let p = ctx.project.ok_or("no project")?;
        self.text = std::fs::read_to_string(p.root.join(self.r.source_rel())).map_err(|e| e.to_string())?;
        self.saved = self.text.clone();
        Ok(())
    }
    fn keep_mine(&mut self, _project: &Project) {}
    fn rename(&mut self, _project: &Project, _from: &AssetRef, _to: &AssetRef) {}
    fn config_key(&self) -> Option<String> {
        Some(self.r.source_rel())
    }
    fn apply(&mut self, _ctx: &mut DocCtx, action: &TabAction) -> Handled {
        let TabAction::Doc(p) = action else { return Handled::No };
        let Some(Append(line)) = p.downcast::<Append>() else { return Handled::No };
        self.text.push_str(line);
        Handled::Yes
    }
}

/// The quoted value of a `key: "<value>"` line.
fn value<'a>(line: &'a str, key: &str) -> Option<&'a str> {
    line.strip_prefix(key)?.strip_prefix(": \"")?.strip_suffix('"')
}

/// A project of the [`Indexed`] domain with `cards` (path, text) opened in an editor.
fn cards(cards: &[(&str, String)]) -> (tempfile::TempDir, Editor) {
    let files: Vec<String> = cards.iter().map(|(path, _)| card(path).source_rel()).collect();
    let dir = game(&files.iter().map(String::as_str).collect::<Vec<_>>());
    for ((path, text), file) in cards.iter().zip(&files) {
        std::fs::write(dir.path().join(file), text).unwrap_or_else(|e| panic!("{path}: {e}"));
    }
    let e = open(list(vec![Box::new(Indexed)]), dir.path());
    (dir, e)
}

/// A current-format card with `id` and references to `refs`.
fn text(id: AssetUuid, refs: &[AssetUuid]) -> String {
    let mut t = format!("v2\nid: \"{id}\"\n");
    for r in refs {
        t.push_str(&format!("ref: \"{r}\"\n"));
    }
    t
}

fn codes(diags: &[Diagnostic]) -> Vec<(&str, &str)> {
    diags.iter().map(|d| (d.code.as_str(), d.asset.as_deref().unwrap_or("-"))).collect()
}

/// The index diagnostic `code` of the open project (panics without one).
fn index_diag(e: &Editor, code: &str) -> Diagnostic {
    e.index_diagnostics.iter().find(|d| d.code == code).cloned().unwrap_or_else(|| panic!("no {code}"))
}

fn rel(path: &str) -> String {
    card(path).source_rel()
}

#[test]
fn index_is_built_on_open_and_rescan() {
    let (a, b) = (AssetUuid::new_v7(), AssetUuid::new_v7());
    let (dir, mut e) = cards(&[("a", text(a, &[])), ("legacy", "()".into())]);
    assert_eq!(e.asset_index.get(a).map(|x| &x.asset), Some(&card("a")), "built on open");
    assert_eq!(e.asset_index.id_of(&card("legacy")), None, "a legacy file is not indexed");
    assert!(e.index_diagnostics.is_empty(), "{:?}", e.index_diagnostics);
    let generation = e.asset_index.generation();

    std::fs::write(dir.path().join(rel("b")), text(b, &[a])).unwrap();
    e.rescan();
    assert_ne!(e.asset_index.generation(), generation);
    assert_eq!(e.asset_index.id_of(&card("b")), Some(b), "rebuilt by a rescan");
    assert_eq!(e.asset_index.refs_to(a).collect::<Vec<_>>(), [&card("b")]);

    // A save re-reads the saved tab's domain part.
    e.apply(Action::OpenAsset(card("a")));
    e.apply(TabAction::doc(Append("ref: \"00000000-0000-7000-8000-000000000000\"\n")));
    e.apply(Action::Save);
    assert_eq!(codes(&e.index_diagnostics), [("AE002", rel("a").as_str())], "refreshed after the save");

    // Each document's context carries the index (writers label their D27 comments with it).
    let seen = e.with_doc_ctx(e.active.unwrap(), |_, ctx| ctx.index.id_of(&card("b")));
    assert_eq!(seen, Some(Some(b)));

    e.apply(Action::CloseProject(crate::action::Unsaved::Discard));
    assert!(e.asset_index.get(a).is_none() && e.index_diagnostics.is_empty(), "empty without a project");
}

#[test]
fn index_diagnostics_reach_console_and_error_count() {
    let (a, gone) = (AssetUuid::new_v7(), AssetUuid::new_v7());
    let (_dir, mut e) = cards(&[
        ("a", text(a, &[])),
        ("b", text(a, &[])),
        ("c", "v2\n".into()),
        ("d", text(AssetUuid::new_v7(), &[gone])),
    ]);
    let expected = [("AE001", rel("b")), ("AE004", rel("c")), ("AE002", rel("d"))];
    let expected: Vec<(&str, &str)> = expected.iter().map(|(c, r)| (*c, r.as_str())).collect();
    assert_eq!(codes(&e.console_diagnostics()), expected);
    assert!(e.console_diagnostics().iter().all(|d| d.domain == Some(IDX)), "tagged with their domain");
    assert_eq!(e.error_count(), 3, "the mainbar's count");
    let d = index_diag(&e, "AE002");
    assert_eq!(d.location.as_deref(), Some("ref"));

    // The active asset's come first; the editor statusbar shows only its own.
    e.apply(Action::OpenAsset(card("d")));
    assert_eq!(codes(&e.console_diagnostics())[0], ("AE002", rel("d").as_str()));
    assert_eq!(codes(&e.active_diagnostics()), [("AE002", rel("d").as_str())]);
    assert_eq!(e.active_first_diagnostic(), Some(d));
}

#[test]
fn duplicate_focus_opens_the_regenerate_dialog() {
    let a = AssetUuid::new_v7();
    let (_dir, mut e) = cards(&[("a", text(a, &[])), ("b", text(a, &[]))]);
    e.apply(Action::FocusDiagnostic(index_diag(&e, "AE001")));
    let Some(Dialog::Confirm { title, message, buttons }) = e.dialog.clone() else { panic!("{:?}", e.dialog) };
    assert_eq!(title, "Duplicate asset UUID");
    assert_eq!(
        message,
        format!(
            "`{}` has the same UUID as `{}` (a copied file?). References keep pointing at `{}`.",
            rel("b"),
            rel("a"),
            rel("a")
        )
    );
    let expected = [
        ("Cancel".to_string(), Action::CloseDialog),
        ("Open".into(), Action::OpenAsset(card("b"))),
        ("Regenerate UUID".into(), Action::RegenerateUuid(card("b"))),
    ];
    assert_eq!(buttons, expected);
    e.apply(buttons[1].1.clone());
    assert!(e.dialog.is_none(), "Open closes the dialog");
    assert_eq!(e.tab().and_then(|t| t.doc().asset()), Some(&card("b")));
}

#[test]
fn regenerate_uuid_rewrites_only_the_loser_and_clears_ae001() {
    let a = AssetUuid::new_v7();
    let (_dir, mut e) = cards(&[("a", text(a, &[])), ("b", text(a, &[])), ("c", text(AssetUuid::new_v7(), &[a]))]);
    let p = e.project.clone().unwrap();
    let (a_before, c_before) = (read(&p, &card("a")), read(&p, &card("c")));
    e.apply(Action::OpenAsset(card("b")));
    e.apply(Action::RegenerateUuid(card("b")));
    assert!(e.dialog.is_none(), "{:?}", e.dialog);
    assert_eq!((read(&p, &card("a")), read(&p, &card("c"))), (a_before, c_before), "only the loser is written");
    let fresh = e.asset_index.id_of(&card("b")).expect("indexed");
    assert!(fresh != a && fresh.is_v7());
    assert_eq!(read(&p, &card("b")), text(fresh, &[]), "only the id line changed");
    assert!(e.index_diagnostics.is_empty(), "{:?}", e.index_diagnostics);
    assert_eq!(e.asset_index.get(a).map(|x| &x.asset), Some(&card("a")), "references keep the winner");
    let doc = e.active_doc::<CardDoc>().unwrap();
    assert_eq!(doc.text, text(fresh, &[]), "the clean tab reloaded");
    assert!(e.messages.iter().any(|m| m.text == format!("Regenerated the UUID of {}", rel("b"))));
}

#[test]
fn regenerate_uuid_saves_a_dirty_tab_first() {
    let a = AssetUuid::new_v7();
    let (_dir, mut e) = cards(&[("a", text(a, &[])), ("b", text(a, &[]))]);
    let p = e.project.clone().unwrap();
    e.apply(Action::OpenAsset(card("b")));
    e.apply(TabAction::doc(Append("note\n")));
    assert!(e.is_dirty());
    e.apply(Action::RegenerateUuid(card("b")));
    let fresh = e.asset_index.id_of(&card("b")).unwrap();
    assert_ne!(fresh, a);
    assert_eq!(read(&p, &card("b")), format!("{}note\n", text(fresh, &[])), "the edit was saved, then the id");
    assert!(!e.is_dirty());
    assert_eq!(e.active_doc::<CardDoc>().unwrap().text, read(&p, &card("b")));
}

#[test]
fn tree_menu_offers_regenerate_only_for_a_loser() {
    let a = AssetUuid::new_v7();
    let (_dir, e) = cards(&[("a", text(a, &[])), ("b", text(a, &[])), ("c", "v2\n".into())]);
    let menu = |path: &str| {
        let row = TreeRow { tree: TreeId::Project(IDX), item: TreeItem::Asset(card(path)) };
        row_menu(&e, &row).into_iter().map(|m| (m.label, m.action)).collect::<Vec<_>>()
    };
    let fixes = |path: &str| {
        let fix = |a: &Action| matches!(a, Action::RegenerateUuid(_) | Action::AssignUuid(_));
        menu(path).into_iter().filter(|(_, a)| fix(a)).collect::<Vec<_>>()
    };
    assert_eq!(fixes("a"), [], "the winner keeps its UUID");
    assert_eq!(fixes("b"), [("Regenerate UUID".to_string(), Action::RegenerateUuid(card("b")))]);
    assert_eq!(fixes("c"), [("Assign UUID".to_string(), Action::AssignUuid(card("c")))]);
    assert_eq!(menu("b")[0].0, "Open", "after the usual items");
}

#[test]
fn missing_id_focus_opens_the_assign_dialog() {
    // A nil id is no valid id (AE004), but has a root `id` line core's default can rewrite.
    let (_dir, mut e) = cards(&[("c", text(AssetUuid::NIL, &[]))]);
    let p = e.project.clone().unwrap();
    e.apply(Action::FocusDiagnostic(index_diag(&e, "AE004")));
    let Some(Dialog::Confirm { title, message, buttons }) = e.dialog.clone() else { panic!("{:?}", e.dialog) };
    assert_eq!(title, "Missing asset UUID");
    assert_eq!(
        message,
        format!(
            "Assign a UUID to `{}`? The file is rewritten in the canonical format (comments are not kept).",
            rel("c")
        )
    );
    let labels: Vec<&str> = buttons.iter().map(|(l, _)| l.as_str()).collect();
    assert_eq!(labels, ["Cancel", "Open", "Assign UUID"]);
    assert_eq!(buttons[2].1, Action::AssignUuid(card("c")));
    e.apply(buttons[2].1.clone());
    assert!(e.dialog.is_none(), "{:?}", e.dialog);
    let id = e.asset_index.id_of(&card("c")).expect("indexed with its new id");
    assert!(id.is_v7());
    assert_eq!(read(&p, &card("c")), text(id, &[]));
    assert!(e.index_diagnostics.is_empty(), "{:?}", e.index_diagnostics);
    assert!(e.messages.iter().any(|m| m.text == format!("Assigned a UUID to {}", rel("c"))));

    // Another diagnostic still goes to its domain; an index diagnostic the index no longer reports opens no dialog.
    let mut stale = Diagnostic::error("AE004", "gone").in_domain(IDX).in_asset(rel("c"));
    e.apply(Action::FocusDiagnostic(stale.clone()));
    assert!(e.dialog.is_none());
    stale.code = "X001".into();
    e.apply(Action::FocusDiagnostic(stale));
    assert!(e.dialog.is_none());
}

#[test]
fn default_set_asset_uuid_fails_cleanly_without_an_id_line() {
    let (_dir, mut e) = cards(&[("c", "v2\nname: \"hand written\"\n".into())]);
    let p = e.project.clone().unwrap();
    let before = read(&p, &card("c"));
    e.apply(Action::AssignUuid(card("c")));
    let Some(Dialog::Message { title, message }) = e.dialog.clone() else { panic!("{:?}", e.dialog) };
    assert_eq!(title, "Error");
    assert!(message.starts_with(&format!("Cannot assign a UUID to {}: ", rel("c"))), "{message}");
    assert!(message.contains("has no `id` line"), "{message}");
    assert_eq!(read(&p, &card("c")), before, "the file is untouched");
    assert_eq!(codes(&e.index_diagnostics), [("AE004", rel("c").as_str())]);
}

const LNK: DomainId = DomainId::new("lnk");
const LINK: &str = "link";

/// A second domain of `*.link.ron` cards (indexed by [`CardBackend`]) that only records its
/// `regenerate_affected` calls.
struct Linked;

#[derive(Default)]
struct LinkState {
    calls: Vec<Vec<AssetRef>>,
}

impl EditorDomain for Linked {
    fn id(&self) -> DomainId {
        LNK
    }
    fn info(&self) -> DomainInfo {
        DomainInfo { title: "Linked", icon: IconId::File.into(), summary: "Cards referring to other cards." }
    }
    fn backend(&self) -> Option<Box<dyn project_core::regen::DomainBackend>> {
        Some(Box::new(CardBackend(LNK, LINK)))
    }
    fn register(&self, r: &mut DomainRegistrar) {
        r.state(|| Box::new(LinkState::default()));
    }
}

impl DomainState for LinkState {
    fn id(&self) -> DomainId {
        LNK
    }
    fn regenerate_affected(editor: &mut Editor, affected: &[AssetRef]) {
        editor.domains.get_mut::<LinkState>().unwrap().calls.push(affected.to_vec());
    }
}

fn link(path: &str) -> AssetRef {
    AssetRef { domain: LNK, path: path.into(), suffix: LINK }
}

fn link_calls(e: &Editor) -> Vec<Vec<AssetRef>> {
    e.domains.get::<LinkState>().unwrap().calls.clone()
}

/// techdoc §4.3: saving a card calls the link domain's `regenerate_affected` exactly when links refer to it (also
/// transitively through other links), with those links; the saving domain's own referrers are its `after_save`'s.
#[test]
fn saving_calls_regenerate_affected_of_a_referring_domain() {
    let (a, x, l) = (AssetUuid::new_v7(), AssetUuid::new_v7(), AssetUuid::new_v7());
    let files = [card("a"), card("x"), card("c"), link("l"), link("m"), link("n")].map(|r| r.source_rel());
    let dir = game(&files.iter().map(String::as_str).collect::<Vec<_>>());
    let texts = [
        text(a, &[]),
        text(x, &[]),
        text(AssetUuid::new_v7(), &[a]),
        text(l, &[a]),
        text(AssetUuid::new_v7(), &[l]),
        text(AssetUuid::new_v7(), &[]),
    ];
    for (f, t) in files.iter().zip(texts) {
        std::fs::write(dir.path().join(f), t).unwrap();
    }
    let mut e = open(list(vec![Box::new(Indexed), Box::new(Linked)]), dir.path());

    // Nothing in the link domain refers to `x`.
    e.apply(Action::OpenAsset(card("x")));
    e.apply(TabAction::doc(Append("note\n")));
    e.apply(Action::Save);
    assert!(!e.is_dirty());
    assert_eq!(link_calls(&e), Vec::<Vec<AssetRef>>::new());

    // `l` refers to `a`, `m` to `l`; card `c` is the card domain's own business.
    e.apply(Action::OpenAsset(card("a")));
    e.apply(TabAction::doc(Append("note\n")));
    e.apply(Action::Save);
    assert_eq!(link_calls(&e), [vec![link("l"), link("m")]]);

    // A Save All that wrote nothing hands nothing over.
    e.apply(Action::SaveAll);
    assert_eq!(link_calls(&e).len(), 1);

    // A disabled domain is not called.
    e.project
        .as_mut()
        .unwrap()
        .settings
        .domains
        .insert(LNK.as_str().into(), project_core::layout::DomainEntry { enabled: false });
    e.apply(TabAction::doc(Append("more\n")));
    e.apply(Action::Save);
    assert_eq!(link_calls(&e).len(), 1);
}

#[test]
fn duplicate_copy_and_paste_mint_fresh_uuids_and_populate_row_menu() {
    let a = AssetUuid::new_v7();
    let (_dir, mut e) = cards(&[("sub/hero", text(a, &[]))]);
    let p = e.project.clone().unwrap();
    let hero = card("sub/hero");

    // Check asset row_menu labels and disabled Paste before copying.
    let asset_row = TreeRow { tree: TreeId::Project(IDX), item: TreeItem::Asset(hero.clone()) };
    let menu = row_menu(&e, &asset_row);
    let labels: Vec<&str> = menu.iter().map(|m| m.label.as_str()).collect();
    assert_eq!(
        labels,
        ["Open", "Duplicate\tCtrl+D", "Copy\tCtrl+C", "Paste\tCtrl+V", "Rename...\tF2", "Move to...", "Delete...\tDel"]
    );
    assert!(menu[3].disabled.is_some(), "Paste disabled before CopyAsset");

    // Duplicate a dirty tab: saves the edit first, creates `sub/hero_copy` with a fresh UUIDv7, and opens it.
    e.apply(Action::OpenAsset(hero.clone()));
    e.apply(TabAction::doc(Append("edited\n")));
    assert!(e.is_dirty());
    e.apply(Action::DuplicateAsset(hero.clone()));
    let copy1 = card("sub/hero_copy");
    assert!(!e.is_dirty());
    assert_eq!(e.tab().and_then(|t| t.doc().asset()), Some(&copy1));
    let id1 = e.asset_index.id_of(&copy1).expect("hero_copy indexed");
    assert_ne!(id1, a);
    assert!(read(&p, &copy1).ends_with("edited\n"));
    assert!(e.index_diagnostics.is_empty(), "no AE001 duplicate UUID: {:?}", e.index_diagnostics);

    // Duplicating again creates `sub/hero_copy_2`.
    e.apply(Action::DuplicateAsset(hero.clone()));
    let copy2 = card("sub/hero_copy_2");
    let id2 = e.asset_index.id_of(&copy2).expect("hero_copy_2 indexed");
    assert_ne!(id2, a);
    assert_ne!(id2, id1);
    assert!(e.index_diagnostics.is_empty());

    // Copy `sub/hero` and paste into root folder `""` -> keeps leaf name `hero` since `hero` is free at root.
    e.apply(Action::CopyAsset(hero.clone()));
    assert_eq!(e.copied_asset, Some(hero.clone()));
    let root_folder = TreeRow { tree: TreeId::Project(IDX), item: TreeItem::Folder(String::new()) };
    let folder_menu = row_menu(&e, &root_folder);
    let paste_entry = folder_menu.iter().find(|m| m.label == "Paste\tCtrl+V").expect("folder Paste entry");
    assert!(paste_entry.disabled.is_none(), "Paste enabled after CopyAsset");
    e.apply(paste_entry.action.clone());

    let pasted_root = card("hero");
    let id_root = e.asset_index.id_of(&pasted_root).expect("pasted root hero indexed");
    assert_ne!(id_root, a);
    assert_eq!(e.tab().and_then(|t| t.doc().asset()), Some(&pasted_root));
    assert!(e.index_diagnostics.is_empty());
}
