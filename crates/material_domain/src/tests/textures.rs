//! Texture assets, game images and the Game Assets tree (decision 0024).

use super::*;

/// Texture assets (design §1.1) are loaded and validated with the project; they open in a Texture tab (a file that
/// does not load reports why and opens nothing), which follows a rename.
#[test]
fn texture_asset_is_loaded_validated_and_opened() {
    let (dir, _) = project();
    let p = Project::open(dir.path()).unwrap();
    let id = AssetId::new(AssetType::Texture, "textures/rock");
    shader_project::ops::create_texture(&p, &id, &shader_graph_core::TextureAsset::new("textures/rock.png")).unwrap();
    std::fs::write(dir.path().join("quillshade/material/textures/broken.texture.ron"), "TextureAsset(").unwrap();
    let mut e = crate::test_editor();
    e.open_project(dir.path()).unwrap();
    assert!(e.domains.material().disk.contains(&id));
    e.validate();
    assert!(e.all_diagnostics().iter().any(|d| d.code == Code::W109), "{:?}", e.all_diagnostics());
    let tabs = e.tabs.len();
    e.apply(Action::OpenAsset(id.to_ref()));
    assert_eq!(e.tabs.len(), tabs + 1);
    assert_eq!(texture_doc(&e).asset.path, "textures/rock.png");
    e.apply(Action::OpenAsset(AssetId::new(AssetType::Texture, "textures/broken").to_ref()));
    assert_eq!(e.tabs.len(), tabs + 1);
    assert!(e.messages.last().is_some_and(|m| m.level == Level::Error && m.text.contains("broken")));
    e.apply(Action::CloseDialog);

    e.apply(Action::ActivateTab(tabs));
    e.apply(Action::RenameDialog(id.to_ref()));
    e.apply(Action::Submit("textures/stone".into()));
    let moved = AssetId::new(AssetType::Texture, "textures/stone");
    assert_eq!(texture_doc(&e).id(), moved);
    assert_eq!(e.tab().unwrap().title(), "quillshade/material/textures/stone.texture");
    assert!(!e.is_dirty());
}

/// Review focus 5 in the editor: open graphs follow a renamed / deleted Texture asset (design §1.2) like they follow
/// curves — a clean document reloads the rewritten file, a dirty one gets the change as an unsaved edit.
#[test]
fn open_documents_follow_texture_rename_and_delete() {
    use shader_graph_core::registry::texture_refs;
    let (dir, _) = project();
    let p = Project::open(dir.path()).unwrap();
    let tex = AssetId::new(AssetType::Texture, "textures/rock");
    ops::create_texture(&p, &tex, &shader_graph_core::TextureAsset::new("textures/rock.png")).unwrap();
    let rock = AssetId::new(AssetType::Material, "env/rock_wet");
    let mut a = assets::load(&p, &rock).unwrap();
    for n in a.nodes.values_mut().filter(|n| n.kind == "param::TextureSample2D") {
        n.params.insert("default".into(), Value::Str("textures/rock".into()));
    }
    ops::save(&p, &rock, &a).unwrap();
    let mut e = crate::test_editor();
    e.open_project(dir.path()).unwrap();
    e.apply(Action::OpenAsset(rock.to_ref()));
    let doc = |e: &Editor| e.tabs[e.doc_index(&rock).unwrap()].get::<Doc>().unwrap().clone();
    let ids = |a: &Asset, e: &Editor| {
        texture_refs(a, &e.domains.material().registry).into_iter().map(|r| r.2).collect::<Vec<_>>()
    };
    let node = texture_refs(&doc(&e).asset, &e.domains.material().registry)[0].0.clone();
    assert_eq!(e.referenced_asset(&node), Some(tex.clone()));

    e.apply(Action::RenameDialog(tex.to_ref()));
    e.apply(Action::Submit("textures/stone".into()));
    let stone = AssetId::new(AssetType::Texture, "textures/stone");
    assert!(e.domains.material().disk.contains(&stone));
    assert_eq!(ids(&doc(&e).asset, &e), vec!["textures/stone"; 2]);
    assert!(!doc(&e).is_dirty());
    assert_eq!(e.referenced_asset(&node), Some(stone.clone()));

    let i = e.doc_index(&rock).unwrap();
    e.tabs[i].get_mut::<Doc>().unwrap().edit(|a| a.nodes.get_mut(&node).unwrap().pos.0 += 10.0);
    e.apply(Action::Delete(stone.to_ref()));
    assert!(!e.domains.material().disk.contains(&stone));
    assert!(ids(&doc(&e).asset, &e).is_empty());
    assert!(doc(&e).is_dirty(), "unsaved edits stay unsaved");
    assert!(ids(&assets::load(&p, &rock).unwrap(), &e).is_empty(), "the file is cleared too");
    assert_eq!(e.referenced_asset(&node), None);
}

