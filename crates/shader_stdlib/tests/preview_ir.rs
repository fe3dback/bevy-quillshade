//! Node preview in the IR (spec §5, decision 0022): the previewed node's outputs replace the material output's.

use shader_graph_core::consts::NoConsts;
use shader_graph_core::ir::{IrPreview, WPO_PIN, build_ir};
use shader_graph_core::model::{
    Asset, AssetKind, MaterialKind, Node, PbrSettings, Preview, PreviewMode, SpriteSettings, UnlitSettings,
};
use shader_graph_core::registry::{NoCurves, NoFunctions};
use shader_graph_core::{Code, Diagnostic, Ir, Value};

fn node(a: &mut Asset, id: &str, kind: &str, params: &[(&str, Value)]) -> String {
    let mut n = Node::new(kind, (0.0, 0.0));
    for (k, v) in params {
        n.params.insert(k.to_string(), v.clone());
    }
    a.nodes.insert(id.to_string(), n);
    id.to_string()
}

fn build(a: &Asset) -> (Ir, Vec<Diagnostic>) {
    let reg = shader_stdlib::registry();
    let r = build_ir(a, "test/mat", &reg, &NoFunctions, &NoCurves, &NoConsts);
    let errors: Vec<_> = r.diagnostics.iter().filter(|d| d.is_error()).collect();
    assert!(errors.is_empty(), "{errors:?}");
    (r.ir.expect("ir"), r.diagnostics)
}

fn has(diags: &[Diagnostic], code: Code, node: Option<&str>) -> bool {
    diags.iter().any(|d| d.code == code && d.node.as_deref() == node)
}

/// Pbr3d: `c1 → mul.a` (mul feeds nothing), `rough → roughness`, `c2 → base_color`.
fn pbr_graph() -> Asset {
    let mut a = Asset::new("mat", AssetKind::Material(MaterialKind::Pbr3d(PbrSettings::default())));
    let out = node(&mut a, "out", "output::Pbr3d", &[]);
    let c1 = node(&mut a, "c1", "stdlib::Constant", &[("value", Value::Float(0.25))]);
    let mul = node(&mut a, "mul", "stdlib::Multiply", &[]);
    let rough = node(&mut a, "rough", "param::Scalar", &[("name", Value::Str("rough".into()))]);
    let c2 = node(&mut a, "c2", "stdlib::Constant3Vector", &[("value", Value::Float3([0.1, 0.2, 0.3]))]);
    a.connect((&c1, "out"), (&mul, "a"));
    a.connect((&rough, "out"), (&out, "roughness"));
    a.connect((&c2, "out"), (&out, "base_color"));
    a
}

#[test]
fn preview_replaces_outputs() {
    let mut a = pbr_graph();
    let (normal, _) = build(&a);
    assert!(normal.main.outputs.contains_key("roughness"));
    assert!(normal.preview.is_none());

    a.preview = Preview { node: Some("mul".into()), mode: PreviewMode::Unlit };
    let (ir, diags) = build(&a);
    assert_eq!(ir.main.outputs.keys().collect::<Vec<_>>(), ["base_color"]);
    let albedo = &ir.main.outputs["base_color"];
    assert!(albedo.starts_with("vec3<f32>(") && albedo.contains("n_mul_out"), "{albedo}");
    assert!(ir.main.lines.iter().any(|l| l.name == "n_mul_out"));
    assert!(ir.params.iter().any(|p| p.name == "rough"));
    assert_eq!(ir.uniform, normal.uniform);
    assert!(has(&diags, Code::W107, Some("mul")), "{diags:?}");
    assert!(!has(&diags, Code::W101, Some("mul")), "previewed node is live: {diags:?}");
    assert!(!Code::W107.is_error() && !Code::W108.is_error());
    assert_eq!(ir.preview, Some(IrPreview { node: "mul".into(), mode: PreviewMode::Unlit }));
}

