//! Compile-time boolean flags (`CompileBool`), runtime booleans (`RuntimeBool`), numeric toggles (`Toggle`),
//! dead-branch pruning in `stdlib::If`, and `build_ir_with_overrides`.

use std::collections::{BTreeMap, BTreeSet};

use shader_graph_core::consts::{ConstType, GlobalConst, GlobalConsts, NoConsts};
use shader_graph_core::ir::{build_ir, build_ir_with_overrides};
use shader_graph_core::model::{Asset, AssetKind, MaterialKind, Node, PbrSettings};
use shader_graph_core::registry::{
    Builtin, FnPort, FunctionSig, NoCurves, NoFunctions, NoGradients, NoTextures, ROUTE_PARAM,
};
use shader_graph_core::{Code, Diagnostic, Ir, IrResult, PinType, Value};

fn node(a: &mut Asset, id: &str, kind: &str, params: &[(&str, Value)]) -> String {
    let mut n = Node::new(kind, (0.0, 0.0));
    for (k, v) in params {
        n.params.insert(k.to_string(), v.clone());
    }
    a.nodes.insert(id.to_string(), n);
    id.to_string()
}

fn pbr() -> Asset {
    Asset::new("mat", AssetKind::Material(MaterialKind::Pbr3d(PbrSettings::default())))
}

fn build(a: &Asset) -> (Ir, Vec<Diagnostic>) {
    let reg = shader_stdlib::registry();
    let r = build_ir(a, "test/mat", &reg, &NoFunctions, &NoCurves, &NoConsts, &NoTextures, &NoGradients);
    let errors: Vec<_> = r.diagnostics.iter().filter(|d| d.is_error()).collect();
    assert!(errors.is_empty(), "{errors:?}");
    (r.ir.expect("ir"), r.diagnostics)
}

#[test]
fn compile_if_direct_assignment_without_select_or_intermediate_let() {
    let mut a = pbr();
    let out = node(&mut a, "out", "output::Pbr3d", &[]);
    let flag = node(&mut a, "flag", "stdlib::CompileBool", &[("value", Value::Bool(true))]);
    let red = node(&mut a, "red", "stdlib::Constant3Vector", &[("value", Value::Float3([1.0, 0.0, 0.0]))]);
    let blue = node(&mut a, "blue", "stdlib::Constant3Vector", &[("value", Value::Float3([0.0, 0.0, 1.0]))]);
    let ifnode = node(&mut a, "ifnode", "stdlib::If", &[]);
    a.connect((&flag, "out"), (&ifnode, "cond"));
    a.connect((&red, "out"), (&ifnode, "true_value"));
    a.connect((&blue, "out"), (&ifnode, "false_value"));
    a.connect((&ifnode, "out"), (&out, "base_color"));

    let (ir, diags) = build(&a);
    assert!(diags.is_empty(), "{diags:?}");
    assert!(ir.main.lines.is_empty(), "no select() or intermediate let should be emitted: {:?}", ir.main.lines);
    assert_eq!(ir.main.outputs.get("base_color").map(String::as_str), Some("vec3<f32>(1.0, 0.0, 0.0)"));
}

