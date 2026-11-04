//! Asset file operations used by the asset browser and tests (regenerate afterwards).
//!
//! The `save*` writers take the model mutably: a model gets its asset id first when it has none (ADR 0047 §6), so the
//! caller keeps the id the file now has; graphs are written with the D27 path comments (ADR 0047 §5).

use project_core::asset_index::{RefNames, annotate_refs};
use shader_graph_core::Value;
use shader_graph_core::consts::GlobalConsts;
use shader_graph_core::curve::Curve;
use shader_graph_core::features::FeatureTable;
use shader_graph_core::gradient::GradientCollection;
use shader_graph_core::model::{Asset, AssetKind, MaterialKind, Node};
use shader_graph_core::registry::{
    ParamKind, Registry, const_refs, const_table_refs, feature_enum_refs, feature_table_refs, gradient_refs,
    texture_refs,
};
use shader_graph_core::texture::{MATERIAL_TEXTURES, TextureAsset};
use shader_stdlib::user::UserNode;

use crate::assets::{self, AssetId, AssetType};
use crate::layout::{MATERIAL, MaterialLayout, Project, USER_NODES_DIR};

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

/// Writes a graph asset file (creating `quillshade/material/` and parent directories) with the D27 path comments
/// labelled by the project's Material assets on disk ([`save_annotated`] with an index built here; the editor passes
/// its own).
pub fn save(project: &Project, id: &AssetId, asset: &mut Asset) -> Result<(), OpError> {
    save_annotated(project, id, asset, &material_index(project))
}

/// Writes a graph asset file with the D27 path comment (ADR 0047 §5) after every UUID reference, labelled by `names`
/// (the project's `AssetIndex`).
pub fn save_annotated(project: &Project, id: &AssetId, asset: &mut Asset, names: &dyn RefNames) -> Result<(), OpError> {
    asset.ensure_id();
    write_source(project, id, &annotate_refs(&asset.to_ron(), names, MATERIAL))
}

/// Writes a curve file (creating `quillshade/material/` and parent directories).
pub fn save_curve(project: &Project, id: &AssetId, curve: &mut Curve) -> Result<(), OpError> {
    curve.ensure_id();
    write_source(project, id, &curve.to_ron())
}

/// Writes a global constant table (creating `quillshade/material/` and parent directories).
pub fn save_consts(project: &Project, id: &AssetId, table: &mut GlobalConsts) -> Result<(), OpError> {
    table.ensure_id();
    write_source(project, id, &table.to_ron())
}

/// Writes a Game Feature enum table (creating `quillshade/material/` and parent directories).
pub fn save_features(project: &Project, id: &AssetId, table: &mut FeatureTable) -> Result<(), OpError> {
    table.ensure_id();
    write_source(project, id, &table.to_ron())
}

/// Writes a Texture asset file (creating `quillshade/material/` and parent directories). No checks: an invalid or
/// shared image path is reported by validation (E012 / E011).
pub fn save_texture(project: &Project, id: &AssetId, texture: &mut TextureAsset) -> Result<(), OpError> {
    texture.ensure_id();
    project.ensure_material_dir()?;
    shader_graph_core::texture::save(&id.file(project), texture)?;
    Ok(())
}

/// Writes a gradient collection file (creating `quillshade/material/` and parent directories).
pub fn save_gradients(project: &Project, id: &AssetId, collection: &mut GradientCollection) -> Result<(), OpError> {
    collection.ensure_id();
    write_source(project, id, &collection.to_ron())
}

/// Writes the user node file `quillshade/material/.nodes/<name>.ron` through the canonical writer
/// ([`UserNode::to_ron`], ADR 0047 §16); the node gets its id first when it has none.
pub fn save_user_node(project: &Project, name: &str, node: &mut UserNode) -> Result<(), OpError> {
    node.ensure_id();
    project.ensure_material_dir()?;
    let file = project.material_dir().join(USER_NODES_DIR).join(format!("{name}.ron"));
    project_core::writer::write_atomic(&file, node.to_ron())?;
    Ok(())
}

