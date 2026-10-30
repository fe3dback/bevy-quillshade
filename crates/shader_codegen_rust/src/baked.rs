//! Generated `crates/quillshade/src/material/baked/` (design §3.1): the `BakedMaterials` / `BakedTextures` structs
//! (`materials.rs`, `textures.rs`) and the static runtime code (`mod.rs`, from `templates/baked_mod.rs`).
//!
//! Field names (design §3.1): the plain folder / material / texture name; a Rust keyword gets the decision-0020
//! prefix (`dir_` for folders, `mat_` for materials, `tex_` for textures), and names that would clash inside one
//! struct (folder `env/water/` next to material `env/water`) are all prefixed. The nested struct types live in
//! modules named like the generated module tree (`dir_env::Group`), unique by construction.

use std::collections::BTreeMap;

use shader_codegen_wesl::paths;
use shader_graph_core::GENERATOR_VERSION;
use shader_graph_core::gradient::GRADIENT_PREFIX;
use shader_graph_core::texture::INTERNAL_PREFIX;

use crate::MaterialEntry;

pub const DIR_PREFIX: &str = "dir_";
pub const MATERIAL_PREFIX: &str = "mat_";
pub const TEXTURE_PREFIX: &str = "tex_";
/// Field (and module) of `BakedTextures` holding the internal textures (`tex.internal.default`).
pub const INTERNAL_GROUP: &str = "internal";
/// Field (and module) of `BakedTextures` holding the baked gradient collections, nested by path
/// (`tex.gradients.common.terrain`, gradient design §5).
pub const GRADIENTS_GROUP: &str = "gradients";
/// Reserved root groups of `BakedTextures`: field / module name and the id prefix of their textures.
const TEXTURE_GROUPS: [(&str, &str); 2] = [(INTERNAL_GROUP, INTERNAL_PREFIX), (GRADIENTS_GROUP, GRADIENT_PREFIX)];
/// Root field of `BakedMaterials` and `BakedTextures` mapping asset ids to handles (`baked.by_path["env/rock_wet"]`).
pub const BY_PATH_FIELD: &str = "by_path";

/// Static part of `baked/mod.rs`; `{{GENERATOR_VERSION}}` is replaced.
const MOD_TEMPLATE: &str = include_str!("../templates/baked_mod.rs");

/// Rust keywords (strict, reserved, 2024 edition): not usable as field names.
const RUST_KEYWORDS: &[&str] = &[
    "as", "async", "await", "break", "const", "continue", "crate", "dyn", "else", "enum", "extern", "false", "fn",
    "for", "gen", "if", "impl", "in", "let", "loop", "match", "mod", "move", "mut", "pub", "ref", "return", "self",
    "static", "struct", "super", "trait", "true", "type", "unsafe", "use", "where", "while", "abstract", "become",
    "box", "do", "final", "macro", "override", "priv", "try", "typeof", "unsized", "virtual", "yield",
];

/// Ids by folder.
#[derive(Default)]
struct Tree {
    dirs: BTreeMap<String, Tree>,
    /// Raw name → id.
    leaves: BTreeMap<String, String>,
}

impl Tree {
    fn of<'a>(ids: impl Iterator<Item = &'a str>) -> Tree {
        let mut t = Tree::default();
        for id in ids {
            let parts = paths::split(id);
            let Some((leaf, dirs)) = parts.split_last() else { continue };
            let mut cur = &mut t;
            for d in dirs {
                cur = cur.dirs.entry(d.clone()).or_default();
            }
            cur.leaves.insert(leaf.clone(), id.to_string());
        }
        t
    }
}

