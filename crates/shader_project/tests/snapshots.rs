//! Snapshot suite (spec §15): one material per stdlib node plus the composite reference graphs.
//! Every generated `.wesl` is compiled with wesl + naga against the real Bevy 0.20 sources
//! (forward, forward without UV/tangents, forward with prepasses + MSAA, deferred = `PREPASS_PIPELINE`), and the
//! WESL/Rust output is compared byte for byte with `tests/snapshots/`.
//!
//! Update snapshots after an intended codegen change: `INSTA_UPDATE=always cargo test -p shader_project --test snapshots`.

use std::collections::BTreeMap;

use shader_codegen_wesl::check::BevySources;
use shader_graph_core::consts::NoConsts;
use shader_graph_core::model::{AssetKind, MaterialKind, SpriteSettings, UnlitSettings};
use shader_graph_core::registry::{
    FUNCTION_CALL_KIND, NoCurves, NoFunctions, NoGradients, NoTextures, NodeRole, SigCtx,
};
use shader_graph_core::{Asset, Env, PinType, Value, conversion};
use shader_project::fixtures::{self, GraphBuilder};
use shader_project::regen::{rust_file, wesl_file};
use shader_project::{AssetId, AssetType, MaterialLayout, Project, RegenOptions, ops, regenerate};

/// Material exercising one node: required textures fed, first output routed into the color.
fn node_material(kind: &str) -> Option<(String, Asset)> {
    let reg = shader_stdlib::registry();
    let def = reg.get(kind)?;
    // Sample Curve needs a curve asset, Global* nodes a constant table: covered by the `fx/curve_bands` and
    // `fx/global_tint` fixtures.
    if kind == FUNCTION_CALL_KIND
        || kind == "stdlib::SampleCurve"
        || kind.starts_with("stdlib::Global")
        || !matches!(def.role(), NodeRole::Normal | NodeRole::CompileIf | NodeRole::Parameter | NodeRole::Reroute)
    {
        return None;
    }
    let sprite = kind == "stdlib::SpriteTexture";
    let mat = if sprite {
        MaterialKind::Sprite2d(SpriteSettings::default())
    } else {
        MaterialKind::Unlit3d(UnlitSettings::default())
    };
    let asset_kind = AssetKind::Material(mat);
    let name = format!("node_{}", shader_graph_core::ident::to_snake_case(&def.info().name));
    let mut g = GraphBuilder::new(&name, asset_kind);
    let out = g.node(mat.output_node_kind(), (600.0, 0.0));
    let n = g.node(kind, (200.0, 0.0));

    let params = reg.resolved_params(kind, &BTreeMap::new());
    let (connected, inputs) = (BTreeMap::new(), BTreeMap::new());
    let env = Env {
        asset_kind,
        asset_path: &name,
        functions: &NoFunctions,
        curves: &NoCurves,
        consts: &NoConsts,
        textures: &NoTextures,
        gradients: &NoGradients,
        compile_overrides: &BTreeMap::new(),
    };
    let sig = def.signature(&SigCtx {
        node_id: &n,
        kind,
        params: &params,
        connected: &connected,
        input_values: &inputs,
        env: &env,
    });
    // Only required texture pins: an optional one (TextureSample `tex`) leaves the node on its `texture` property, a
    // fixed binding of the default texture (W110).
    for p in sig.inputs.iter().filter(|p| p.ty == PinType::Texture2D && p.required) {
        let t = g.node_p("param::TextureObject", (0.0, 0.0), &[("name", Value::Str(format!("tex_{}", p.id)))]);
        g.link((&t, "out"), (&n, &p.id));
    }
    let first = sig.outputs.first()?;
    let color = "color";
    let target = if sprite { PinType::Float4 } else { PinType::Float3 };
    match first.ty {
        PinType::Texture2D => {
            let s = g.node("stdlib::TextureSample", (400.0, 0.0));
            g.link((&n, &first.id), (&s, "tex"));
            g.link((&s, "rgb"), (&out, color));
        }
        PinType::RuntimeBool => {
            let s = g.node("stdlib::RuntimeIf", (400.0, 0.0));
            g.link((&n, &first.id), (&s, "cond"));
            g.link((&s, "out"), (&out, color));
        }
        PinType::CompileBool => {
            let s = g.node("stdlib::If", (400.0, 0.0));
            g.link((&n, &first.id), (&s, "cond"));
            g.link((&s, "out"), (&out, color));
        }
        PinType::MaterialAttributesPbr => {
            let s = g.node("stdlib::BreakMaterialAttributesPbr", (400.0, 0.0));
            g.link((&n, &first.id), (&s, "attributes"));
            g.link((&s, "base_color"), (&out, color));
        }
        t if conversion(t, target).is_some() => g.link((&n, &first.id), (&out, color)),
        _ => {
            let s = g.node("stdlib::AppendVector", (400.0, 0.0));
            g.link((&n, &first.id), (&s, "a"));
            g.link((&s, "out"), (&out, color));
        }
    }
    Some((name, g.finish()))
}