/// Gives the Material source file `r` the asset id `id` (the UUID quick fixes Regenerate UUID and Assign UUID, ADR
/// 0047 §11, §16): the file is loaded, its id set and written back through its kind's canonical writer (a graph with
/// the D27 path comments labelled by `names`; a user node `quillshade/material/.nodes/<name>.ron` through
/// [`UserNode::to_ron`], so a hand-formatted file becomes canonical). A file that does not load is refused untouched.
pub fn set_asset_uuid(
    project: &Project,
    r: &project_core::AssetRef,
    id: project_core::AssetUuid,
    names: &dyn RefNames,
) -> Result<(), OpError> {
    let invalid = |e: String| OpError::Invalid(format!("`{r}`: {e}"));
    if r.domain == MATERIAL && r.suffix.is_empty() {
        let name =
            r.path.strip_prefix(&format!("{USER_NODES_DIR}/")).ok_or_else(|| invalid("not a user node".into()))?;
        let text = std::fs::read_to_string(project.root.join(r.source_rel()))?;
        let mut node = UserNode::from_ron(&text).map_err(invalid)?;
        node.id = id;
        return save_user_node(project, name, &mut node);
    }
    let Some(asset_id) = AssetId::from_ref(r) else {
        return Err(OpError::Invalid(format!("`{r}` is not a Material asset")));
    };
    match asset_id.ty {
        AssetType::Material | AssetType::Function => {
            let mut a = assets::load(project, &asset_id).map_err(invalid)?;
            a.id = id;
            save_annotated(project, &asset_id, &mut a, names)
        }
        AssetType::Curve => {
            let mut c = assets::load_curve(project, &asset_id).map_err(invalid)?;
            c.id = id;
            save_curve(project, &asset_id, &mut c)
        }
        AssetType::Consts => {
            let mut t = assets::load_consts(project, &asset_id).map_err(invalid)?;
            t.id = id;
            save_consts(project, &asset_id, &mut t)
        }
        AssetType::Features => {
            let mut t = assets::load_features(project, &asset_id).map_err(invalid)?;
            t.id = id;
            t.regenerate_sub_ids();
            save_features(project, &asset_id, &mut t)
        }
        AssetType::Texture => {
            let mut t = assets::load_texture(project, &asset_id).map_err(invalid)?;
            t.id = id;
            save_texture(project, &asset_id, &mut t)
        }
        AssetType::Gradients => {
            let mut g = assets::load_gradients(project, &asset_id).map_err(invalid)?;
            g.id = id;
            save_gradients(project, &asset_id, &mut g)
        }
    }
}

fn write_source(project: &Project, id: &AssetId, text: &str) -> Result<(), OpError> {
    project.ensure_material_dir()?;
    project_core::writer::write_atomic(&id.file(project), text)?;
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
    save_consts(project, &id, &mut GlobalConsts::new(id.name(), Vec::new()))?;
    Ok(id)
}

/// New, empty Game Feature enum table.
pub fn create_features(project: &Project, path: &str) -> Result<AssetId, OpError> {
    check_path(path)?;
    let id = AssetId::new(AssetType::Features, path);
    if id.file(project).exists() {
        return Err(OpError::Exists(id.source_rel()));
    }
    save_features(project, &id, &mut FeatureTable::new(id.name(), Vec::new()))?;
    Ok(id)
}

/// New gradient collection: 128 empty slots with fresh random ids (gradient design §3).
pub fn create_gradients(project: &Project, path: &str) -> Result<AssetId, OpError> {
    check_path(path)?;
    let id = AssetId::new(AssetType::Gradients, path);
    if id.file(project).exists() {
        return Err(OpError::Exists(id.source_rel()));
    }
    save_gradients(project, &id, &mut GradientCollection::random())?;
    Ok(id)
}

/// New Texture asset `id` for the game image `texture.path`. Refused when the image path is invalid (E012 rules) or
/// another Texture asset already points at the same file (one game file — one Texture asset, design §1.1).
pub fn create_texture(project: &Project, id: &AssetId, texture: &mut TextureAsset) -> Result<(), OpError> {
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
    save(project, &id, &mut template(id.name(), kind))?;
    Ok(id)
}

/// Renames / moves an asset: the file is written at the new path (same id, so every reference to it stays valid) and
/// the old one removed. No other file changes (ADR 0047 §13): references store the asset's UUID, their D27 path
/// comments follow on the referrer's next save. A Texture asset keeps its image path: the game file is never moved.
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
            save_curve(project, &new_id, &mut curve)?;
        }
        AssetType::Consts => {
            let mut table = assets::load_consts(project, id).map_err(OpError::Invalid)?;
            table.name = new_id.name().to_string();
            save_consts(project, &new_id, &mut table)?;
        }
        AssetType::Features => {
            let mut table = assets::load_features(project, id).map_err(OpError::Invalid)?;
            table.name = new_id.name().to_string();
            save_features(project, &new_id, &mut table)?;
        }
        AssetType::Texture => {
            let mut texture = assets::load_texture(project, id).map_err(OpError::Invalid)?;
            save_texture(project, &new_id, &mut texture)?;
        }
        AssetType::Gradients => {
            let mut collection = assets::load_gradients(project, id).map_err(OpError::Invalid)?;
            save_gradients(project, &new_id, &mut collection)?;
        }
        AssetType::Material | AssetType::Function => {
            let mut asset = assets::load(project, id).map_err(OpError::Invalid)?;
            asset.name = new_id.name().to_string();
            save_annotated(project, &new_id, &mut asset, &material_index(project))?;
        }
    }
    std::fs::remove_file(id.file(project))?;
    Ok(new_id)
}

