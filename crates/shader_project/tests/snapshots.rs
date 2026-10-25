//! Snapshot suite (spec §15): one material per stdlib node plus the composite reference graphs.
//! Every generated `.wesl` is compiled with wesl + naga against the real Bevy 0.20 sources
//! (forward, forward without UV/tangents, deferred = `PREPASS_PIPELINE`), and the WESL/Rust
//! output is compared byte for byte with `tests/snapshots/`.
//!
//! Update snapshots after an intended codegen change: `INSTA_UPDATE=always cargo test -p shader_project --test snapshots`.

use std::collections::BTreeMap;

use shader_codegen_wesl::check::BevySources;
use shader_graph_core::model::{AssetKind, MaterialKind, SpriteSettings, UnlitSettings};
use shader_graph_core::registry::{FUNCTION_CALL_KIND, NoFunctions, NodeRole, SigCtx};
use shader_graph_core::{Asset, Env, PinType, Value, conversion};
use shader_project::fixtures::{self, GraphBuilder};
use shader_project::regen::{rust_file, wesl_file};
use shader_project::{AssetId, AssetType, Project, RegenOptions, ops, regenerate};

/// Material exercising one node: required textures fed, first output routed into the color.
fn node_material(kind: &str) -> Option<(String, Asset)> {
    let reg = shader_stdlib::registry();
    let def = reg.get(kind)?;
    if kind == FUNCTION_CALL_KIND || !matches!(def.role(), NodeRole::Normal | NodeRole::Parameter | NodeRole::Reroute) {
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
    let env = Env { asset_kind, asset_path: &name, functions: &NoFunctions };
    let sig = def.signature(&SigCtx {
        node_id: &n,
        kind,
        params: &params,
        connected: &connected,
        input_values: &inputs,
        env: &env,
    });
    for p in sig.inputs.iter().filter(|p| p.ty == PinType::Texture2D) {
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
        PinType::Bool => {
            let s = g.node("stdlib::BoolSelect", (400.0, 0.0));
            g.link((&n, &first.id), (&s, "cond"));
            g.link((&s, "out"), (&out, color));
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
    std::fs::write(dir.path().join("Cargo.toml"), "[workspace]\nmembers = [\"crates/shaders\"]\n").unwrap();
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
        let text = std::fs::read_to_string(wesl_file(&p, id)).unwrap();
        snapshot(&format!("wesl__{}", id.name()), &text);
    }
}

#[test]
fn composite_materials_generate_and_compile() {
    let (_dir, p) = project();
    let mut ids = Vec::new();
    for (path, is_fn, asset) in fixtures::all() {
        let id = AssetId::new(if is_fn { AssetType::Function } else { AssetType::Material }, path);
        ops::save(&p, &id, &asset).unwrap();
        ids.push(id);
    }
    let bevy = BevySources::discover();
    let report = regenerate(&p, &RegenOptions { compile_check: bevy.as_ref(), ..Default::default() }).unwrap();
    let errors: Vec<String> = report.diagnostics().filter(|d| d.is_error()).map(|d| d.to_string()).collect();
    assert!(errors.is_empty(), "errors:\n{}", errors.join("\n"));
    for id in &ids {
        let flat = id.path.replace('/', "__");
        snapshot(&format!("wesl__{flat}"), &std::fs::read_to_string(wesl_file(&p, id)).unwrap());
        if let Some(rs) = rust_file(&p, id) {
            snapshot(&format!("rust__{flat}"), &std::fs::read_to_string(rs).unwrap());
        }
    }
    for f in ["Cargo.toml", "src/lib.rs", "src/materials/mod.rs"] {
        let text = std::fs::read_to_string(p.crate_dir().join(f)).unwrap();
        snapshot(&format!("crate__{}", f.replace(['/', '.'], "_")), &text);
    }
}