/// The Game Assets tree (design §4.2) lists the images under `assets/` (not `assets/quillshade/`) and the folders
/// that hold them; it is read-only: its rows offer no rename / move / delete and nothing can be dropped into it. RMB on a
/// file creates its Texture asset (disabled once one exists).
#[test]
fn game_assets_tree_rows() {
    use crate::ui::tree::{self, GameAssetsTree, ProjectTree, TreeItem, TreeRow, TreeSource};
    let (dir, mut e) = project();
    for f in [
        "textures/env/rock.png",
        "textures/env/notes.txt",
        "docs/readme.md",
        "top.KTX2",
        "quillshade/material/internal/x.png",
    ] {
        let p = dir.path().join("assets").join(f);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, b"x").unwrap();
    }
    e.refresh_game_files();
    let folder = |f: &str| TreeItem::Folder(f.into());
    let file = |f: &str| TreeItem::GameFile(f.into());
    assert_eq!(
        GameAssetsTree.rows(&e),
        vec![
            (0, folder("")),
            (1, folder("textures")),
            (2, folder("textures/env")),
            (3, file("textures/env/rock.png")),
            (1, file("top.KTX2")),
        ]
    );
    assert!(tree::source(TreeId::GameAssets).read_only());

    let row = |tree, item| TreeRow { tree, item };
    let rock = row(TreeId::GameAssets, file("textures/env/rock.png"));
    let env = row(TreeId::GameAssets, folder("textures/env"));
    let destructive = |a: &Action| {
        matches!(
            a,
            Action::RenameDialog(_)
                | Action::MoveDialog(_)
                | Action::DeleteDialog(_)
                | Action::DeleteFolderDialog(..)
                | Action::MoveTo(..)
                | Action::NewFolderDialog(_)
                | Action::NewAssetDialog { .. }
        )
    };
    let menu = tree::row_menu(&e, &rock);
    assert_eq!(menu.iter().map(|m| m.label.as_str()).collect::<Vec<_>>(), ["Create Texture Asset"]);
    assert!(menu[0].disabled.is_none());
    assert!(!menu.iter().any(|m| destructive(&m.action)));
    assert!(tree::row_menu(&e, &env).is_empty());
    assert!(tree::row_menu(&e, &row(TreeId::GameAssets, folder(""))).is_empty());
    assert_eq!(tree::click_actions(&env, 1), vec![Action::ToggleFolder(TreeId::GameAssets, "textures/env".into())]);
    assert_eq!(tree::click_actions(&rock, 2), vec![Action::OpenGameImage("textures/env/rock.png".into())]);
    assert_eq!(tree::drop_action(&env, &rock), None);
    let material =
        row(TreeId::Project(MATERIAL), TreeItem::Asset(AssetId::new(AssetType::Material, "env/rock_wet").to_ref()));
    assert_eq!(tree::drop_action(&env, &material), None);
    assert_eq!(tree::drop_action(&row(TreeId::Project(MATERIAL), folder("fx")), &rock), None);

    // "Create Texture Asset": `quillshade/material/textures/env/rock.texture.ron`, shown in the Project tree.
    let id = AssetId::new(AssetType::Texture, "textures/env/rock");
    e.apply(menu[0].action.clone());
    assert!(id.file(e.project.as_ref().unwrap()).is_file());
    assert!(ProjectTree(MATERIAL).rows(&e).iter().any(|(_, item)| *item == TreeItem::Asset(id.to_ref())));
    let again = tree::row_menu(&e, &rock);
    assert!(again[0].disabled.as_deref().is_some_and(|t| t.contains("textures/env/rock")), "{:?}", again[0].disabled);
    // Applied anyway (a stale menu): refused, nothing new.
    let count = e.domains.material().disk.textures.len();
    e.apply(menu[0].action.clone());
    assert_eq!(e.domains.material().disk.textures.len(), count);
}