/// Applies `edit` to every graph asset of the project and saves the ones it changed (`edit` returns whether it did),
/// with their D27 path comments ([`save_annotated`]). Files that do not load are skipped.
fn rewrite_graphs(project: &Project, mut edit: impl FnMut(&mut Asset, &Registry) -> bool) -> Result<(), OpError> {
    let (registry, _) = crate::regen::project_registry(project);
    let names = material_index(project);
    for other in assets::scan(project).into_iter().filter(|o| o.ty.is_graph()) {
        let Ok(mut a) = assets::load(project, &other) else { continue };
        if edit(&mut a, &registry) {
            save_annotated(project, &other, &mut a, &names)?;
        }
    }
    Ok(())
}

/// The Material part of the project's `AssetIndex` as the files are now: what graphs rewritten here label their
/// D27 path comments with (ADR 0047 §5; Material references stay inside the domain).
fn material_index(project: &Project) -> project_core::AssetIndex {
    use project_core::regen::DomainBackend;
    project_core::AssetIndex::build(vec![crate::backend::MaterialBackend::default().index(project)])
}

/// Clears every reference of `asset`'s nodes to the gradient collection named by `names` (the values a reference to
/// it stores: its UUID, or its path in a hand-written file — [`ref_names`]): the collection parameter becomes `""`
/// and the slot parameters ([`ParamKind::GradientSlot`]) of the same nodes `0`, so they output white (W111, gradient
/// design §6); returns whether anything changed. Used when the collection is deleted.
pub fn clear_gradient_refs(asset: &mut Asset, registry: &Registry, names: &[String]) -> bool {
    let refs = gradient_refs(asset, registry);
    clear_refs(asset, registry, refs, names, |k| *k == ParamKind::GradientSlot)
}

/// Clears every reference of `asset`'s nodes to the Texture asset named by `names` ([`ref_names`]); they then use the
/// default texture (design §1.2). Returns whether anything changed. Used when the Texture asset is deleted.
pub fn clear_texture_refs(asset: &mut Asset, registry: &Registry, names: &[String]) -> bool {
    let refs = texture_refs(asset, registry);
    clear_refs(asset, registry, refs, names, |_| false)
}

/// Clears every reference of `asset`'s nodes to the constant table named by `names` ([`ref_names`]): the table
/// parameter becomes `""` and the constant parameters ([`ParamKind::GlobalConst`]) of the same nodes `0`, so they
/// output 1.0 (W106, decision 0017); returns whether anything changed. Used when the table is deleted.
pub fn clear_const_table_refs(asset: &mut Asset, registry: &Registry, names: &[String]) -> bool {
    let refs = const_table_refs(asset, registry);
    clear_refs(asset, registry, refs, names, |k| matches!(k, ParamKind::GlobalConst(_)))
}

/// Clears every reference of `asset`'s nodes to the feature table named by `names` ([`ref_names`]): the table
/// parameter and the enum parameter ([`ParamKind::FeatureEnum`]) of the same nodes become `""`, and edges into
/// those nodes are removed; returns whether anything changed. Used when the feature table is deleted.
pub fn clear_feature_table_refs(asset: &mut Asset, registry: &Registry, names: &[String]) -> bool {
    let refs = feature_table_refs(asset, registry);
    let touched: Vec<String> = refs.iter().filter(|r| names.contains(&r.2)).map(|r| r.0.clone()).collect();
    let mut changed = clear_refs(asset, registry, refs, names, |k| *k == ParamKind::FeatureEnum);
    for node in &touched {
        if let Some(n) = asset.nodes.get_mut(node)
            && !n.inputs.is_empty()
        {
            n.inputs.clear();
            changed = true;
        }
    }
    if !touched.is_empty() {
        let before = asset.edges.len();
        asset.edges.retain(|e| !touched.contains(&e.to.0));
        changed |= asset.edges.len() != before;
    }
    changed
}

/// Sets the parameters of `refs` (`(node, param, value)`, from `texture_refs` & co.) whose value is one of `names` to
/// `""`, and the parameters of the same nodes whose kind passes `sub_item` (a slot, a constant, a feature enum) to
/// their default; returns whether anything changed.
fn clear_refs(
    asset: &mut Asset,
    registry: &Registry,
    refs: Vec<(String, String, String)>,
    names: &[String],
    sub_item: impl Fn(&ParamKind) -> bool,
) -> bool {
    let mut changed = false;
    for (node, param, _) in refs.into_iter().filter(|r| names.contains(&r.2)) {
        let Some(n) = asset.nodes.get_mut(&node) else { continue };
        if let Some(def) = registry.get(&n.kind) {
            for p in def.params().into_iter().filter(|p| sub_item(&p.kind)) {
                n.params.insert(p.id, p.default);
            }
        }
        n.params.insert(param, Value::Str(String::new()));
        changed = true;
    }
    changed
}