#[test]
fn preview_of_bool_or_missing_node_is_ignored() {
    let mut a = Asset::new("mat", AssetKind::Material(MaterialKind::Pbr3d(PbrSettings::default())));
    let out = node(&mut a, "out", "output::Pbr3d", &[]);
    let flag = node(&mut a, "flag", "param::Bool", &[("name", Value::Str("flag".into()))]);
    let sel = node(&mut a, "sel", "stdlib::BoolSelect", &[]);
    a.connect((&flag, "out"), (&sel, "cond"));
    a.connect((&sel, "out"), (&out, "base_color"));
    let (normal, _) = build(&a);

    for (target, at) in [("flag", Some("flag")), ("zzz", None), ("out", Some("out"))] {
        a.preview = Preview { node: Some(target.into()), mode: PreviewMode::Full };
        let (ir, diags) = build(&a);
        assert_eq!(ir.main.outputs, normal.main.outputs, "{target}");
        assert_eq!(ir.preview, None, "{target}");
        assert!(has(&diags, Code::W107, at), "{target}: {diags:?}");
        let w107: Vec<_> = diags.iter().filter(|d| d.code == Code::W107).collect();
        assert!(w107.iter().all(|d| d.message.starts_with("preview ignored")), "{w107:?}");
    }
}

#[test]
fn preview_dangling_id_loaded_from_file_is_ignored_with_warning() {
    let mut a = pbr_graph();
    let (normal, _) = build(&a);
    a.preview = Preview { node: Some("zzz".into()), mode: PreviewMode::Lit };
    let loaded = Asset::from_ron(&a.to_ron()).unwrap();
    assert_eq!(loaded.preview.node.as_deref(), Some("zzz"));
    let (ir, diags) = build(&loaded);
    assert_eq!(ir.preview, None);
    assert_eq!(ir.main.outputs, normal.main.outputs);
    assert!(
        diags.iter().any(|d| d.code == Code::W107 && d.node.is_none() && d.message.contains("`zzz` does not exist")),
        "{diags:?}"
    );
}

#[test]
fn preview_of_float2_resolved_node_pads_albedo() {
    let mut a = pbr_graph();
    let uv = node(&mut a, "uv", "stdlib::TextureCoordinate", &[]);
    let mul2 = node(&mut a, "mul2", "stdlib::Multiply", &[]);
    a.connect((&uv, "out"), (&mul2, "a"));
    a.preview = Preview { node: Some(mul2.clone()), mode: PreviewMode::Unlit };
    let (ir, diags) = build(&a);
    assert_eq!(ir.main.outputs["base_color"], "vec3<f32>(n_mul2_out, 0.0)");
    assert!(ir.main.lines.iter().any(|l| l.name == "n_mul2_out" && l.ty == "vec2<f32>"), "{:?}", ir.main.lines);
    assert!(ir.preview.is_some());
    assert!(diags.iter().any(|d| d.code == Code::W107 && d.message.starts_with("preview active")), "{diags:?}");
}

#[test]
fn preview_of_texture_node_is_ignored_without_trace() {
    let mut a = pbr_graph();
    let (normal, normal_diags) = build(&a);
    let tex = node(&mut a, "tex", "param::TextureObject", &[("name", Value::Str("tex".into()))]);
    let (with_tex, _) = build(&a);
    a.preview = Preview { node: Some(tex.clone()), mode: PreviewMode::Unlit };
    let (ir, diags) = build(&a);
    assert_eq!(ir.preview, None);
    assert_eq!(ir.main.outputs, normal.main.outputs);
    assert_eq!(ir.params.len(), with_tex.params.len(), "dead texture is not collected");
    assert!(!ir.params.iter().any(|p| p.name == "tex"));
    assert!(
        diags.iter().any(|d| d.code == Code::W107
            && d.node.as_deref() == Some("tex")
            && d.message.contains("has no output that converts to a color")),
        "{diags:?}"
    );
    assert!(has(&diags, Code::W101, Some("tex")));
    assert!(normal_diags.iter().all(|d| diags.contains(d)));
}

#[test]
fn preview_ignored_messages() {
    let mut a = pbr_graph();
    a.nodes.insert("odd".into(), Node::new("stdlib::NoSuchNode", (0.0, 0.0)));
    for (target, text) in [("odd", "cannot be resolved"), ("out", "material output node cannot be previewed")] {
        a.preview = Preview { node: Some(target.into()), mode: PreviewMode::Unlit };
        let reg = shader_stdlib::registry();
        let r = build_ir(&a, "test/mat", &reg, &NoFunctions, &NoCurves, &NoConsts);
        assert!(
            r.diagnostics
                .iter()
                .any(|d| d.code == Code::W107 && d.node.as_deref() == Some(target) && d.message.contains(text)),
            "{target}: {:?}",
            r.diagnostics
        );
    }
}

