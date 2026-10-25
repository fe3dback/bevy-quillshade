//! Declarative node registry (spec §7): node definitions describe pins, parameters and WESL emission.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::Arc;

use crate::consts::{ConstLookup, ConstType, GlobalConst};
use crate::curve::{Interp, curve_fn_alias};
use crate::diag::{Code, Diagnostic};
use crate::model::{AssetKind, NodeId};
use crate::types::PinType;
use crate::value::Value;

/// Editable node property kinds (shown in the properties panel).
#[derive(Debug, Clone, PartialEq)]
pub enum ParamKind {
    Float {
        min: Option<f32>,
        max: Option<f32>,
    },
    Float2,
    Float3,
    Float4,
    Color,
    Bool,
    Int {
        min: Option<i32>,
        max: Option<i32>,
    },
    /// Single line string (names, groups, texture paths).
    Str,
    /// Multi-line WESL code.
    Code,
    /// One of the listed options (stored as `Value::Str`).
    Enum(Vec<String>),
    /// One of the listed pin types (stored as `Value::Type`).
    PinType(Vec<PinType>),
    /// Node id of a named route declaration in the same asset (stored as `Value::Str`).
    Route,
    /// Path of a curve asset of the project (`env/falloff`, stored as `Value::Str`; decision 0015).
    Curve,
    /// Name of a global constant of this type from any constant table (stored as `Value::Str`; decision 0017).
    GlobalConst(ConstType),
}

#[derive(Debug, Clone, PartialEq)]
pub struct ParamDef {
    pub id: String,
    pub label: String,
    pub kind: ParamKind,
    pub default: Value,
}