/// The Project tree ends with the virtual read-only `Internal` folder of built-in textures (design §2).
#[test]
fn internal_folder_lists_internal_textures() {
    use crate::ui::tree::{self, INTERNAL_FOLDER, ProjectTree, TreeItem, TreeRow, TreeSource};
    use shader_stdlib::internal::DEFAULT_TEXTURE;
    let (_dir, mut e) = project();
    let internal = TreeItem::VirtualFolder(INTERNAL_FOLDER.into(), "Internal");
    let default = TreeItem::Virtual(AssetId::new(AssetType::Texture, DEFAULT_TEXTURE).to_ref());
    let rows = ProjectTree(MATERIAL).rows(&e);
    assert_eq!(rows[rows.len() - 2..], [(1, internal.clone()), (2, default.clone())]);
    assert_eq!(tree::row_label(TreeId::Project(MATERIAL), &internal), "Internal");
    assert_eq!(tree::row_label(TreeId::Project(MATERIAL), &default), "default");

    let row = |item| TreeRow { tree: TreeId::Project(MATERIAL), item };
    assert!(tree::row_menu(&e, &row(internal.clone())).is_empty());
    assert!(tree::row_menu(&e, &row(default.clone())).is_empty());
    // Toggles, but is never the target of "New ...".
    assert_eq!(
        tree::click_actions(&row(internal.clone()), 1),
        vec![Action::ToggleFolder(TreeId::Project(MATERIAL), INTERNAL_FOLDER.into())]
    );
    let material = row(TreeItem::Asset(AssetId::new(AssetType::Material, "env/rock_wet").to_ref()));
    assert_eq!(tree::drop_action(&row(internal.clone()), &material), None);
    assert_eq!(tree::drop_action(&row(TreeItem::Folder("fx".into())), &row(default.clone())), None);
    // Real folders still take drops.
    assert!(tree::drop_action(&row(TreeItem::Folder("fx".into())), &material).is_some());

    e.apply(Action::ToggleFolder(TreeId::Project(MATERIAL), INTERNAL_FOLDER.into()));
    assert!(!ProjectTree(MATERIAL).rows(&e).iter().any(|(_, item)| *item == default));
}

/// Game folder names may contain characters sorting before `/` (`-`, `.`, space): a folder's subfolders still come
/// right after it (and its files), not after its siblings.
#[test]
fn game_assets_tree_nests_folders_by_component() {
    use crate::ui::tree::{GameAssetsTree, TreeItem, TreeSource};
    let (dir, mut e) = project();
    for f in ["a/x.png", "a-b/y.png", "a/c/z.png", "a b/w.png"] {
        let p = dir.path().join("assets").join(f);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, b"x").unwrap();
    }
    e.refresh_game_files();
    let folder = |f: &str| TreeItem::Folder(f.into());
    let file = |f: &str| TreeItem::GameFile(f.into());
    assert_eq!(
        GameAssetsTree.rows(&e),
        vec![
            (0, folder("")),
            (1, folder("a")),
            (2, file("a/x.png")),
            (2, folder("a/c")),
            (3, file("a/c/z.png")),
            (1, folder("a b")),
            (2, file("a b/w.png")),
            (1, folder("a-b")),
            (2, file("a-b/y.png")),
        ]
    );
}