#[test]
fn preview_keeps_vertex_stage() {
    let mut a = Asset::new("mat", AssetKind::Material(MaterialKind::Unlit3d(UnlitSettings::default())));
    let out = node(&mut a, "out", "output::Unlit3d", &[]);
    let wpo = node(&mut a, "wpo", "stdlib::Constant3Vector", &[("value", Value::Float3([0.0, 1.0, 0.0]))]);
    let c = node(&mut a, "c", "stdlib::Constant", &[("value", Value::Float(0.5))]);
    a.connect((&wpo, "out"), (&out, WPO_PIN));
    a.connect((&c, "out"), (&out, "color"));
    a.preview = Preview { node: Some(c.clone()), mode: PreviewMode::Lit };
    let (ir, _) = build(&a);
    let vertex = ir.vertex.expect("vertex stage kept");
    assert_eq!(vertex.outputs[WPO_PIN], "vec3<f32>(0.0)");
    assert_eq!(ir.main.outputs.keys().collect::<Vec<_>>(), ["color"]);
    assert_eq!(ir.main.outputs["color"], "vec3<f32>(0.5f)");
}

#[test]
fn preview_on_sprite_writes_opaque_color() {
    let mut a = Asset::new("mat", AssetKind::Material(MaterialKind::Sprite2d(SpriteSettings::default())));
    node(&mut a, "out", "output::Sprite2d", &[]);
    let c = node(&mut a, "c", "stdlib::Constant4Vector", &[("value", Value::Float4([0.1, 0.2, 0.3, 0.4]))]);
    a.preview = Preview { node: Some(c), mode: PreviewMode::Full };
    let (ir, diags) = build(&a);
    let color = &ir.main.outputs["color"];
    assert!(color.starts_with("vec4<f32>((") && color.ends_with(").xyz, 1.0)"), "{color}");
    assert!(diags.iter().any(|d| d.code == Code::W107 && d.message.contains("mode full")), "{diags:?}");
}

#[test]
fn preview_in_functions_is_ignored_silently() {
    let mut a = Asset::new("fun", AssetKind::Function);
    let out = node(&mut a, "out", "output::FunctionOutput", &[]);
    let c = node(&mut a, "c", "stdlib::Constant3Vector", &[]);
    a.connect((&c, "out"), (&out, "value"));
    a.preview = Preview { node: Some(c), mode: PreviewMode::Unlit };
    let (ir, diags) = build(&a);
    assert_eq!(ir.preview, None);
    assert!(!diags.iter().any(|d| d.code == Code::W107), "{diags:?}");
}

/// Multiply whose preview also fills the Opacity slot (no stdlib node does yet).
struct OpaqueSlotMul(std::sync::Arc<dyn shader_graph_core::registry::NodeDef>, shader_graph_core::registry::NodeInfo);

impl shader_graph_core::registry::NodeDef for OpaqueSlotMul {
    fn info(&self) -> &shader_graph_core::registry::NodeInfo {
        &self.1
    }
    fn signature(&self, ctx: &shader_graph_core::registry::SigCtx) -> shader_graph_core::registry::Signature {
        self.0.signature(ctx)
    }
    fn emit(&self, ctx: &mut shader_graph_core::registry::EmitCtx) {
        self.0.emit(ctx)
    }
    fn preview_output(
        &self,
        sig: &shader_graph_core::registry::Signature,
    ) -> shader_graph_core::preview::ShaderOutputDto {
        let mut dto = shader_graph_core::preview::ShaderOutputDto::main_output(sig);
        let src = shader_graph_core::preview::PreviewSource::pin("out");
        dto.slots.insert(shader_graph_core::preview::PreviewSlot::Opacity, src);
        dto
    }
}

