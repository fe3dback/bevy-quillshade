//! IR → Rust generator of the Material module `crates/quillshade/src/material/` (spec §11, design §4.2).

use std::collections::{BTreeMap, BTreeSet};

pub mod baked;
pub mod manifest;

use shader_codegen_wesl::{header, params_struct, paths};
use shader_graph_core::ident::{input_ident, to_pascal_case};
use shader_graph_core::registry::{Builtin, ParamType};
use shader_graph_core::value::rust_f32;
use shader_graph_core::{
    BlendMode, GENERATOR_VERSION, Ir, MaterialKind, MaterialParam, Shading, TextureSource, UniformTy, Value,
};

/// Module of the Material domain inside the umbrella crate (`crate::material`, `crates/quillshade/src/material/`).
pub const MODULE: &str = "material";

/// Rust path of an item inside the Material module (`crate::material::baked::MaterialValues`).
pub fn module_path(item: &str) -> String {
    format!("crate::{MODULE}::{item}")
}

/// Rust type names generated for a material.
pub struct TypeNames {
    /// Main type with `new()` (`RockWetExt` for Pbr3d, `FooMaterial` otherwise).
    pub main: String,
    /// Asset type added to `Assets<_>` (`RockWetMaterial`).
    pub material: String,
    pub params: String,
}

pub fn type_names(ir: &Ir) -> TypeNames {
    let base = to_pascal_case(&ir.name);
    let material = format!("{base}Material");
    let main = match ir.kind.material() {
        Some(MaterialKind::Pbr3d(_)) => format!("{base}Ext"),
        _ => material.clone(),
    };
    TypeNames { main, material, params: params_struct(&ir.name) }
}

/// Asset path of the generated WESL (`quillshade/material/autogen/dir_env/mat_rock_wet.wesl`).
pub fn shader_asset_path(ir: &Ir) -> String {
    paths::module_file(&paths::material_module(&ir.path))
}

fn rust_arg_type(t: ParamType) -> &'static str {
    match t {
        ParamType::Scalar => "f32",
        ParamType::Vec2 => "Vec2",
        ParamType::Vec3 => "Vec3",
        ParamType::Vec4 => "Vec4",
        ParamType::Color => "LinearRgba",
        ParamType::Bool => "bool",
        ParamType::Texture => "Handle<Image>",
    }
}

/// Expression storing argument `v` of type `t` into its uniform field.
fn to_uniform(t: ParamType, v: &str) -> String {
    match t {
        ParamType::Color => format!("{v}.to_vec4()"),
        ParamType::Bool => format!("u32::from({v})"),
        _ => v.to_string(),
    }
}

fn default_literal(ty: UniformTy, v: &Value) -> String {
    let f = v.floats();
    let at = |i: usize| rust_f32(if f.len() == 1 { f[0] } else { f.get(i).copied().unwrap_or(0.0) });
    match ty {
        UniformTy::Vec4 => format!("Vec4::new({}, {}, {}, {})", at(0), at(1), at(2), at(3)),
        UniformTy::Vec3 => format!("Vec3::new({}, {}, {})", at(0), at(1), at(2)),
        UniformTy::Vec2 => format!("Vec2::new({}, {})", at(0), at(1)),
        UniformTy::F32 => at(0),
        UniformTy::U32 => {
            if v.as_bool() {
                "1".into()
            } else {
                "0".into()
            }
        }
    }
}

fn alpha_mode_3d(blend: BlendMode) -> String {
    match blend {
        BlendMode::Opaque => "AlphaMode::Opaque".into(),
        BlendMode::Mask => format!("AlphaMode::Mask({})", rust_f32(shader_codegen_wesl::MASK_CUTOFF)),
        BlendMode::Blend => "AlphaMode::Blend".into(),
        BlendMode::Add => "AlphaMode::Add".into(),
        BlendMode::Multiply => "AlphaMode::Multiply".into(),
    }
}

fn alpha_mode_2d(blend: BlendMode) -> String {
    match blend {
        BlendMode::Opaque => "AlphaMode2d::Opaque".into(),
        BlendMode::Mask => format!("AlphaMode2d::Mask({})", rust_f32(shader_codegen_wesl::MASK_CUTOFF)),
        _ => "AlphaMode2d::Blend".into(),
    }
}