/// Double-clicking a game image opens it in a read-only image tab (design §4.4): once per file, remembered in the
/// user config as `@game/<path>`.
#[test]
fn open_game_image_tab() {
    use crate::tab::{MaterialKey, parse_config_key};
    use crate::ui::tree::{self, TreeItem, TreeRow};
    let (dir, mut e) = project();
    let path = "textures/env/rock.png";
    let file = dir.path().join("assets").join(path);
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(&file, b"not decoded by the editor state").unwrap();
    e.refresh_game_files();

    let row = TreeRow { tree: TreeId::GameAssets, item: TreeItem::GameFile(path.into()) };
    assert!(tree::click_actions(&row, 1).is_empty());
    let open = tree::click_actions(&row, 2);
    assert_eq!(open, vec![Action::OpenGameImage(path.into())]);

    let rock = AssetId::new(AssetType::Material, "env/rock_wet");
    e.apply(Action::OpenAsset(rock.to_ref()));
    for a in open.clone() {
        e.apply(a);
    }
    assert_eq!(e.tabs.len(), 2);
    assert_eq!(e.active, Some(1));
    let Some(img) = e.active_doc::<ImageDoc>() else { panic!("image tab expected") };
    assert_eq!(img.path, path);
    let doc = e.tab().unwrap().doc();
    assert_eq!(doc.view(), crate::texture::IMAGE);
    assert!(doc.is_read_only());
    assert!(!doc.is_dirty());
    assert_eq!(doc.asset(), None);
    assert_eq!(e.tab().unwrap().owner(), None);
    assert_eq!(doc.base_title(), "assets/textures/env/rock.png");
    let key = doc.config_key().unwrap();
    assert_eq!(key, "@game/textures/env/rock.png");
    assert_eq!(parse_config_key(&key), Some(MaterialKey::GameImage(path.into())));
    assert!(e.doc().is_none(), "no graph commands on an image tab");

    // Opening it again activates the same tab.
    e.apply(Action::OpenAsset(rock.to_ref()));
    assert_eq!(e.active, Some(0));
    e.apply(Action::OpenGameImage(path.into()));
    assert_eq!(e.tabs.len(), 2);
    assert_eq!(e.active, Some(1));

    // A changed file bumps the revision: the view decodes it again.
    let revision = e.tab().unwrap().doc().revision();
    std::fs::write(&file, b"changed, and longer than before").unwrap();
    e.reload_read_only_tabs();
    assert!(e.tab().unwrap().doc().revision() > revision);

    // Restored on the next launch.
    e.save_tabs_to_config();
    assert_eq!(e.config.tabs.values().next().unwrap().open[1], "material:@game/textures/env/rock.png");
    let config = e.config.clone();
    let mut next = crate::test_editor().with_config(config.clone(), None);
    next.open_project(dir.path()).unwrap();
    assert_eq!(next.tabs.len(), 2);
    assert_eq!(next.active, Some(1));
    assert!(next.active_doc::<ImageDoc>().is_some_and(|i| i.path == path));

    // A file that is gone is not restored; opening a missing file only warns.
    std::fs::remove_file(&file).unwrap();
    let mut gone = crate::test_editor().with_config(config, None);
    gone.open_project(dir.path()).unwrap();
    assert_eq!(gone.tabs.len(), 1);
    gone.apply(Action::OpenGameImage(path.into()));
    assert_eq!(gone.tabs.len(), 1);
    assert!(gone.messages.last().is_some_and(|m| m.text.contains(path)));

    // Paths leaving `assets/` (or not naming a ktx2 / png) open nothing, even when the file exists.
    let outside = dir.path().join("outside.png");
    std::fs::write(&outside, b"x").unwrap();
    std::fs::write(dir.path().join("assets/notes.txt"), b"x").unwrap();
    let absolute = outside.to_string_lossy().replace('\\', "/");
    for bad in ["../outside.png", "textures/../../outside.png", absolute.as_str(), "notes.txt"] {
        gone.apply(Action::OpenGameImage(bad.into()));
        assert_eq!(gone.tabs.len(), 1, "{bad}");
        assert!(gone.messages.last().is_some_and(|m| m.text.starts_with("cannot open game image: ")), "{bad}");
    }
}