/// Field names of one struct: (raw folder → field, raw leaf → field). `reserved` names are taken by other fields.
fn group_fields(
    tree: &Tree,
    leaf_prefix: &str,
    reserved: &[&str],
) -> (BTreeMap<String, String>, BTreeMap<String, String>) {
    // (raw, is_dir, prefixed)
    let mut entries: Vec<(&str, bool, bool)> = tree
        .dirs
        .keys()
        .map(|d| (d.as_str(), true))
        .chain(tree.leaves.keys().map(|l| (l.as_str(), false)))
        .map(|(raw, dir)| (raw, dir, RUST_KEYWORDS.contains(&raw) || reserved.contains(&raw)))
        .collect();
    let name = |(raw, dir, prefixed): (&str, bool, bool)| match (prefixed, dir) {
        (false, _) => raw.to_string(),
        (true, true) => format!("{DIR_PREFIX}{raw}"),
        (true, false) => format!("{leaf_prefix}{raw}"),
    };
    // Prefix every unprefixed member of a clash until names are unique (prefixed names never clash: folder names
    // and leaf names are unique among themselves, and the prefixes differ).
    loop {
        let mut by_name: BTreeMap<String, Vec<usize>> = BTreeMap::new();
        for (i, e) in entries.iter().enumerate() {
            by_name.entry(name(*e)).or_default().push(i);
        }
        let mut changed = false;
        for i in by_name.values().filter(|v| v.len() > 1).flatten() {
            if !entries[*i].2 {
                entries[*i].2 = true;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    let mut dirs = BTreeMap::new();
    let mut leaves = BTreeMap::new();
    for e in entries {
        if e.1 {
            dirs.insert(e.0.to_string(), name(e))
        } else {
            leaves.insert(e.0.to_string(), name(e))
        };
    }
    (dirs, leaves)
}

/// One struct of the nested tree.
#[derive(Default)]
struct Group {
    /// Field name → field.
    fields: BTreeMap<String, Field>,
}

enum Field {
    /// An asset: its id.
    Leaf(String),
    /// A folder: its module name and struct.
    Dir(String, Group),
}

fn build_group(tree: &Tree, leaf_prefix: &str, reserved: &[&str]) -> Group {
    let (dirs, leaves) = group_fields(tree, leaf_prefix, reserved);
    let mut g = Group::default();
    for (raw, sub) in &tree.dirs {
        let module = format!("{DIR_PREFIX}{raw}");
        g.fields.insert(dirs[raw].clone(), Field::Dir(module, build_group(sub, leaf_prefix, &[])));
    }
    for (raw, id) in &tree.leaves {
        g.fields.insert(leaves[raw].clone(), Field::Leaf(id.clone()));
    }
    g
}

fn material_tree<'a>(ids: impl Iterator<Item = &'a str>) -> Group {
    build_group(&Tree::of(ids), MATERIAL_PREFIX, &[BY_PATH_FIELD])
}

/// Project textures by folder, plus the internal ones in the `internal` field and the baked gradient collections in
/// the `gradients` field, nested by path (which, like `by_path`, no project name takes).
fn texture_tree<'a>(ids: impl Iterator<Item = &'a str>) -> Group {
    let ids: Vec<&str> = ids.collect();
    let project = ids.iter().copied().filter(|id| !TEXTURE_GROUPS.iter().any(|(_, prefix)| id.starts_with(prefix)));
    let reserved = [INTERNAL_GROUP, GRADIENTS_GROUP, BY_PATH_FIELD];
    let mut g = build_group(&Tree::of(project), TEXTURE_PREFIX, &reserved);
    for (group, prefix) in TEXTURE_GROUPS {
        let names: Vec<&str> = ids.iter().filter_map(|id| id.strip_prefix(prefix)).collect();
        if names.is_empty() {
            continue;
        }
        let mut sub = build_group(&Tree::of(names.into_iter()), TEXTURE_PREFIX, &[]);
        // Leaves hold the full id (`@internal/default`).
        fn restore(g: &mut Group, prefix: &str) {
            for f in g.fields.values_mut() {
                match f {
                    Field::Leaf(id) => *id = format!("{prefix}{id}"),
                    Field::Dir(_, sub) => restore(sub, prefix),
                }
            }
        }
        restore(&mut sub, prefix);
        g.fields.insert(group.to_string(), Field::Dir(group.to_string(), sub));
    }
    g
}

fn field_paths_of(g: &Group, prefix: &mut Vec<String>, out: &mut BTreeMap<String, Vec<String>>) {
    for (name, f) in &g.fields {
        prefix.push(name.clone());
        match f {
            Field::Leaf(id) => {
                out.insert(id.clone(), prefix.clone());
            }
            Field::Dir(_, sub) => field_paths_of(sub, prefix, out),
        }
        prefix.pop();
    }
}

/// Field path of every material in `BakedMaterials` (`kw/loop/type` → `["kw", "dir_loop", "mat_type"]`).
pub fn material_field_paths<'a>(ids: impl Iterator<Item = &'a str>) -> BTreeMap<String, Vec<String>> {
    let mut out = BTreeMap::new();
    field_paths_of(&material_tree(ids), &mut Vec::new(), &mut out);
    out
}

/// Field path of every texture in `BakedTextures` (`@internal/default` → `["internal", "default"]`).
pub fn texture_field_paths<'a>(ids: impl Iterator<Item = &'a str>) -> BTreeMap<String, Vec<String>> {
    let mut out = BTreeMap::new();
    field_paths_of(&texture_tree(ids), &mut Vec::new(), &mut out);
    out
}

