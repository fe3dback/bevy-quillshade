//! Baked materials manifest `assets/shaders/autogen/baked.ron` (design §3.2.1): every valid Texture asset (path +
//! loader settings) and every material's parameter defaults and texture choices. The generated crate embeds it as
//! its compiled snapshot (`COMPILED_MANIFEST`) and reloads it at runtime; its serde mirror types live in the template
//! of `baked/mod.rs` ([`crate::baked`]), so the RON shape here and there must agree.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use shader_graph_core::registry::ParamType;
use shader_graph_core::{GENERATOR_VERSION, Ir, TextureSettings, Value};

/// Format version of the manifest (`BakedManifest::version`).
pub const MANIFEST_VERSION: u32 = 1;

/// One Texture asset (or internal texture, or baked gradient collection) the game loads.
#[derive(Debug, Clone, PartialEq)]
pub struct ManifestTexture {
    /// Texture asset id (`textures/env/rock`), `@internal/<name>` or `@gradients/<collection>`.
    pub id: String,
    /// Image path relative to the game's `assets/`.
    pub path: String,
    pub settings: TextureSettings,
}

/// Values of one material.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ManifestMaterial {
    /// Material asset path (`env/rock_wet`).
    pub path: String,
    /// Uniform parameter (`inp_*`) → default value. Texture parameters are in [`ManifestMaterial::textures`].
    pub params: BTreeMap<String, ParamValue>,
    /// Texture binding ident (`inp_albedo`, `ft_n02`) → Texture asset id.
    pub textures: BTreeMap<String, String>,
    /// Layout feature of the material ([`Ir::layout_feature`], decision 0029); empty in an entry written without IR.
    pub layout: String,
    /// The keys of [`ManifestMaterial::textures`] in binding order. With the same `layout` as the game's compiled
    /// one, the game maps its bindings to these by position (another fixed texture node wired in hot-reloads).
    pub bindings: Vec<String>,
}

/// A parameter value; colors are linear RGBA (as stored in the uniform).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum ParamValue {
    Float(f32),
    Vec2([f32; 2]),
    Vec3([f32; 3]),
    Vec4([f32; 4]),
    Color([f32; 4]),
    Bool(bool),
    Int(i32),
}

impl ParamValue {
    /// Value of a parameter of type `ty` with the graph default `v` (same broadcast rule as `{Name}Params::default`).
    pub fn of(ty: ParamType, v: &Value) -> Option<ParamValue> {
        let f = v.floats();
        let at = |i: usize| if f.len() == 1 { f[0] } else { f.get(i).copied().unwrap_or(0.0) };
        Some(match ty {
            ParamType::Scalar => ParamValue::Float(at(0)),
            ParamType::Vec2 => ParamValue::Vec2([at(0), at(1)]),
            ParamType::Vec3 => ParamValue::Vec3([at(0), at(1), at(2)]),
            ParamType::Vec4 => ParamValue::Vec4([at(0), at(1), at(2), at(3)]),
            ParamType::Color => ParamValue::Color([at(0), at(1), at(2), at(3)]),
            ParamType::Bool => ParamValue::Bool(v.as_bool()),
            ParamType::Texture => return None,
        })
    }
}

impl ManifestMaterial {
    /// Values of a material at regen time: parameter defaults and the texture of every binding.
    pub fn from_ir(ir: &Ir) -> Self {
        let params = ir
            .params
            .iter()
            .filter_map(|p| Some((shader_graph_core::ident::input_ident(&p.name), ParamValue::of(p.ty, &p.default)?)))
            .collect();
        let textures = ir.textures.iter().map(|t| (t.ident.clone(), t.texture.clone())).collect();
        let bindings = ir.textures.iter().map(|t| t.ident.clone()).collect();
        ManifestMaterial { path: ir.path.clone(), params, textures, layout: ir.layout_feature(), bindings }
    }
}

#[derive(Serialize, Deserialize)]
#[serde(rename = "BakedManifest")]
struct File {
    version: u32,
    #[serde(default)]
    textures: BTreeMap<String, TextureDef>,
    #[serde(default)]
    materials: BTreeMap<String, MaterialValues>,
    /// Reserved for material instances; always empty.
    #[serde(default)]
    instances: BTreeMap<String, ron::Value>,
}

#[derive(Serialize, Deserialize)]
struct TextureDef {
    path: String,
    #[serde(default, skip_serializing_if = "TextureSettings::is_default")]
    settings: TextureSettings,
}

#[derive(Serialize, Deserialize)]
struct MaterialValues {
    #[serde(default)]
    params: BTreeMap<String, ParamValue>,
    #[serde(default)]
    textures: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    layout: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    bindings: Vec<String>,
}

