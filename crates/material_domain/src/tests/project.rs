//! Assets, tabs, projects and the user config; generated text tabs; diagnostics.

use super::*;

#[test]
fn create_rename_delete_assets() {
    let (dir, mut e) = project();
    e.apply(new_asset("material", 1));
    e.apply(Action::Submit("fx/glow".into()));
    let id = AssetId::new(AssetType::Material, "fx/glow");
    assert!(e.doc_index(&id).is_some());
    assert!(dir.path().join("assets/quillshade/material/autogen/dir_fx/mat_glow.wesl").is_file());
    e.apply(Action::MoveTo(id.to_ref(), "fx2".into()));
    let moved = AssetId::new(AssetType::Material, "fx2/glow");
    assert!(e.doc_index(&moved).is_some());
    assert!(!dir.path().join("assets/quillshade/material/autogen/dir_fx/mat_glow.wesl").exists());
    assert!(dir.path().join("assets/quillshade/material/autogen/dir_fx2/mat_glow.wesl").is_file());
    e.apply(Action::Delete(moved.to_ref()));
    assert!(e.doc_index(&moved).is_none());
    assert!(!dir.path().join("assets/quillshade/material/autogen/dir_fx2/mat_glow.wesl").exists());
    // Function rename updates open callers in memory.
    let rock = AssetId::new(AssetType::Material, "env/rock_wet");
    e.apply(Action::OpenAsset(rock.to_ref()));
    e.apply(Action::RenameDialog(AssetId::new(AssetType::Function, "common/wet_mask").to_ref()));
    e.apply(Action::Submit("common/wetness".into()));
    let d = e.tabs[e.doc_index(&rock).unwrap()].get::<Doc>().unwrap();
    assert!(d.asset.nodes.values().any(|n| n.kind == "function::common/wetness"));
    assert!(!d.is_dirty());
}

#[test]
fn close_dirty_tab_asks() {
    let (_dir, mut e) = project();
    e.apply(Action::OpenAsset(AssetId::new(AssetType::Material, "env/rock_wet").to_ref()));
    e.apply(MaterialAction::AddNode { kind: "stdlib::Time".into(), pos: (0.0, 0.0), link: None, params: Vec::new() });
    e.apply(Action::CloseTab(0));
    assert!(matches!(e.dialog, Some(Dialog::Confirm { .. })));
    assert_eq!(e.tabs.len(), 1);
    e.apply(Action::DiscardTab(0));
    assert!(e.tabs.is_empty());
    assert!(e.active.is_none());
}

/// Opens four materials; `dirty` ones get an unsaved node. Returns their names in tab order.
pub(super) fn four_tabs(e: &mut Editor, dirty: &[usize]) -> Vec<String> {
    let names = ["env/rock_wet", "env/flag", "fx/hologram", "fx/coords"];
    for (i, n) in names.into_iter().enumerate() {
        e.apply(Action::OpenAsset(AssetId::new(AssetType::Material, n).to_ref()));
        if dirty.contains(&i) {
            e.apply(MaterialAction::AddNode {
                kind: "stdlib::Time".into(),
                pos: (0.0, 0.0),
                link: None,
                params: Vec::new(),
            });
        }
    }
    names.iter().map(|n| format!("quillshade/material/{n}.material")).collect()
}

pub(super) fn titles(e: &Editor) -> Vec<String> {
    e.tabs.iter().map(Tab::title).collect()
}

#[test]
fn tab_menu_closes_groups_of_tabs() {
    let close = |which, at| Action::CloseTabs { which, at, unsaved: Unsaved::Ask };
    let (_dir, mut e) = project();
    let names = four_tabs(&mut e, &[]);
    e.apply(Action::ActivateTab(3));
    e.apply(close(CloseTabs::Left, 2));
    assert_eq!(titles(&e), names[2..]);
    assert_eq!(e.active, Some(1), "the active tab stays active");
    e.apply(close(CloseTabs::Right, 0));
    assert_eq!(titles(&e), names[2..3]);
    assert_eq!(e.active, Some(0), "the active tab closed: the last one left takes over");

    let (_dir, mut e) = project();
    four_tabs(&mut e, &[]);
    e.apply(Action::ActivateTab(0));
    e.apply(close(CloseTabs::Others, 2));
    assert_eq!(titles(&e), names[2..3]);
    assert_eq!(e.active, Some(0));
    e.apply(close(CloseTabs::All, 0));
    assert!(e.tabs.is_empty() && e.active.is_none());
}