/// Raw folder path of a chain of folder modules (`["dir_env", "dir_loop"]` → `env/loop`).
fn folder_path(modules: &[&str]) -> String {
    modules.iter().map(|m| m.strip_prefix(DIR_PREFIX).unwrap_or(m)).collect::<Vec<_>>().join("/")
}

fn marker() -> String {
    format!("// AUTOGENERATED by shader-editor v{GENERATOR_VERSION} — do not edit\n")
}

/// Writes the fields of a nested struct and the modules of its folder structs (`pub mod dir_env { pub struct Group
/// {..} }`, recursively).
struct StructGen<'a> {
    /// Field type of an asset, from its id.
    leaf_ty: &'a dyn Fn(&str) -> String,
    derive: &'a str,
    /// `impl` block of a folder struct (`None`: no impl).
    group_impl: &'a dyn Fn(&Group) -> Option<String>,
    /// Doc line of a folder struct, from the module names leading to it (`["dir_env", "dir_rocks"]`).
    group_doc: &'a dyn Fn(&[&str]) -> String,
}

fn indent(text: &str, by: usize) -> String {
    let pad = " ".repeat(by);
    text.lines().map(|l| if l.is_empty() { "\n".to_string() } else { format!("{pad}{l}\n") }).collect()
}

impl StructGen<'_> {
    fn fields(&self, g: &Group) -> String {
        let mut o = String::new();
        for (name, f) in &g.fields {
            match f {
                Field::Leaf(id) => o.push_str(&format!("    pub {name}: {},\n", (self.leaf_ty)(id))),
                Field::Dir(module, _) => o.push_str(&format!("    pub {name}: {module}::Group,\n")),
            }
        }
        o
    }

    /// Modules of the folders of `g`; `path` holds the module names leading to `g` (empty at the root).
    fn modules(&self, g: &Group, path: &[&str]) -> String {
        let mut o = String::new();
        for f in g.fields.values() {
            let Field::Dir(module, sub) = f else { continue };
            let sub_path: Vec<&str> = path.iter().copied().chain([module.as_str()]).collect();
            o.push('\n');
            o.push_str(&format!("pub mod {module} {{\n"));
            let mut inner = String::new();
            if sub.fields.values().any(|f| matches!(f, Field::Leaf(_))) || (self.group_impl)(sub).is_some() {
                inner.push_str("use bevy::prelude::*;\n\n");
            }
            inner.push_str(&format!("/// {}\n", (self.group_doc)(&sub_path)));
            inner.push_str(&format!("#[derive({})]\n", self.derive));
            inner.push_str("pub struct Group {\n");
            inner.push_str(&self.fields(sub));
            inner.push_str("}\n");
            if let Some(i) = (self.group_impl)(sub) {
                inner.push('\n');
                inner.push_str(&i);
            }
            inner.push_str(&self.modules(sub, &sub_path));
            o.push_str(&indent(&inner, 4));
            o.push_str("}\n");
        }
        o
    }
}