pub(super) fn texture_doc(e: &Editor) -> &crate::texture::TextureDoc {
    e.active_doc::<crate::texture::TextureDoc>().expect("texture tab expected")
}

/// The Texture tab (design §4.4): settings edits are undoable steps of the document, validation and saving see them.
#[test]
fn texture_tab_edit_undo_redo() {
    use crate::tab::MaterialKey;
    use crate::texture::TextureField;
    use shader_graph_core::texture::{FilterMode, SamplerSettings, TextureSettings};
    let (_dir, mut e) = project();
    let project = e.project.clone().unwrap();
    let id = AssetId::new(AssetType::Texture, "textures/rock_albedo");
    e.apply(Action::OpenAsset(id.to_ref()));
    assert_eq!(e.tabs.len(), 1, "Texture assets open in a tab");
    let doc = e.tab().unwrap().doc();
    assert_eq!(doc.view(), crate::texture::TEXTURE);
    assert_eq!(doc.asset(), Some(&id.to_ref()));
    assert_eq!(doc.watermark(), Some("texture"));
    assert_eq!(doc.base_title(), "quillshade/material/textures/rock_albedo.texture");
    assert!(!doc.is_read_only() && !doc.is_dirty());
    assert_eq!(parse_config_key(&doc.config_key().unwrap()), Some(MaterialKey::Asset(id.clone())));
    assert_eq!(texture_doc(&e).asset.settings, TextureSettings::default());

    // Mag filter Nearest over the game's default sampler: a custom sampler.
    let custom = TextureSettings {
        sampler: Some(SamplerSettings { mag_filter: FilterMode::Nearest, ..SamplerSettings::image_plugin_default() }),
        ..TextureSettings::default()
    };
    e.apply(Action::from(TextureEdit::new(TextureAction::SetSettings(custom.clone()))));
    assert!(e.is_dirty());
    assert_eq!(texture_doc(&e).asset.settings, custom);
    assert_eq!(e.overridden_set().textures[&id].as_ref().unwrap().settings, custom, "validation sees the edit");
    e.apply(Action::Tab(TabAction::Undo));
    assert!(!e.is_dirty());
    assert_eq!(texture_doc(&e).asset.settings, TextureSettings::default());
    e.apply(Action::Tab(TabAction::Redo));
    assert_eq!(texture_doc(&e).asset.settings, custom);
    e.apply(Action::Tab(TabAction::Undo));

    // The same through the form: "Default" sampler off, then the mag filter select.
    let field = |f| TextureKey::new(TextureProp::Field(f));
    e.apply(set_value(field(TextureField::DefaultSampler), Value::Bool(false), false));
    assert_eq!(texture_doc(&e).asset.settings.sampler, Some(SamplerSettings::image_plugin_default()));
    e.apply(set_value(field(TextureField::MagFilter), Value::Str("Nearest".into()), false));
    assert_eq!(texture_doc(&e).asset.settings, custom);
    assert_eq!(shown(&e, &field(TextureField::MagFilter)), Some(Value::Str("Nearest".into())));
    // A dragged number field is one step.
    for v in [4.0, 6.0, 8.0] {
        e.apply(set_component(field(TextureField::LodMaxClamp), 0, v, false));
    }
    e.apply(set_component(field(TextureField::LodMaxClamp), 0, 9.0, true));
    assert_eq!(texture_doc(&e).asset.settings.sampler.unwrap().lod_max_clamp, 9.0);
    e.apply(Action::Tab(TabAction::Undo));
    assert_eq!(texture_doc(&e).asset.settings, custom);
    e.apply(Action::Tab(TabAction::Undo));
    e.apply(Action::Tab(TabAction::Undo));
    assert!(!e.is_dirty(), "back to the saved settings");
    e.apply(Action::Tab(TabAction::Redo));
    e.apply(Action::Tab(TabAction::Redo));

    // Saved like any document.
    e.apply(Action::Save);
    assert!(!e.is_dirty());
    assert_eq!(assets::load_texture(&project, &id).unwrap().settings, custom);

    // A sampler wgpu cannot create is kept as edited and reported (E013); Properties redraw with the problem.
    let aniso = TextureField::Anisotropy.set(&custom, &PropValue::Int(8));
    e.apply(Action::from(TextureEdit::new(TextureAction::SetSettings(aniso.clone()))));
    assert_eq!(texture_doc(&e).asset.settings, aniso);
    let props = e.ui.props;
    e.validate();
    assert!(
        e.domains.material().diags[&id].iter().any(|d| d.code == Code::E013),
        "{:?}",
        e.domains.material().diags[&id]
    );
    assert!(e.ui.props > props);
    let props = e.ui.props;
    e.validate();
    assert_eq!(e.ui.props, props, "unchanged problems: no redraw");
}

