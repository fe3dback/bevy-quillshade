//! Asset file operations used by the asset browser and tests (regenerate afterwards).

use shader_graph_core::Value;
use shader_graph_core::consts::GlobalConsts;
use shader_graph_core::curve::Curve;
use shader_graph_core::gradient::GradientCollection;
use shader_graph_core::model::{Asset, AssetKind, MaterialKind, Node};
use shader_graph_core::registry::{
    FUNCTION_PREFIX, ParamKind, Registry, const_refs, curve_refs, gradient_refs, texture_refs,
};
use shader_graph_core::texture::{MATERIAL_TEXTURES, TextureAsset};

use crate::assets::{self, AssetId, AssetType};
use crate::layout::{MaterialLayout, Project};

#[derive(Debug, thiserror::Error)]
pub enum OpError {
    #[error("{0}")]
    Invalid(String),
    #[error("`{0}` already exists")]
    Exists(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

/// New asset with its required output node.
pub fn template(name: &str, kind: AssetKind) -> Asset {
    let mut a = Asset::new(name, kind);
    match kind {
        AssetKind::Material(m) => {
            a.add_node(Node::new(m.output_node_kind(), (400.0, 0.0)));
        }
        AssetKind::Function => {
            a.add_node(
                Node::new("output::FunctionOutput", (400.0, 0.0)).with_param("name", Value::Str("result".into())),
            );
        }
    }
    a
}

fn check_path(path: &str) -> Result<(), OpError> {
    project_core::ident::check_path(path).map_err(OpError::Invalid)
}

/// Writes a graph asset file (creating `quillshade/material/` and parent directories).
pub fn save(project: &Project, id: &AssetId, asset: &Asset) -> Result<(), OpError> {
    write_source(project, id, &asset.to_ron())
}

/// Writes a curve file (creating `quillshade/material/` and parent directories).
pub fn save_curve(project: &Project, id: &AssetId, curve: &Curve) -> Result<(), OpError> {
    write_source(project, id, &curve.to_ron())
}

/// Writes a global constant table (creating `quillshade/material/` and parent directories).
pub fn save_consts(project: &Project, id: &AssetId, table: &GlobalConsts) -> Result<(), OpError> {
    write_source(project, id, &table.to_ron())
}

/// Writes a Texture asset file (creating `quillshade/material/` and parent directories). No checks: an invalid or
/// shared image path is reported by validation (E012 / E011).
pub fn save_texture(project: &Project, id: &AssetId, texture: &TextureAsset) -> Result<(), OpError> {
    project.ensure_material_dir()?;
    shader_graph_core::texture::save(&id.file(project), texture)?;
    Ok(())
}

/// Writes a gradient collection file (creating `quillshade/material/` and parent directories).
pub fn save_gradients(project: &Project, id: &AssetId, collection: &GradientCollection) -> Result<(), OpError> {
    write_source(project, id, &collection.to_ron())
}

fn write_source(project: &Project, id: &AssetId, text: &str) -> Result<(), OpError> {
    project.ensure_material_dir()?;
    let file = id.file(project);
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = file.with_extension("ron.tmp");
    std::fs::write(&tmp, text)?;
    std::fs::rename(&tmp, &file)?;
    Ok(())
}

pub fn create_material(project: &Project, path: &str, kind: MaterialKind) -> Result<AssetId, OpError> {
    create(project, path, AssetKind::Material(kind))
}

pub fn create_function(project: &Project, path: &str) -> Result<AssetId, OpError> {
    create(project, path, AssetKind::Function)
}

/// New, empty global constant table.
pub fn create_consts(project: &Project, path: &str) -> Result<AssetId, OpError> {
    check_path(path)?;
    let id = AssetId::new(AssetType::Consts, path);
    if id.file(project).exists() {
        return Err(OpError::Exists(id.source_rel()));
    }
    save_consts(project, &id, &GlobalConsts::new(id.name(), Vec::new()))?;
    Ok(id)
}

/// New gradient collection: 128 empty slots with fresh random ids (gradient design §3).
pub fn create_gradients(project: &Project, path: &str) -> Result<AssetId, OpError> {
    check_path(path)?;
    let id = AssetId::new(AssetType::Gradients, path);
    if id.file(project).exists() {
        return Err(OpError::Exists(id.source_rel()));
    }
    save_gradients(project, &id, &GradientCollection::random())?;
    Ok(id)
}

/// New Texture asset `id` for the game image `texture.path`. Refused when the image path is invalid (E012 rules) or
/// another Texture asset already points at the same file (one game file — one Texture asset, design §1.1).
pub fn create_texture(project: &Project, id: &AssetId, texture: &TextureAsset) -> Result<(), OpError> {
    if id.ty != AssetType::Texture {
        return Err(OpError::Invalid(format!("{} is not a texture", id.source_rel())));
    }
    check_path(&id.path)?;
    if id.file(project).exists() {
        return Err(OpError::Exists(id.source_rel()));
    }
    TextureAsset::check_path(&texture.path, &MATERIAL_TEXTURES).map_err(OpError::Invalid)?;
    if let Some(other) = assets::scan(project)
        .into_iter()
        .filter(|o| o.ty == AssetType::Texture)
        .find(|o| assets::load_texture(project, o).is_ok_and(|t| t.path == texture.path))
    {
        return Err(OpError::Invalid(format!("`{}` already has a Texture asset: {other}", texture.path)));
    }
    save_texture(project, id, texture)
}

fn create(project: &Project, path: &str, kind: AssetKind) -> Result<AssetId, OpError> {
    check_path(path)?;
    let id = AssetId::new(AssetType::of(&kind), path);
    if id.file(project).exists() {
        return Err(OpError::Exists(id.source_rel()));
    }
    save(project, &id, &template(id.name(), kind))?;
    Ok(id)
}

/// Renames/moves an asset; function, curve, Texture asset and gradient collection references in other assets are
/// updated (constants are referenced by name, so moving their table changes nothing for its users). A Texture asset
/// keeps its image path: the game file is never moved.
pub fn rename(project: &Project, id: &AssetId, new_path: &str) -> Result<AssetId, OpError> {
    check_path(new_path)?;
    let new_id = AssetId::new(id.ty, new_path);
    if new_id == *id {
        return Ok(new_id);
    }
    if new_id.file(project).exists() {
        return Err(OpError::Exists(new_id.source_rel()));
    }
    match id.ty {
        AssetType::Curve => {
            let mut curve = assets::load_curve(project, id).map_err(OpError::Invalid)?;
            curve.name = new_id.name().to_string();
            save_curve(project, &new_id, &curve)?;
        }
        AssetType::Consts => {
            let mut table = assets::load_consts(project, id).map_err(OpError::Invalid)?;
            table.name = new_id.name().to_string();
            save_consts(project, &new_id, &table)?;
        }
        AssetType::Texture => {
            let texture = assets::load_texture(project, id).map_err(OpError::Invalid)?;
            save_texture(project, &new_id, &texture)?;
        }
        AssetType::Gradients => {
            let collection = assets::load_gradients(project, id).map_err(OpError::Invalid)?;
            save_gradients(project, &new_id, &collection)?;
        }
        AssetType::Material | AssetType::Function => {
            let mut asset = assets::load(project, id).map_err(OpError::Invalid)?;
            asset.name = new_id.name().to_string();
            save(project, &new_id, &asset)?;
        }
    }
    std::fs::remove_file(id.file(project))?;
    match id.ty {
        AssetType::Curve => rewrite_graphs(project, |a, r| rename_curve_refs(a, r, &id.path, new_path))?,
        AssetType::Texture => rewrite_graphs(project, |a, r| rename_texture_refs(a, r, &id.path, new_path))?,
        AssetType::Gradients => rewrite_graphs(project, |a, r| rename_gradient_refs(a, r, &id.path, new_path))?,
        _ => {}
    }
    if id.ty == AssetType::Function {
        let (old_kind, new_kind) = (format!("{FUNCTION_PREFIX}{}", id.path), format!("{FUNCTION_PREFIX}{new_path}"));
        for other in assets::scan(project).into_iter().filter(|o| o.ty.is_graph()) {
            let Ok(mut a) = assets::load(project, &other) else { continue };
            let mut changed = false;
            for n in a.nodes.values_mut() {
                if n.kind == old_kind {
                    n.kind = new_kind.clone();
                    changed = true;
                }
            }
            if changed {
                save(project, &other, &a)?;
            }
        }
    }
    Ok(new_id)
}

/// Applies `edit` to every graph asset of the project and saves the ones it changed (`edit` returns whether it did).
/// Files that do not load are skipped.
fn rewrite_graphs(project: &Project, mut edit: impl FnMut(&mut Asset, &Registry) -> bool) -> Result<(), OpError> {
    let (registry, _) = crate::regen::project_registry(project);
    for other in assets::scan(project).into_iter().filter(|o| o.ty.is_graph()) {
        let Ok(mut a) = assets::load(project, &other) else { continue };
        if edit(&mut a, &registry) {
            save(project, &other, &a)?;
        }
    }
    Ok(())
}

/// Points every node of `asset` that samples curve `old` to `new`; returns whether anything changed.
pub fn rename_curve_refs(asset: &mut Asset, registry: &Registry, old: &str, new: &str) -> bool {
    set_refs(asset, curve_refs(asset, registry), old, new)
}

/// Points every node of `asset` that refers to Texture asset `old` to `new`; returns whether anything changed.
pub fn rename_texture_refs(asset: &mut Asset, registry: &Registry, old: &str, new: &str) -> bool {
    set_refs(asset, texture_refs(asset, registry), old, new)
}

/// Points every node of `asset` that samples gradient collection `old` to `new` (slot ids stay); returns whether
/// anything changed.
pub fn rename_gradient_refs(asset: &mut Asset, registry: &Registry, old: &str, new: &str) -> bool {
    set_refs(asset, gradient_refs(asset, registry), old, new)
}

/// Clears every reference of `asset`'s nodes to gradient collection `id`: the collection parameter becomes `""` and
/// the slot parameters ([`ParamKind::GradientSlot`]) of the same nodes `0`, so they output white (W111, gradient
/// design §6); returns whether anything changed. Used when the collection is deleted.
pub fn clear_gradient_refs(asset: &mut Asset, registry: &Registry, id: &str) -> bool {
    let refs: Vec<_> = gradient_refs(asset, registry).into_iter().filter(|r| r.2 == id).collect();
    for (node, _, _) in &refs {
        let Some(n) = asset.nodes.get_mut(node) else { continue };
        let Some(def) = registry.get(&n.kind) else { continue };
        for p in def.params().into_iter().filter(|p| p.kind == ParamKind::GradientSlot) {
            n.params.insert(p.id, Value::Int(0));
        }
    }
    set_refs(asset, refs, id, "")
}

/// Clears every reference of `asset`'s nodes to Texture asset `id` (they then use the default texture, design
/// §1.2); returns whether anything changed. Used when the Texture asset is deleted.
pub fn clear_texture_refs(asset: &mut Asset, registry: &Registry, id: &str) -> bool {
    set_refs(asset, texture_refs(asset, registry), id, "")
}

/// Sets the parameters of `refs` (`(node, param, value)`, from `curve_refs` & co.) whose value is `old` to `new`;
/// returns whether anything changed.
fn set_refs(asset: &mut Asset, refs: Vec<(String, String, String)>, old: &str, new: &str) -> bool {
    let mut changed = false;
    for (node, param, _) in refs.into_iter().filter(|r| r.2 == old) {
        if let Some(n) = asset.nodes.get_mut(&node) {
            n.params.insert(param, Value::Str(new.to_string()));
            changed = true;
        }
    }
    changed
}

/// Clears every reference of `asset`'s nodes to global constant `name` (they then output 1.0, W106); returns
/// whether anything changed. Used when the constant changes its type or is deleted (decision 0017).
pub fn clear_const_refs(asset: &mut Asset, registry: &Registry, name: &str) -> bool {
    set_refs(asset, const_refs(asset, registry), name, "")
}

/// Moves an asset into directory `dir` (`""` = the Material source root).
pub fn move_to(project: &Project, id: &AssetId, dir: &str) -> Result<AssetId, OpError> {
    let new_path = if dir.is_empty() { id.name().to_string() } else { format!("{dir}/{}", id.name()) };
    rename(project, id, &new_path)
}

/// Deletes the asset source; autogen is removed by the following full regeneration. Deleting a Texture asset keeps
/// its game image and clears the references to it in every graph (design §1.2); deleting a gradient collection
/// clears the references to it too (gradient design §6).
pub fn delete(project: &Project, id: &AssetId) -> Result<(), OpError> {
    std::fs::remove_file(id.file(project))?;
    match id.ty {
        AssetType::Texture => rewrite_graphs(project, |a, r| clear_texture_refs(a, r, &id.path))?,
        AssetType::Gradients => rewrite_graphs(project, |a, r| clear_gradient_refs(a, r, &id.path))?,
        _ => {}
    }
    Ok(())
}
