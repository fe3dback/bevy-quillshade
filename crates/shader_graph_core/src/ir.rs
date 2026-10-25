//! Model → IR (spec §6.3): type resolution, conversions, topological order, validation.

use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet, HashMap};

use crate::consts::ConstLookup;
use crate::curve::Interp;
use crate::diag::{Code, Diagnostic, Diagnostics};
use crate::ident::check_ident;
use crate::model::{Asset, AssetKind, Edge, NodeId};
use crate::registry::{
    Builtin, CurveLookup, EmitCtx, EmitSink, Env, FnPort, FunctionLookup, FunctionSig, IrLine, MaterialParam, NodeDef,
    NodeRole, Operand, ParamType, ROUTE_PARAM, Registry, SigCtx, Signature, Stage,
};
use crate::types::{PinType, conversion};
use crate::value::Value;

/// Input pin of material output nodes evaluated in the vertex stage.
pub const WPO_PIN: &str = "world_position_offset";

/// One generated stage (fragment / vertex / function body).
#[derive(Debug, Clone, Default)]
pub struct IrStage {
    pub lines: Vec<IrLine>,
    pub builtins: BTreeSet<Builtin>,
    /// Sink pin → expression (material output pins or function output names).
    pub outputs: BTreeMap<String, String>,
}

impl IrStage {
    fn from_sink(sink: &EmitSink) -> Self {
        IrStage { lines: sink.lines.clone(), builtins: sink.builtins.clone(), outputs: sink.outputs.clone() }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UniformTy {
    Vec4,
    Vec3,
    Vec2,
    F32,
    U32,
}

impl UniformTy {
    pub fn wesl(self) -> &'static str {
        match self {
            UniformTy::Vec4 => "vec4<f32>",
            UniformTy::Vec3 => "vec3<f32>",
            UniformTy::Vec2 => "vec2<f32>",
            UniformTy::F32 => "f32",
            UniformTy::U32 => "u32",
        }
    }

    pub fn rust(self) -> &'static str {
        match self {
            UniformTy::Vec4 => "Vec4",
            UniformTy::Vec3 => "Vec3",
            UniformTy::Vec2 => "Vec2",
            UniformTy::F32 => "f32",
            UniformTy::U32 => "u32",
        }
    }