/// Doc line for a constructor argument (named as the argument).
fn param_doc(p: &MaterialParam) -> String {
    let mut s = format!("`{}`", input_ident(&p.name));
    if !p.group.is_empty() {
        s += &format!(" (group {})", p.group);
    }
    if let (Some(a), Some(b)) = (p.min, p.max) {
        s += &format!(", UI range {}..{}", rust_f32(a), rust_f32(b));
    }
    s
}

/// Generates `materials/dir_{sub}/mat_{name}.rs`. Fields, constructor arguments and setters of parameters are
/// named `inp_{name}` (`set_inp_{name}`), like in WESL (decision 0020).
pub fn material_module(ir: &Ir, source_file: &str) -> String {
    let kind = *ir.kind.material().expect("material IR");
    let names = type_names(ir);
    let has_uniform = !ir.uniform.is_empty();
    let builtins = ir.all_builtins();
    // `get_world_from_local(in.instance_index)` needs the instance index in VertexOutput.
    let needs_instance_index = builtins.contains(&Builtin::WorldFromLocal) && !kind.is_2d();
    let two_sided = match kind {
        MaterialKind::Pbr3d(s) => s.two_sided,
        MaterialKind::Unlit3d(s) => s.two_sided,
        MaterialKind::Sprite2d(_) => false,
    };
    let wpo = ir.vertex.is_some();
    let mut o = String::new();
    let line = |o: &mut String, s: &str| {
        o.push_str(s);
        o.push('\n');
    };

    line(&mut o, &header(source_file));
    // Imports.
    if has_uniform && ir.params.iter().any(|p| p.ty == ParamType::Color) {
        line(&mut o, "use bevy::color::ColorToComponents;");
    }
    line(&mut o, "use bevy::mesh::MeshVertexBufferLayoutRef;");
    match kind {
        MaterialKind::Pbr3d(_) => {
            line(
                &mut o,
                "use bevy::pbr::{ExtendedMaterial, MaterialExtension, MaterialExtensionKey, MaterialExtensionPipeline};",
            );
        }
        MaterialKind::Unlit3d(_) => {
            line(&mut o, "use bevy::pbr::{MaterialPipeline, MaterialPipelineKey};");
        }
        MaterialKind::Sprite2d(_) => {
            line(
                &mut o,
                "use bevy::sprite_render::{AlphaMode2d, Material2dKey, Material2dPipeline, MaterialExtension2d, SpriteMaterialPlugin};",
            );
        }
    }
    line(&mut o, "use bevy::prelude::*;");
    let mut rr = vec!["AsBindGroup"];
    if has_uniform {
        rr.push("ShaderType");
    }
    rr.extend(["RenderPipelineDescriptor", "SpecializedMeshPipelineError"]);
    line(&mut o, &format!("use bevy::render::render_resource::{{{}}};", rr.join(", ")));
    line(&mut o, "use bevy::shader::ShaderRef;");
    line(&mut o, "");
    line(&mut o, &format!("use {};", module_path("baked::MaterialValues")));
    line(&mut o, "");
    line(&mut o, "/// Asset path of the generated shader.");
    line(&mut o, &format!("pub const SHADER_PATH: &str = \"{}\";", shader_asset_path(ir)));
    line(&mut o, "");
    line(&mut o, "/// Id of this material in the baked manifest (`baked.ron`).");
    line(&mut o, &format!("pub const BAKED_ID: &str = \"{}\";", ir.path));
    line(&mut o, "");
    line(
        &mut o,
        "/// WESL feature of this material's binding layout, enabled in `specialize`: a hot-reloaded shader whose",
    );
    line(
        &mut o,
        "/// bindings no longer match this code fails to compile instead of crashing the renderer (rebuild the game).",
    );
    line(&mut o, &format!("pub const LAYOUT_FEATURE: &str = \"{}\";", ir.layout_feature()));
    line(&mut o, "");

    // Params struct.
    if has_uniform {
        line(
            &mut o,
            &format!(
                "/// Uniform block at `@binding(100)`; layout matches `{}` in WESL ({} bytes).",
                names.params, ir.uniform.size
            ),
        );
        line(&mut o, "#[derive(ShaderType, Reflect, Debug, Clone, Copy, PartialEq)]");
        line(&mut o, &format!("pub struct {} {{", names.params));
        for f in &ir.uniform.fields {
            line(&mut o, &format!("    pub {}: {},", f.name, f.ty.rust()));
        }
        line(&mut o, "}");
        line(&mut o, "");
        line(&mut o, &format!("impl Default for {} {{", names.params));
        line(&mut o, "    /// Default values from the graph.");
        line(&mut o, "    fn default() -> Self {");
        line(&mut o, "        Self {");
        for f in &ir.uniform.fields {
            let v = match f.param {
                Some(i) => default_literal(f.ty, &ir.params[i].default),
                None => "0.0".into(),
            };
            line(&mut o, &format!("            {}: {v},", f.name));
        }
        line(&mut o, "        }");
        line(&mut o, "    }");
        line(&mut o, "}");
        line(&mut o, "");
    }

    // Material / extension struct.
    let doc = match kind {
        MaterialKind::Pbr3d(_) => {
            format!("Extension of `StandardMaterial`; use [`{}`] as the asset type.", names.material)
        }
        MaterialKind::Unlit3d(_) => "Unlit 3D material (`MeshMaterial3d`).".to_string(),
        MaterialKind::Sprite2d(_) => "Sprite material: add `SpriteMaterial(handle)` next to `Sprite`.".to_string(),
    };
    line(&mut o, &format!("/// {doc}"));
    if !ir.scene.is_empty() {
        // View textures are bound only for cameras with their prepass (Scene category); without it the reads fall
        // back silently, so the requirement is spelled out where the game picks the material up.
        let components: Vec<String> = ir.scene.iter().map(|t| format!("`{}`", t.camera_component())).collect();
        line(&mut o, "///");
        line(
            &mut o,
            &format!(
                "/// Reads scene textures: add {} to the cameras that render it (without, they read as empty).",
                components.join(", ")
            ),
        );
    }
    let no_fields = !has_uniform && ir.textures.is_empty();
    // Without parameters every field has a default (fixed textures: `Handle::default()`), and `new()` takes no
    // arguments: clippy's `new_without_default` wants `Default`.
    if ir.params.is_empty() {
        line(&mut o, "#[derive(Asset, AsBindGroup, Reflect, Debug, Clone, Default)]");
    } else {
        line(&mut o, "#[derive(Asset, AsBindGroup, Reflect, Debug, Clone)]");
    }
    line(&mut o, &format!("pub struct {} {{", names.main));
    if has_uniform {
        line(&mut o, "    #[uniform(100)]");
        line(&mut o, &format!("    pub params: {},", names.params));
    }
    for t in &ir.textures {
        // The texture id is not written here: swapping a node's texture keeps the Rust code (design §1.3); a
        // collection id is only in the doc, so swapping collections changes the code only with the bindings
        // (gradient design §7).
        let of = match &t.source {
            TextureSource::Param(_) => None,
            TextureSource::Fixed { node, origin: Some(f) } => {
                Some(format!("Fixed texture of node `{node}` of function `{f}`"))
            }
            TextureSource::Fixed { node, origin: None } => Some(format!("Fixed texture of node `{node}`")),
            TextureSource::Gradient(c) => Some(format!("Gradient collection `{c}`")),
        };
        if let Some(of) = of {
            line(&mut o, &format!("    /// {of}; `new` sets `Handle::default()` (Bevy's white image)."));
        }
        line(&mut o, &format!("    #[texture({})]", t.binding));
        line(&mut o, &format!("    #[sampler({})]", t.binding + 1));
        line(&mut o, &format!("    pub {}: Handle<Image>,", t.ident));
    }
    line(&mut o, "}");
    line(&mut o, "");

    // Trait impl.
    let mut defs = vec!["LAYOUT_FEATURE"];
    if needs_instance_index {
        defs.push("\"VERTEX_OUTPUT_INSTANCE_INDEX\"");
    }
    let push_defs = |o: &mut String| {
        for d in &defs {
            o.push_str(&format!("        descriptor.vertex.shader_defs.push({d}.into());\n"));
        }
        o.push_str("        if let Some(fragment) = descriptor.fragment.as_mut() {\n");
        for d in &defs {
            o.push_str(&format!("            fragment.shader_defs.push({d}.into());\n"));
        }
        o.push_str("        }\n");
    };
    match kind {
        MaterialKind::Pbr3d(_) => {
            line(&mut o, &format!("impl MaterialExtension for {} {{", names.main));
            line(&mut o, "    fn fragment_shader() -> ShaderRef {");
            line(&mut o, "        SHADER_PATH.into()");
            line(&mut o, "    }");
            line(&mut o, "");
            line(&mut o, "    fn deferred_fragment_shader() -> ShaderRef {");
            line(&mut o, "        SHADER_PATH.into()");
            line(&mut o, "    }");
            if wpo {
                line(&mut o, "");
                line(&mut o, "    fn vertex_shader() -> ShaderRef {");
                line(&mut o, "        SHADER_PATH.into()");
                line(&mut o, "    }");
            }
            line(&mut o, "");
            line(&mut o, "    fn specialize(");
            line(&mut o, "        _pipeline: &MaterialExtensionPipeline,");
            line(&mut o, "        descriptor: &mut RenderPipelineDescriptor,");
            line(&mut o, "        _layout: &MeshVertexBufferLayoutRef,");
            line(&mut o, "        _key: MaterialExtensionKey<Self>,");
            line(&mut o, "    ) -> Result<(), SpecializedMeshPipelineError> {");
            push_defs(&mut o);
            line(&mut o, "        Ok(())");
            line(&mut o, "    }");
            line(&mut o, "}");
            line(&mut o, "");
            line(&mut o, &format!("/// Asset type: `StandardMaterial` extended by [`{}`].", names.main));
            line(&mut o, &format!("pub type {} = ExtendedMaterial<StandardMaterial, {}>;", names.material, names.main));
            line(&mut o, "");
        }
        MaterialKind::Unlit3d(s) => {
            line(&mut o, &format!("impl Material for {} {{", names.main));
            line(&mut o, "    fn fragment_shader() -> ShaderRef {");
            line(&mut o, "        SHADER_PATH.into()");
            line(&mut o, "    }");
            if wpo {
                line(&mut o, "");
                line(&mut o, "    fn vertex_shader() -> ShaderRef {");
                line(&mut o, "        SHADER_PATH.into()");
                line(&mut o, "    }");
            }
            line(&mut o, "");
            line(&mut o, "    fn alpha_mode(&self) -> AlphaMode {");
            line(&mut o, &format!("        {}", alpha_mode_3d(s.blend)));
            line(&mut o, "    }");
            line(&mut o, "");
            line(&mut o, "    fn specialize(");
            line(&mut o, "        _pipeline: &MaterialPipeline,");
            line(&mut o, "        descriptor: &mut RenderPipelineDescriptor,");
            line(&mut o, "        _layout: &MeshVertexBufferLayoutRef,");
            line(&mut o, "        _key: MaterialPipelineKey<Self>,");
            line(&mut o, "    ) -> Result<(), SpecializedMeshPipelineError> {");
            if two_sided {
                line(&mut o, "        descriptor.primitive.cull_mode = None;");
            }
            push_defs(&mut o);
            line(&mut o, "        Ok(())");
            line(&mut o, "    }");
            line(&mut o, "}");
            line(&mut o, "");
        }
        MaterialKind::Sprite2d(s) => {
            line(&mut o, &format!("impl MaterialExtension2d for {} {{", names.main));
            line(&mut o, "    fn fragment_shader() -> Option<ShaderRef> {");
            line(&mut o, "        Some(SHADER_PATH.into())");
            line(&mut o, "    }");
            line(&mut o, "");
            line(&mut o, "    fn alpha_mode(&self) -> Option<AlphaMode2d> {");
            line(&mut o, &format!("        Some({})", alpha_mode_2d(s.blend)));
            line(&mut o, "    }");
            line(&mut o, "");
            line(&mut o, "    fn specialize(");
            line(&mut o, "        _pipeline: &Material2dPipeline,");
            line(&mut o, "        descriptor: &mut RenderPipelineDescriptor,");
            line(&mut o, "        _layout: &MeshVertexBufferLayoutRef,");
            line(&mut o, "        _key: Material2dKey<Self>,");
            line(&mut o, "    ) -> Result<(), SpecializedMeshPipelineError> {");
            push_defs(&mut o);
            line(&mut o, "        Ok(())");
            line(&mut o, "    }");
            line(&mut o, "}");
            line(&mut o, "");
        }
    }

    // Constructor, setters: crate-internal since baked materials (design §3.1: games take materials from
    // `BakedMaterials`); nothing in the crate calls them, hence `allow(dead_code)`.
    line(&mut o, &format!("impl {} {{", names.main));
    line(&mut o, "    /// All graph parameters are required, sorted by name:");
    for p in &ir.params {
        line(&mut o, &format!("    /// - {}", param_doc(p)));
    }
    if ir.params.len() > 7 {
        line(&mut o, "    #[allow(clippy::too_many_arguments)]");
    }
    line(&mut o, "    #[allow(dead_code)]");
    let args: Vec<String> =
        ir.params.iter().map(|p| format!("{}: {}", input_ident(&p.name), rust_arg_type(p.ty))).collect();
    line(&mut o, &format!("    pub(crate) fn new({}) -> Self {{", args.join(", ")));
    if no_fields {
        line(&mut o, "        Self {}");
    } else {
        line(&mut o, "        Self {");
        if has_uniform {
            line(&mut o, &format!("            params: {} {{", names.params));
            for f in &ir.uniform.fields {
                let v = match f.param {
                    Some(i) => {
                        let p = &ir.params[i];
                        let arg = input_ident(&p.name);
                        let conv = to_uniform(p.ty, &arg);
                        if conv == arg { arg } else { format!("{}: {conv}", f.name) }
                    }
                    None => format!("{}: 0.0", f.name),
                };
                line(&mut o, &format!("                {v},"));
            }
            line(&mut o, "            },");
        }
        for t in &ir.textures {
            match t.source {
                TextureSource::Param(_) => line(&mut o, &format!("            {},", t.ident)),
                TextureSource::Fixed { .. } | TextureSource::Gradient(_) => {
                    line(&mut o, &format!("            {}: Handle::default(),", t.ident))
                }
            }
        }
        line(&mut o, "        }");
    }
    line(&mut o, "    }");
    from_values(&mut o, ir, &names.params);
    for p in &ir.params {
        let field = input_ident(&p.name);
        line(&mut o, "");
        line(&mut o, "    #[allow(dead_code)]");
        line(&mut o, &format!("    pub(crate) fn set_{field}(&mut self, v: {}) {{", rust_arg_type(p.ty)));
        if p.ty == ParamType::Texture {
            line(&mut o, &format!("        self.{field} = v;"));
        } else {
            line(&mut o, &format!("        self.params.{field} = {};", to_uniform(p.ty, "v")));
        }
        line(&mut o, "    }");
    }
    if let MaterialKind::Pbr3d(s) = kind {
        line(&mut o, "");
        line(&mut o, "    /// Full material: `StandardMaterial` defaults with blend/cull settings from the graph.");
        line(&mut o, &format!("    pub fn material(self) -> {} {{", names.material));
        line(&mut o, "        ExtendedMaterial {");
        line(&mut o, "            base: StandardMaterial {");
        line(&mut o, &format!("                alpha_mode: {},", alpha_mode_3d(s.blend)));
        if s.two_sided {
            line(&mut o, "                double_sided: true,");
            line(&mut o, "                cull_mode: None,");
        }
        if s.shading == Shading::Unlit {
            line(&mut o, "                unlit: true,");
        }
        line(&mut o, "                ..default()");
        line(&mut o, "            },");
        line(&mut o, "            extension: self,");
        line(&mut o, "        }");
        line(&mut o, "    }");
    }
    line(&mut o, "}");
    line(&mut o, "");

    // Registration.
    line(&mut o, "pub(crate) fn register(app: &mut App) {");
    match kind {
        MaterialKind::Pbr3d(_) | MaterialKind::Unlit3d(_) => {
            line(&mut o, &format!("    app.add_plugins(MaterialPlugin::<{}>::default());", names.material))
        }
        MaterialKind::Sprite2d(_) => {
            line(&mut o, &format!("    app.add_plugins(SpriteMaterialPlugin::<{}>::default());", names.material))
        }
    }
    line(&mut o, "}");

    if has_uniform {
        line(&mut o, "");
        line(&mut o, "#[cfg(test)]");
        line(&mut o, "mod tests {");
        line(&mut o, "    use super::*;");
        line(&mut o, "");
        line(&mut o, "    #[test]");
        line(&mut o, "    fn uniform_layout_matches_wesl() {");
        line(&mut o, &format!("        assert_eq!({}::min_size().get(), {});", names.params, ir.uniform.size));
        line(&mut o, "    }");
        line(&mut o, "}");
    }
    o
}