impl ParamDef {
    pub fn new(id: &str, label: &str, kind: ParamKind, default: Value) -> Self {
        ParamDef { id: id.into(), label: label.into(), kind, default }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct PinDef {
    pub id: String,
    pub label: String,
    pub ty: PinType,
    /// Polymorphic pin (drawn as `Any` while unresolved).
    pub poly: bool,
    /// Value used when the input is not connected (editable inline unless `required`).
    pub default: Value,
    /// Input must be connected (E004 otherwise).
    pub required: bool,
    /// Output-node pin inactive for current settings (W102 when connected).
    pub inactive: bool,
    /// Unconnected input has a meaningful built-in fallback (shown instead of a value field).
    pub implicit: Option<String>,
    /// Sensible value range of a scalar input (inline slider bounds in the editor; not enforced in codegen).
    pub range: Option<(f32, f32)>,
}

impl PinDef {
    pub fn new(id: &str, label: &str, ty: PinType) -> Self {
        PinDef {
            id: id.into(),
            label: label.into(),
            ty,
            poly: false,
            default: Value::zero(ty),
            required: false,
            inactive: false,
            implicit: None,
            range: None,
        }
    }

    pub fn poly(mut self) -> Self {
        self.poly = true;
        self
    }

    pub fn default(mut self, v: Value) -> Self {
        self.default = v;
        self
    }

    pub fn required(mut self) -> Self {
        self.required = true;
        self
    }

    pub fn inactive(mut self, inactive: bool) -> Self {
        self.inactive = inactive;
        self
    }

    pub fn implicit(mut self, what: &str) -> Self {
        self.implicit = Some(what.into());
        self
    }

    pub fn range(mut self, min: f32, max: f32) -> Self {
        self.range = Some((min, max));
        self
    }

    /// Display type: `Any` for unresolved polymorphic pins.
    pub fn display_type(&self, resolved: bool) -> PinType {
        if self.poly && !resolved { PinType::Any } else { self.ty }
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Signature {
    pub inputs: Vec<PinDef>,
    pub outputs: Vec<PinDef>,
}

impl Signature {
    pub fn input(&self, id: &str) -> Option<&PinDef> {
        self.inputs.iter().find(|p| p.id == id)
    }

    pub fn output(&self, id: &str) -> Option<&PinDef> {
        self.outputs.iter().find(|p| p.id == id)
    }
}

/// Special roles the IR builder treats differently.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeRole {
    Normal,
    /// `MaterialOutput` of the given material kind (sink node).
    MaterialOutput,
    FunctionInput,
    /// Function result (sink node).
    FunctionOutput,
    /// Material parameter (Scalar/Vector/Color/Texture/Bool).
    Parameter,
    Reroute,
    /// Named route declaration (`in` → `out`): a wire end that its usages read without a visible wire.
    RouteDeclaration,
    /// Named route usage: `out` carries the input of the declaration whose node id is in [`ROUTE_PARAM`].
    /// Its `in` pin is fed by an implicit edge ([`crate::ir::route_edges`]), never by a stored one.
    RouteUsage,
}

/// Parameter of a [`NodeRole::RouteUsage`] node: the node id of its declaration (decision 0011).
pub const ROUTE_PARAM: &str = "route";

/// Curve paths `asset` refers to through [`ParamKind::Curve`] parameters: `(node id, param id, path)`.
pub fn curve_refs(asset: &crate::model::Asset, registry: &Registry) -> Vec<(NodeId, String, String)> {
    let mut out = Vec::new();
    for (id, n) in &asset.nodes {
        let Some(def) = registry.get(&n.kind) else { continue };
        for p in def.params().into_iter().filter(|p| p.kind == ParamKind::Curve) {
            let path = n.params.get(&p.id).unwrap_or(&p.default).as_str();
            if !path.is_empty() {
                out.push((id.clone(), p.id, path.to_string()));
            }
        }
    }
    out
}

/// Global constant names `asset` refers to through [`ParamKind::GlobalConst`] parameters:
/// `(node id, param id, name)`.
pub fn const_refs(asset: &crate::model::Asset, registry: &Registry) -> Vec<(NodeId, String, String)> {
    let mut out = Vec::new();
    for (id, n) in &asset.nodes {
        let Some(def) = registry.get(&n.kind) else { continue };
        for p in def.params().into_iter().filter(|p| matches!(p.kind, ParamKind::GlobalConst(_))) {
            let name = n.params.get(&p.id).unwrap_or(&p.default).as_str();
            if !name.is_empty() {
                out.push((id.clone(), p.id, name.to_string()));
            }
        }
    }
    out
}

#[derive(Debug, Clone)]
pub struct NodeInfo {
    pub id: String,
    pub name: String,
    pub category: String,
    /// Search synonyms (`*`, `lerp`, `+`, …).
    pub keywords: Vec<String>,
    pub description: String,
}

impl NodeInfo {
    pub fn new(id: &str, name: &str, category: &str) -> Self {
        NodeInfo {
            id: id.into(),
            name: name.into(),
            category: category.into(),
            keywords: Vec::new(),
            description: String::new(),
        }
    }

    pub fn keywords(mut self, kw: &[&str]) -> Self {
        self.keywords = kw.iter().map(|s| s.to_string()).collect();
        self
    }

    pub fn describe(mut self, d: &str) -> Self {
        self.description = d.into();
        self
    }
}

/// Values provided by the shader environment. Materials resolve them from `VertexOutput`/view
/// bindings; functions receive them as hidden trailing arguments, so functions stay usable from
/// any material type and stage.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Builtin {
    Uv0,
    Uv1,
    WorldPosition,
    WorldNormal,
    WorldTangent,
    VertexColor,
    /// `@builtin(position)` in fragment stage: framebuffer coordinates + depth.
    FragCoord,
    Time,
    DeltaTime,
    ViewSize,
    CameraPosition,
    WorldFromLocal,
    ViewFromWorld,
    ClipFromWorld,
}

impl Builtin {
    pub const ALL: [Builtin; 14] = [
        Builtin::Uv0,
        Builtin::Uv1,
        Builtin::WorldPosition,
        Builtin::WorldNormal,
        Builtin::WorldTangent,
        Builtin::VertexColor,
        Builtin::FragCoord,
        Builtin::Time,
        Builtin::DeltaTime,
        Builtin::ViewSize,
        Builtin::CameraPosition,
        Builtin::WorldFromLocal,
        Builtin::ViewFromWorld,
        Builtin::ClipFromWorld,
    ];

    /// Identifier of the builtin inside generated code (local `let` or hidden fn argument).
    pub fn var(self) -> &'static str {
        match self {
            Builtin::Uv0 => "b_uv0",
            Builtin::Uv1 => "b_uv1",
            Builtin::WorldPosition => "b_world_position",
            Builtin::WorldNormal => "b_world_normal",
            Builtin::WorldTangent => "b_world_tangent",
            Builtin::VertexColor => "b_vertex_color",
            Builtin::FragCoord => "b_frag_coord",
            Builtin::Time => "b_time",
            Builtin::DeltaTime => "b_delta_time",
            Builtin::ViewSize => "b_view_size",
            Builtin::CameraPosition => "b_camera_position",
            Builtin::WorldFromLocal => "b_world_from_local",
            Builtin::ViewFromWorld => "b_view_from_world",
            Builtin::ClipFromWorld => "b_clip_from_world",
        }
    }

    pub fn wesl_type(self) -> &'static str {
        match self {
            Builtin::Uv0 | Builtin::Uv1 | Builtin::ViewSize => "vec2<f32>",
            Builtin::WorldPosition | Builtin::WorldNormal | Builtin::CameraPosition => "vec3<f32>",
            Builtin::WorldTangent | Builtin::VertexColor | Builtin::FragCoord => "vec4<f32>",
            Builtin::Time | Builtin::DeltaTime => "f32",
            Builtin::WorldFromLocal | Builtin::ViewFromWorld | Builtin::ClipFromWorld => "mat4x4<f32>",
        }
    }