#[test]
fn compile_if_prunes_dead_branch_params_textures_functions_builtins_and_w101() {
    let mut a = pbr();
    let out = node(&mut a, "out", "output::Pbr3d", &[]);
    let flag = node(&mut a, "flag", "stdlib::CompileBool", &[("value", Value::Bool(false))]);
    let ifnode = node(&mut a, "ifnode", "stdlib::If", &[]);

    // True branch (inactive because flag == false): uses a scalar param, a texture sample param, a function call, and Time.
    let p_dead = node(&mut a, "pdead", "param::Scalar", &[("name", Value::Str("dead_scalar".into()))]);
    let tex_dead = node(
        &mut a,
        "texdead",
        "param::TextureSample2D",
        &[("name", Value::Str("dead_tex".into())), ("default", Value::Str("textures/rock".into()))],
    );
    let fn_dead = node(&mut a, "fndead", "function::common/heavy", &[]);
    let time_dead = node(&mut a, "timedead", "stdlib::Time", &[]);
    let mul1 = node(&mut a, "mul1", "stdlib::Multiply", &[]);
    let mul2 = node(&mut a, "mul2", "stdlib::Multiply", &[]);
    let mul3 = node(&mut a, "mul3", "stdlib::Multiply", &[]);
    a.connect((&p_dead, "out"), (&mul1, "a"));
    a.connect((&time_dead, "out"), (&mul1, "b"));
    a.connect((&mul1, "out"), (&mul2, "a"));
    a.connect((&fn_dead, "res"), (&mul2, "b"));
    a.connect((&tex_dead, "rgb"), (&mul3, "a"));
    a.connect((&mul2, "out"), (&mul3, "b"));

    // False branch (active): uses a single constant vector.
    let green = node(&mut a, "green", "stdlib::Constant3Vector", &[("value", Value::Float3([0.0, 1.0, 0.0]))]);

    // Truly disconnected node: must still emit W101.
    let orphan = node(&mut a, "orphan", "stdlib::Constant", &[("value", Value::Float(42.0))]);

    a.connect((&flag, "out"), (&ifnode, "cond"));
    a.connect((&mul3, "out"), (&ifnode, "true_value"));
    a.connect((&green, "out"), (&ifnode, "false_value"));
    a.connect((&ifnode, "out"), (&out, "base_color"));

    let mut fns = BTreeMap::new();
    fns.insert(
        "common/heavy".to_string(),
        FunctionSig {
            path: "common/heavy".into(),
            name: "heavy".into(),
            inputs: vec![],
            outputs: vec![FnPort {
                name: "res".into(),
                ty: PinType::Float,
                default: Value::Float(0.0),
                required: false,
                range: None,
            }],
            builtins: vec![Builtin::WorldPosition],
            textures: vec![],
            gradients: vec![],
            scene: vec![],
        },
    );
    let textures = BTreeSet::from(["textures/rock".to_string()]);
    let reg = shader_stdlib::registry();
    let res = build_ir(&a, "test/mat", &reg, &fns, &NoCurves, &NoConsts, &textures, &NoGradients);
    let errors: Vec<_> = res.diagnostics.iter().filter(|d| d.is_error()).collect();
    assert!(errors.is_empty(), "{errors:?}");
    let ir = res.ir.expect("ir");

    // Dead branch resources must be completely absent from Ir.
    assert!(ir.params.is_empty(), "dead branch params must be pruned: {:?}", ir.params);
    assert!(ir.textures.is_empty(), "dead branch textures must be pruned: {:?}", ir.textures);
    assert!(ir.functions.is_empty(), "dead branch functions must be pruned: {:?}", ir.functions);
    assert!(ir.all_builtins().is_empty(), "dead branch builtins must be pruned: {:?}", ir.all_builtins());
    assert!(ir.main.lines.is_empty(), "dead branch lines must be pruned: {:?}", ir.main.lines);
    assert_eq!(ir.main.outputs.get("base_color").map(String::as_str), Some("vec3<f32>(0.0, 1.0, 0.0)"));

    // Only `orphan` should trigger W101; inactive branch nodes must NOT trigger W101.
    let w101: Vec<&str> =
        res.diagnostics.iter().filter(|d| d.code == Code::W101).filter_map(|d| d.node.as_deref()).collect();
    assert_eq!(w101, [orphan.as_str()]);
}