/// `from_values` of a material module (design §3.2): values from the manifest (missing or mistyped → the compiled
/// default, `{Name}Params::default()`, with a warning), images through `tex`. Called by `BakedMaterials::insert`.
fn from_values(o: &mut String, ir: &Ir, params: &str) {
    let has_uniform = !ir.uniform.is_empty();
    let (v, tex) = (
        if has_uniform || !ir.textures.is_empty() { "v" } else { "_v" },
        if ir.textures.is_empty() { "_tex" } else { "tex" },
    );
    o.push('\n');
    o.push_str(
        "    /// Material from baked values: `v` from the manifest (a missing or mistyped value keeps the compiled\n",
    );
    o.push_str("    /// default, with a warning), `tex` maps a Texture asset id to its image.\n");
    o.push_str(&format!(
        "    pub(crate) fn from_values({v}: &MaterialValues, {tex}: &dyn Fn(&str) -> Handle<Image>) -> Self {{\n"
    ));
    if !has_uniform && ir.textures.is_empty() {
        o.push_str("        Self {}\n");
        o.push_str("    }\n");
        return;
    }
    if has_uniform {
        o.push_str(&format!("        let d = {params}::default();\n"));
    }
    o.push_str("        Self {\n");
    if has_uniform {
        o.push_str(&format!("            params: {params} {{\n"));
        for f in &ir.uniform.fields {
            let value = match f.param.map(|i| ir.params[i].ty) {
                None => "0.0".to_string(),
                Some(ty) => {
                    let (get, map) = match ty {
                        ParamType::Scalar => ("float", ""),
                        ParamType::Vec2 => ("vec2", ""),
                        ParamType::Vec3 => ("vec3", ""),
                        ParamType::Vec4 => ("vec4", ""),
                        ParamType::Color => ("color", ""),
                        ParamType::Bool => ("boolean", ".map(u32::from)"),
                        ParamType::Texture => unreachable!("texture parameters are not uniform fields"),
                    };
                    format!("v.{get}(BAKED_ID, \"{}\"){map}.unwrap_or(d.{})", f.name, f.name)
                }
            };
            o.push_str(&format!("                {}: {value},\n", f.name));
        }
        o.push_str("            },\n");
    }
    for t in &ir.textures {
        o.push_str(&format!("            {}: tex(v.texture(BAKED_ID, \"{}\")),\n", t.ident, t.ident));
    }
    o.push_str("        }\n");
    o.push_str("    }\n");
}