    /// Vertex attribute a mesh may lack (W103).
    pub fn optional_attribute(self) -> Option<&'static str> {
        match self {
            Builtin::Uv1 => Some("UV_1"),
            Builtin::WorldTangent => Some("TANGENT"),
            Builtin::VertexColor => Some("COLOR"),
            _ => None,
        }
    }
}

/// Port of a project function as seen by callers.
#[derive(Debug, Clone, PartialEq)]
pub struct FnPort {
    pub name: String,
    pub ty: PinType,
    pub default: Value,
    pub required: bool,
}

/// Callable signature of a project function.
#[derive(Debug, Clone, PartialEq)]
pub struct FunctionSig {
    /// Asset path without suffix (`common/wet_mask`).
    pub path: String,
    /// WESL fn name (= asset name).
    pub name: String,
    pub inputs: Vec<FnPort>,
    pub outputs: Vec<FnPort>,
    /// Hidden builtin arguments, appended after declared inputs in this order.
    pub builtins: Vec<Builtin>,
}

impl FunctionSig {
    /// Name of the result struct when the function has several outputs.
    pub fn out_struct(&self) -> String {
        format!("{}Out", crate::ident::to_pascal_case(&self.name))
    }
}

pub trait FunctionLookup {
    fn function(&self, path: &str) -> Option<&FunctionSig>;
}

/// No project functions (tests, isolated graphs).
pub struct NoFunctions;

impl FunctionLookup for NoFunctions {
    fn function(&self, _path: &str) -> Option<&FunctionSig> {
        None
    }
}

impl FunctionLookup for BTreeMap<String, FunctionSig> {
    fn function(&self, path: &str) -> Option<&FunctionSig> {
        self.get(path)
    }
}

/// Curve assets of the project that can be sampled (loaded without errors).
pub trait CurveLookup {
    fn has_curve(&self, path: &str) -> bool;
}

/// No project curves (tests, signatures).
pub struct NoCurves;

impl CurveLookup for NoCurves {
    fn has_curve(&self, _path: &str) -> bool {
        false
    }
}