/// `src/baked/materials.rs`: `BakedMaterials` with a reserved handle per material (`entries` are the materials of
/// the crate, including failed ones that keep their old module).
pub fn materials_rs(entries: &[MaterialEntry]) -> String {
    let types: BTreeMap<&str, String> = entries
        .iter()
        .map(|e| {
            let module = paths::material_components(&e.path).join("::");
            (e.path.as_str(), crate::module_path(&format!("materials::{module}::{}", e.material)))
        })
        .collect();
    let tree = material_tree(entries.iter().map(|e| e.path.as_str()));
    let reserve_body = |g: &Group| {
        let world = if g.fields.is_empty() { "_world" } else { "world" };
        let mut o = String::new();
        o.push_str(
            "    /// Reserves a handle in `Assets<M>` for every material (the loader inserts the materials there).\n",
        );
        o.push_str(&format!("    pub(crate) fn reserve({world}: &mut World) -> Self {{\n"));
        if g.fields.is_empty() {
            o.push_str("        Self {}\n");
        } else {
            o.push_str("        Self {\n");
            for (name, f) in &g.fields {
                match f {
                    Field::Leaf(id) => o.push_str(&format!(
                        "            {name}: world.resource::<Assets<{}>>().reserve_handle(),\n",
                        types[id.as_str()]
                    )),
                    Field::Dir(module, _) => {
                        o.push_str(&format!("            {name}: {module}::Group::reserve(world),\n"))
                    }
                }
            }
            o.push_str("        }\n");
        }
        o.push_str("    }\n");
        o
    };
    let leaf_ty = |id: &str| format!("Handle<{}>", types[id]);
    let group_impl = |g: &Group| Some(format!("impl Group {{\n{}}}\n", reserve_body(g)));
    let group_doc = |modules: &[&str]| format!("Materials of `{}/`.", folder_path(modules));
    let generator =
        StructGen { leaf_ty: &leaf_ty, derive: "Clone, Debug", group_impl: &group_impl, group_doc: &group_doc };

    let mut o = marker();
    o.push_str("//! [`BakedMaterials`]: a handle per generated material, nested by folder (design §3.1).\n\n");
    o.push_str("use std::collections::HashMap;\n\n");
    o.push_str("use bevy::prelude::*;\n\n");
    o.push_str("use super::MaterialValues;\n\n");
    o.push_str(
        "/// Every generated material, nested by folder (`baked.env.rock_wet`). The handles are reserved when the\n",
    );
    o.push_str("/// plugin is built; `BakedMaterialsLoader::load` inserts the materials behind them.\n");
    o.push_str("#[derive(Resource, Clone, Debug)]\n");
    o.push_str("pub struct BakedMaterials {\n");
    o.push_str(&generator.fields(&tree));
    o.push_str("    /// Every material by asset id (`env/rock_wet`: its path under `quillshade/material/` without\n");
    o.push_str(
        "    /// `.material.ron`), for game data that names materials; [`BakedMaterials::get`] gives the typed handle.\n",
    );
    o.push_str(&format!("    pub {BY_PATH_FIELD}: HashMap<String, UntypedHandle>,\n"));
    o.push_str("}\n");
    o.push('\n');
    o.push_str("impl BakedMaterials {\n");
    o.push_str("    /// Asset ids (`env/rock_wet`) of every material, as keyed in the manifest.\n");
    o.push_str(&ids_const("pub(crate)", entries.iter().map(|e| e.path.as_str())));
    o.push('\n');
    o.push_str(&root_reserve(&tree, entries));
    o.push('\n');
    o.push_str("    /// Material `id` (`env/rock_wet`) as `Handle<M>`, `M` being its asset type (`RockWetMaterial`); `None` for an\n");
    o.push_str("    /// unknown id or another type.\n");
    o.push_str("    pub fn get<M: Asset>(&self, id: &str) -> Option<Handle<M>> {\n");
    o.push_str(&format!("        self.{BY_PATH_FIELD}.get(id)?.clone().try_typed().ok()\n"));
    o.push_str("    }\n");
    o.push_str(&dispatch(entries));
    o.push_str("}\n");
    o.push_str(&generator.modules(&tree, &[]));
    o
}

/// `BakedMaterials::reserve`: the folder groups reserve their handles, then `by_path` lists them all.
fn root_reserve(tree: &Group, entries: &[MaterialEntry]) -> String {
    let paths = material_field_paths(entries.iter().map(|e| e.path.as_str()));
    let world = if tree.fields.is_empty() { "_world" } else { "world" };
    let mut o = String::new();
    o.push_str(
        "    /// Reserves a handle in `Assets<M>` for every material (the loader inserts the materials there).\n",
    );
    o.push_str(&format!("    pub(crate) fn reserve({world}: &mut World) -> Self {{\n"));
    // Without materials: `Self { by_path: HashMap::new() }` (no `let`, clippy's `let_and_return`).
    o.push_str(if paths.is_empty() { "        Self {\n" } else { "        let mut s = Self {\n" });
    for (name, f) in &tree.fields {
        match f {
            Field::Leaf(id) => o.push_str(&format!(
                "            {name}: world.resource::<Assets<{}>>().reserve_handle(),\n",
                material_type(entries, id)
            )),
            Field::Dir(module, _) => o.push_str(&format!("            {name}: {module}::Group::reserve(world),\n")),
        }
    }
    o.push_str(&format!("            {BY_PATH_FIELD}: HashMap::new(),\n"));
    o.push_str(if paths.is_empty() { "        }\n" } else { "        };\n" });
    if !paths.is_empty() {
        o.push_str(&format!("        s.{BY_PATH_FIELD} = HashMap::from([\n"));
        for (id, path) in &paths {
            o.push_str(&format!("            (\"{id}\".to_string(), s.{}.clone().untyped()),\n", path.join(".")));
        }
        o.push_str("        ]);\n");
        o.push_str("        s\n");
    }
    o.push_str("    }\n");
    o
}

