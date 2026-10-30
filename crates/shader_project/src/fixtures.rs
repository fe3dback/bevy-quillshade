//! Reference graphs used by tests, snapshot suites and `examples/sample_game` (spec §15, 17).

use shader_graph_core::consts::{ConstType, GlobalConst, GlobalConsts};
use shader_graph_core::curve::Curve;
use shader_graph_core::gradient::{GradientCollection, Stop, srgb_to_linear};
use shader_graph_core::model::{
    Asset, AssetKind, BlendMode, MaterialKind, Node, PbrOutputMode, PbrSettings, SpriteSettings, UnlitSettings,
};
use shader_graph_core::texture::{AddressMode, FilterMode, SamplerSettings, TextureAsset, TextureSettings};
use shader_graph_core::{PinType, Value};
use shader_stdlib::nodes::color::{GRADIENT_COLLECTION_PARAM, GRADIENT_SLOT_PARAM, SAMPLE_GRADIENT_KIND};

use crate::assets::{AssetId, AssetType};
use crate::layout::Project;
use crate::ops::{self, OpError};

/// Programmatic graph construction with deterministic node ids (`n01`, `n02`, …).
/// Scale from the sketch coordinates used below to canvas coordinates.
const SPREAD: (f32, f32) = (1.4, 1.35);

pub struct GraphBuilder {
    pub asset: Asset,
    counter: usize,
}

impl GraphBuilder {
    pub fn new(name: &str, kind: AssetKind) -> Self {
        GraphBuilder { asset: Asset::new(name, kind), counter: 0 }
    }

    /// Adds a node; `pos` is on a compact sketch grid, spread out to fit real node sizes.
    pub fn node(&mut self, kind: &str, pos: (f32, f32)) -> String {
        self.counter += 1;
        let id = format!("n{:02}", self.counter);
        self.asset.nodes.insert(id.clone(), Node::new(kind, (pos.0 * SPREAD.0, pos.1 * SPREAD.1)));
        id
    }

    /// Node with params.
    pub fn node_p(&mut self, kind: &str, pos: (f32, f32), params: &[(&str, Value)]) -> String {
        let id = self.node(kind, pos);
        for (k, v) in params {
            self.param(&id, k, v.clone());
        }
        id
    }

    pub fn param(&mut self, node: &str, key: &str, v: Value) {
        self.asset.nodes.get_mut(node).expect("node").params.insert(key.into(), v);
    }

    pub fn input(&mut self, node: &str, pin: &str, v: Value) {
        self.asset.nodes.get_mut(node).expect("node").inputs.insert(pin.into(), v);
    }

    pub fn link(&mut self, from: (&str, &str), to: (&str, &str)) {
        self.asset.connect(from, to);
    }

    pub fn finish(self) -> Asset {
        self.asset
    }
}

fn s(v: &str) -> Value {
    Value::Str(v.into())
}

fn f(v: f32) -> Value {
    Value::Float(v)
}

/// Function with 2 inputs and 2 outputs (spec §17 acceptance criterion).
pub fn wet_mask() -> Asset {
    let mut g = GraphBuilder::new("wet_mask", AssetKind::Function);
    let wet = g.node_p(
        "output::FunctionInput",
        (0.0, 0.0),
        &[
            ("name", s("wetness")),
            ("type", Value::Type(PinType::Float)),
            ("default", Value::Float4([0.5; 4])),
            ("order", Value::Int(0)),
        ],
    );
    let base = g.node_p(
        "output::FunctionInput",
        (0.0, 120.0),
        &[
            ("name", s("base_color")),
            ("type", Value::Type(PinType::Float3)),
            ("default", Value::Float4([0.5; 4])),
            ("order", Value::Int(1)),
        ],
    );
    let noise = g.node_p("stdlib::Noise", (0.0, 240.0), &[("function", s("Value"))]);
    g.input(&noise, "scale", f(2.0));
    let one_minus = g.node("stdlib::OneMinus", (200.0, 0.0));
    g.link((&wet, "out"), (&one_minus, "x"));
    let lo = g.node("stdlib::Subtract", (380.0, 0.0));
    g.link((&one_minus, "out"), (&lo, "a"));
    g.input(&lo, "b", f(0.1));
    let hi = g.node("stdlib::Add", (380.0, 100.0));
    g.link((&one_minus, "out"), (&hi, "a"));
    g.input(&hi, "b", f(0.1));
    let mask = g.node("stdlib::SmoothStep", (560.0, 60.0));
    g.link((&lo, "out"), (&mask, "min"));
    g.link((&hi, "out"), (&mask, "max"));
    g.link((&noise, "out"), (&mask, "x"));
    let dark = g.node("stdlib::Multiply", (200.0, 160.0));
    g.link((&base, "out"), (&dark, "a"));
    g.input(&dark, "b", f(0.4));
    let tint = g.node("stdlib::Lerp", (560.0, 200.0));
    g.link((&base, "out"), (&tint, "a"));
    g.link((&dark, "out"), (&tint, "b"));
    g.link((&mask, "out"), (&tint, "alpha"));
    let out_mask = g.node_p(
        "output::FunctionOutput",
        (760.0, 60.0),
        &[("name", s("mask")), ("type", Value::Type(PinType::Float)), ("order", Value::Int(0))],
    );
    g.link((&mask, "out"), (&out_mask, "value"));
    let out_tint = g.node_p(
        "output::FunctionOutput",
        (760.0, 200.0),
        &[("name", s("tint")), ("type", Value::Type(PinType::Float3)), ("order", Value::Int(1))],
    );
    g.link((&tint, "out"), (&out_tint, "value"));
    g.finish()
}