/// One game file, one Texture asset (design §1.1, review focus 1): a path edit, an undo or a "Create Texture Asset"
/// that would make two Texture assets point at one file is refused (open documents count as edited).
#[test]
fn texture_path_edit_refuses_duplicate() {
    let (_dir, mut e) = project();
    let albedo = AssetId::new(AssetType::Texture, "textures/rock_albedo");
    let normal = AssetId::new(AssetType::Texture, "textures/rock_normal");
    let set_path = |p: &str| Action::from(TextureEdit::new(TextureAction::SetPath(p.into())));
    e.apply(Action::OpenAsset(albedo.to_ref()));

    // The file of textures/rock_normal.
    e.apply(set_path("textures/rock_normal.png"));
    assert_eq!(texture_doc(&e).asset.path, "textures/rock_albedo.ktx2", "model unchanged");
    assert!(!e.is_dirty());
    let last = e.messages.last().unwrap();
    assert_eq!(last.level, Level::Error);
    assert!(
        last.text.contains("textures/rock_normal.png") && last.text.contains(&normal.source_rel()),
        "{}",
        last.text
    );
    assert!(matches!(e.dialog, Some(Dialog::Message { .. })));
    e.apply(Action::CloseDialog);

    // An invalid path (E012 rules) is refused too.
    e.apply(set_path("../outside.png"));
    assert_eq!(texture_doc(&e).asset.path, "textures/rock_albedo.ktx2");
    assert!(e.messages.last().unwrap().text.contains("must not contain"), "{}", e.messages.last().unwrap().text);
    e.apply(Action::CloseDialog);

    // A free file is taken (a missing one is only W109).
    e.apply(set_path(" textures/rock_albedo.png "));
    assert_eq!(texture_doc(&e).asset.path, "textures/rock_albedo.png");
    assert!(e.is_dirty());

    // Claims are disk OR open tabs: the file rock_albedo left is still its own on disk until it is saved, and the new
    // one is claimed by the unsaved tab. Neither can be taken by a path edit or "Create Texture Asset".
    let ktx2 = "textures/rock_albedo.ktx2";
    let before = e.domains.material().disk.textures.len();
    assert_eq!(e.texture_for_file(ktx2), Some(albedo.clone()));
    assert_eq!(e.create_texture_for_file(ktx2), None);
    assert!(e.messages.last().unwrap().text.contains(&albedo.source_rel()));
    e.apply(Action::CloseDialog);
    assert_eq!(e.texture_for_file("textures/rock_albedo.png"), Some(albedo.clone()));
    assert_eq!(e.create_texture_for_file("textures/rock_albedo.png"), None);
    e.apply(Action::CloseDialog);
    assert_eq!(e.domains.material().disk.textures.len(), before, "nothing written");
    e.apply(Action::OpenAsset(normal.to_ref()));
    for taken in [ktx2, "textures/rock_albedo.png"] {
        e.apply(set_path(taken));
        assert_eq!(texture_doc(&e).asset.path, "textures/rock_normal.png", "{taken}");
        assert!(e.messages.last().unwrap().text.contains(&albedo.source_rel()), "{taken}");
        e.apply(Action::CloseDialog);
    }

    // Saved: the old file is free.
    e.apply(Action::OpenAsset(albedo.to_ref()));
    e.apply(Action::Save);
    e.apply(Action::OpenAsset(normal.to_ref()));
    e.apply(set_path(ktx2));
    assert_eq!(texture_doc(&e).asset.path, ktx2);

    // Undo in rock_albedo would point it back at that file, now claimed by the unsaved rock_normal tab: refused.
    e.apply(Action::OpenAsset(albedo.to_ref()));
    e.apply(Action::Tab(TabAction::Undo));
    assert_eq!(texture_doc(&e).asset.path, "textures/rock_albedo.png");
    assert!(e.messages.last().unwrap().text.contains(&normal.source_rel()));
    e.apply(Action::CloseDialog);

    // Saving rock_normal and regenerating from disk: no two assets on one file.
    e.apply(Action::OpenAsset(normal.to_ref()));
    e.apply(Action::Save);
    e.validate();
    assert!(!e.all_diagnostics().iter().any(|d| d.code == Code::E011), "{:?}", e.all_diagnostics());
}