/// One generated material for crate-level files.
pub struct MaterialEntry {
    /// Asset path (`env/rock_wet`).
    pub path: String,
    pub main: String,
    pub material: String,
    pub params: Option<String>,
}

impl MaterialEntry {
    pub fn from_ir(ir: &Ir) -> Self {
        let n = type_names(ir);
        MaterialEntry {
            path: ir.path.clone(),
            main: n.main.clone(),
            material: n.material.clone(),
            params: (!ir.uniform.is_empty()).then_some(n.params),
        }
    }
}

/// All files of the Material module: path relative to the module directory (`crates/quillshade/src/material`) →
/// content. Includes `mod.rs`, every `materials/**/mod.rs`, the given material modules (keyed by asset path) and
/// `baked/` (design §3.1: `BakedMaterials` over `entries`, `BakedTextures` over `texture_ids`: every Texture asset
/// that parses, not only the manifest's valid ones, plus the internal textures). Modules are named by
/// `paths::material_components` (`dir_env::mat_rock_wet`, decision 0020). The umbrella crate's `Cargo.toml` and
/// `src/lib.rs` come from `project_core::umbrella`.
pub fn crate_files(
    entries: &[MaterialEntry],
    modules: &BTreeMap<String, String>,
    texture_ids: &[String],
) -> BTreeMap<String, String> {
    let mut files = BTreeMap::new();

    // Directory tree: dir path ("" = materials root) → (child dirs, leaf modules).
    let mut dirs: BTreeMap<String, (BTreeSet<String>, BTreeSet<String>)> = BTreeMap::new();
    dirs.entry(String::new()).or_default();
    // Whether `materials` re-exports anything: a glob re-export of nothing is `unused_imports` in the game.
    let mut root_reexports = false;
    for e in entries {
        let parts = paths::material_components(&e.path);
        let (name, sub) = parts.split_last().expect("non-empty path");
        let mut cur = String::new();
        for s in sub {
            let parent = cur.clone();
            cur = if cur.is_empty() { s.clone() } else { format!("{cur}/{s}") };
            dirs.entry(parent).or_default().0.insert(s.clone());
            dirs.entry(cur.clone()).or_default();
        }
        dirs.entry(cur).or_default().1.insert(name.clone());
    }
    for (dir, (children, leaves)) in &dirs {
        let mut o = String::new();
        o.push_str("// AUTOGENERATED by shader-editor v");
        o.push_str(&GENERATOR_VERSION.to_string());
        o.push_str(" — do not edit\n");
        for c in children.iter().chain(leaves) {
            o.push_str(&format!("pub mod {c};\n"));
        }
        if children.is_empty() && leaves.is_empty() {
            o.push_str("\npub(crate) fn register(_app: &mut bevy::prelude::App) {}\n");
        } else {
            o.push_str("\npub(crate) fn register(app: &mut bevy::prelude::App) {\n");
            for c in children.iter().chain(leaves) {
                o.push_str(&format!("    {c}::register(app);\n"));
            }
            o.push_str("}\n");
        }
        // Root re-exports of unique type names.
        if dir.is_empty() {
            let mut count: BTreeMap<&str, usize> = BTreeMap::new();
            for e in entries {
                *count.entry(&e.main).or_default() += 1;
            }
            let mut uses = Vec::new();
            for e in entries {
                if count[e.main.as_str()] > 1 {
                    continue;
                }
                let module = paths::material_components(&e.path).join("::");
                let mut items = vec![e.main.clone()];
                if e.material != e.main {
                    items.push(e.material.clone());
                }
                if let Some(p) = &e.params {
                    items.push(p.clone());
                }
                if items.len() == 1 {
                    uses.push(format!("pub use {module}::{};", items[0]));
                } else {
                    uses.push(format!("pub use {module}::{{{}}};", items.join(", ")));
                }
            }
            root_reexports = !uses.is_empty();
            if !uses.is_empty() {
                o.push('\n');
                for u in uses {
                    o.push_str(&u);
                    o.push('\n');
                }
            }
        }
        let path = if dir.is_empty() { "materials/mod.rs".to_string() } else { format!("materials/{dir}/mod.rs") };
        files.insert(path, o);
    }
    for (path, text) in modules {
        files.insert(format!("materials/{}.rs", paths::material_components(path).join("/")), text.clone());
    }
    files.insert("mod.rs".to_string(), mod_rs(root_reexports));
    files.insert("baked/mod.rs".to_string(), baked::mod_rs());
    files.insert("baked/materials.rs".to_string(), baked::materials_rs(entries));
    files.insert("baked/textures.rs".to_string(), baked::textures_rs(texture_ids));
    files
}