/// Reference material `rock_wet` (spec §17): 2 textures, 1 color, 2 scalars, Normal, Roughness,
/// Emissive, Opacity Mask; calls `common/wet_mask`.
pub fn rock_wet() -> Asset {
    let kind = AssetKind::Material(MaterialKind::Pbr3d(PbrSettings { blend: BlendMode::Mask, ..Default::default() }));
    let mut g = GraphBuilder::new("rock_wet", kind);
    let out = g.node("output::Pbr3d", (900.0, 0.0));
    let albedo = g.node_p(
        "param::TextureSample2D",
        (0.0, 0.0),
        &[("name", s("albedo")), ("default", s("textures/rock_albedo"))],
    );
    let normal = g.node_p(
        "param::TextureSample2D",
        (0.0, 220.0),
        &[("name", s("normal_map")), ("sampler_type", s("Normal")), ("default", s("textures/rock_normal"))],
    );
    let tint = g.node_p(
        "param::Color",
        (0.0, 440.0),
        &[("name", s("tint")), ("default", Value::Color([0.9, 0.85, 0.8, 1.0]))],
    );
    let wet = g.node_p(
        "param::Scalar",
        (0.0, 600.0),
        &[("name", s("wetness")), ("default", f(0.3)), ("min", f(0.0)), ("max", f(1.0)), ("group", s("Surface"))],
    );
    let speed = g.node_p(
        "param::Scalar",
        (0.0, 700.0),
        &[("name", s("speed")), ("default", f(1.0)), ("min", f(0.0)), ("max", f(10.0))],
    );
    let base = g.node("stdlib::Multiply", (250.0, 40.0));
    g.link((&albedo, "rgb"), (&base, "a"));
    g.link((&tint, "rgb"), (&base, "b"));
    let wm = g.node("function::common/wet_mask", (450.0, 120.0));
    g.link((&wet, "out"), (&wm, "wetness"));
    g.link((&base, "out"), (&wm, "base_color"));
    g.link((&wm, "tint"), (&out, "base_color"));
    let rough = g.node("stdlib::Lerp", (680.0, 160.0));
    g.input(&rough, "a", f(0.9));
    g.input(&rough, "b", f(0.15));
    g.link((&wm, "mask"), (&rough, "alpha"));
    g.link((&rough, "out"), (&out, "roughness"));
    g.link((&normal, "rgb"), (&out, "normal"));
    let time = g.node("stdlib::Time", (250.0, 600.0));
    let ts = g.node("stdlib::Multiply", (400.0, 620.0));
    g.link((&time, "out"), (&ts, "a"));
    g.link((&speed, "out"), (&ts, "b"));
    let sin = g.node("stdlib::Sin", (550.0, 620.0));
    g.link((&ts, "out"), (&sin, "x"));
    let pulse = g.node("stdlib::Saturate", (680.0, 620.0));
    g.link((&sin, "out"), (&pulse, "x"));
    let glow = g.node_p("stdlib::Constant3Vector", (550.0, 720.0), &[("value", Value::Float3([0.0, 0.05, 0.1]))]);
    let em = g.node("stdlib::Multiply", (780.0, 650.0));
    g.link((&pulse, "out"), (&em, "a"));
    g.link((&glow, "out"), (&em, "b"));
    let em2 = g.node("stdlib::Multiply", (860.0, 520.0));
    g.link((&em, "out"), (&em2, "a"));
    g.link((&wm, "mask"), (&em2, "b"));
    g.link((&em2, "out"), (&out, "emissive"));
    g.link((&albedo, "a"), (&out, "opacity_mask"));
    let mut a = g.finish();
    a.comments.push(shader_graph_core::Frame {
        id: "c01".into(),
        title: "Emissive pulse".into(),
        rect: (230.0 * SPREAD.0, 560.0 * SPREAD.1, 720.0 * SPREAD.0, 260.0 * SPREAD.1),
        nodes: vec![],
        color: None,
        hue: None,
    });
    a
}

/// Unlit additive hologram with World Position Offset.
pub fn hologram() -> Asset {
    let kind = AssetKind::Material(MaterialKind::Unlit3d(UnlitSettings { blend: BlendMode::Add, two_sided: true }));
    let mut g = GraphBuilder::new("hologram", kind);
    let out = g.node("output::Unlit3d", (900.0, 0.0));
    let color =
        g.node_p("param::Color", (0.0, 0.0), &[("name", s("color")), ("default", Value::Color([0.2, 0.8, 1.0, 1.0]))]);
    let scan = g.node_p("param::Scalar", (0.0, 300.0), &[("name", s("scan_speed")), ("default", f(0.5))]);
    let fres = g.node("stdlib::Fresnel", (0.0, 120.0));
    g.input(&fres, "exponent", f(3.0));
    let wp = g.node("stdlib::WorldPosition", (0.0, 420.0));
    let br = g.node("stdlib::BreakOutFloat3", (150.0, 420.0));
    g.link((&wp, "out"), (&br, "v"));
    let time = g.node("stdlib::Time", (0.0, 520.0));
    let y20 = g.node("stdlib::Multiply", (300.0, 400.0));
    g.link((&br, "y"), (&y20, "a"));
    g.input(&y20, "b", f(20.0));
    let ts = g.node("stdlib::Multiply", (300.0, 520.0));
    g.link((&time, "out"), (&ts, "a"));
    g.link((&scan, "out"), (&ts, "b"));
    let sum = g.node("stdlib::Add", (450.0, 450.0));
    g.link((&y20, "out"), (&sum, "a"));
    g.link((&ts, "out"), (&sum, "b"));
    let frac = g.node("stdlib::Frac", (580.0, 450.0));
    g.link((&sum, "out"), (&frac, "x"));
    let lines = g.node("stdlib::Step", (700.0, 450.0));
    g.input(&lines, "a", f(0.5));
    g.link((&frac, "out"), (&lines, "b"));
    let lw = g.node("stdlib::Multiply", (700.0, 300.0));
    g.link((&lines, "out"), (&lw, "a"));
    g.input(&lw, "b", f(0.3));
    let glow = g.node("stdlib::Add", (700.0, 150.0));
    g.link((&fres, "out"), (&glow, "a"));
    g.link((&lw, "out"), (&glow, "b"));
    let col = g.node("stdlib::Multiply", (800.0, 40.0));
    g.link((&color, "rgb"), (&col, "a"));
    g.link((&glow, "out"), (&col, "b"));
    g.link((&col, "out"), (&out, "color"));
    let op = g.node("stdlib::Saturate", (800.0, 200.0));
    g.link((&glow, "out"), (&op, "x"));
    g.link((&op, "out"), (&out, "opacity"));
    // World Position Offset: wobble along the normal.
    let n = g.node("stdlib::VertexNormalWS", (300.0, 700.0));
    let t3 = g.node("stdlib::Multiply", (300.0, 800.0));
    g.link((&time, "out"), (&t3, "a"));
    g.input(&t3, "b", f(3.0));
    let y10 = g.node("stdlib::Multiply", (300.0, 900.0));
    g.link((&br, "y"), (&y10, "a"));
    g.input(&y10, "b", f(10.0));
    let ph = g.node("stdlib::Add", (450.0, 850.0));
    g.link((&t3, "out"), (&ph, "a"));
    g.link((&y10, "out"), (&ph, "b"));
    let sn = g.node("stdlib::Sin", (580.0, 850.0));
    g.link((&ph, "out"), (&sn, "x"));
    let amp = g.node("stdlib::Multiply", (700.0, 850.0));
    g.link((&sn, "out"), (&amp, "a"));
    g.input(&amp, "b", f(0.02));
    let wpo = g.node("stdlib::Multiply", (820.0, 750.0));
    g.link((&n, "out"), (&wpo, "a"));
    g.link((&amp, "out"), (&wpo, "b"));
    g.link((&wpo, "out"), (&out, "world_position_offset"));
    g.finish()
}