/// Internal textures (design §2) open in a read-only Texture tab: double click in the `Internal` folder, the
/// "Open texture" button of a node using one, the config.
#[test]
fn internal_texture_tab_is_read_only() {
    use crate::tab::MaterialKey;
    use crate::ui::tree::{self, TreeItem, TreeRow};
    use shader_graph_core::texture::TextureSettings;
    use shader_stdlib::internal::{DEFAULT_TEXTURE, INTERNAL_TEXTURES};
    let (dir, mut e) = project();
    let id = AssetId::new(AssetType::Texture, DEFAULT_TEXTURE);
    let row = TreeRow {
        tree: TreeId::Project(MATERIAL),
        item: TreeItem::Virtual(AssetId::new(AssetType::Texture, DEFAULT_TEXTURE).to_ref()),
    };
    assert!(tree::click_actions(&row, 1).is_empty());
    let open = tree::click_actions(&row, 2);
    assert_eq!(open, vec![Action::OpenAsset(id.to_ref())]);
    for a in open.clone() {
        e.apply(a);
    }
    assert_eq!(e.tabs.len(), 1);
    let doc = e.tab().unwrap().doc();
    assert_eq!(doc.view(), crate::texture::TEXTURE);
    assert!(doc.is_read_only());
    assert_eq!(doc.asset(), None, "not a project asset");
    assert_eq!(e.tab().unwrap().owner(), None);
    assert_eq!(doc.base_title(), DEFAULT_TEXTURE);
    let key = doc.config_key().unwrap();
    assert_eq!(key, DEFAULT_TEXTURE);
    assert_eq!(parse_config_key(&key), Some(MaterialKey::Internal(DEFAULT_TEXTURE.into())));
    let t = texture_doc(&e);
    assert_eq!(t.asset.settings, (INTERNAL_TEXTURES[0].settings)());
    assert_eq!(t.asset.path, "quillshade/material/internal/default.png", "where regeneration copies it");
    assert_eq!(t.image_key(), DEFAULT_TEXTURE, "the preview decodes the embedded bytes");
    assert!(!e.overridden_set().textures.contains_key(&id));

    // Edits, undo and save do nothing.
    let settings = t.asset.settings.clone();
    e.apply(Action::from(TextureEdit::new(TextureAction::SetSettings(TextureSettings::default()))));
    e.apply(Action::from(TextureEdit::new(TextureAction::SetPath("textures/x.png".into()))));
    e.apply(Action::Tab(TabAction::Undo));
    assert_eq!(texture_doc(&e).asset.settings, settings);
    assert_eq!(texture_doc(&e).asset.path, "quillshade/material/internal/default.png");
    assert!(!e.is_dirty());
    assert!(!e.save(0));
    assert!(!dir.path().join("quillshade/material/@internal").exists());

    // Opened again from a node referencing it: the same tab.
    let rock = AssetId::new(AssetType::Material, "env/rock_wet");
    e.apply(Action::OpenAsset(rock.to_ref()));
    let sample = add(&mut e, "stdlib::TextureSample");
    e.apply(set_value(
        GraphProp::Param { node: sample.clone(), key: "texture".into() },
        Value::Str(DEFAULT_TEXTURE.into()),
        false,
    ));
    assert_eq!(e.referenced_asset(&sample), Some(id.clone()));
    assert!(e.texture_choices().iter().any(|(v, _)| v == DEFAULT_TEXTURE), "the picker lists internal textures");
    assert!(e.texture_choices().iter().any(|(v, _)| v == "textures/rock_albedo"));
    e.apply(Action::OpenAsset(id.to_ref()));
    assert_eq!(e.tabs.len(), 2);
    assert_eq!(e.active, Some(0));

    // Restored on the next launch.
    e.save_tabs_to_config();
    let mut next = crate::test_editor().with_config(e.config.clone(), None);
    next.open_project(dir.path()).unwrap();
    assert!(next.tabs.iter().any(|t| t.get::<crate::texture::TextureDoc>().is_some_and(|t| t.id() == id)));
}