/// The values a reference to Material asset `id` stores, as its files are now: its UUID (ADR 0047 §4) and its path
/// (a hand-written file may still name it so; `resolve` passes paths through). A duplicate loser (a copied file,
/// AE001) has no UUID here: references to its id name the original ([`project_core::AssetIndex::ref_id_of`]).
pub fn ref_names(project: &Project, id: &AssetId) -> Vec<String> {
    let uuid = material_index(project).ref_id_of(&id.to_ref()).map(|u| u.to_string());
    std::iter::once(id.path.clone()).chain(uuid).collect()
}

/// Clears every reference of `asset`'s nodes to global constant `(table, id)` (the constant parameter becomes `0`, so
/// they output 1.0, W106); returns whether anything changed. Used when the constant changes its type or is deleted
/// (decision 0017).
pub fn clear_const_refs(asset: &mut Asset, registry: &Registry, table: &str, id: u32) -> bool {
    let mut changed = false;
    for (node, param, t, cid) in const_refs(asset, registry) {
        if t == table
            && cid == id
            && let Some(n) = asset.nodes.get_mut(&node)
        {
            n.params.insert(param, Value::Int(0));
            changed = true;
        }
    }
    changed
}

/// Clears every reference of `asset`'s nodes to feature enum `(table, enum_key)` (where `enum_key` is in `enum_keys`,
/// e.g. the enum's UUID string or name); returns whether anything changed. Used when an enum is deleted from a
/// `.features.ron` table.
pub fn clear_feature_enum_refs(asset: &mut Asset, registry: &Registry, table: &str, enum_keys: &[String]) -> bool {
    let mut changed = false;
    let mut touched = Vec::new();
    for (node, param, t, eid) in feature_enum_refs(asset, registry) {
        if t == table
            && enum_keys.contains(&eid)
            && let Some(n) = asset.nodes.get_mut(&node)
        {
            n.params.insert(param, Value::Str(String::new()));
            n.inputs.clear();
            touched.push(node);
            changed = true;
        }
    }
    if !touched.is_empty() {
        let before = asset.edges.len();
        asset.edges.retain(|e| !touched.contains(&e.to.0));
        changed |= asset.edges.len() != before;
    }
    changed
}

/// Clears edges and stored pin inputs on `asset`'s nodes that switch on `(table, enum_key)` and target any pin in
/// `variant_pins`; returns whether anything changed. Used when a variant is deleted from a `FeatureEnum`.
pub fn clear_feature_variant_refs(
    asset: &mut Asset,
    registry: &Registry,
    table: &str,
    enum_keys: &[String],
    variant_pins: &[String],
) -> bool {
    let mut changed = false;
    let mut touched = Vec::new();
    for (node, _, t, eid) in feature_enum_refs(asset, registry) {
        if t == table && enum_keys.contains(&eid) {
            if let Some(n) = asset.nodes.get_mut(&node) {
                for pin in variant_pins {
                    if n.inputs.remove(pin).is_some() {
                        changed = true;
                    }
                }
            }
            touched.push(node);
        }
    }
    if !touched.is_empty() {
        let before = asset.edges.len();
        asset.edges.retain(|e| !(touched.contains(&e.to.0) && variant_pins.contains(&e.to.1)));
        changed |= asset.edges.len() != before;
    }
    changed
}

/// Moves an asset into directory `dir` (`""` = the Material source root).
pub fn move_to(project: &Project, id: &AssetId, dir: &str) -> Result<AssetId, OpError> {
    let new_path = if dir.is_empty() { id.name().to_string() } else { format!("{dir}/{}", id.name()) };
    rename(project, id, &new_path)
}

/// Deletes the asset source; autogen is removed by the following full regeneration. Deleting a Texture asset keeps
/// its game image and clears the references to it in every graph (design §1.2); deleting a gradient collection,
/// constant table, or feature table clears the references to it too (gradient design §6, decision 0017). References
/// are matched by the asset's UUID (and its path, [`ref_names`]; ADR 0047 §13); curve and function references stay
/// dangling.
pub fn delete(project: &Project, id: &AssetId) -> Result<(), OpError> {
    let names = ref_names(project, id);
    std::fs::remove_file(id.file(project))?;
    match id.ty {
        AssetType::Consts => rewrite_graphs(project, |a, r| clear_const_table_refs(a, r, &names))?,
        AssetType::Features => rewrite_graphs(project, |a, r| clear_feature_table_refs(a, r, &names))?,
        AssetType::Texture => rewrite_graphs(project, |a, r| clear_texture_refs(a, r, &names))?,
        AssetType::Gradients => rewrite_graphs(project, |a, r| clear_gradient_refs(a, r, &names))?,
        _ => {}
    }
    Ok(())
}