/// The manifest text: a marker comment line, then `BakedManifest(..)` with entries sorted by id (deterministic).
pub fn manifest_ron(textures: &[ManifestTexture], materials: &[ManifestMaterial]) -> String {
    let file = File {
        version: MANIFEST_VERSION,
        textures: textures
            .iter()
            .map(|t| (t.id.clone(), TextureDef { path: t.path.clone(), settings: t.settings.clone() }))
            .collect(),
        materials: materials
            .iter()
            .map(|m| {
                let v = MaterialValues {
                    params: m.params.clone(),
                    textures: m.textures.clone(),
                    layout: m.layout.clone(),
                    bindings: m.bindings.clone(),
                };
                (m.path.clone(), v)
            })
            .collect(),
        instances: BTreeMap::new(),
    };
    // Struct names are left out (the generated mirror types have other names); only the top level is named, for
    // readers. RON accepts both forms.
    let cfg = ron::ser::PrettyConfig::new()
        .struct_names(false)
        .indentor("  ".to_string())
        .new_line("\n".to_string())
        .separate_tuple_members(false);
    let body = ron::ser::to_string_pretty(&file, cfg).expect("manifest serialization");
    format!(
        "// AUTOGENERATED by shader-editor v{GENERATOR_VERSION} — do not edit (baked materials manifest, design §3.2.1)\n\
         BakedManifest{body}\n"
    )
}