impl CurveLookup for BTreeSet<String> {
    fn has_curve(&self, path: &str) -> bool {
        self.contains(path)
    }
}

/// Graph-wide information available to node definitions.
pub struct Env<'a> {
    pub asset_kind: AssetKind,
    /// Path of the asset being built (to detect self-recursion).
    pub asset_path: &'a str,
    pub functions: &'a dyn FunctionLookup,
    pub curves: &'a dyn CurveLookup,
    pub consts: &'a dyn ConstLookup,
}

/// Inputs for computing a node signature.
pub struct SigCtx<'a> {
    pub node_id: &'a str,
    pub kind: &'a str,
    pub params: &'a BTreeMap<String, Value>,
    /// Source types of connected inputs.
    pub connected: &'a BTreeMap<String, PinType>,
    /// Inline default overrides of unconnected inputs.
    pub input_values: &'a BTreeMap<String, Value>,
    pub env: &'a Env<'a>,
}

impl SigCtx<'_> {
    pub fn param(&self, id: &str) -> &Value {
        static ZERO: Value = Value::Float(0.0);
        self.params.get(id).unwrap_or(&ZERO)
    }

    /// Resolved polymorphic type over `pins`: maximum dimension among connected inputs
    /// (Int counts as Float), falling back to inline defaults. `Color` resolves to `Float4`.
    pub fn poly_type(&self, pins: &[&str]) -> PinType {
        let mut dim = 1u8;
        for p in pins {
            let d = match self.connected.get(*p) {
                Some(t) if *t == PinType::Int => 1,
                Some(t) => t.dim().unwrap_or(1),
                None => self.input_values.get(*p).and_then(|v| v.pin_type()).and_then(|t| t.dim()).unwrap_or(1),
            };
            dim = dim.max(d);
        }
        PinType::float_of_dim(dim)
    }

    /// Whether any of `pins` is connected (poly pins then display their resolved type).
    pub fn any_connected(&self, pins: &[&str]) -> bool {
        pins.iter().any(|p| self.connected.contains_key(*p))
    }

    pub fn material_kind(&self) -> Option<&crate::model::MaterialKind> {
        self.env.asset_kind.material()
    }
}

/// Pipeline stage the node is emitted into.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Stage {
    Fragment,
    Vertex,
    /// Body of a project function (stage-agnostic, builtins are arguments).
    Function,
}

/// Value flowing along an edge in generated code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Operand {
    Value(String),
    /// Texture + sampler identifiers.
    Texture {
        tex: String,
        sampler: String,
    },
}

impl Operand {
    pub fn expr(&self) -> &str {
        match self {
            Operand::Value(s) => s,
            Operand::Texture { tex, .. } => tex,
        }
    }
}

/// Material parameter value type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ParamType {
    Scalar,
    Vec2,
    Vec3,
    Vec4,
    Color,
    Bool,
    Texture,
}

impl ParamType {
    pub fn pin_type(self) -> PinType {
        match self {
            ParamType::Scalar => PinType::Float,
            ParamType::Vec2 => PinType::Float2,
            ParamType::Vec3 => PinType::Float3,
            ParamType::Vec4 => PinType::Float4,
            ParamType::Color => PinType::Color,
            ParamType::Bool => PinType::Bool,
            ParamType::Texture => PinType::Texture2D,
        }
    }
}

/// Material parameter collected from parameter nodes; becomes a uniform field or a texture
/// binding and an argument of the generated Rust constructor.
#[derive(Debug, Clone, PartialEq)]
pub struct MaterialParam {
    /// snake_case identifier used in WESL and Rust.
    pub name: String,
    pub ty: ParamType,
    pub default: Value,
    pub group: String,
    /// Sort key for constructor argument order (ties broken by node id = creation order).
    pub order: i32,
    pub min: Option<f32>,
    pub max: Option<f32>,
    pub node: NodeId,
}

/// One `let` line of generated code.
#[derive(Debug, Clone, PartialEq)]
pub struct IrLine {
    pub node: NodeId,
    pub name: String,
    pub ty: String,
    pub expr: String,
}