fn project() -> (tempfile::TempDir, Project) {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("Cargo.toml"), "[workspace]\nmembers = [\"crates/quillshade\"]\n").unwrap();
    std::fs::create_dir_all(dir.path().join("assets")).unwrap();
    let p = Project::open(dir.path()).unwrap();
    (dir, p)
}

fn snapshot(name: &str, text: &str) {
    insta::with_settings!({ snapshot_path => "snapshots", prepend_module_to_snapshot => false, omit_expression => true }, {
        insta::assert_snapshot!(name.to_string(), text);
    });
}

#[test]
fn every_stdlib_node_generates_and_compiles() {
    let reg = shader_stdlib::registry();
    let (_dir, p) = project();
    let mut ids = Vec::new();
    for def in reg.defs() {
        if let Some((name, asset)) = node_material(&def.info().id) {
            let id = AssetId::new(AssetType::Material, format!("nodes/{name}"));
            ops::save(&p, &id, &asset).unwrap();
            ids.push(id);
        }
    }
    assert!(ids.len() > 80, "only {} node materials", ids.len());
    let bevy = BevySources::discover();
    if bevy.is_none() {
        eprintln!("warning: Bevy 0.20 sources not in the cargo registry; WESL compile check skipped");
    }
    let report = regenerate(&p, &RegenOptions { compile_check: bevy.as_ref(), ..Default::default() }).unwrap();
    let errors: Vec<String> = report.diagnostics().filter(|d| d.is_error()).map(|d| d.to_string()).collect();
    assert!(errors.is_empty(), "errors:\n{}", errors.join("\n"));
    for id in &ids {
        let text = std::fs::read_to_string(wesl_file(&p, id).unwrap()).unwrap();
        snapshot(&format!("wesl__{}", id.name()), &text);
    }
}

#[test]
fn composite_materials_generate_and_compile() {
    let (_dir, p) = project();
    fixtures::save_all(&p).unwrap();
    let mut ids: Vec<AssetId> = fixtures::all()
        .into_iter()
        .map(|(path, is_fn, _)| AssetId::new(if is_fn { AssetType::Function } else { AssetType::Material }, path))
        .collect();
    ids.extend(fixtures::curves().into_iter().map(|(path, _)| AssetId::new(AssetType::Curve, path)));
    let bevy = BevySources::discover();
    let report = regenerate(&p, &RegenOptions { compile_check: bevy.as_ref(), ..Default::default() }).unwrap();
    let errors: Vec<String> = report.diagnostics().filter(|d| d.is_error()).map(|d| d.to_string()).collect();
    assert!(errors.is_empty(), "errors:\n{}", errors.join("\n"));
    for id in &ids {
        let flat = match id.ty {
            AssetType::Curve => format!("curves__{}", id.path.replace('/', "__")),
            _ => id.path.replace('/', "__"),
        };
        snapshot(&format!("wesl__{flat}"), &std::fs::read_to_string(wesl_file(&p, id).unwrap()).unwrap());
        if let Some(rs) = rust_file(&p, id) {
            snapshot(&format!("rust__{flat}"), &std::fs::read_to_string(rs).unwrap());
        }
    }
    for f in [
        "Cargo.toml",
        "src/lib.rs",
        "src/material/mod.rs",
        "src/material/materials/mod.rs",
        "src/material/baked/mod.rs",
        "src/material/baked/materials.rs",
        "src/material/baked/textures.rs",
    ] {
        let text = std::fs::read_to_string(p.crate_dir().join(f)).unwrap();
        snapshot(&format!("crate__{}", f.replace(['/', '.'], "_")), &text);
    }
    snapshot("manifest", &std::fs::read_to_string(p.autogen_dir().join("baked.ron")).unwrap());
}