    pub fn size(self) -> u32 {
        match self {
            UniformTy::Vec4 => 16,
            UniformTy::Vec3 => 12,
            UniformTy::Vec2 => 8,
            UniformTy::F32 | UniformTy::U32 => 4,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UniformField {
    pub name: String,
    pub ty: UniformTy,
    pub offset: u32,
    /// Index into [`Ir::params`]; `None` for padding.
    pub param: Option<usize>,
}

/// Layout of the material parameter uniform: fields sorted by size (vec4, vec3+pad, vec2,
/// scalars), explicit padding, total size a multiple of 16. Shared by WESL and Rust generators.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UniformLayout {
    pub fields: Vec<UniformField>,
    pub size: u32,
}

impl UniformLayout {
    pub fn is_empty(&self) -> bool {
        self.fields.is_empty()
    }

    pub fn compute(params: &[MaterialParam]) -> UniformLayout {
        let rank = |t: ParamType| match t {
            ParamType::Color | ParamType::Vec4 => 0,
            ParamType::Vec3 => 1,
            ParamType::Vec2 => 2,
            ParamType::Scalar | ParamType::Bool => 3,
            ParamType::Texture => 4,
        };
        let mut idx: Vec<usize> = (0..params.len()).filter(|i| params[*i].ty != ParamType::Texture).collect();
        idx.sort_by_key(|i| rank(params[*i].ty));
        let mut fields = Vec::new();
        let mut offset = 0u32;
        let mut pad = 0usize;
        let mut push_pad = |fields: &mut Vec<UniformField>, offset: &mut u32| {
            fields.push(UniformField { name: format!("_pad{pad}"), ty: UniformTy::F32, offset: *offset, param: None });
            pad += 1;
            *offset += 4;
        };
        for i in idx {
            let ty = match params[i].ty {
                ParamType::Color | ParamType::Vec4 => UniformTy::Vec4,
                ParamType::Vec3 => UniformTy::Vec3,
                ParamType::Vec2 => UniformTy::Vec2,
                ParamType::Scalar => UniformTy::F32,
                ParamType::Bool => UniformTy::U32,
                ParamType::Texture => unreachable!(),
            };
            fields.push(UniformField { name: params[i].name.clone(), ty, offset, param: Some(i) });
            offset += ty.size();
            if ty == UniformTy::Vec3 {
                push_pad(&mut fields, &mut offset);
            }
        }
        if !fields.is_empty() {
            while offset % 16 != 0 {
                push_pad(&mut fields, &mut offset);
            }
        }
        UniformLayout { fields, size: offset }
    }
}

/// Typed, validated graph ready for code generation.
#[derive(Debug, Clone)]
pub struct Ir {
    /// Asset path without suffix (`env/rock_wet`).
    pub path: String,
    pub name: String,
    pub kind: AssetKind,
    /// Fragment stage for materials, function body for functions.
    pub main: IrStage,
    /// Vertex stage (World Position Offset), materials only.
    pub vertex: Option<IrStage>,
    /// Material parameters in constructor order.
    pub params: Vec<MaterialParam>,
    pub uniform: UniformLayout,
    pub stdlib: BTreeSet<(String, String)>,
    /// Project functions called (paths).
    pub functions: BTreeSet<String>,
    /// Curves sampled (asset path, interpolation).
    pub curves: BTreeSet<(String, Interp)>,
    /// Standalone fns of Custom nodes (name → full definition).
    pub custom_fns: BTreeMap<String, String>,
    /// Function assets: callable signature (with hidden builtins).
    pub function_sig: Option<FunctionSig>,
    pub uses_normal: bool,
}

impl Ir {
    pub fn texture_params(&self) -> impl Iterator<Item = &MaterialParam> {
        self.params.iter().filter(|p| p.ty == ParamType::Texture)
    }

    /// All builtins used by any stage.
    pub fn all_builtins(&self) -> BTreeSet<Builtin> {
        let mut b = self.main.builtins.clone();
        if let Some(v) = &self.vertex {
            b.extend(v.builtins.iter().copied());
        }
        b
    }
}

/// Result of [`build_ir`]: the IR (unless the graph is structurally broken) and diagnostics.
pub struct IrResult {
    pub ir: Option<Ir>,
    pub diagnostics: Vec<Diagnostic>,
}

impl IrResult {
    pub fn has_errors(&self) -> bool {
        self.diagnostics.iter().any(Diagnostic::is_error)
    }
}

struct ResolvedNode<'a> {
    def: &'a dyn NodeDef,
    params: BTreeMap<String, Value>,
    sig0: Signature,
}

/// Declared function signature (without hidden builtins), computed from FunctionInput/Output
/// nodes only, so callers can be typed even before the function body is generated.
pub fn declared_function_sig(asset: &Asset, path: &str, registry: &Registry) -> FunctionSig {
    let mut inputs: Vec<(i32, &str, FnPort)> = Vec::new();
    let mut outputs: Vec<(i32, &str, FnPort)> = Vec::new();
    for (id, node) in &asset.nodes {
        let Some(def) = registry.get(&node.kind) else { continue };
        let params = registry.resolved_params(&node.kind, &node.params);
        let get = |k: &str| params.get(k).cloned().unwrap_or(Value::Float(0.0));
        let ty = get("type").as_type().unwrap_or(PinType::Float);
        let port = FnPort {
            name: get("name").as_str().to_string(),
            ty,
            default: get("default").coerce(ty),
            required: get("required").as_bool() || ty == PinType::Texture2D,
        };
        let order = get("order").as_i32();
        match def.role() {
            NodeRole::FunctionInput => inputs.push((order, id, port)),
            NodeRole::FunctionOutput => outputs.push((order, id, port)),
            _ => {}
        }
    }
    inputs.sort_by(|a, b| (a.0, a.1).cmp(&(b.0, b.1)));
    outputs.sort_by(|a, b| (a.0, a.1).cmp(&(b.0, b.1)));
    FunctionSig {
        path: path.to_string(),
        name: asset.name.clone(),
        inputs: inputs.into_iter().map(|x| x.2).collect(),
        outputs: outputs.into_iter().map(|x| x.2).collect(),
        builtins: Vec::new(),
    }
}

/// Implicit edges of named routes: declaration `out` → usage `in`, for every usage whose
/// [`ROUTE_PARAM`] names an existing declaration (decision 0011). They are never stored in the file;
/// usages whose `in` already has an edge are skipped (so applying this twice adds nothing).
pub fn route_edges(asset: &Asset, registry: &Registry) -> Vec<Edge> {
    let role = |kind: &str| registry.get(kind).map(|d| d.role());
    let mut out = Vec::new();
    for (id, node) in &asset.nodes {
        if role(&node.kind) != Some(NodeRole::RouteUsage) || asset.edge_into(id, "in").is_some() {
            continue;
        }
        let decl = node.params.get(ROUTE_PARAM).map(|v| v.as_str()).unwrap_or_default();
        if asset.nodes.get(decl).is_some_and(|d| role(&d.kind) == Some(NodeRole::RouteDeclaration)) {
            out.push(Edge { from: (decl.to_string(), "out".into()), to: (id.clone(), "in".into()) });
        }
    }
    out
}

/// `asset` with its [`route_edges`] added: the graph that types, cycles and codegen see.
pub fn with_routes<'a>(asset: &'a Asset, registry: &Registry) -> Cow<'a, Asset> {
    let edges = route_edges(asset, registry);
    if edges.is_empty() {
        return Cow::Borrowed(asset);
    }
    let mut a = asset.clone();
    a.edges.extend(edges);
    Cow::Owned(a)
}