/// Reads a manifest written by [`manifest_ron`] (the previous one, to keep the entries of failed materials).
pub fn parse_manifest(text: &str) -> Result<(Vec<ManifestTexture>, Vec<ManifestMaterial>), String> {
    let file: File = ron::Options::default().from_str(text).map_err(|e| e.to_string())?;
    if file.version != MANIFEST_VERSION {
        return Err(format!("unsupported manifest version {} (expected {MANIFEST_VERSION})", file.version));
    }
    let textures =
        file.textures.into_iter().map(|(id, t)| ManifestTexture { id, path: t.path, settings: t.settings }).collect();
    let materials = file
        .materials
        .into_iter()
        .map(|(path, m)| ManifestMaterial {
            path,
            params: m.params,
            textures: m.textures,
            layout: m.layout,
            bindings: m.bindings,
        })
        .collect();
    Ok((textures, materials))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use shader_graph_core::texture::{AddressMode, FilterMode, SamplerSettings, TextureSettings};

    use super::*;

    fn textures() -> Vec<ManifestTexture> {
        let custom = TextureSettings {
            is_srgb: false,
            expand_grayscale: false,
            sampler: Some(SamplerSettings {
                address_mode_u: AddressMode::Repeat,
                address_mode_v: AddressMode::MirrorRepeat,
                address_mode_w: AddressMode::ClampToBorder,
                mag_filter: FilterMode::Linear,
                min_filter: FilterMode::Linear,
                mipmap_filter: FilterMode::Linear,
                lod_min_clamp: 0.5,
                lod_max_clamp: 8.0,
                anisotropy_clamp: 16,
            }),
        };
        vec![
            ManifestTexture {
                id: "@internal/default".into(),
                path: "shaders/internal/default.png".into(),
                settings: TextureSettings { sampler: Some(SamplerSettings::default()), ..Default::default() },
            },
            ManifestTexture { id: "textures/env/rock".into(), path: "textures/rock.ktx2".into(), settings: custom },
            ManifestTexture {
                id: "textures/plain".into(),
                path: "textures/plain.png".into(),
                settings: Default::default(),
            },
        ]
    }

    fn materials() -> Vec<ManifestMaterial> {
        let params = BTreeMap::from([
            ("inp_wetness".to_string(), ParamValue::Float(0.3)),
            ("inp_tint".to_string(), ParamValue::Color([0.6288158, 0.1, 0.7, 1.0])),
            ("inp_offset".to_string(), ParamValue::Vec2([1.0, -2.5])),
            ("inp_dir".to_string(), ParamValue::Vec3([0.0, 1.0, 0.0])),
            ("inp_mask".to_string(), ParamValue::Vec4([0.1, 0.2, 0.3, 0.4])),
            ("inp_on".to_string(), ParamValue::Bool(true)),
            ("inp_count".to_string(), ParamValue::Int(-3)),
        ]);
        let textures = BTreeMap::from([
            ("inp_albedo".to_string(), "textures/env/rock".to_string()),
            ("ft_n02".to_string(), "@internal/default".to_string()),
        ]);
        vec![
            ManifestMaterial {
                path: "env/rock_wet".into(),
                params,
                textures,
                layout: "SG_LAYOUT_0123abcd".into(),
                bindings: vec!["inp_albedo".into(), "ft_n02".into()],
            },
            ManifestMaterial { path: "kw/loop/type".into(), ..Default::default() },
        ]
    }

    #[test]
    fn manifest_round_trip() {
        let (t, m) = (textures(), materials());
        let text = manifest_ron(&t, &m);
        let first = text.lines().next().unwrap();
        assert!(first.starts_with("// AUTOGENERATED by shader-editor v") && first.contains("do not edit"), "{text}");
        assert!(text.contains("\nBakedManifest(\n"), "{text}");
        assert!(text.contains("version: 1,"), "{text}");
        assert!(text.contains("\"inp_wetness\": Float(0.3)"), "{text}");
        assert!(text.contains("\"inp_tint\": Color((0.6288158, 0.1, 0.7, 1.0))"), "{text}");
        assert!(text.contains("instances: {}"), "{text}");
        // Default settings are omitted, like in Texture asset files.
        assert!(!text.contains("textures/plain.png\",\n      settings"), "{text}");
        assert_eq!(parse_manifest(&text).unwrap(), (t.clone(), m.clone()));

        // Deterministic: the input order does not matter.
        let (mut rt, mut rm) = (t, m);
        rt.reverse();
        rm.reverse();
        assert_eq!(manifest_ron(&rt, &rm), text);

        assert!(parse_manifest("garbage").is_err());
        let newer = text.replace("version: 1,", "version: 2,");
        assert!(parse_manifest(&newer).unwrap_err().contains("version"));
    }

    #[test]
    fn from_ir_takes_defaults_and_bindings() {
        use shader_graph_core::ir::{TextureBinding, TextureSource};
        use shader_graph_core::registry::{MaterialParam, ParamType};
        use shader_graph_core::{AssetKind, Value};

        let param = |name: &str, ty, default| MaterialParam {
            name: name.into(),
            ty,
            default,
            group: String::new(),
            min: None,
            max: None,
            node: "n01".into(),
        };
        let params = vec![
            param("albedo", ParamType::Texture, Value::Str("textures/rock".into())),
            param("tint", ParamType::Color, Value::Color([0.5, 0.25, 1.0, 1.0])),
            param("wetness", ParamType::Scalar, Value::Float(0.3)),
            param("offset", ParamType::Vec2, Value::Float2([1.0, 2.0])),
            param("dir", ParamType::Vec3, Value::Float3([0.0, 1.0, 0.0])),
            param("mask", ParamType::Vec4, Value::Float4([0.1, 0.2, 0.3, 0.4])),
            param("on", ParamType::Bool, Value::Bool(true)),
        ];
        let textures = vec![
            TextureBinding {
                ident: "inp_albedo".into(),
                source: TextureSource::Param("albedo".into()),
                texture: "textures/rock".into(),
                binding: 101,
            },
            TextureBinding {
                ident: "ft_n02".into(),
                source: TextureSource::Fixed { node: "n02".into(), origin: None },
                texture: "@internal/default".into(),
                binding: 103,
            },
        ];
        let ir = shader_graph_core::Ir {
            path: "env/rock_wet".into(),
            name: "rock_wet".into(),
            kind: AssetKind::Function,
            main: Default::default(),
            vertex: None,
            uniform: shader_graph_core::ir::UniformLayout::compute(&params),
            params,
            textures,
            stdlib: Default::default(),
            functions: Default::default(),
            curves: Default::default(),
            custom_fns: Default::default(),
            function_sig: None,
            uses_normal: false,
            preview: None,
        };
        let m = ManifestMaterial::from_ir(&ir);
        assert_eq!(m.path, "env/rock_wet");
        assert_eq!(
            m.params,
            BTreeMap::from([
                ("inp_tint".to_string(), ParamValue::Color([0.5, 0.25, 1.0, 1.0])),
                ("inp_wetness".to_string(), ParamValue::Float(0.3)),
                ("inp_offset".to_string(), ParamValue::Vec2([1.0, 2.0])),
                ("inp_dir".to_string(), ParamValue::Vec3([0.0, 1.0, 0.0])),
                ("inp_mask".to_string(), ParamValue::Vec4([0.1, 0.2, 0.3, 0.4])),
                ("inp_on".to_string(), ParamValue::Bool(true)),
            ])
        );
        assert_eq!(
            m.textures,
            BTreeMap::from([
                ("inp_albedo".to_string(), "textures/rock".to_string()),
                ("ft_n02".to_string(), "@internal/default".to_string()),
            ])
        );
        assert_eq!(m.bindings, ["inp_albedo", "ft_n02"]);
        assert_eq!(m.layout, ir.layout_feature());
    }
}
