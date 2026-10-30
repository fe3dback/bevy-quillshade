//! The Material domain's registrations (design §6.1, §9.1).

use super::*;

/// The Assets and Help menus list exactly what they did before domains registered them: the "New …" templates in
/// order, New folder, Generate (once Regenerate); Quick Start first in Help.
#[test]
fn material_new_menu_unchanged() {
    use editor_core::menu::MainMenu;
    use editor_core::ui::menu::menu_entries;
    let (_dir, e) = project();
    let labels = |menu| menu_entries(menu, &e).into_iter().map(|m| m.label).collect::<Vec<_>>();
    assert_eq!(
        labels(MainMenu::Assets),
        [
            "New Material (Pbr3d)...",
            "New Material (Unlit3d)...",
            "New Material (Sprite2d)...",
            "New Function...",
            "New Curve...",
            "New Global Consts...",
            "New Gradient Collection...",
            "New folder...",
            "",
            "Generate",
        ]
    );
    assert_eq!(labels(MainMenu::Help), ["Quick Start", "Project Page", "", "About"]);
    let icons: Vec<_> = menu_entries(MainMenu::Assets, &e).into_iter().take(3).map(|m| m.icon).collect();
    assert_eq!(icons, [Some(crate::icons::PBR), Some(crate::icons::UNLIT), Some(crate::icons::sprite())]);
}

/// Design §8.2: core's steps (the generated crate in Cargo.toml, `QuillshadePlugin`) come first, then Material's,
/// titled with the domain's name.
#[test]
fn quick_start_core_steps_first() {
    use editor_core::quick_start::{CORE_STEPS, steps};
    let list = crate::domain::material_only();
    let titles: Vec<(Option<DomainId>, String, bool)> =
        steps(&list).into_iter().map(|s| (s.domain, s.title, s.optional)).collect();
    assert_eq!(CORE_STEPS[0].0, "Crate", "the first step is core's crate step");
    let core = |t: &str| (None, t.to_string(), false);
    let material = |t: &str, optional| (Some(MATERIAL), format!("Material: {t}"), optional);
    assert_eq!(
        titles,
        [
            core("Crate"),
            core("Plugin"),
            material("Files", false),
            material("Spawn", false),
            material("Loading", true),
            material("Hot reload", false),
        ]
    );
}

/// The welcome screen's line of the domain (design §8.1).
#[test]
fn material_welcome_summary() {
    use editor_core::domain::EditorDomain;
    assert_eq!(
        crate::domain::MaterialDomain.info().summary,
        "Material — node-based materials in quillshade/material/, generated into assets/quillshade/material/ and \
         crates/quillshade."
    );
}

/// Open tabs are saved as `material:<key>` and restored from it; keys saved before domains existed (no prefix) are
/// read as Material's, so existing users keep their tabs.
#[test]
fn material_tabs_restore_from_config() {
    let (dir, mut e) = project();
    e.regenerate();
    let rock = AssetId::new(AssetType::Material, "env/rock_wet");
    e.apply(Action::OpenAsset(rock.to_ref()));
    e.apply(MaterialAction::ShowGenerated(rock.clone(), WESL));
    e.apply(Action::OpenAsset(pulse().to_ref()));
    e.save_tabs_to_config();
    let root = e.project.as_ref().unwrap().root.clone();
    let saved = e.config.tabs[&root].clone();
    assert_eq!(
        saved.open,
        [
            "material:quillshade/material/env/rock_wet.material.ron",
            "material:quillshade/material/env/rock_wet.material.ron#wesl",
            "material:quillshade/material/common/pulse.curve.ron",
        ]
    );
    assert_eq!(saved.active, Some(2));

    let reopen = |open: Vec<String>, active| {
        let mut config = UserConfig::default();
        config.tabs.insert(root.clone(), crate::config::ProjectTabs { open, active, ..Default::default() });
        let mut e = crate::test_editor();
        e.config = config;
        e.open_project(dir.path()).unwrap();
        e
    };
    let again = reopen(saved.open.clone(), Some(1));
    assert_eq!(
        again.tabs.iter().map(|t| t.title()).collect::<Vec<_>>(),
        e.tabs.iter().map(|t| t.title()).collect::<Vec<_>>()
    );
    assert_eq!(again.active, Some(1));
    assert!(again.dialog.is_none());

    // Legacy keys (no domain prefix) are Material's; unknown domains and keys Material does not parse are skipped.
    // The saved active index (entry 2, the curve) names its own entry, whatever was skipped before it.
    let legacy = ["light:x", "material:nonsense", "quillshade/material/common/pulse.curve.ron", "@internal/default"];
    let old = reopen(legacy.map(String::from).to_vec(), Some(2));
    let titles: Vec<String> = old.tabs.iter().map(|t| t.title()).collect();
    assert_eq!(titles, ["quillshade/material/common/pulse.curve", "@internal/default"]);
    assert_eq!(old.active, Some(0));
    assert!(old.dialog.is_none());
    assert_eq!(reopen(legacy.map(String::from).to_vec(), Some(0)).active, Some(1), "skipped: the last tab");
}