#[test]
fn full_preview_forces_alpha_over_an_opacity_slot() {
    let mut reg = shader_stdlib::registry();
    let mul = reg.get("stdlib::Multiply").unwrap().clone();
    let info = shader_graph_core::registry::NodeInfo { id: "test::OpaqueSlotMul".into(), ..mul.info().clone() };
    reg.register(OpaqueSlotMul(mul, info));
    let settings = PbrSettings { blend: shader_graph_core::model::BlendMode::Blend, ..Default::default() };
    let mut a = Asset::new("mat", AssetKind::Material(MaterialKind::Pbr3d(settings)));
    node(&mut a, "out", "output::Pbr3d", &[]);
    node(&mut a, "m", "test::OpaqueSlotMul", &[]);
    a.preview = Preview { node: Some("m".into()), mode: PreviewMode::Full };
    let r = build_ir(&a, "test/mat", &reg, &NoFunctions, &NoCurves, &NoConsts);
    let ir = r.ir.expect("ir");
    assert!(ir.preview.is_some(), "{:?}", r.diagnostics);
    assert_eq!(ir.main.outputs.keys().collect::<Vec<_>>(), ["base_color"], "alpha stays 1 (decision 0022)");
}

const MAKE: &str = "stdlib::MakeMaterialAttributesPbr";

/// Pbr3d with a Make (base_color inline) that feeds nothing; `settings` picks the output mode.
fn attributes_graph(settings: PbrSettings) -> Asset {
    let mut a = Asset::new("mat", AssetKind::Material(MaterialKind::Pbr3d(settings)));
    node(&mut a, "out", "output::Pbr3d", &[]);
    node(&mut a, "make", MAKE, &[]);
    a.nodes.get_mut("make").unwrap().inputs.insert("base_color".into(), Value::Float3([0.2, 0.4, 0.6]));
    a
}

const LIT_OPAQUE_FIELDS: [&str; 7] =
    ["ambient_occlusion", "base_color", "emissive", "metallic", "normal", "roughness", "specular"];

#[test]
fn preview_attributes_unlit_albedo_only() {
    let mut a = attributes_graph(PbrSettings::default());
    a.preview = Preview { node: Some("make".into()), mode: PreviewMode::Unlit };
    let (ir, _) = build(&a);
    assert_eq!(ir.main.outputs.keys().collect::<Vec<_>>(), ["base_color"]);
    assert!(ir.main.outputs["base_color"].contains("n_make_attributes.base_color"), "{:?}", ir.main.outputs);
    assert!(ir.preview.is_some());
}

#[test]
fn preview_attributes_full_maps_fields() {
    let mut a = attributes_graph(PbrSettings::default());
    a.preview = Preview { node: Some("make".into()), mode: PreviewMode::Full };
    let (ir, _) = build(&a);
    assert_eq!(ir.main.outputs.keys().map(String::as_str).collect::<Vec<_>>(), LIT_OPAQUE_FIELDS);
    for (k, v) in &ir.main.outputs {
        assert_eq!(v, &format!("n_make_attributes.{k}"));
    }
}

#[test]
fn preview_attributes_full_in_wrapped_mode() {
    use shader_graph_core::model::PbrOutputMode;
    let mut a = attributes_graph(PbrSettings { output: PbrOutputMode::Wrapped, ..Default::default() });
    a.connect(("make", "attributes"), ("out", "attributes"));
    node(&mut a, "make2", MAKE, &[]);
    a.preview = Preview { node: Some("make2".into()), mode: PreviewMode::Full };
    let (ir, _) = build(&a);
    assert_eq!(ir.main.outputs.keys().map(String::as_str).collect::<Vec<_>>(), LIT_OPAQUE_FIELDS);
    for (k, v) in &ir.main.outputs {
        assert_eq!(v, &format!("n_make2_attributes.{k}"));
    }
}

#[test]
fn preview_lerp_of_attributes() {
    let mut a = attributes_graph(PbrSettings::default());
    node(&mut a, "make2", MAKE, &[]);
    node(&mut a, "lerp", "stdlib::Lerp", &[]);
    a.connect(("make", "attributes"), ("lerp", "a"));
    a.connect(("make2", "attributes"), ("lerp", "b"));
    a.preview = Preview { node: Some("lerp".into()), mode: PreviewMode::Lit };
    let (ir, diags) = build(&a);
    assert!(ir.preview.is_some(), "{diags:?}");
    assert!(!diags.iter().any(|d| d.code == Code::W107 && d.message.starts_with("preview ignored")), "{diags:?}");
    assert_eq!(ir.main.outputs["base_color"], "n_lerp_out.base_color");
}