#[test]
fn closing_unsaved_tabs_from_the_tab_menu_asks() {
    let (_dir, mut e) = project();
    let names = four_tabs(&mut e, &[1, 3]);
    e.apply(Action::CloseTabs { which: CloseTabs::Unmodified, at: 0, unsaved: Unsaved::Ask });
    assert!(e.dialog.is_none(), "only clean tabs: no question");
    assert_eq!(titles(&e), [names[1].clone(), names[3].clone()]);

    e.apply(Action::CloseTabs { which: CloseTabs::Others, at: 0, unsaved: Unsaved::Ask });
    let Some(Dialog::Confirm { buttons, .. }) = e.dialog.clone() else { panic!("asks") };
    assert_eq!(e.tabs.len(), 2);
    e.apply(buttons.last().unwrap().1.clone());
    assert!(e.dialog.is_none());
    assert_eq!(titles(&e), names[1..2]);
    assert!(e.tabs[0].doc().is_dirty(), "the kept tab keeps its changes");
    assert!(
        e.domains.material().disk.assets[&AssetId::new(AssetType::Material, "fx/coords")]
            .as_ref()
            .unwrap()
            .nodes
            .values()
            .any(|n| n.kind == "stdlib::Time"),
        "Save saved the closed one"
    );

    e.apply(Action::CloseTabs { which: CloseTabs::All, at: 0, unsaved: Unsaved::Discard });
    assert!(e.tabs.is_empty());
}

#[test]
fn closing_the_project_remembers_its_tabs() {
    let (dir, mut e) = project();
    let names = four_tabs(&mut e, &[2]);
    e.apply(Action::CloseProject(Unsaved::Ask));
    assert!(matches!(e.dialog, Some(Dialog::Confirm { .. })), "unsaved changes: asks");
    assert!(e.project.is_some());
    e.apply(Action::CloseProject(Unsaved::Discard));
    assert!(e.project.is_none() && e.tabs.is_empty() && e.domains.material().disk.ids().next().is_none());
    e.open_project(dir.path()).unwrap();
    assert_eq!(titles(&e), names);
    assert!(!e.is_dirty(), "discarded");
}

/// Opening another project with unsaved tabs asks Cancel / Discard / Save, as closing tabs does.
#[test]
fn switching_projects_asks_about_unsaved_tabs() {
    let (dir, mut e) = project();
    let (other, _) = project();
    let coords = AssetId::new(AssetType::Material, "fx/coords");
    let has_time = |dir: &Path| {
        std::fs::read_to_string(dir.join("quillshade/material/fx/coords.material.ron"))
            .unwrap()
            .contains("stdlib::Time")
    };
    four_tabs(&mut e, &[3]);
    let ask = |e: &mut Editor| {
        e.apply(Action::OpenProject(other.path().into()));
        let Some(Dialog::Confirm { buttons, .. }) = e.dialog.clone() else { panic!("asks: {:?}", e.dialog) };
        let labels: Vec<&str> = buttons.iter().map(|(l, _)| l.as_str()).collect();
        assert_eq!(labels, ["Cancel", "Discard", "Save"]);
        buttons
    };
    let root = e.project.as_ref().unwrap().root.clone();
    let buttons = ask(&mut e);
    e.apply(buttons[0].1.clone());
    assert!(e.dialog.is_none() && e.is_dirty(), "Cancel keeps everything");
    assert_eq!(e.project.as_ref().unwrap().root, root);

    e.apply(Action::OpenProject(other.path().join("missing")));
    assert!(matches!(e.dialog, Some(Dialog::Message { .. })), "no project: the error, no question");
    e.apply(Action::CloseDialog);

    let buttons = ask(&mut e);
    e.apply(buttons[2].1.clone());
    assert!(has_time(dir.path()), "Save saved {coords:?}");
    assert_ne!(e.project.as_ref().unwrap().root, root, "and switched");

    four_tabs(&mut e, &[3]);
    e.apply(Action::OpenProject(dir.path().into()));
    let Some(Dialog::Confirm { buttons, .. }) = e.dialog.clone() else { panic!("asks") };
    e.apply(buttons[1].1.clone());
    assert!(!has_time(other.path()), "Discard saved nothing");
    assert_eq!(e.project.as_ref().unwrap().root, root, "and switched");
}