/// Sprite dissolve (Sprite2d).
pub fn dissolve() -> Asset {
    let kind = AssetKind::Material(MaterialKind::Sprite2d(SpriteSettings { blend: BlendMode::Blend }));
    let mut g = GraphBuilder::new("dissolve", kind);
    let out = g.node("output::Sprite2d", (1000.0, 0.0));
    let amount = g.node_p(
        "param::Scalar",
        (0.0, 0.0),
        &[("name", s("amount")), ("default", f(0.4)), ("min", f(0.0)), ("max", f(1.0))],
    );
    let edge_color = g.node_p(
        "param::Color",
        (0.0, 100.0),
        &[("name", s("edge_color")), ("default", Value::Color([1.0, 0.6, 0.1, 1.0]))],
    );
    let edge_w = g.node_p("param::Scalar", (0.0, 260.0), &[("name", s("edge_width")), ("default", f(0.05))]);
    let sprite = g.node("stdlib::SpriteTexture", (0.0, 400.0));
    let uv = g.node_p("stdlib::TextureCoordinate", (200.0, 600.0), &[("tiling", Value::Float2([8.0, 8.0]))]);
    let p3 = g.node("stdlib::AppendVector", (350.0, 600.0));
    g.link((&uv, "out"), (&p3, "a"));
    let noise = g.node_p("stdlib::Noise", (500.0, 600.0), &[("function", s("Perlin"))]);
    g.link((&p3, "out"), (&noise, "position"));
    let diff = g.node("stdlib::Subtract", (650.0, 500.0));
    g.link((&noise, "out"), (&diff, "a"));
    g.link((&amount, "out"), (&diff, "b"));
    let visible = g.node("stdlib::Step", (780.0, 450.0));
    g.input(&visible, "a", f(0.0));
    g.link((&diff, "out"), (&visible, "b"));
    let inner = g.node("stdlib::Step", (780.0, 560.0));
    g.link((&edge_w, "out"), (&inner, "a"));
    g.link((&diff, "out"), (&inner, "b"));
    let not_inner = g.node("stdlib::OneMinus", (880.0, 560.0));
    g.link((&inner, "out"), (&not_inner, "x"));
    let edge = g.node("stdlib::Multiply", (950.0, 500.0));
    g.link((&not_inner, "out"), (&edge, "a"));
    g.link((&visible, "out"), (&edge, "b"));
    let rgb = g.node("stdlib::Lerp", (850.0, 200.0));
    g.link((&sprite, "rgb"), (&rgb, "a"));
    g.link((&edge_color, "rgb"), (&rgb, "b"));
    g.link((&edge, "out"), (&rgb, "alpha"));
    let a = g.node("stdlib::Multiply", (850.0, 350.0));
    g.link((&sprite, "a"), (&a, "a"));
    g.link((&visible, "out"), (&a, "b"));
    let rgba = g.node("stdlib::AppendVector", (950.0, 250.0));
    g.link((&rgb, "out"), (&rgba, "a"));
    g.link((&a, "out"), (&rgba, "b"));
    g.link((&rgba, "out"), (&out, "color"));
    g.finish()
}

/// Two-sided flag waving through World Position Offset.
pub fn flag() -> Asset {
    let kind = AssetKind::Material(MaterialKind::Pbr3d(PbrSettings { two_sided: true, ..Default::default() }));
    let mut g = GraphBuilder::new("flag", kind);
    let out = g.node("output::Pbr3d", (900.0, 0.0));
    let color =
        g.node_p("param::Color", (0.0, 0.0), &[("name", s("color")), ("default", Value::Color([0.8, 0.1, 0.1, 1.0]))]);
    let strength = g.node_p("param::Scalar", (0.0, 300.0), &[("name", s("strength")), ("default", f(0.2))]);
    g.link((&color, "rgb"), (&out, "base_color"));
    let rough = g.node_p("stdlib::Constant", (600.0, 150.0), &[("value", f(0.6))]);
    g.link((&rough, "out"), (&out, "roughness"));
    let uv = g.node("stdlib::TextureCoordinate", (0.0, 400.0));
    let br = g.node("stdlib::BreakOutFloat2", (150.0, 400.0));
    g.link((&uv, "out"), (&br, "v"));
    let time = g.node("stdlib::Time", (0.0, 520.0));
    let t2 = g.node("stdlib::Multiply", (300.0, 520.0));
    g.link((&time, "out"), (&t2, "a"));
    g.input(&t2, "b", f(2.0));
    let x6 = g.node("stdlib::Multiply", (300.0, 420.0));
    g.link((&br, "x"), (&x6, "a"));
    g.input(&x6, "b", f(6.0));
    let ph = g.node("stdlib::Add", (450.0, 470.0));
    g.link((&x6, "out"), (&ph, "a"));
    g.link((&t2, "out"), (&ph, "b"));
    let sn = g.node("stdlib::Sin", (580.0, 470.0));
    g.link((&ph, "out"), (&sn, "x"));
    let fall = g.node("stdlib::Multiply", (700.0, 470.0));
    g.link((&sn, "out"), (&fall, "a"));
    g.link((&br, "x"), (&fall, "b"));
    let amp = g.node("stdlib::Multiply", (800.0, 400.0));
    g.link((&fall, "out"), (&amp, "a"));
    g.link((&strength, "out"), (&amp, "b"));
    let dir = g.node_p("stdlib::Constant3Vector", (700.0, 300.0), &[("value", Value::Float3([0.0, 0.0, 1.0]))]);
    let wpo = g.node("stdlib::Multiply", (850.0, 330.0));
    g.link((&dir, "out"), (&wpo, "a"));
    g.link((&amp, "out"), (&wpo, "b"));
    g.link((&wpo, "out"), (&out, "world_position_offset"));
    g.finish()
}