/// `include_str!` path of the baked manifest from the Material module's `mod.rs`: up through the module's
/// directories, `src/` and the crate directory to the game root, then down to `assets/quillshade/material/autogen/`.
fn compiled_manifest_path() -> String {
    let up = project_core::layout::CRATE_DIR.split('/').count() + 1 + MODULE.split("::").count();
    format!("{}assets/{}/baked.ron", "../".repeat(up), paths::AUTOGEN.join("/"))
}

fn mod_rs(materials_reexports: bool) -> String {
    let materials_glob = if materials_reexports { "pub use materials::*;\n" } else { "" };
    let manifest = compiled_manifest_path();
    format!(
        r#"// AUTOGENERATED by shader-editor v{GENERATOR_VERSION} — do not edit
//! Materials generated by shader-editor. Add [`crate::QuillshadePlugin`] to the app, call
//! [`BakedMaterialsLoader::load`] and take the materials from [`BakedMaterials`] (`baked.env.rock_wet`), images of
//! Texture assets from [`BakedTextures`].

use bevy::prelude::*;

pub mod baked;
pub mod materials;

pub use baked::{{
    BakedMaterials, BakedMaterialsLoader, BakedMaterialsProgress, BakedMaterialsReady, BakedPlugin, BakedTextures,
}};
{materials_glob}
/// The baked manifest at build time (design §3.2): the compiled snapshot of every Texture asset and material value.
pub const COMPILED_MANIFEST: &str = include_str!("{manifest}");

/// Registers every generated material and the baked materials loader ([`BakedPlugin`]); added by
/// [`crate::QuillshadePlugin`] while the Material domain is enabled.
pub struct QuillshadeMaterialInternalPlugin;

impl Plugin for QuillshadeMaterialInternalPlugin {{
    fn build(&self, app: &mut App) {{
        materials::register(app);
        app.add_plugins(BakedPlugin);
    }}
}}
"#
    )
}