/// Mutable state shared by all nodes of one stage during emission.
#[derive(Debug, Default)]
pub struct EmitSink {
    pub lines: Vec<IrLine>,
    pub operands: HashMap<(NodeId, String), Operand>,
    pub stdlib: BTreeSet<(String, String)>,
    pub builtins: BTreeSet<Builtin>,
    pub functions: BTreeSet<String>,
    /// Sampled curves (asset path, interpolation).
    pub curves: BTreeSet<(String, Interp)>,
    pub custom_fns: BTreeMap<String, String>,
    pub params: BTreeMap<NodeId, MaterialParam>,
    /// Sink-node values: material output pins / function outputs, keyed by pin or output name.
    pub outputs: BTreeMap<String, String>,
    pub diagnostics: Vec<Diagnostic>,
}

/// Per-node emission context.
pub struct EmitCtx<'a> {
    pub node_id: &'a str,
    pub kind: &'a str,
    pub params: &'a BTreeMap<String, Value>,
    pub sig: &'a Signature,
    pub stage: Stage,
    pub env: &'a Env<'a>,
    pub(crate) inputs: BTreeMap<String, Operand>,
    pub(crate) connected: BTreeSet<String>,
    pub sink: &'a mut EmitSink,
}

impl EmitCtx<'_> {
    /// Expression of an input (converted to the pin type, or the default literal).
    pub fn input(&self, pin: &str) -> String {
        match self.inputs.get(pin) {
            Some(op) => op.expr().to_string(),
            None => "0.0".into(),
        }
    }

    pub fn input_texture(&self, pin: &str) -> Option<(String, String)> {
        match self.inputs.get(pin) {
            Some(Operand::Texture { tex, sampler }) => Some((tex.clone(), sampler.clone())),
            _ => None,
        }
    }

    pub fn is_connected(&self, pin: &str) -> bool {
        self.connected.contains(pin)
    }

    pub fn param(&self, id: &str) -> &Value {
        static ZERO: Value = Value::Float(0.0);
        self.params.get(id).unwrap_or(&ZERO)
    }

    pub fn out_type(&self, pin: &str) -> PinType {
        self.sig.output(pin).map(|p| p.ty).unwrap_or(PinType::Float)
    }

    pub fn in_type(&self, pin: &str) -> PinType {
        self.sig.input(pin).map(|p| p.ty).unwrap_or(PinType::Float)
    }

    /// Variable name of an output pin.
    pub fn var(&self, pin: &str) -> String {
        format!("n_{}_{}", self.node_id, pin)
    }

    /// Defines output `pin` as `let n_{id}_{pin}: T = expr;`.
    pub fn define(&mut self, pin: &str, expr: impl Into<String>) {
        let ty = self.out_type(pin).wesl().to_string();
        self.define_typed(pin, &ty, expr);
    }

    /// Like [`define`](Self::define) with an explicit WESL type (e.g. a function result struct).
    pub fn define_typed(&mut self, pin: &str, ty: &str, expr: impl Into<String>) {
        let name = self.var(pin);
        self.sink.lines.push(IrLine {
            node: self.node_id.into(),
            name: name.clone(),
            ty: ty.into(),
            expr: expr.into(),
        });
        self.sink.operands.insert((self.node_id.into(), pin.into()), Operand::Value(name));
    }

    /// Makes output `pin` refer to an existing operand without emitting a line.
    pub fn alias(&mut self, pin: &str, op: Operand) {
        self.sink.operands.insert((self.node_id.into(), pin.into()), op);
    }

    /// Uses a stdlib function and returns its callable name.
    pub fn stdlib(&mut self, module: &str, func: &str) -> String {
        self.sink.stdlib.insert((module.into(), func.into()));
        func.to_string()
    }

    /// Uses an environment value and returns its identifier.
    pub fn builtin(&mut self, b: Builtin) -> String {
        self.sink.builtins.insert(b);
        b.var().to_string()
    }

    /// Registers a call of a project function; returns its signature.
    pub fn use_function(&mut self, path: &str) -> Option<FunctionSig> {
        let sig = self.env.functions.function(path)?.clone();
        self.sink.functions.insert(path.into());
        Some(sig)
    }

    /// Registers sampling of curve `path`; returns the name to call (`None` if the curve is not loadable).
    pub fn use_curve(&mut self, path: &str, interp: Interp) -> Option<String> {
        if !self.env.curves.has_curve(path) {
            return None;
        }
        self.sink.curves.insert((path.into(), interp));
        Some(curve_fn_alias(path, interp))
    }

    /// A global constant of the project (`None` if no table defines `name`).
    pub fn global_const(&self, name: &str) -> Option<&GlobalConst> {
        self.env.consts.global_const(name)
    }

    /// Adds a standalone fn definition (Custom node).
    pub fn add_fn(&mut self, name: &str, code: String) {
        self.sink.custom_fns.insert(name.into(), code);
    }

    pub fn declare_param(&mut self, p: MaterialParam) {
        self.sink.params.insert(self.node_id.into(), p);
    }

    /// Records the value of a sink pin (material output pin / function output).
    pub fn set_output(&mut self, key: &str, expr: String) {
        self.sink.outputs.insert(key.into(), expr);
    }

    pub fn diag(&mut self, code: Code, msg: impl Into<String>) {
        self.sink.diagnostics.push(Diagnostic::new(code, msg).at_node(self.node_id));
    }

    pub fn diag_pin(&mut self, code: Code, pin: &str, msg: impl Into<String>) {
        self.sink.diagnostics.push(Diagnostic::new(code, msg).at_node(self.node_id).at_pin(pin));
    }
}