/// Translucent water: panning normal maps, Custom node, Blend opacity.
pub fn water() -> Asset {
    let kind = AssetKind::Material(MaterialKind::Pbr3d(PbrSettings { blend: BlendMode::Blend, ..Default::default() }));
    let mut g = GraphBuilder::new("water", kind);
    let out = g.node("output::Pbr3d", (1000.0, 0.0));
    let tex = g.node_p(
        "param::TextureObject",
        (0.0, 300.0),
        &[("name", s("normals")), ("default", s("textures/water_normal"))],
    );
    let deep = g.node_p(
        "param::Color",
        (0.0, 0.0),
        &[("name", s("deep_color")), ("default", Value::Color([0.0, 0.15, 0.3, 1.0]))],
    );
    let p1 = g.node("stdlib::Panner", (200.0, 250.0));
    g.input(&p1, "speed", Value::Float2([0.03, 0.01]));
    let p2 = g.node("stdlib::Panner", (200.0, 400.0));
    g.input(&p2, "speed", Value::Float2([-0.02, 0.025]));
    let s1 = g.node_p("stdlib::TextureSample", (400.0, 250.0), &[("sampler_type", s("Normal"))]);
    g.link((&tex, "out"), (&s1, "tex"));
    g.link((&p1, "out"), (&s1, "uv"));
    let s2 = g.node_p("stdlib::TextureSample", (400.0, 420.0), &[("sampler_type", s("Normal"))]);
    g.link((&tex, "out"), (&s2, "tex"));
    g.link((&p2, "out"), (&s2, "uv"));
    let sum = g.node("stdlib::Add", (600.0, 330.0));
    g.link((&s1, "rgb"), (&sum, "a"));
    g.link((&s2, "rgb"), (&sum, "b"));
    let n = g.node("stdlib::Normalize", (750.0, 330.0));
    g.link((&sum, "out"), (&n, "x"));
    g.link((&n, "out"), (&out, "normal"));
    g.link((&deep, "rgb"), (&out, "base_color"));
    let fres = g.node("stdlib::Fresnel", (400.0, 600.0));
    let custom = g.node_p(
        "stdlib::Custom",
        (600.0, 600.0),
        &[
            ("inputs", s("f: Float, k: Float")),
            ("output_type", Value::Type(PinType::Float)),
            ("code", s("let v = saturate(f * k);\nreturn 0.55 + 0.4 * v;")),
        ],
    );
    g.link((&fres, "out"), (&custom, "f"));
    g.input(&custom, "k", f(1.5));
    // Shoreline: fades out where the water meets the ground (needs `DepthPrepass` on the camera).
    let shore = g.node("stdlib::DepthFade", (800.0, 600.0));
    g.link((&custom, "out"), (&shore, "opacity"));
    g.input(&shore, "distance", f(0.3));
    g.link((&shore, "out"), (&out, "opacity"));
    let r = g.node_p("stdlib::Constant", (800.0, 150.0), &[("value", f(0.05))]);
    g.link((&r, "out"), (&out, "roughness"));
    let sp = g.node_p("stdlib::Constant", (800.0, 220.0), &[("value", f(0.8))]);
    g.link((&sp, "out"), (&out, "specular"));
    g.finish()
}