/// Rust path of the material asset type of `id` (`crate::material::materials::dir_env::mat_rock_wet::RockWetMaterial`).
fn material_type(entries: &[MaterialEntry], id: &str) -> String {
    let e = entries.iter().find(|e| e.path == id).expect("material entry");
    crate::module_path(&format!("materials::{}::{}", paths::material_components(&e.path).join("::"), e.material))
}

/// Per-material functions of `BakedMaterials` (design §3.3): `init_assets` (asset registration without render
/// plugins), `insert` (build from manifest values at the reserved handle) and `unresolved` (tests).
fn dispatch(entries: &[MaterialEntry]) -> String {
    let paths = material_field_paths(entries.iter().map(|e| e.path.as_str()));
    let mut sorted: Vec<&MaterialEntry> = entries.iter().collect();
    sorted.sort_by(|a, b| a.path.cmp(&b.path));
    let module = |e: &MaterialEntry| {
        crate::module_path(&format!("materials::{}", paths::material_components(&e.path).join("::")))
    };
    let field = |e: &MaterialEntry| paths[&e.path].join(".");
    let empty = sorted.is_empty();
    let mut o = String::new();

    o.push('\n');
    o.push_str(
        "    /// Registers every material asset type that is not registered yet (headless: no material plugins).\n",
    );
    let app = if empty { "_app" } else { "app" };
    o.push_str(&format!("    pub(crate) fn init_assets({app}: &mut App) {{\n"));
    for e in &sorted {
        o.push_str(&format!("        super::init_asset::<{}::{}>(app);\n", module(e), e.material));
    }
    o.push_str("    }\n");

    o.push('\n');
    o.push_str("    /// Builds material `id` from `v` (images through `tex`) and inserts it at its reserved handle.\n");
    let (world, v, tex) = if empty { ("_world", "_v", "_tex") } else { ("world", "v", "tex") };
    o.push_str(&format!(
        "    pub(crate) fn insert(&self, {world}: &mut World, id: &str, {v}: &MaterialValues, {tex}: &dyn Fn(&str) -> Handle<Image>) {{\n"
    ));
    if empty {
        o.push_str("        warn!(\"baked materials: material `{id}` is not compiled into the game\");\n");
        o.push_str("    }\n");
    } else {
        o.push_str("        match id {\n");
    }
    for e in &sorted {
        let build = if e.main == e.material {
            format!("{}::{}::from_values(v, tex)", module(e), e.main)
        } else {
            format!("{}::{}::from_values(v, tex).material()", module(e), e.main)
        };
        o.push_str(&format!(
            "            \"{}\" => super::insert_material(world, &self.{}, {build}),\n",
            e.path,
            field(e)
        ));
    }
    if !empty {
        o.push_str("            _ => warn!(\"baked materials: material `{id}` is not compiled into the game\"),\n");
        o.push_str("        }\n");
        o.push_str("    }\n");
    }

    o.push('\n');
    o.push_str("    /// Ids of the materials whose handle has no asset yet.\n");
    o.push_str("    #[doc(hidden)]\n");
    let world = if empty { "_world" } else { "world" };
    o.push_str(&format!("    pub fn unresolved(&self, {world}: &World) -> Vec<&'static str> {{\n"));
    if empty {
        o.push_str("        Vec::new()\n");
    } else {
        o.push_str("        let mut out = Vec::new();\n");
        for e in &sorted {
            o.push_str(&format!("        if !super::has_material(world, &self.{}) {{\n", field(e)));
            o.push_str(&format!("            out.push(\"{}\");\n", e.path));
            o.push_str("        }\n");
        }
        o.push_str("        out\n");
    }
    o.push_str("    }\n");
    o
}

/// `{vis} const IDS: &[&str] = &[..];` (sorted, one per line).
fn ids_const<'a>(vis: &str, ids: impl Iterator<Item = &'a str>) -> String {
    let mut ids: Vec<&str> = ids.collect();
    ids.sort();
    if ids.is_empty() {
        return format!("    {vis} const IDS: &[&str] = &[];\n");
    }
    let mut o = format!("    {vis} const IDS: &[&str] = &[\n");
    for id in ids {
        o.push_str(&format!("        \"{id}\",\n"));
    }
    o.push_str("    ];\n");
    o
}