/// Builds the IR of `asset` located at `path` (`{sub}/{name}` without suffix).
pub fn build_ir(
    asset: &Asset,
    path: &str,
    registry: &Registry,
    functions: &dyn FunctionLookup,
    curves: &dyn CurveLookup,
    consts: &dyn ConstLookup,
) -> IrResult {
    let routed = with_routes(asset, registry);
    let asset = routed.as_ref();
    let mut diags = Diagnostics::default();
    let env = Env { asset_kind: asset.kind, asset_path: path, functions, curves, consts };

    if let Err(e) = check_ident(&asset.name) {
        diags.push(Diagnostic::new(Code::E005, format!("asset name: {e}")));
    }

    // 1. Resolve definitions.
    let mut nodes: BTreeMap<&str, ResolvedNode> = BTreeMap::new();
    for (id, node) in &asset.nodes {
        if id.is_empty() || !id.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit()) {
            diags.push(Diagnostic::new(Code::E009, format!("node id `{id}` must match [a-z0-9]+")).at_node(id));
            continue;
        }
        let Some(def) = registry.get(&node.kind) else {
            diags.push(Diagnostic::new(Code::E009, format!("unknown node kind `{}`", node.kind)).at_node(id));
            continue;
        };
        if let Some(fpath) = node.kind.strip_prefix(crate::registry::FUNCTION_PREFIX) {
            if fpath == path {
                diags.push(Diagnostic::new(Code::E006, "function calls itself (recursion)").at_node(id));
                continue;
            }
            if functions.function(fpath).is_none() {
                diags.push(Diagnostic::new(Code::E006, format!("function `{fpath}` does not exist")).at_node(id));
                continue;
            }
        }
        let params = registry.resolved_params(&node.kind, &node.params);
        let empty = BTreeMap::new();
        let sig0 = def.signature(&SigCtx {
            node_id: id,
            kind: &node.kind,
            params: &params,
            connected: &empty,
            input_values: &node.inputs,
            env: &env,
        });
        nodes.insert(id.as_str(), ResolvedNode { def: def.as_ref(), params, sig0 });
    }

    // 2. Validate edges.
    let mut edges: Vec<&Edge> = Vec::new();
    let mut seen_inputs: BTreeSet<(&str, &str)> = BTreeSet::new();
    for e in &asset.edges {
        let (Some(from), Some(to)) = (nodes.get(e.from.0.as_str()), nodes.get(e.to.0.as_str())) else {
            continue; // unknown node already reported
        };
        if from.sig0.output(&e.from.1).is_none() {
            diags.push(
                Diagnostic::new(Code::E009, format!("edge from unknown output pin `{}`", e.from.1)).at_node(&e.from.0),
            );
            continue;
        }
        if to.sig0.input(&e.to.1).is_none() {
            diags.push(
                Diagnostic::new(Code::E009, format!("edge into unknown input pin `{}`", e.to.1)).at_node(&e.to.0),
            );
            continue;
        }
        if !seen_inputs.insert((e.to.0.as_str(), e.to.1.as_str())) {
            diags.push(
                Diagnostic::new(Code::E003, "input has more than one connection").at_node(&e.to.0).at_pin(&e.to.1),
            );
            continue;
        }
        edges.push(e);
    }
    let edge_into: HashMap<(&str, &str), &Edge> =
        edges.iter().map(|e| ((e.to.0.as_str(), e.to.1.as_str()), *e)).collect();

    // 3. Sink nodes and role checks (E001).
    let mut material_outputs = Vec::new();
    let mut function_outputs = Vec::new();
    for (id, n) in &nodes {
        match n.def.role() {
            NodeRole::MaterialOutput => material_outputs.push(*id),
            NodeRole::FunctionOutput => function_outputs.push(*id),
            NodeRole::FunctionInput if !asset.kind.is_function() => {
                diags.push(Diagnostic::new(Code::E009, "FunctionInput is only allowed in functions").at_node(*id))
            }
            NodeRole::Parameter if asset.kind.is_function() => diags.push(
                Diagnostic::new(Code::E009, "parameters are only allowed in materials; use FunctionInput").at_node(*id),
            ),
            NodeRole::RouteUsage if !edge_into.contains_key(&(*id, "in")) => diags.push(
                Diagnostic::new(Code::E004, "named route not found: pick a declaration in the properties").at_node(*id),
            ),
            _ => {}
        }
    }
    let mut roots: Vec<(Stage, &str)> = Vec::new();
    match &asset.kind {
        AssetKind::Material(mk) => {
            let want = mk.output_node_kind();
            let good: Vec<&str> = material_outputs.iter().copied().filter(|id| asset.nodes[*id].kind == want).collect();
            for id in material_outputs.iter().filter(|id| asset.nodes[**id].kind != want) {
                diags.push(
                    Diagnostic::new(Code::E001, format!("output node does not match material type {}", mk.label()))
                        .at_node(*id),
                );
            }
            match good.len() {
                0 => diags.push(Diagnostic::new(Code::E001, format!("missing MaterialOutput ({})", mk.label()))),
                1 => {
                    roots.push((Stage::Fragment, good[0]));
                    roots.push((Stage::Vertex, good[0]));
                }
                _ => {
                    for id in &good[1..] {
                        diags.push(Diagnostic::new(Code::E001, "duplicate MaterialOutput").at_node(*id));
                    }
                    roots.push((Stage::Fragment, good[0]));
                    roots.push((Stage::Vertex, good[0]));
                }
            }
            for id in &function_outputs {
                diags.push(Diagnostic::new(Code::E001, "FunctionOutput is only allowed in functions").at_node(*id));
            }
        }
        AssetKind::Function => {
            if function_outputs.is_empty() {
                diags.push(Diagnostic::new(Code::E001, "function has no FunctionOutput"));
            }
            for id in &material_outputs {
                diags.push(Diagnostic::new(Code::E001, "MaterialOutput is not allowed in functions").at_node(*id));
            }
            for id in &function_outputs {
                roots.push((Stage::Function, id));
            }
        }
    }

    // 4. Cycles (E002).
    let succ: BTreeMap<&str, BTreeSet<&str>> = {
        let mut m: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
        for e in &edges {
            m.entry(e.from.0.as_str()).or_default().insert(e.to.0.as_str());
        }
        m
    };
    if let Some(cycle) = find_cycle(&nodes.keys().copied().collect::<Vec<_>>(), &succ) {
        for id in cycle {
            diags.push(Diagnostic::new(Code::E002, "node is part of a cycle").at_node(id));
        }
        return IrResult { ir: None, diagnostics: diags.into_vec() };
    }

    // 5. Live sets per stage (inactive output pins are not traversed).
    let pin_stage = |pin: &str| if pin == WPO_PIN { Stage::Vertex } else { Stage::Fragment };
    let mut live: BTreeMap<Stage, BTreeSet<&str>> = BTreeMap::new();
    for (stage, root) in &roots {
        let mut set = BTreeSet::new();
        let root_node = &nodes[root];
        let mut stack: Vec<&str> = Vec::new();
        for pin in &root_node.sig0.inputs {
            let in_stage = match stage {
                Stage::Function => true,
                s => pin_stage(&pin.id) == *s,
            };
            if !in_stage {
                continue;
            }
            if let Some(e) = edge_into.get(&(*root, pin.id.as_str())) {
                if pin.inactive {
                    diags.push(
                        Diagnostic::new(Code::W102, format!("`{}` is inactive for the current blend mode", pin.label))
                            .at_node(*root)
                            .at_pin(&pin.id),
                    );
                    continue;
                }
                stack.push(e.from.0.as_str());
            }
        }
        if stack.is_empty() && *stage == Stage::Vertex {
            continue;
        }
        set.insert(*root);
        while let Some(n) = stack.pop() {
            if !set.insert(n) {
                continue;
            }
            for e in &edges {
                if e.to.0 == n {
                    stack.push(e.from.0.as_str());
                }
            }
        }
        live.entry(*stage).or_default().extend(set);
    }
    let all_live: BTreeSet<&str> = live.values().flatten().copied().collect();
    for (id, n) in &nodes {
        let exempt =
            matches!(n.def.role(), NodeRole::FunctionInput | NodeRole::MaterialOutput | NodeRole::FunctionOutput);
        if !exempt && !all_live.contains(id) {
            diags.push(Diagnostic::new(Code::W101, "node does not affect any output").at_node(*id));
        }
    }

    // 6. Emit stages in topological order.
    let mut stages: BTreeMap<Stage, EmitSink> = BTreeMap::new();
    for (stage, set) in &live {
        let order = topo_order(set, &edges);
        let mut sink = EmitSink::default();
        let mut types: HashMap<(&str, String), PinType> = HashMap::new();
        for id in order {
            let n = &nodes[id];
            let node = &asset.nodes[id];
            let mut connected = BTreeMap::new();
            for pin in &n.sig0.inputs {
                if let Some(e) = edge_into.get(&(id, pin.id.as_str()))
                    && let Some(t) = types.get(&(e.from.0.as_str(), e.from.1.clone()))
                {
                    connected.insert(pin.id.clone(), *t);
                }
            }
            let sig = n.def.signature(&SigCtx {
                node_id: id,
                kind: &node.kind,
                params: &n.params,
                connected: &connected,
                input_values: &node.inputs,
                env: &env,
            });
            let is_root = matches!(n.def.role(), NodeRole::MaterialOutput);
            let mut inputs = BTreeMap::new();
            let mut connected_pins = BTreeSet::new();
            for pin in &sig.inputs {
                if is_root && *stage != Stage::Function && pin_stage(&pin.id) != *stage {
                    continue;
                }
                match edge_into.get(&(id, pin.id.as_str())) {
                    Some(_) if is_root && pin.inactive => {}
                    Some(e) => {
                        let key = (e.from.0.clone(), e.from.1.clone());
                        let (Some(op), Some(from_ty)) =
                            (sink.operands.get(&key), types.get(&(e.from.0.as_str(), e.from.1.clone())))
                        else {
                            continue;
                        };
                        match conversion(*from_ty, pin.ty) {
                            None => diags.push(
                                Diagnostic::new(
                                    Code::E003,
                                    format!(
                                        "cannot connect {} to {} input `{}`",
                                        from_ty.label(),
                                        pin.ty.label(),
                                        pin.label
                                    ),
                                )
                                .at_node(id)
                                .at_pin(&pin.id),
                            ),
                            Some(conv) => {
                                if conv.is_lossy(*from_ty, pin.ty) {
                                    diags.push(
                                        Diagnostic::new(
                                            Code::W104,
                                            format!("{} truncated to {}", from_ty.label(), pin.ty.label()),
                                        )
                                        .at_node(id)
                                        .at_pin(&pin.id),
                                    );
                                }
                                let operand = match op {
                                    Operand::Texture { .. } => op.clone(),
                                    Operand::Value(expr) => Operand::Value(conv.apply(expr, *from_ty, pin.ty)),
                                };
                                inputs.insert(pin.id.clone(), operand);
                                connected_pins.insert(pin.id.clone());
                            }
                        }
                    }
                    None if pin.required => diags.push(
                        Diagnostic::new(Code::E004, format!("required input `{}` is not connected", pin.label))
                            .at_node(id)
                            .at_pin(&pin.id),
                    ),
                    None if pin.ty == PinType::Texture2D => {}
                    None => {
                        let v = node.inputs.get(&pin.id).unwrap_or(&pin.default);
                        inputs.insert(pin.id.clone(), Operand::Value(v.wesl_literal(pin.ty)));
                    }
                }
            }
            let mut ctx = EmitCtx {
                node_id: id,
                kind: &node.kind,
                params: &n.params,
                sig: &sig,
                stage: *stage,
                env: &env,
                inputs,
                connected: connected_pins,
                sink: &mut sink,
            };
            n.def.emit(&mut ctx);
            for out in &sig.outputs {
                types.insert((id, out.id.clone()), out.ty);
            }
        }
        diags.extend(sink.diagnostics.drain(..));
        stages.insert(*stage, sink);
    }

    // 7. Collect parameters, imports, signatures.
    let mut stdlib = BTreeSet::new();
    let mut functions_used = BTreeSet::new();
    let mut curves_used = BTreeSet::new();
    let mut custom_fns = BTreeMap::new();
    let mut params_by_node: BTreeMap<NodeId, MaterialParam> = BTreeMap::new();
    for sink in stages.values() {
        stdlib.extend(sink.stdlib.iter().cloned());
        functions_used.extend(sink.functions.iter().cloned());
        curves_used.extend(sink.curves.iter().cloned());
        custom_fns.extend(sink.custom_fns.iter().map(|(k, v)| (k.clone(), v.clone())));
        params_by_node.extend(sink.params.iter().map(|(k, v)| (k.clone(), v.clone())));
    }
    let mut params: Vec<MaterialParam> = params_by_node.into_values().collect();
    params.sort_by(|a, b| (a.order, &a.node).cmp(&(b.order, &b.node)));
    let mut names = BTreeSet::new();
    for p in &params {
        if let Err(e) = check_ident(&p.name) {
            diags.push(Diagnostic::new(Code::E005, format!("parameter name: {e}")).at_node(&p.node));
        } else if !names.insert(p.name.clone()) {
            diags.push(Diagnostic::new(Code::E005, format!("duplicate parameter name `{}`", p.name)).at_node(&p.node));
        }
        if p.ty == ParamType::Texture && names.contains(&format!("{}_sampler", p.name)) {
            diags.push(
                Diagnostic::new(Code::E005, format!("`{}_sampler` clashes with a texture sampler name", p.name))
                    .at_node(&p.node),
            );
        }
        if functions_used.iter().any(|f| f.rsplit('/').next() == Some(p.name.as_str())) {
            diags.push(
                Diagnostic::new(Code::E005, format!("parameter `{}` clashes with a function name", p.name))
                    .at_node(&p.node),
            );
        }
    }
    {
        let mut fn_names: BTreeMap<&str, &str> = BTreeMap::new();
        for f in &functions_used {
            let name = f.rsplit('/').next().unwrap_or(f);
            if let Some(other) = fn_names.insert(name, f) {
                diags.push(Diagnostic::new(
                    Code::E006,
                    format!("functions `{other}` and `{f}` have the same name; rename one of them"),
                ));
            }
        }
    }

    let main_stage = match asset.kind {
        AssetKind::Function => Stage::Function,
        AssetKind::Material(_) => Stage::Fragment,
    };
    let main = stages.get(&main_stage).map(IrStage::from_sink).unwrap_or_default();
    let vertex = stages.get(&Stage::Vertex).map(IrStage::from_sink);

    let function_sig = if asset.kind.is_function() {
        let mut sig = declared_function_sig(asset, path, registry);
        sig.builtins = main.builtins.iter().copied().collect();
        let mut seen = BTreeSet::new();
        for (id, n) in &nodes {
            if matches!(n.def.role(), NodeRole::FunctionInput | NodeRole::FunctionOutput) {
                let name = n.params.get("name").map(|v| v.as_str().to_string()).unwrap_or_default();
                let dir = if n.def.role() == NodeRole::FunctionInput { "in" } else { "out" };
                if let Err(e) = check_ident(&name) {
                    diags.push(Diagnostic::new(Code::E005, format!("function {dir}put name: {e}")).at_node(*id));
                } else if !seen.insert((dir, name.clone())) {
                    let code = if dir == "out" { Code::E001 } else { Code::E005 };
                    diags.push(Diagnostic::new(code, format!("duplicate function {dir}put `{name}`")).at_node(*id));
                }
            }
        }
        for o in &sig.outputs {
            if o.ty == PinType::Texture2D {
                diags.push(Diagnostic::new(
                    Code::E003,
                    format!("function output `{}`: Texture2D outputs are not supported in WESL", o.name),
                ));
            }
        }
        Some(sig)
    } else {
        for b in main.builtins.iter().chain(vertex.iter().flat_map(|v| v.builtins.iter())) {
            if let Some(attr) = b.optional_attribute() {
                diags.push(Diagnostic::new(
                    Code::W103,
                    format!("uses vertex attribute {attr}; meshes without it get a default value"),
                ));
            }
        }
        None
    };

    let uses_normal = main.outputs.contains_key("normal");
    let uniform = UniformLayout::compute(&params);
    IrResult {
        ir: Some(Ir {
            path: path.to_string(),
            name: asset.name.clone(),
            kind: asset.kind,
            main,
            vertex,
            params,
            uniform,
            stdlib,
            functions: functions_used,
            curves: curves_used,
            custom_fns,
            function_sig,
            uses_normal,
        }),
        diagnostics: diags.into_vec(),
    }
}