/// Coverage material: most Coordinates/Vector/Utility nodes in one graph.
pub fn coords() -> Asset {
    let kind = AssetKind::Material(MaterialKind::Unlit3d(UnlitSettings::default()));
    let mut g = GraphBuilder::new("coords", kind);
    let out = g.node("output::Unlit3d", (1200.0, 0.0));
    let atlas = g.node_p("param::TextureObject", (0.0, 0.0), &[("name", s("atlas")), ("default", s("textures/atlas"))]);
    let fb = g.node("stdlib::FlipBook", (200.0, 0.0));
    g.link((&atlas, "out"), (&fb, "tex"));
    let rot = g.node("stdlib::Rotator", (0.0, 200.0));
    let scale = g.node("stdlib::ScaleUVsByCenter", (200.0, 200.0));
    g.link((&rot, "out"), (&scale, "uv"));
    g.input(&scale, "scale", Value::Float2([2.0, 2.0]));
    g.link((&scale, "out"), (&fb, "uv"));
    let obj = g.node("stdlib::ObjectPosition", (0.0, 400.0));
    let osc = g.node("stdlib::ObjectScale", (0.0, 480.0));
    let cam = g.node("stdlib::CameraVectorWS", (0.0, 560.0));
    let depth = g.node("stdlib::PixelDepth", (0.0, 640.0));
    let vc = g.node("stdlib::VertexColor", (0.0, 720.0));
    let tv = g.node_p("stdlib::TransformVector", (200.0, 560.0), &[("from", s("World")), ("to", s("View"))]);
    g.link((&cam, "out"), (&tv, "v"));
    let d1 = g.node("stdlib::DotProduct", (400.0, 450.0));
    g.link((&obj, "out"), (&d1, "a"));
    g.link((&osc, "out"), (&d1, "b"));
    let mask = g.node_p(
        "stdlib::ComponentMask",
        (400.0, 560.0),
        &[("r", Value::Bool(true)), ("g", Value::Bool(false)), ("b", Value::Bool(true))],
    );
    g.link((&tv, "out"), (&mask, "x"));
    let len = g.node("stdlib::Length", (550.0, 560.0));
    g.link((&mask, "out"), (&len, "x"));
    let remap = g.node("stdlib::Remap", (550.0, 640.0));
    g.link((&depth, "out"), (&remap, "x"));
    g.input(&remap, "in_max", f(50.0));
    let iff = g.node("stdlib::If", (700.0, 500.0));
    g.link((&d1, "out"), (&iff, "a"));
    g.link((&len, "out"), (&iff, "b"));
    g.link((&remap, "out"), (&iff, "gt"));
    g.input(&iff, "eq", f(0.5));
    g.input(&iff, "lt", f(0.25));
    let tint = g.node("stdlib::Multiply", (900.0, 300.0));
    g.link((&fb, "rgb"), (&tint, "a"));
    g.link((&vc, "rgb"), (&tint, "b"));
    let bl = g.node_p("stdlib::Blend", (1000.0, 300.0), &[("mode", s("Overlay"))]);
    g.link((&tint, "out"), (&bl, "base"));
    g.link((&iff, "out"), (&bl, "blend"));
    let des = g.node("stdlib::Desaturation", (1100.0, 300.0));
    g.link((&bl, "out"), (&des, "color"));
    g.input(&des, "fraction", f(0.25));
    g.link((&des, "out"), (&out, "color"));
    g.finish()
}

/// Unlit bands shaped by two curve assets (decision 0015): Catmull-Rom `common/pulse` scrolling over V, Simple
/// `common/steps` over U.
pub fn curve_bands() -> Asset {
    let kind = AssetKind::Material(MaterialKind::Unlit3d(UnlitSettings::default()));
    let mut g = GraphBuilder::new("curve_bands", kind);
    let out = g.node("output::Unlit3d", (1000.0, 0.0));
    let uv = g.node("stdlib::TextureCoordinate", (0.0, 0.0));
    let br = g.node("stdlib::BreakOutFloat2", (150.0, 0.0));
    g.link((&uv, "out"), (&br, "v"));
    let time = g.node("stdlib::Time", (0.0, 150.0));
    let slow = g.node("stdlib::Multiply", (150.0, 150.0));
    g.link((&time, "out"), (&slow, "a"));
    g.input(&slow, "b", f(0.25));
    let scroll = g.node("stdlib::Add", (300.0, 100.0));
    g.link((&br, "y"), (&scroll, "a"));
    g.link((&slow, "out"), (&scroll, "b"));
    let frac = g.node("stdlib::Frac", (420.0, 100.0));
    g.link((&scroll, "out"), (&frac, "x"));
    let pulse = g.node_p("stdlib::SampleCurve", (550.0, 100.0), &[("curve", s("common/pulse"))]);
    g.link((&frac, "out"), (&pulse, "t"));
    let steps = g.node_p("stdlib::SampleCurve", (550.0, -50.0), &[("curve", s("common/steps")), ("type", s("Simple"))]);
    g.link((&br, "x"), (&steps, "t"));
    let mask = g.node("stdlib::Multiply", (700.0, 50.0));
    g.link((&pulse, "out"), (&mask, "a"));
    g.link((&steps, "out"), (&mask, "b"));
    let lerp = g.node("stdlib::Lerp", (850.0, 0.0));
    g.input(&lerp, "a", Value::Float3([0.05, 0.02, 0.1]));
    g.input(&lerp, "b", Value::Float3([1.0, 0.55, 0.1]));
    g.link((&mask, "out"), (&lerp, "alpha"));
    g.link((&lerp, "out"), (&out, "color"));
    g.finish()
}

/// Unlit gradient driven by the global constants of `common/tuning` (decision 0017): the base and glow colors,
/// the glow amount, stripes scaled by a vec2 and faded in by a bool, the striped branch picked by a compile flag.
pub fn global_tint() -> Asset {
    let kind = AssetKind::Material(MaterialKind::Unlit3d(UnlitSettings::default()));
    let mut g = GraphBuilder::new("global_tint", kind);
    let global = |name: &str| [("const", s(name))];
    let out = g.node("output::Unlit3d", (1100.0, 0.0));
    let base = g.node_p("stdlib::GlobalConstVec3", (0.0, 0.0), &global("base_color"));
    let glow = g.node_p("stdlib::GlobalConstVec4", (0.0, 100.0), &global("glow_color"));
    let rgb = g.node_p(
        "stdlib::ComponentMask",
        (150.0, 100.0),
        &[("r", Value::Bool(true)), ("g", Value::Bool(true)), ("b", Value::Bool(true)), ("a", Value::Bool(false))],
    );
    g.link((&glow, "out"), (&rgb, "x"));
    let amount = g.node_p("stdlib::GlobalConstF32", (0.0, 200.0), &global("glow_amount"));
    let color = g.node("stdlib::Lerp", (300.0, 50.0));
    g.link((&base, "out"), (&color, "a"));
    g.link((&rgb, "out"), (&color, "b"));
    g.link((&amount, "out"), (&color, "alpha"));
    let uv = g.node("stdlib::TextureCoordinate", (0.0, 350.0));
    let scale = g.node_p("stdlib::GlobalConstVec2", (0.0, 450.0), &global("stripe_scale"));
    let scaled = g.node("stdlib::Multiply", (150.0, 400.0));
    g.link((&uv, "out"), (&scaled, "a"));
    g.link((&scale, "out"), (&scaled, "b"));
    let br = g.node("stdlib::BreakOutFloat2", (300.0, 400.0));
    g.link((&scaled, "out"), (&br, "v"));
    let frac = g.node("stdlib::Frac", (450.0, 400.0));
    g.link((&br, "x"), (&frac, "x"));
    let on = g.node_p("stdlib::GlobalBool", (450.0, 500.0), &global("use_stripes"));
    let fade = g.node("stdlib::Lerp", (600.0, 400.0));
    g.input(&fade, "a", f(1.0));
    g.link((&frac, "out"), (&fade, "b"));
    g.link((&on, "out"), (&fade, "alpha"));
    let striped = g.node("stdlib::Multiply", (750.0, 200.0));
    g.link((&color, "out"), (&striped, "a"));
    g.link((&fade, "out"), (&striped, "b"));
    let flag = g.node_p("stdlib::GlobalCompileFlag", (750.0, 50.0), &global("fancy_stripes"));
    let pick = g.node("stdlib::BoolSelect", (900.0, 100.0));
    g.link((&flag, "out"), (&pick, "cond"));
    g.link((&striped, "out"), (&pick, "true_value"));
    g.link((&color, "out"), (&pick, "false_value"));
    g.link((&pick, "out"), (&out, "color"));
    g.finish()
}

