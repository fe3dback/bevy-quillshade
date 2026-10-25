//! Shader graph core: asset model, type system, node registry, validation and IR.
//!
//! Independent from Bevy so the CLI and code generators build fast and run headless.

pub mod diag;
pub mod ident;
pub mod ir;
pub mod model;
pub mod registry;
pub mod types;
pub mod value;

pub use diag::{Code, Diagnostic, Diagnostics};
pub use ir::{Ir, IrResult, IrStage, UniformField, UniformLayout, UniformTy, build_ir};
pub use model::{
    Asset, AssetKind, BlendMode, Edge, Frame, MaterialKind, Meta, Node, NodeId, PbrSettings, Shading, SpriteSettings,
    UnlitSettings,
};
pub use registry::{
    Builtin, EmitCtx, Env, FnPort, FunctionLookup, FunctionSig, MaterialParam, NodeDef, NodeInfo, NodeRole, Operand,
    ParamDef, ParamKind, ParamType, PinDef, Registry, SigCtx, Signature, Stage,
};
pub use types::{Conversion, PinType, conversion};
pub use value::Value;

/// Version of the generator written into autogen headers.
pub const GENERATOR_VERSION: u32 = 1;
