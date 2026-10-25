//! Asset file operations used by the asset browser and tests (regenerate afterwards).

use shader_graph_core::Value;
use shader_graph_core::ident::check_ident;
use shader_graph_core::model::{Asset, AssetKind, MaterialKind, Node};
use shader_graph_core::registry::FUNCTION_PREFIX;

use crate::assets::{self, AssetId, AssetType};
use crate::layout::Project;

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
    if path.is_empty() {
        return Err(OpError::Invalid("empty name".into()));
    }
    for part in path.split('/') {
        check_ident(part).map_err(OpError::Invalid)?;
    }
    Ok(())
}

/// Writes an asset file (creating `shaders/` and parent directories).
pub fn save(project: &Project, id: &AssetId, asset: &Asset) -> Result<(), OpError> {
    project.ensure_shaders_dir()?;
    let file = id.file(project);
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = file.with_extension("ron.tmp");
    std::fs::write(&tmp, asset.to_ron())?;
    std::fs::rename(&tmp, &file)?;
    Ok(())
}

pub fn create_material(project: &Project, path: &str, kind: MaterialKind) -> Result<AssetId, OpError> {
    create(project, path, AssetKind::Material(kind))
}

pub fn create_function(project: &Project, path: &str) -> Result<AssetId, OpError> {
    create(project, path, AssetKind::Function)
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

pub fn create_folder(project: &Project, path: &str) -> Result<(), OpError> {
    check_path(path)?;
    project.ensure_shaders_dir()?;
    std::fs::create_dir_all(project.shaders_dir().join(path))?;
    Ok(())
}

/// Renames/moves an asset; function references in other assets are updated.
pub fn rename(project: &Project, id: &AssetId, new_path: &str) -> Result<AssetId, OpError> {
    check_path(new_path)?;
    let new_id = AssetId::new(id.ty, new_path);
    if new_id == *id {
        return Ok(new_id);
    }
    if new_id.file(project).exists() {
        return Err(OpError::Exists(new_id.source_rel()));
    }
    let mut asset = assets::load(project, id).map_err(OpError::Invalid)?;
    asset.name = new_id.name().to_string();
    save(project, &new_id, &asset)?;
    std::fs::remove_file(id.file(project))?;
    if id.ty == AssetType::Function {
        let (old_kind, new_kind) = (format!("{FUNCTION_PREFIX}{}", id.path), format!("{FUNCTION_PREFIX}{new_path}"));
        for other in assets::scan(project) {
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

/// Moves an asset into directory `dir` (`""` = shaders root).
pub fn move_to(project: &Project, id: &AssetId, dir: &str) -> Result<AssetId, OpError> {
    let new_path = if dir.is_empty() { id.name().to_string() } else { format!("{dir}/{}", id.name()) };
    rename(project, id, &new_path)
}

/// Deletes the asset source; autogen is removed by the following full regeneration.
pub fn delete(project: &Project, id: &AssetId) -> Result<(), OpError> {
    std::fs::remove_file(id.file(project))?;
    Ok(())
}

/// Deletes an empty folder (or one containing only empty folders).
pub fn delete_folder(project: &Project, path: &str) -> Result<(), OpError> {
    let dir = project.shaders_dir().join(path);
    crate::writer::remove_empty_dirs(&dir);
    std::fs::remove_dir(&dir)?;
    Ok(())
}