/// Function returning a whole PBR layer as `MaterialAttributes (PBR)` (decision 0023): a tinted rough rock.
pub fn rock_layer() -> Asset {
    let mut g = GraphBuilder::new("rock_layer", AssetKind::Function);
    let tint = g.node_p(
        "output::FunctionInput",
        (0.0, 0.0),
        &[
            ("name", s("tint")),
            ("type", Value::Type(PinType::Float3)),
            ("default", Value::Float4([0.45; 4])),
            ("order", Value::Int(0)),
        ],
    );
    let make = g.node("stdlib::MakeMaterialAttributesPbr", (250.0, 0.0));
    g.link((&tint, "out"), (&make, "base_color"));
    g.input(&make, "roughness", f(0.9));
    g.input(&make, "normal", Value::Float3([0.1, 0.0, 1.0]));
    let out = g.node_p(
        "output::FunctionOutput",
        (500.0, 0.0),
        &[("name", s("layer")), ("type", Value::Type(PinType::MaterialAttributesPbr)), ("order", Value::Int(0))],
    );
    g.link((&make, "attributes"), (&out, "value"));
    g.finish()
}

/// Layered terrain (decision 0023): Pbr3d `Wrapped`, grass on flat ground and the `common/rock_layer` function on
/// slopes, blended by `WorldAlignedBlend` through one `Lerp` of material attributes; WPO lifts the surface a bit.
/// The grass is tinted by height (decision 0037): the "Terrain height" gradient of `common/terrain` sampled at
/// `saturate(world_position.y * 0.25 + 0.5)`.
pub fn terrain_layers() -> Asset {
    let kind =
        AssetKind::Material(MaterialKind::Pbr3d(PbrSettings { output: PbrOutputMode::Wrapped, ..Default::default() }));
    let mut g = GraphBuilder::new("terrain_layers", kind);
    let out = g.node("output::Pbr3d", (900.0, 0.0));
    let grass_color =
        g.node_p("stdlib::Constant3Vector", (-150.0, -150.0), &[("value", Value::Float3([0.15, 0.45, 0.1]))]);
    let grass = g.node("stdlib::MakeMaterialAttributesPbr", (250.0, -150.0));
    g.input(&grass, "roughness", f(0.7));
    let rock_color = g.node_p("stdlib::Constant3Vector", (0.0, 100.0), &[("value", Value::Float3([0.4, 0.38, 0.35]))]);
    let rock = g.node("function::common/rock_layer", (250.0, 100.0));
    g.link((&rock_color, "out"), (&rock, "tint"));
    let slope = g.node("stdlib::WorldAlignedBlend", (250.0, 300.0));
    let blend = g.node("stdlib::Lerp", (600.0, 0.0));
    g.link((&rock, "layer"), (&blend, "a"));
    g.link((&grass, "attributes"), (&blend, "b"));
    g.link((&slope, "mask"), (&blend, "alpha"));
    g.link((&blend, "out"), (&out, "attributes"));
    let lift = g.node_p("stdlib::Constant3Vector", (600.0, 250.0), &[("value", Value::Float3([0.0, 0.02, 0.0]))]);
    g.link((&lift, "out"), (&out, shader_graph_core::ir::WPO_PIN));
    // Height tint: T = saturate(world_position.y * 0.25 + 0.5) picks a color of `common/terrain` slot 0.
    let wp = g.node("stdlib::WorldPosition", (-750.0, -350.0));
    let br = g.node("stdlib::BreakOutFloat3", (-600.0, -350.0));
    g.link((&wp, "out"), (&br, "v"));
    let scale = g.node("stdlib::Multiply", (-450.0, -350.0));
    g.link((&br, "y"), (&scale, "a"));
    g.input(&scale, "b", f(0.25));
    let offset = g.node("stdlib::Add", (-300.0, -350.0));
    g.link((&scale, "out"), (&offset, "a"));
    g.input(&offset, "b", f(0.5));
    let height = g.node("stdlib::Saturate", (-150.0, -350.0));
    g.link((&offset, "out"), (&height, "x"));
    let terrain = &gradients()[0].1;
    let gradient = g.node_p(
        SAMPLE_GRADIENT_KIND,
        (0.0, -350.0),
        &[
            (GRADIENT_COLLECTION_PARAM, s("common/terrain")),
            (GRADIENT_SLOT_PARAM, Value::Int(terrain.slots[0].id as i32)),
        ],
    );
    g.link((&height, "out"), (&gradient, "t"));
    let tint = g.node("stdlib::Multiply", (100.0, -150.0));
    g.link((&grass_color, "out"), (&tint, "a"));
    g.link((&gradient, "rgb"), (&tint, "b"));
    g.link((&tint, "out"), (&grass, "base_color"));
    g.finish()
}