/// Design §6.6: the Texture, game image and built-in texture tabs keep their keys (`material:<key>`, and legacy keys
/// without the prefix): existing user configs restore them as `texture_kit`'s documents of `MaterialTexture`.
#[test]
fn texture_tab_keys_restore_as_kit_tabs() {
    use crate::texture::{ImageDoc, TextureDoc};
    let (dir, e) = project();
    std::fs::create_dir_all(dir.path().join("assets/textures")).unwrap();
    std::fs::write(dir.path().join("assets/textures/rock.png"), b"x").unwrap();
    let root = e.project.as_ref().unwrap().root.clone();
    let reopen = |open: &[&str]| {
        let mut config = UserConfig::default();
        let open = open.iter().map(|k| k.to_string()).collect();
        config.tabs.insert(root.clone(), crate::config::ProjectTabs { open, active: Some(0), ..Default::default() });
        let mut e = crate::test_editor();
        e.config = config;
        e.open_project(dir.path()).unwrap();
        e
    };
    let keys = [
        "material:quillshade/material/textures/rock_albedo.texture.ron",
        "material:@game/textures/rock.png",
        "material:@internal/default",
    ];
    for open in [keys.to_vec(), keys.map(|k| k.trim_start_matches("material:")).to_vec()] {
        let e = reopen(&open);
        assert_eq!(e.tabs.len(), 3, "{open:?}");
        let albedo = AssetId::new(AssetType::Texture, "textures/rock_albedo");
        assert!(e.tabs[0].get::<TextureDoc>().is_some_and(|t| t.id() == albedo && !t.is_internal()));
        assert!(e.tabs[1].get::<ImageDoc>().is_some_and(|i| i.path == "textures/rock.png"));
        assert!(e.tabs[2].get::<TextureDoc>().is_some_and(|t| t.is_internal() && t.aref.path == "@internal/default"));
        let saved: Vec<String> = e.tabs.iter().map(|t| format!("material:{}", t.doc().config_key().unwrap())).collect();
        assert_eq!(saved, keys, "saved back under the same keys");
        let (texture, image) = (crate::texture::TEXTURE, crate::texture::IMAGE);
        assert_eq!(e.tabs.iter().map(|t| t.doc().view()).collect::<Vec<_>>(), [texture, image, texture]);
    }
}