#[test]
fn ctrl_click_disconnects_every_wire_of_a_pin() {
    let (_dir, mut e) = project();
    e.apply(Action::OpenAsset(AssetId::new(AssetType::Material, "env/rock_wet").to_ref()));
    let d = e.doc().unwrap();
    let out = d.asset.nodes.iter().find(|(_, n)| n.kind == "output::Pbr3d").unwrap().0.clone();
    let into = d.asset.edges.iter().filter(|x| x.to.0 == out).count();
    assert!(into > 1);
    let edge = d.asset.edges.iter().find(|x| x.to.0 == out).unwrap().clone();
    let fan_out = |e: &Editor| e.doc().unwrap().asset.edges.iter().filter(|x| x.from == edge.from).count();
    assert!(fan_out(&e) > 0);
    e.apply(GraphEdit::DisconnectPin { node: edge.from.0.clone(), pin: edge.from.1.clone(), output: true });
    assert_eq!(fan_out(&e), 0);
    e.apply(Action::Tab(TabAction::Undo));
    e.apply(GraphEdit::DisconnectPin { node: out.clone(), pin: edge.to.1.clone(), output: false });
    let d = e.doc().unwrap();
    assert!(d.asset.edge_into(&out, &edge.to.1).is_none());
    assert_eq!(d.asset.edges.iter().filter(|x| x.to.0 == out).count(), into - 1, "only that pin");
}

#[test]
fn tabs_are_restored_on_next_launch() {
    let (dir, mut e) = project();
    let config = dir.path().join("config/config.ron");
    e.config_path = Some(config.clone());
    e.apply(Action::OpenAsset(AssetId::new(AssetType::Material, "env/rock_wet").to_ref()));
    e.apply(Action::OpenAsset(AssetId::new(AssetType::Material, "fx/hologram").to_ref()));
    e.regenerate();
    e.apply(MaterialAction::ShowGenerated(AssetId::new(AssetType::Material, "fx/hologram"), RUST));
    e.apply(Action::ActivateTab(0));
    e.apply(Action::Quit);
    assert!(e.quit);

    let mut next = crate::test_editor().with_config(UserConfig::load(&config), Some(config));
    next.open_project(dir.path()).unwrap();
    let open: Vec<_> = next.tabs.iter().map(Tab::title).collect();
    assert_eq!(
        open,
        [
            "quillshade/material/env/rock_wet.material",
            "quillshade/material/fx/hologram.material",
            "crates/quillshade/src/material/materials/dir_fx/mat_hologram.rs"
        ]
    );
    assert_eq!(next.active, Some(0));
}

/// Zen mode (design §3, Review Focus 3): it hides every panel while on; Ctrl+3 in Zen flips its own flag only; Zen
/// off shows exactly the panels whose flags are on. Each step is saved at once.
#[test]
fn zen_hides_panels_and_restores_flags() {
    use crate::config::Panel::{Console, LeftSidebar, Log, Properties};
    let (dir, mut e) = project();
    let config = dir.path().join("config/config.ron");
    e.config_path = Some(config.clone());
    e.apply(Action::ToggleLog);
    let shown = |e: &Editor| [LeftSidebar, Console, Log, Properties].map(|p| e.config.panel_visible(p));
    assert_eq!(shown(&e), [true, true, false, true], "left on, log off, properties on");

    e.apply(Action::ToggleZen);
    assert!(e.config.zen_mode);
    assert_eq!(shown(&e), [false; 4], "Zen hides every panel, the console header too");
    assert_eq!(UserConfig::load(&config), e.config, "Zen saved");

    e.apply(Action::ToggleProperties);
    assert!(!e.config.show_properties, "Ctrl+3 in Zen flips its flag");
    assert_eq!(shown(&e), [false; 4], "still hidden");
    assert_eq!(UserConfig::load(&config), e.config, "the flag saved");

    e.apply(Action::ToggleZen);
    assert!(!e.config.zen_mode);
    assert_eq!(shown(&e), [true, true, false, false], "left shown, the log header only, properties hidden");
    assert_eq!(UserConfig::load(&config), e.config, "Zen off saved");

    e.apply(Action::ToggleLeftSidebar);
    assert!(!e.config.show_left_sidebar);
    assert_eq!(UserConfig::load(&config), e.config, "Ctrl+1 saved");
}