/// Function with a fixed texture (design §1.3): `textures/rock_albedo` sampled at the mesh UV, times `tint`. The
/// texture becomes a hidden trailing input of the function (`ft_common_tex_layer_n02`), after the builtin `b_uv0`.
pub fn tex_layer() -> Asset {
    let mut g = GraphBuilder::new("tex_layer", AssetKind::Function);
    let tint = g.node_p(
        "output::FunctionInput",
        (0.0, 0.0),
        &[
            ("name", s("tint")),
            ("type", Value::Type(PinType::Float3)),
            ("default", Value::Float4([1.0; 4])),
            ("order", Value::Int(0)),
        ],
    );
    let sample = g.node_p("stdlib::TextureSample", (0.0, 150.0), &[("texture", s("textures/rock_albedo"))]);
    let mul = g.node("stdlib::Multiply", (300.0, 50.0));
    g.link((&sample, "rgb"), (&mul, "a"));
    g.link((&tint, "out"), (&mul, "b"));
    let out = g.node_p(
        "output::FunctionOutput",
        (550.0, 50.0),
        &[("name", s("rgb")), ("type", Value::Type(PinType::Float3)), ("order", Value::Int(0))],
    );
    g.link((&mul, "out"), (&out, "value"));
    g.finish()
}

/// Function calling `common/tex_layer`: forwards its hidden texture as a hidden input of its own (design §1.3).
pub fn tex_layer_outer() -> Asset {
    let mut g = GraphBuilder::new("tex_layer_outer", AssetKind::Function);
    let scale = g.node_p(
        "output::FunctionInput",
        (0.0, 0.0),
        &[
            ("name", s("scale")),
            ("type", Value::Type(PinType::Float)),
            ("default", Value::Float4([0.5; 4])),
            ("order", Value::Int(0)),
        ],
    );
    let layer = g.node("function::common/tex_layer", (0.0, 150.0));
    let mul = g.node("stdlib::Multiply", (300.0, 50.0));
    g.link((&layer, "rgb"), (&mul, "a"));
    g.link((&scale, "out"), (&mul, "b"));
    let out = g.node_p(
        "output::FunctionOutput",
        (550.0, 50.0),
        &[("name", s("rgb")), ("type", Value::Type(PinType::Float3)), ("order", Value::Int(0))],
    );
    g.link((&mul, "out"), (&out, "value"));
    g.finish()
}

/// Material calling `common/tex_layer` twice and `common/tex_layer_outer` once (Review Focus 2): the function's fixed
/// texture is one material binding (per origin node, not per call), passed at all three call sites.
pub fn tex_layers() -> Asset {
    let mut g = GraphBuilder::new("tex_layers", AssetKind::Material(MaterialKind::Pbr3d(PbrSettings::default())));
    let out = g.node("output::Pbr3d", (900.0, 0.0));
    let warm = g.node("function::common/tex_layer", (0.0, -150.0));
    g.input(&warm, "tint", Value::Float3([1.0, 0.8, 0.6]));
    let cold = g.node("function::common/tex_layer", (0.0, 50.0));
    g.input(&cold, "tint", Value::Float3([0.6, 0.8, 1.0]));
    let outer = g.node("function::common/tex_layer_outer", (0.0, 250.0));
    g.input(&outer, "scale", f(0.25));
    let mix = g.node("stdlib::Lerp", (300.0, -50.0));
    g.link((&warm, "rgb"), (&mix, "a"));
    g.link((&cold, "rgb"), (&mix, "b"));
    g.input(&mix, "alpha", f(0.5));
    let add = g.node("stdlib::Add", (600.0, 50.0));
    g.link((&mix, "out"), (&add, "a"));
    g.link((&outer, "rgb"), (&add, "b"));
    g.link((&add, "out"), (&out, "base_color"));
    g.finish()
}

/// Function whose name and ports are Rust/WGSL keywords (decision 0020): `out = in * 2`, `let = in`.
pub fn keyword_fn() -> Asset {
    let mut g = GraphBuilder::new("fn", AssetKind::Function);
    let input = g.node_p(
        "output::FunctionInput",
        (0.0, 0.0),
        &[("name", s("in")), ("type", Value::Type(PinType::Float)), ("order", Value::Int(0))],
    );
    let twice = g.node("stdlib::Multiply", (200.0, 0.0));
    g.link((&input, "out"), (&twice, "a"));
    g.input(&twice, "b", f(2.0));
    let out = g.node_p(
        "output::FunctionOutput",
        (400.0, 0.0),
        &[("name", s("out")), ("type", Value::Type(PinType::Float)), ("order", Value::Int(0))],
    );
    g.link((&twice, "out"), (&out, "value"));
    let pass = g.node_p(
        "output::FunctionOutput",
        (400.0, 120.0),
        &[("name", s("let")), ("type", Value::Type(PinType::Float)), ("order", Value::Int(1))],
    );
    g.link((&input, "out"), (&pass, "value"));
    g.finish()
}

/// Unlit material `kw/loop/type` with keyword names everywhere (decision 0020): parameters `self` and `mesh`,
/// a call of `kw/fn`.
pub fn keyword_names() -> Asset {
    let kind = AssetKind::Material(MaterialKind::Unlit3d(UnlitSettings::default()));
    let mut g = GraphBuilder::new("type", kind);
    let out = g.node("output::Unlit3d", (800.0, 0.0));
    let this = g.node_p("param::Scalar", (0.0, 0.0), &[("name", s("self")), ("default", f(0.5))]);
    let mesh =
        g.node_p("param::Color", (0.0, 150.0), &[("name", s("mesh")), ("default", Value::Color([0.9, 0.4, 0.1, 1.0]))]);
    let call = g.node("function::kw/fn", (250.0, 0.0));
    g.link((&this, "out"), (&call, "in"));
    let color = g.node("stdlib::Multiply", (550.0, 50.0));
    g.link((&mesh, "rgb"), (&color, "a"));
    g.link((&call, "out"), (&color, "b"));
    g.link((&color, "out"), (&out, "color"));
    g.finish()
}

