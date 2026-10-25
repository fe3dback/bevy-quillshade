//! Closure-based [`NodeDef`] builder used by all stdlib nodes.

use shader_graph_core::registry::{
    EmitCtx, NodeAction, NodeDef, NodeInfo, NodeRole, ParamDef, PinDef, SigCtx, Signature,
};
use shader_graph_core::{PinType, Value};

type SigFn = dyn Fn(&SigCtx) -> Signature + Send + Sync;
type EmitFn = dyn Fn(&mut EmitCtx) + Send + Sync;

pub struct Def {
    info: NodeInfo,
    role: NodeRole,
    params: Vec<ParamDef>,
    actions: Vec<NodeAction>,
    sig: Box<SigFn>,
    emit: Box<EmitFn>,
}

impl NodeDef for Def {
    fn info(&self) -> &NodeInfo {
        &self.info
    }

    fn role(&self) -> NodeRole {
        self.role
    }

    fn params(&self) -> Vec<ParamDef> {
        self.params.clone()
    }

    fn context_actions(&self) -> Vec<NodeAction> {
        self.actions.clone()
    }

    fn signature(&self, ctx: &SigCtx) -> Signature {
        (self.sig)(ctx)
    }

    fn emit(&self, ctx: &mut EmitCtx) {
        (self.emit)(ctx)
    }
}

pub struct DefBuilder {
    info: NodeInfo,
    role: NodeRole,
    params: Vec<ParamDef>,
    actions: Vec<NodeAction>,
}

pub fn def(id: &str, name: &str, category: &str) -> DefBuilder {
    DefBuilder {
        info: NodeInfo::new(id, name, category),
        role: NodeRole::Normal,
        params: Vec::new(),
        actions: Vec::new(),
    }
}

impl DefBuilder {
    pub fn kw(mut self, kw: &[&str]) -> Self {
        self.info = self.info.keywords(kw);
        self
    }

    pub fn desc(mut self, d: &str) -> Self {
        self.info = self.info.describe(d);
        self
    }

    pub fn role(mut self, role: NodeRole) -> Self {
        self.role = role;
        self
    }

    pub fn param(mut self, p: ParamDef) -> Self {
        self.params.push(p);
        self
    }

    pub fn action(mut self, a: NodeAction) -> Self {
        self.actions.push(a);
        self
    }

    pub fn build(
        self,
        sig: impl Fn(&SigCtx) -> Signature + Send + Sync + 'static,
        emit: impl Fn(&mut EmitCtx) + Send + Sync + 'static,
    ) -> Def {
        Def {
            info: self.info,
            role: self.role,
            params: self.params,
            actions: self.actions,
            sig: Box::new(sig),
            emit: Box::new(emit),
        }
    }
}

/// Input pin with a float default.
pub fn pin_in(id: &str, label: &str, ty: PinType, default: f32) -> PinDef {
    PinDef::new(id, label, ty).default(Value::Float(default).coerce(ty))
}

pub fn pin_out(id: &str, label: &str, ty: PinType) -> PinDef {
    PinDef::new(id, label, ty)
}

/// Polymorphic inputs resolved to one common type.
pub fn poly_ins(ctx: &SigCtx, pins: &[(&str, &str, f32)]) -> (Vec<PinDef>, PinType, bool) {
    let ids: Vec<&str> = pins.iter().map(|p| p.0).collect();
    let ty = ctx.poly_type(&ids);
    let resolved = ctx.any_connected(&ids) || ty != PinType::Float;
    let ins = pins.iter().map(|(id, label, d)| pin_in(id, label, ty, *d).poly()).collect();
    (ins, ty, resolved)
}

pub fn sig(inputs: Vec<PinDef>, outputs: Vec<PinDef>) -> Signature {
    Signature { inputs, outputs }
}

/// Output pin that is polymorphic when inputs are unresolved.
pub fn poly_out(id: &str, ty: PinType) -> PinDef {
    pin_out(id, "", ty).poly()
}

/// Stdlib function name with a dimension suffix (`sg_remap`, `sg_remap2`, …).
pub fn dim_fn(base: &str, ty: PinType) -> String {
    match ty.dim().unwrap_or(1) {
        1 => base.to_string(),
        d => format!("{base}{d}"),
    }
}