#[test]
fn compile_if_reports_type_and_required_pin_diagnostics_in_inactive_branch() {
    let mut a = pbr();
    let out = node(&mut a, "out", "output::Pbr3d", &[]);
    let flag = node(&mut a, "flag", "stdlib::CompileBool", &[("value", Value::Bool(true))]);
    let ifnode = node(&mut a, "ifnode", "stdlib::If", &[]);
    let red = node(&mut a, "red", "stdlib::Constant3Vector", &[("value", Value::Float3([1.0, 0.0, 0.0]))]);

    // Inactive branch (`false_value`): `TextureSize` without its required `tex` pin connected (E004),
    // and a `RuntimeBool` wired into `Add.b` (E003).
    let tsize = node(&mut a, "tsize", "stdlib::TextureSize", &[]);
    let rbool = node(&mut a, "rbool", "stdlib::RuntimeBool", &[("value", Value::Bool(true))]);
    let bad_add = node(&mut a, "badadd", "stdlib::Add", &[]);
    a.connect((&tsize, "out"), (&bad_add, "a"));
    a.connect((&rbool, "out"), (&bad_add, "b"));
    a.connect((&flag, "out"), (&ifnode, "cond"));
    a.connect((&red, "out"), (&ifnode, "true_value"));
    a.connect((&bad_add, "out"), (&ifnode, "false_value"));
    a.connect((&ifnode, "out"), (&out, "base_color"));

    let reg = shader_stdlib::registry();
    let res: IrResult = build_ir(&a, "test/mat", &reg, &NoFunctions, &NoCurves, &NoConsts, &NoTextures, &NoGradients);
    assert!(
        res.diagnostics.iter().any(|d| d.code == Code::E004 && d.node.as_deref() == Some("tsize")),
        "missing required pin in inactive branch must still report E004: {:?}",
        res.diagnostics
    );
    assert!(
        res.diagnostics.iter().any(|d| d.code == Code::E003 && d.node.as_deref() == Some("badadd")),
        "incompatible edge in inactive branch must still report E003: {:?}",
        res.diagnostics
    );
}