/// Global constant table fixtures: (`quillshade/material/`-relative path, table).
pub fn const_tables() -> Vec<(&'static str, GlobalConsts)> {
    let c = GlobalConst::new;
    vec![(
        "common/tuning",
        GlobalConsts::new(
            "tuning",
            vec![
                c("base_color", ConstType::Vec3, Value::Float3([0.05, 0.1, 0.3])),
                c("glow_color", ConstType::Vec4, Value::Float4([1.0, 0.5, 0.1, 1.0])),
                c("glow_amount", ConstType::F32, Value::Float(0.35)),
                c("stripe_scale", ConstType::Vec2, Value::Float2([8.0, 1.0])),
                c("use_stripes", ConstType::Bool, Value::Bool(true)),
                c("fancy_stripes", ConstType::CompileFlag, Value::Bool(true)),
            ],
        ),
    )]
}

/// Curve fixtures: (`quillshade/material/`-relative path, curve).
pub fn curves() -> Vec<(&'static str, Curve)> {
    vec![
        ("common/pulse", Curve::new("pulse", vec![(0.0, 0.0), (0.25, 1.0), (0.5, 0.3), (0.75, 0.1), (1.0, 0.0)])),
        ("common/steps", Curve::new("steps", vec![(0.0, 0.2), (0.33, 0.2), (0.34, 1.0), (0.66, 1.0), (0.67, 0.5)])),
    ]
}

/// Gradient collection fixtures (decision 0037): (`quillshade/material/`-relative path, collection). Slot ids come
/// from a fixed sequence, so `make_sample` is reproducible. Colors are given as sRGB hex and stored linear.
pub fn gradients() -> Vec<(&'static str, GradientCollection)> {
    let rgb = |hex: u32| {
        let c = |shift: u32| srgb_to_linear(((hex >> shift) & 0xff) as f32 / 255.0);
        [c(16), c(8), c(0), 1.0]
    };
    let stop = |t: f32, hex: u32| Stop { t, color: rgb(hex), mid: 0.5 };
    let mut next = 1000;
    let mut terrain = GradientCollection::new(|| {
        next += 7;
        next
    });
    let height = &mut terrain.slots[0];
    height.name = "Terrain height".into();
    height.stops =
        vec![stop(0.0, 0x2e6b2e), stop(0.45, 0x8a6a3c), Stop { mid: 0.3, ..stop(0.8, 0xe8e8e8) }, stop(1.0, 0xffffff)];
    let water = &mut terrain.slots[1];
    water.name = "Water depth".into();
    water.stops = vec![stop(0.0, 0x7fd4e8), stop(0.5, 0x1f78b4), stop(1.0, 0x0a1f4d)];
    vec![("common/terrain", terrain)]
}

/// Texture asset fixtures (design §1.1): (`quillshade/material/`-relative path, asset). The images are in
/// `examples/sample_game/assets/textures/`; `rock_albedo` and `water_normal` are `.ktx2` (made from the `.png` next to
/// them by the `png_to_ktx2` example, design §4.4), the rest `.png`, so the sample exercises both formats.
pub fn textures() -> Vec<(&'static str, TextureAsset)> {
    let linear_repeat = SamplerSettings {
        address_mode_u: AddressMode::Repeat,
        address_mode_v: AddressMode::Repeat,
        address_mode_w: AddressMode::Repeat,
        mag_filter: FilterMode::Linear,
        min_filter: FilterMode::Linear,
        mipmap_filter: FilterMode::Linear,
        ..Default::default()
    };
    let nearest =
        SamplerSettings { mag_filter: FilterMode::Nearest, min_filter: FilterMode::Nearest, ..Default::default() };
    let with = |path: &str, settings: TextureSettings| TextureAsset { settings, ..TextureAsset::new(path) };
    let linear_data = TextureSettings { is_srgb: false, ..Default::default() };
    vec![
        ("textures/rock_albedo", TextureAsset::new("textures/rock_albedo.ktx2")),
        ("textures/rock_normal", with("textures/rock_normal.png", linear_data.clone())),
        (
            "textures/water_normal",
            with("textures/water_normal.ktx2", TextureSettings { sampler: Some(linear_repeat), ..linear_data }),
        ),
        (
            "textures/atlas",
            with("textures/atlas.png", TextureSettings { sampler: Some(nearest), ..Default::default() }),
        ),
        ("textures/sprite", TextureAsset::new("textures/sprite.png")),
    ]
}

/// Writes every fixture (graphs, curves, constant tables, Texture assets, gradient collections) into `project`.
pub fn save_all(project: &Project) -> Result<(), OpError> {
    for (path, texture) in textures() {
        ops::save_texture(project, &AssetId::new(AssetType::Texture, path), &texture)?;
    }
    for (path, is_fn, asset) in all() {
        let id = AssetId::new(if is_fn { AssetType::Function } else { AssetType::Material }, path);
        ops::save(project, &id, &asset)?;
    }
    for (path, curve) in curves() {
        ops::save_curve(project, &AssetId::new(AssetType::Curve, path), &curve)?;
    }
    for (path, table) in const_tables() {
        ops::save_consts(project, &AssetId::new(AssetType::Consts, path), &table)?;
    }
    for (path, collection) in gradients() {
        ops::save_gradients(project, &AssetId::new(AssetType::Gradients, path), &collection)?;
    }
    Ok(())
}

/// All fixture assets: (`quillshade/material/`-relative path, is_function, asset).
pub fn all() -> Vec<(&'static str, bool, Asset)> {
    vec![
        ("common/wet_mask", true, wet_mask()),
        ("common/rock_layer", true, rock_layer()),
        ("common/tex_layer", true, tex_layer()),
        ("common/tex_layer_outer", true, tex_layer_outer()),
        ("env/rock_wet", false, rock_wet()),
        ("env/tex_layers", false, tex_layers()),
        ("env/flag", false, flag()),
        ("env/water", false, water()),
        ("env/terrain_layers", false, terrain_layers()),
        ("fx/hologram", false, hologram()),
        ("fx/coords", false, coords()),
        ("fx/curve_bands", false, curve_bands()),
        ("fx/global_tint", false, global_tint()),
        ("ui/dissolve", false, dissolve()),
        ("kw/fn", true, keyword_fn()),
        ("kw/loop/type", false, keyword_names()),
    ]
}