/// Node preview (spec §5, decision 0022): every material kind × preview mode generates and compiles, and the
/// generated Rust is the same as without the preview (only `.wesl` changes, the game hot-reloads it).
#[test]
fn preview_modes_generate_and_compile() {
    use shader_graph_core::model::{PbrSettings, Preview, PreviewMode};
    let kinds = [
        ("pbr3d", MaterialKind::Pbr3d(PbrSettings::default()), "base_color"),
        ("unlit3d", MaterialKind::Unlit3d(UnlitSettings::default()), "color"),
        ("sprite2d", MaterialKind::Sprite2d(SpriteSettings::default()), "color"),
    ];
    let (_dir, p) = project();
    let mut ids = Vec::new();
    for (kind_name, mat, color) in kinds {
        for mode in PreviewMode::ALL {
            let name = format!("{kind_name}_{}", mode.label());
            let mut g = GraphBuilder::new(&name, AssetKind::Material(mat));
            let out = g.node(mat.output_node_kind(), (600.0, 0.0));
            let uv = g.node("stdlib::TextureCoordinate", (0.0, 0.0));
            let lerp = g.node("stdlib::Lerp", (300.0, 0.0));
            // Float2 → Float truncation (W104) on purpose.
            g.link((&uv, "out"), (&lerp, "alpha"));
            g.link((&lerp, "out"), (&out, color));
            if matches!(mat, MaterialKind::Pbr3d(_)) {
                let rough = g.node_p("param::Scalar", (300.0, 200.0), &[("name", Value::Str("rough".into()))]);
                g.link((&rough, "out"), (&out, "roughness"));
            }
            if matches!(mat, MaterialKind::Unlit3d(_)) {
                let c =
                    g.node_p("stdlib::Constant3Vector", (300.0, 200.0), &[("value", Value::Float3([0.0, 0.1, 0.0]))]);
                g.link((&c, "out"), (&out, shader_graph_core::ir::WPO_PIN));
            }
            let mut asset = g.finish();
            asset.preview = Preview { node: Some(lerp), mode };
            let id = AssetId::new(AssetType::Material, format!("preview/{name}"));
            ops::save(&p, &id, &asset).unwrap();
            ids.push((id, asset));
        }
    }
    let bevy = BevySources::discover();
    if bevy.is_none() {
        eprintln!("warning: Bevy 0.20 sources not in the cargo registry; WESL compile check skipped");
    }
    let report = regenerate(&p, &RegenOptions { compile_check: bevy.as_ref(), ..Default::default() }).unwrap();
    let errors: Vec<String> = report.diagnostics().filter(|d| d.is_error()).map(|d| d.to_string()).collect();
    assert!(errors.is_empty(), "errors:\n{}", errors.join("\n"));
    let mut rust = BTreeMap::new();
    for (id, _) in &ids {
        assert!(report.assets[id].ir.as_ref().is_some_and(|ir| ir.preview.is_some()), "{id:?}: preview ignored");
        let text = std::fs::read_to_string(wesl_file(&p, id).unwrap()).unwrap();
        snapshot(&format!("wesl__preview__{}", id.name()), &text);
        rust.insert(id.clone(), std::fs::read_to_string(rust_file(&p, id).unwrap()).unwrap());
    }

    // Preview off: the Rust modules do not change.
    for (id, asset) in &mut ids {
        asset.preview.node = None;
        ops::save(&p, id, asset).unwrap();
    }
    let report = regenerate(&p, &RegenOptions::default()).unwrap();
    assert!(!report.has_errors());
    for (id, _) in &ids {
        let text = std::fs::read_to_string(rust_file(&p, id).unwrap()).unwrap();
        assert_eq!(rust[id], text, "{id:?}: the Rust module changed with preview");
    }
}