#[test]
fn compile_logic_global_compile_bool_and_routes_forward_compile_bool() {
    let mut a = pbr();
    let out = node(&mut a, "out", "output::Pbr3d", &[]);
    let g_flag = node(
        &mut a,
        "gflag",
        "stdlib::GlobalCompileBool",
        &[("table", Value::Str("tuning".into())), ("const", Value::Int(1))],
    );
    let p_flag = node(
        &mut a,
        "pflag",
        "param::CompileBool",
        &[("name", Value::Str("allow_effect".into())), ("default", Value::Bool(false))],
    );
    let c_not = node(&mut a, "cnot", "stdlib::CompileNot", &[]);
    let c_and = node(&mut a, "cand", "stdlib::CompileAnd", &[]);
    let c_false = node(&mut a, "cfalse", "stdlib::CompileBool", &[("value", Value::Bool(false))]);
    let c_or = node(&mut a, "cor", "stdlib::CompileOr", &[]);
    let reroute = node(&mut a, "reroute", "stdlib::Reroute", &[]);
    let decl = node(&mut a, "decl", "stdlib::NamedRoute", &[("name", Value::Str("cond_route".into()))]);
    let usage = node(&mut a, "usage", "stdlib::NamedRouteUsage", &[(ROUTE_PARAM, Value::Str("decl".into()))]);
    let ifnode = node(&mut a, "ifnode", "stdlib::If", &[]);
    let snow = node(&mut a, "snow", "stdlib::Constant3Vector", &[("value", Value::Float3([0.9, 0.95, 1.0]))]);
    let rock = node(&mut a, "rock", "stdlib::Constant3Vector", &[("value", Value::Float3([0.2, 0.2, 0.2]))]);

    // Expression: (use_snow && !allow_effect) || false -> Reroute -> NamedRoute -> NamedRouteUsage -> If.cond
    a.connect((&p_flag, "out"), (&c_not, "a"));
    a.connect((&g_flag, "out"), (&c_and, "a"));
    a.connect((&c_not, "out"), (&c_and, "b"));
    a.connect((&c_and, "out"), (&c_or, "a"));
    a.connect((&c_false, "out"), (&c_or, "b"));
    a.connect((&c_or, "out"), (&reroute, "in"));
    a.connect((&reroute, "out"), (&decl, "in"));
    a.connect((&usage, "out"), (&ifnode, "cond"));
    a.connect((&snow, "out"), (&ifnode, "true_value"));
    a.connect((&rock, "out"), (&ifnode, "false_value"));
    a.connect((&ifnode, "out"), (&out, "base_color"));

    let consts = BTreeMap::from([(
        "tuning".to_string(),
        GlobalConsts::new(
            "tuning",
            vec![GlobalConst::with_id(1, "use_snow", ConstType::CompileBool, Value::Bool(true))],
        ),
    )]);
    let reg = shader_stdlib::registry();

    // Default `allow_effect = false` -> `!allow_effect = true` -> `use_snow && true = true` -> snow branch selected.
    let res_default = build_ir(&a, "test/mat", &reg, &NoFunctions, &NoCurves, &consts, &NoTextures, &NoGradients);
    assert!(!res_default.has_errors(), "{:?}", res_default.diagnostics);
    let ir_default = res_default.ir.expect("ir_default");
    assert!(ir_default.main.lines.is_empty(), "{:?}", ir_default.main.lines);
    assert_eq!(ir_default.main.outputs.get("base_color").map(String::as_str), Some("vec3<f32>(0.9, 0.95, 1.0)"));
    assert_eq!(ir_default.compile_flags, BTreeMap::from([("allow_effect".to_string(), false)]));

    // Override `allow_effect = true` -> `!allow_effect = false` -> rock branch selected, and layout_feature changes.
    let overrides = BTreeMap::from([("allow_effect".to_string(), true)]);
    let res_overridden = build_ir_with_overrides(
        &a,
        "test/mat",
        &reg,
        &NoFunctions,
        &NoCurves,
        &consts,
        &NoTextures,
        &NoGradients,
        &overrides,
    );
    assert!(!res_overridden.has_errors(), "{:?}", res_overridden.diagnostics);
    let ir_overridden = res_overridden.ir.expect("ir_overridden");
    assert!(ir_overridden.main.lines.is_empty(), "{:?}", ir_overridden.main.lines);
    assert_eq!(ir_overridden.main.outputs.get("base_color").map(String::as_str), Some("vec3<f32>(0.2, 0.2, 0.2)"));
    assert_eq!(ir_overridden.compile_flags, BTreeMap::from([("allow_effect".to_string(), true)]));
    assert_ne!(ir_default.layout_feature(), ir_overridden.layout_feature());
}