/// `src/baked/textures.rs`: `BakedTextures` with a handle per texture id: every Texture asset that parses, valid or
/// not (design §3.1: an asset error must not change the public shape of the crate), and the internal textures; lookup
/// by id. Ids without a manifest entry get the default texture at runtime.
pub fn textures_rs(ids: &[String]) -> String {
    let ids: Vec<&str> = ids.iter().map(String::as_str).collect();
    let tree = texture_tree(ids.iter().copied());
    let paths = texture_field_paths(ids.iter().copied());
    let leaf_ty = |_: &str| "Handle<Image>".to_string();
    let group_impl = |_: &Group| None;
    let group_doc = |modules: &[&str]| match modules.split_first() {
        Some((&INTERNAL_GROUP, [])) => format!("Internal textures (`{INTERNAL_PREFIX}*`)."),
        Some((&INTERNAL_GROUP, rest)) => format!("Internal textures (`{INTERNAL_PREFIX}{}/*`).", folder_path(rest)),
        Some((&GRADIENTS_GROUP, [])) => format!("Baked gradient collections (`{GRADIENT_PREFIX}*`)."),
        Some((&GRADIENTS_GROUP, rest)) => {
            let path = folder_path(rest);
            format!("Gradient collections of `{path}/` (`{GRADIENT_PREFIX}{path}/*`).")
        }
        _ => format!("Texture assets of `{}/`.", folder_path(modules)),
    };
    let generator = StructGen {
        leaf_ty: &leaf_ty,
        derive: "Clone, Debug, Default",
        group_impl: &group_impl,
        group_doc: &group_doc,
    };

    let mut o = marker();
    o.push_str("//! [`BakedTextures`]: an image handle per Texture asset, nested by folder (design §3.1), per internal texture\n");
    o.push_str("//! and per baked gradient collection.\n\n");
    o.push_str("use std::collections::HashMap;\n\n");
    o.push_str("use bevy::prelude::*;\n\n");
    o.push_str("/// The image of every Texture asset, nested by folder (`tex.textures.rock_albedo`), internal textures under\n");
    o.push_str(
        "/// `internal` (`tex.internal.default`), baked gradient collections under `gradients`, nested by path\n",
    );
    o.push_str("/// (`tex.gradients.common.terrain`). Default handles until `BakedMaterialsLoader::load` fills them; use these\n");
    o.push_str("/// instead of `AssetServer::load` on a file that has a Texture asset, so the asset's loader settings apply.\n");
    o.push_str("#[derive(Resource, Clone, Debug, Default)]\n");
    o.push_str("pub struct BakedTextures {\n");
    o.push_str(&generator.fields(&tree));
    o.push_str("    /// Every texture by id (`textures/rock_albedo`: its path under `quillshade/material/` without\n");
    o.push_str("    /// `.texture.ron`; `@internal/default`; `@gradients/common/terrain`: a baked gradient collection), for game\n");
    o.push_str("    /// data that names textures. Filled by `BakedMaterialsLoader::load` and kept in sync with the fields on hot\n");
    o.push_str("    /// reload; also holds a texture created in the editor after the build that a material uses (it has no field\n");
    o.push_str("    /// until a rebuild).\n");
    o.push_str(&format!("    pub {BY_PATH_FIELD}: HashMap<String, Handle<Image>>,\n"));
    o.push_str("}\n");
    o.push('\n');
    o.push_str("impl BakedTextures {\n");
    o.push_str("    /// Texture ids (`textures/rock_albedo`, `@internal/default`, `@gradients/common/terrain`), as keyed in the\n");
    o.push_str("    /// manifest.\n");
    o.push_str(&ids_const("pub(crate)", ids.iter().copied()));
    o.push('\n');
    o.push_str(
        "    /// Image of texture `id` (`textures/rock_albedo`); `None` before `load()` and for an unknown id.\n",
    );
    o.push_str("    pub fn get(&self, id: &str) -> Option<&Handle<Image>> {\n");
    o.push_str(&format!("        self.{BY_PATH_FIELD}.get(id)\n"));
    o.push_str("    }\n\n");
    o.push_str("    /// Sets the handle of texture `id`: its field (if any) and its `by_path` entry.\n");
    o.push_str("    pub(crate) fn set(&mut self, id: &str, h: Handle<Image>) {\n");
    o.push_str("        if let Some(slot) = self.slot(id) {\n");
    o.push_str("            *slot = h.clone();\n");
    o.push_str("        }\n");
    o.push_str(&format!("        self.{BY_PATH_FIELD}.insert(id.to_string(), h);\n"));
    o.push_str("    }\n\n");
    o.push_str("    /// Texture `id` left the manifest: a field falls back to `fallback`, a texture without one leaves `by_path`.\n");
    o.push_str("    pub(crate) fn unset(&mut self, id: &str, fallback: Handle<Image>) {\n");
    o.push_str("        if self.slot(id).is_some() {\n");
    o.push_str("            self.set(id, fallback);\n");
    o.push_str("        } else {\n");
    o.push_str(&format!("            self.{BY_PATH_FIELD}.remove(id);\n"));
    o.push_str("        }\n");
    o.push_str("    }\n\n");
    o.push_str("    /// The field of texture `id`.\n");
    o.push_str("    #[doc(hidden)]\n");
    o.push_str("    pub fn field(&self, id: &str) -> Option<&Handle<Image>> {\n");
    o.push_str("        match id {\n");
    for (id, path) in &paths {
        o.push_str(&format!("            \"{id}\" => Some(&self.{}),\n", path.join(".")));
    }
    o.push_str("            _ => None,\n");
    o.push_str("        }\n");
    o.push_str("    }\n\n");
    let me = if paths.is_empty() { "_id" } else { "id" };
    o.push_str(&format!("    fn slot(&mut self, {me}: &str) -> Option<&mut Handle<Image>> {{\n"));
    if paths.is_empty() {
        o.push_str("        None\n");
    } else {
        o.push_str("        match id {\n");
        for (id, path) in &paths {
            o.push_str(&format!("            \"{id}\" => Some(&mut self.{}),\n", path.join(".")));
        }
        o.push_str("            _ => None,\n");
        o.push_str("        }\n");
    }
    o.push_str("    }\n");
    o.push_str("}\n");
    o.push_str(&generator.modules(&tree, &[]));
    o
}