#[test]
fn generated_files_open_in_text_tabs() {
    let (_dir, mut e) = project();
    let rock = AssetId::new(AssetType::Material, "env/rock_wet");
    let mask = AssetId::new(AssetType::Function, "common/wet_mask");
    // Opening the project generated every domain without its module (design §6.5); with the WESL gone again, as if
    // never generated: a warning instead of an empty tab.
    std::fs::remove_file(super::graph::rock_wet_wesl(&e)).unwrap();
    e.apply(MaterialAction::ShowGenerated(rock.clone(), WESL));
    assert!(e.tabs.is_empty());
    e.regenerate();
    e.apply(Action::OpenAsset(rock.to_ref()));
    e.apply(MaterialAction::ShowGenerated(rock.clone(), WESL));
    assert_eq!(e.tabs.len(), 2);
    assert_eq!(e.active, Some(1));
    assert!(e.doc().is_none(), "graph commands must not reach a text tab");
    let t = e.tab().and_then(Tab::get::<TextDoc>).unwrap();
    assert_eq!(t.rel, "assets/quillshade/material/autogen/dir_env/mat_rock_wet.wesl");
    assert!(t.text.as_deref().unwrap().starts_with("// AUTOGENERATED"));
    // Showing it again activates the open tab.
    e.apply(Action::ActivateTab(0));
    e.apply(MaterialAction::ShowGenerated(rock.clone(), WESL));
    assert_eq!((e.tabs.len(), e.active), (2, Some(1)));
    e.apply(Action::Tab(TabAction::Copy));
    assert!(e.copy_text.as_deref().unwrap().contains("fn fragment"));
    // Functions have no Rust module.
    e.apply(MaterialAction::ShowGenerated(mask.clone(), RUST));
    assert_eq!(e.tabs.len(), 2);
    // Regeneration refreshes the text; deleting the asset closes all of its tabs.
    e.apply(MaterialAction::ShowGenerated(mask.clone(), WESL));
    let (path, original) =
        (e.tabs[1].get::<TextDoc>().unwrap().path.clone(), e.tabs[1].get::<TextDoc>().unwrap().text.clone());
    std::fs::write(&path, format!("{}// stale\n", original.as_deref().unwrap())).unwrap();
    e.apply(Action::ActivateTab(1));
    assert!(
        e.tabs[1].get::<TextDoc>().unwrap().text.as_deref().unwrap().ends_with("// stale\n"),
        "activation re-reads"
    );
    let (rock_rev, mask_rev) =
        (e.tabs[1].get::<TextDoc>().unwrap().revision, e.tabs[2].get::<TextDoc>().unwrap().revision);
    e.apply(Action::RegenAll);
    assert_eq!(e.tabs[1].get::<TextDoc>().unwrap().text, original);
    assert!(e.tabs[1].get::<TextDoc>().unwrap().revision > rock_rev);
    assert_eq!(e.tabs[2].get::<TextDoc>().unwrap().revision, mask_rev, "an unchanged file keeps its view");
    e.apply(Action::Delete(rock.to_ref()));
    assert_eq!(e.tabs.len(), 1);
    assert_eq!(e.tabs[0].owner(), Some(&mask.to_ref()));
}

/// Regeneration rewriting the file of the active text tab rebuilds its right sidebar (the "N lines" row is keyed on
/// `ui.props`).
#[test]
fn regenerated_active_text_tab_refreshes_sidebar() {
    let (_dir, mut e) = project();
    e.regenerate();
    let rock = AssetId::new(AssetType::Material, "env/rock_wet");
    e.apply(Action::OpenAsset(rock.to_ref()));
    e.apply(MaterialAction::ShowGenerated(rock, WESL));
    let lines = |e: &Editor| e.tabs[1].get::<TextDoc>().unwrap().text.as_deref().unwrap().lines().count();
    let before = lines(&e);
    // The graph stripped down to its output node, saved without regenerating.
    e.apply(Action::ActivateTab(0));
    e.apply(Action::Tab(TabAction::SelectAll));
    e.apply(Action::Tab(TabAction::DeleteSelection));
    assert!(e.save(0));
    e.apply(Action::ActivateTab(1));
    let props = e.ui.props;
    e.apply(Action::RegenAll);
    assert_ne!(lines(&e), before, "the WESL line count changed");
    assert_ne!(e.ui.props, props, "the active text tab's sidebar is rebuilt");
    // A reload of an inactive text tab leaves the sidebar alone.
    e.apply(Action::ActivateTab(0));
    let (path, text) =
        (e.tabs[1].get::<TextDoc>().unwrap().path.clone(), e.tabs[1].get::<TextDoc>().unwrap().text.clone());
    std::fs::write(&path, format!("{}// stale\n", text.as_deref().unwrap())).unwrap();
    let props = e.ui.props;
    e.apply(Action::RegenAll);
    assert_eq!(e.tabs[1].get::<TextDoc>().unwrap().text, text, "reloaded");
    assert_eq!(e.ui.props, props);
}

#[test]
fn clicking_a_diagnostic_opens_its_asset() {
    let (_dir, mut e) = project();
    let id = AssetId::new(AssetType::Material, "env/rock_wet");
    let node = e.domains.material().disk.assets[&id].as_ref().unwrap().nodes.keys().next().unwrap().clone();
    let diag = Diagnostic::new(Code::W101, "x").in_asset(id.source_rel()).at_node(node.clone());
    e.apply(Action::FocusDiagnostic(diag.to_generic(MATERIAL)));
    assert_eq!(e.doc().map(|d| &d.id), Some(&id));
    assert!(e.doc().unwrap().selection.contains(&node));
}