#[test]
fn runtime_bool_and_toggle_nodes_emit_expected_wesl() {
    let mut a = pbr();
    let out = node(&mut a, "out", "output::Pbr3d", &[]);
    let toggle = node(&mut a, "tog", "stdlib::Toggle", &[("value", Value::Bool(true))]);
    let gtog = node(
        &mut a,
        "gtog",
        "stdlib::GlobalToggle",
        &[("table", Value::Str("tuning".into())), ("const", Value::Int(1))],
    );
    let mul = node(&mut a, "mul", "stdlib::Multiply", &[]);
    a.connect((&toggle, "out"), (&mul, "a"));
    a.connect((&gtog, "out"), (&mul, "b"));

    let rbool = node(&mut a, "rb", "stdlib::RuntimeBool", &[("value", Value::Bool(true))]);
    let grbool = node(
        &mut a,
        "grb",
        "stdlib::GlobalRuntimeBool",
        &[("table", Value::Str("tuning".into())), ("const", Value::Int(2))],
    );
    let prbool = node(&mut a, "prb", "param::RuntimeBool", &[("name", Value::Str("active".into()))]);
    let cmp = node(&mut a, "cmp", "stdlib::Compare", &[("op", Value::Str(">=".into()))]);
    let r_and = node(&mut a, "rand", "stdlib::And", &[]);
    let r_or = node(&mut a, "ror", "stdlib::Or", &[]);
    let r_not = node(&mut a, "rnot", "stdlib::Not", &[]);
    let r_if = node(&mut a, "rif", "stdlib::RuntimeIf", &[]);
    let r_cmp_if = node(&mut a, "rcmpif", "stdlib::RuntimeCompareIf", &[]);

    a.connect((&mul, "out"), (&cmp, "a"));
    a.connect((&rbool, "out"), (&r_and, "a"));
    a.connect((&grbool, "out"), (&r_and, "b"));
    a.connect((&r_and, "out"), (&r_or, "a"));
    a.connect((&prbool, "out"), (&r_or, "b"));
    a.connect((&cmp, "out"), (&r_not, "a"));
    a.connect((&r_not, "out"), (&r_if, "cond"));
    a.connect((&mul, "out"), (&r_if, "true_value"));
    a.connect((&r_if, "out"), (&r_cmp_if, "gt"));
    a.connect((&r_or, "out"), (&r_if, "cond"));
    // Fix single connection on r_if.cond: connect r_not -> r_or.b, and prbool -> r_and.a
    a.edges.clear();
    a.connect((&toggle, "out"), (&mul, "a"));
    a.connect((&gtog, "out"), (&mul, "b"));
    a.connect((&mul, "out"), (&cmp, "a"));
    a.connect((&rbool, "out"), (&r_and, "a"));
    a.connect((&grbool, "out"), (&r_and, "b"));
    a.connect((&prbool, "out"), (&r_not, "a"));
    a.connect((&r_and, "out"), (&r_or, "a"));
    a.connect((&r_not, "out"), (&r_or, "b"));
    a.connect((&r_or, "out"), (&r_if, "cond"));
    a.connect((&mul, "out"), (&r_if, "true_value"));
    a.connect((&cmp, "out"), (&r_not, "a"));
    // Keep all nodes connected cleanly:
    a.edges.clear();
    a.connect((&toggle, "out"), (&mul, "a"));
    a.connect((&gtog, "out"), (&mul, "b"));
    a.connect((&mul, "out"), (&cmp, "a"));
    a.connect((&rbool, "out"), (&r_and, "a"));
    a.connect((&grbool, "out"), (&r_and, "b"));
    a.connect((&r_and, "out"), (&r_or, "a"));
    a.connect((&cmp, "out"), (&r_or, "b"));
    a.connect((&prbool, "out"), (&r_not, "a"));
    let r_and2 = node(&mut a, "rand2", "stdlib::And", &[]);
    a.connect((&r_or, "out"), (&r_and2, "a"));
    a.connect((&r_not, "out"), (&r_and2, "b"));
    a.connect((&r_and2, "out"), (&r_if, "cond"));
    a.connect((&mul, "out"), (&r_if, "true_value"));
    a.connect((&r_if, "out"), (&r_cmp_if, "gt"));
    a.connect((&r_cmp_if, "out"), (&out, "roughness"));

    let consts = BTreeMap::from([(
        "tuning".to_string(),
        GlobalConsts::new(
            "tuning",
            vec![
                GlobalConst::with_id(1, "boost", ConstType::Toggle, Value::Bool(true)),
                GlobalConst::with_id(2, "invert", ConstType::RuntimeBool, Value::Bool(false)),
            ],
        ),
    )]);
    let reg = shader_stdlib::registry();
    let res = build_ir(&a, "test/mat", &reg, &NoFunctions, &NoCurves, &consts, &NoTextures, &NoGradients);
    assert!(!res.has_errors(), "{:?}", res.diagnostics);
    let ir = res.ir.expect("ir");
    assert_eq!(ir.params.len(), 1);
    assert_eq!(ir.params[0].name, "active");
    assert!(ir.main.lines.iter().any(|l| l.name == "n_rif_out" && l.expr.starts_with("select(")));
}