/// Declarative node definition.
pub trait NodeDef: Send + Sync {
    fn info(&self) -> &NodeInfo;

    fn role(&self) -> NodeRole {
        NodeRole::Normal
    }

    fn params(&self) -> Vec<ParamDef> {
        Vec::new()
    }

    /// Pins for the given params and connected input types. Pin ids must depend on params only.
    fn signature(&self, ctx: &SigCtx) -> Signature;

    /// Emits WESL for the node. Errors are reported through `ctx.diag`.
    fn emit(&self, ctx: &mut EmitCtx);
}

/// Registry of node definitions by kind.
#[derive(Clone, Default)]
pub struct Registry {
    defs: BTreeMap<String, Arc<dyn NodeDef>>,
}

/// Kind prefix of project function calls (`function::{sub}/{name}`).
pub const FUNCTION_PREFIX: &str = "function::";
/// Registry key of the generic function call definition.
pub const FUNCTION_CALL_KIND: &str = "function::*";

impl Registry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&mut self, def: impl NodeDef + 'static) {
        let id = def.info().id.clone();
        self.defs.insert(id, Arc::new(def));
    }

    pub fn register_arc(&mut self, def: Arc<dyn NodeDef>) {
        let id = def.info().id.clone();
        self.defs.insert(id, def);
    }

    pub fn get(&self, kind: &str) -> Option<&Arc<dyn NodeDef>> {
        if kind.starts_with(FUNCTION_PREFIX) {
            return self.defs.get(FUNCTION_CALL_KIND);
        }
        self.defs.get(kind)
    }

    pub fn defs(&self) -> impl Iterator<Item = &Arc<dyn NodeDef>> {
        self.defs.values()
    }

    /// Params of a node merged with definition defaults.
    pub fn resolved_params(&self, kind: &str, stored: &BTreeMap<String, Value>) -> BTreeMap<String, Value> {
        let mut out = BTreeMap::new();
        if let Some(def) = self.get(kind) {
            for p in def.params() {
                out.insert(p.id.clone(), p.default.clone());
            }
        }
        for (k, v) in stored {
            out.insert(k.clone(), v.clone());
        }
        out
    }
}