/// `src/baked/mod.rs`: static code (serde mirrors of the manifest, the manifest loader, …).
pub fn mod_rs() -> String {
    MOD_TEMPLATE.replace("{{GENERATOR_VERSION}}", &GENERATOR_VERSION.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::ManifestTexture;

    #[test]
    fn template_has_lf_line_endings() {
        // The template is matched and emitted as is: a CRLF checkout breaks both (see `.gitattributes`).
        assert!(!MOD_TEMPLATE.contains('\r'), "templates/baked_mod.rs checked out with CRLF: see .gitattributes");
    }

    #[test]
    fn field_names_keyword_and_clash() {
        let ids = ["kw/loop/type", "env/water", "env/water/x", "env/rock_wet", "top", "fx/hologram"];
        let f = material_field_paths(ids.iter().copied());
        let dotted = |id: &str| f[id].join(".");
        assert_eq!(dotted("kw/loop/type"), "kw.dir_loop.mat_type");
        assert_eq!(dotted("env/water"), "env.mat_water");
        assert_eq!(dotted("env/water/x"), "env.dir_water.x");
        assert_eq!(dotted("env/rock_wet"), "env.rock_wet");
        assert_eq!(dotted("top"), "top");
        assert_eq!(dotted("fx/hologram"), "fx.hologram");
        assert_eq!(f.len(), ids.len());

        // A prefixed name meeting a plain one: the plain one is prefixed too.
        let f = material_field_paths(["a/b/c", "a/b", "a/dir_b"].into_iter());
        assert_eq!(f["a/b/c"].join("."), "a.dir_b.c");
        assert_eq!(f["a/b"].join("."), "a.mat_b");
        assert_eq!(f["a/dir_b"].join("."), "a.mat_dir_b");

        // Textures: internal ones live in the `internal` group, which a project folder or texture never takes.
        let ids = ["@internal/default", "textures/env/rock_albedo", "internal/x", "internal", "fn"];
        let t = texture_field_paths(ids.iter().copied());
        assert_eq!(t["@internal/default"].join("."), "internal.default");
        assert_eq!(t["textures/env/rock_albedo"].join("."), "textures.env.rock_albedo");
        assert_eq!(t["internal/x"].join("."), "dir_internal.x");
        assert_eq!(t["internal"].join("."), "tex_internal");
        assert_eq!(t["fn"].join("."), "tex_fn");

        // Gradient collections live in the `gradients` group, nested by path, which a project folder or texture
        // never takes (gradient design §5).
        let ids = ["@gradients/common/terrain", "@gradients/sky", "gradients/x", "gradients"];
        let t = texture_field_paths(ids.iter().copied());
        assert_eq!(t["@gradients/common/terrain"].join("."), "gradients.common.terrain");
        assert_eq!(t["@gradients/sky"].join("."), "gradients.sky");
        assert_eq!(t["gradients/x"].join("."), "dir_gradients.x");
        assert_eq!(t["gradients"].join("."), "tex_gradients");
        let rs = textures_rs(&ids.map(String::from));
        assert!(rs.contains("    pub gradients: gradients::Group,\n"), "{rs}");
        assert!(rs.contains("    pub dir_gradients: dir_gradients::Group,\n"), "{rs}");
        assert!(rs.contains("\"@gradients/common/terrain\" => Some(&self.gradients.common.terrain)"), "{rs}");
        assert!(rs.contains("/// Gradient collections of `common/` (`@gradients/common/*`).\n"), "{rs}");
        assert!(rs.contains("/// Texture assets of `gradients/`.\n"), "{rs}");

        // `by_path` is the id → handle map at the root of both structs.
        let f = material_field_paths(["by_path/x", "by_path", "a/by_path"].into_iter());
        assert_eq!(f["by_path/x"].join("."), "dir_by_path.x");
        assert_eq!(f["by_path"].join("."), "mat_by_path");
        assert_eq!(f["a/by_path"].join("."), "a.by_path", "only the root reserves it");
        let t = texture_field_paths(["by_path"].into_iter());
        assert_eq!(t["by_path"].join("."), "tex_by_path");
    }

    /// A project without materials or textures still gets the `by_path` maps.
    #[test]
    fn empty_structs_have_by_path() {
        let m = materials_rs(&[]);
        assert!(m.contains("pub struct BakedMaterials {\n    /// Every material by asset id"), "{m}");
        assert!(m.contains("    pub(crate) fn reserve(_world: &mut World) -> Self {\n        Self {\n            by_path: HashMap::new(),\n        }\n    }\n"), "{m}");
        let t = textures_rs(&[]);
        assert!(t.contains("    pub by_path: HashMap<String, Handle<Image>>,\n}"), "{t}");
    }

    /// The serde mirrors in the template know every field and variant `manifest_ron` writes.
    #[test]
    fn template_mirrors_manifest_fields() {
        use std::collections::BTreeMap;

        use shader_graph_core::texture::{AddressMode, FilterMode, SamplerSettings, TextureSettings};

        use crate::manifest::{ManifestMaterial, ParamValue, manifest_ron};

        let settings = TextureSettings {
            is_srgb: false,
            expand_grayscale: false,
            sampler: Some(SamplerSettings {
                address_mode_u: AddressMode::Repeat,
                address_mode_v: AddressMode::Repeat,
                address_mode_w: AddressMode::Repeat,
                mag_filter: FilterMode::Linear,
                min_filter: FilterMode::Linear,
                mipmap_filter: FilterMode::Linear,
                lod_min_clamp: 1.0,
                lod_max_clamp: 2.0,
                anisotropy_clamp: 4,
            }),
        };
        let t = ManifestTexture { id: "a".into(), path: "a.png".into(), settings };
        let params = [
            ParamValue::Float(1.0),
            ParamValue::Vec2([1.0; 2]),
            ParamValue::Vec3([1.0; 3]),
            ParamValue::Vec4([1.0; 4]),
            ParamValue::Color([1.0; 4]),
            ParamValue::Bool(true),
            ParamValue::Int(1),
        ];
        let m = ManifestMaterial {
            path: "m".into(),
            params: params.iter().enumerate().map(|(i, v)| (format!("p{i}"), *v)).collect(),
            textures: BTreeMap::from([("t".to_string(), "a".to_string())]),
            layout: "SG_LAYOUT_0123abcd".into(),
            bindings: vec!["t".into()],
        };
        let text = manifest_ron(&[t], &[m]);
        let mut fields = 0;
        for line in text.lines().skip(1) {
            if let Some((key, _)) = line.trim().split_once(':')
                && !key.starts_with('"')
            {
                assert!(MOD_TEMPLATE.contains(&format!("    pub {key}: ")), "template has no field `{key}`");
                fields += 1;
            }
        }
        // manifest 4 (version, textures, materials, instances) + texture 2 + settings 3 + sampler 9 + material 4
        assert_eq!(fields, 4 + 2 + 3 + 9 + 4, "{text}");
        for v in params {
            let name = format!("{v:?}");
            let name = &name[..name.find('(').unwrap()];
            assert!(MOD_TEMPLATE.contains(&format!("    {name}(")), "template has no ParamValue::{name}");
        }
        let variants =
            AddressMode::ALL.iter().map(|m| format!("{m:?}")).chain(FilterMode::ALL.iter().map(|m| format!("{m:?}")));
        for v in variants {
            assert!(MOD_TEMPLATE.contains(&format!("    {v},\n")), "template has no variant {v}");
        }
        let default = format!("pub const DEFAULT_TEXTURE: &str = \"{}\";", shader_stdlib::internal::DEFAULT_TEXTURE);
        assert!(MOD_TEMPLATE.contains(&default), "{default}");
    }
}