/// Pbr3d `Wrapped` output (decision 0023): Make → `attributes`, plus World Position Offset.
#[test]
fn wrapped_output_generates_and_compiles() {
    use shader_graph_core::model::{PbrOutputMode, PbrSettings};
    let (_dir, p) = project();
    let mat = MaterialKind::Pbr3d(PbrSettings { output: PbrOutputMode::Wrapped, ..Default::default() });
    let mut g = GraphBuilder::new("pbr3d", AssetKind::Material(mat));
    let out = g.node(mat.output_node_kind(), (600.0, 0.0));
    let tint = g.node_p("param::Color", (0.0, 0.0), &[("name", Value::Str("tint".into()))]);
    let make = g.node("stdlib::MakeMaterialAttributesPbr", (300.0, 0.0));
    let wpo = g.node_p("stdlib::Constant3Vector", (300.0, 300.0), &[("value", Value::Float3([0.0, 0.1, 0.0]))]);
    g.link((&tint, "rgb"), (&make, "base_color"));
    g.link((&make, "attributes"), (&out, "attributes"));
    g.link((&wpo, "out"), (&out, shader_graph_core::ir::WPO_PIN));
    let id = AssetId::new(AssetType::Material, "wrapped/pbr3d");
    ops::save(&p, &id, &g.finish()).unwrap();
    let bevy = BevySources::discover();
    if bevy.is_none() {
        eprintln!("warning: Bevy 0.20 sources not in the cargo registry; WESL compile check skipped");
    }
    let report = regenerate(&p, &RegenOptions { compile_check: bevy.as_ref(), ..Default::default() }).unwrap();
    let errors: Vec<String> = report.diagnostics().filter(|d| d.is_error()).map(|d| d.to_string()).collect();
    assert!(errors.is_empty(), "errors:\n{}", errors.join("\n"));
    let text = std::fs::read_to_string(wesl_file(&p, &id).unwrap()).unwrap();
    snapshot("wesl__wrapped__pbr3d", &text);
}

/// Preview of a Make node in `full` mode (decision 0023): every attribute field feeds its slot.
#[test]
fn preview_attributes_generates_and_compiles() {
    use shader_graph_core::model::{PbrSettings, Preview, PreviewMode};
    let (_dir, p) = project();
    let mat = MaterialKind::Pbr3d(PbrSettings::default());
    let mut g = GraphBuilder::new("pbr3d_attributes_full", AssetKind::Material(mat));
    let out = g.node(mat.output_node_kind(), (600.0, 0.0));
    let rough = g.node_p("param::Scalar", (300.0, 200.0), &[("name", Value::Str("rough".into()))]);
    g.link((&rough, "out"), (&out, "roughness"));
    let make = g.node("stdlib::MakeMaterialAttributesPbr", (300.0, 0.0));
    g.input(&make, "base_color", Value::Float3([0.2, 0.4, 0.6]));
    g.input(&make, "roughness", Value::Float(0.8));
    let mut asset = g.finish();
    asset.preview = Preview { node: Some(make), mode: PreviewMode::Full };
    let id = AssetId::new(AssetType::Material, "preview/pbr3d_attributes_full");
    ops::save(&p, &id, &asset).unwrap();
    let bevy = BevySources::discover();
    if bevy.is_none() {
        eprintln!("warning: Bevy 0.20 sources not in the cargo registry; WESL compile check skipped");
    }
    let report = regenerate(&p, &RegenOptions { compile_check: bevy.as_ref(), ..Default::default() }).unwrap();
    let errors: Vec<String> = report.diagnostics().filter(|d| d.is_error()).map(|d| d.to_string()).collect();
    assert!(errors.is_empty(), "errors:\n{}", errors.join("\n"));
    assert!(report.assets[&id].ir.as_ref().is_some_and(|ir| ir.preview.is_some()), "preview ignored");
    let text = std::fs::read_to_string(wesl_file(&p, &id).unwrap()).unwrap();
    snapshot("wesl__preview__pbr3d_attributes_full", &text);
    let rust = std::fs::read_to_string(rust_file(&p, &id).unwrap()).unwrap();

    // Preview off: the Rust module does not change.
    asset.preview.node = None;
    ops::save(&p, &id, &asset).unwrap();
    assert!(!regenerate(&p, &RegenOptions::default()).unwrap().has_errors());
    assert_eq!(rust, std::fs::read_to_string(rust_file(&p, &id).unwrap()).unwrap());
}