/// Deterministic Kahn topological order of `set` (ties broken by node id).
fn topo_order<'a>(set: &BTreeSet<&'a str>, edges: &[&Edge]) -> Vec<&'a str> {
    let mut indeg: BTreeMap<&str, usize> = set.iter().map(|n| (*n, 0)).collect();
    let mut succ: BTreeMap<&str, Vec<&'a str>> = BTreeMap::new();
    for e in edges {
        let (f, t) = (e.from.0.as_str(), e.to.0.as_str());
        if let (Some(f), Some(t)) = (set.get(f), set.get(t)) {
            *indeg.get_mut(t).unwrap() += 1;
            succ.entry(f).or_default().push(t);
        }
    }
    let mut ready: BTreeSet<&'a str> = set.iter().copied().filter(|n| indeg[n] == 0).collect();
    let mut out = Vec::new();
    while let Some(n) = ready.pop_first() {
        out.push(n);
        for s in succ.get(n).into_iter().flatten() {
            let d = indeg.get_mut(s).unwrap();
            *d -= 1;
            if *d == 0 {
                ready.insert(*s);
            }
        }
    }
    out
}

/// Returns the nodes of one cycle if the graph has any.
pub fn find_cycle<'a>(nodes: &[&'a str], succ: &BTreeMap<&'a str, BTreeSet<&'a str>>) -> Option<Vec<&'a str>> {
    #[derive(Clone, Copy, PartialEq)]
    enum C {
        White,
        Grey,
        Black,
    }
    let mut color: BTreeMap<&str, C> = nodes.iter().map(|n| (*n, C::White)).collect();
    for start in nodes {
        if color[start] != C::White {
            continue;
        }
        // Iterative DFS with an explicit path.
        let mut stack: Vec<(&'a str, Vec<&'a str>)> =
            vec![(start, succ.get(start).map(|s| s.iter().copied().collect()).unwrap_or_default())];
        color.insert(start, C::Grey);
        while let Some((node, children)) = stack.last_mut() {
            if let Some(next) = children.pop() {
                match color.get(next).copied().unwrap_or(C::Black) {
                    C::White => {
                        color.insert(next, C::Grey);
                        let kids = succ.get(next).map(|s| s.iter().copied().collect()).unwrap_or_default();
                        stack.push((next, kids));
                    }
                    C::Grey => {
                        let pos = stack.iter().position(|(n, _)| *n == next).unwrap_or(0);
                        return Some(stack[pos..].iter().map(|(n, _)| *n).collect());
                    }
                    C::Black => {}
                }
            } else {
                color.insert(node, C::Black);
                stack.pop();
            }
        }
    }
    None
}

/// Whether connecting `from_node` → `to_node` would create a cycle in `asset`.
pub fn would_create_cycle(asset: &Asset, from_node: &str, to_node: &str) -> bool {
    if from_node == to_node {
        return true;
    }
    let mut stack = vec![to_node];
    let mut seen = BTreeSet::new();
    while let Some(n) = stack.pop() {
        if n == from_node {
            return true;
        }
        if !seen.insert(n) {
            continue;
        }
        for e in &asset.edges {
            if e.from.0 == n {
                stack.push(e.to.0.as_str());
            }
        }
    }
    false
}
