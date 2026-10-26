//! Shader graph core: asset model, type system, node registry, validation and IR.
//!
//! Independent from Bevy so the CLI and code generators build fast and run headless.

pub mod attributes;
pub mod consts;
pub mod curve;
pub mod diag;
pub mod ident;
pub mod ir;
pub mod model;
pub mod preview;
pub mod registry;
pub mod texture;
pub mod types;
pub mod value;

pub use consts::{ConstLookup, ConstType, GlobalConst, GlobalConsts};
pub use curve::{Curve, Interp};
pub use diag::{Code, Diagnostic, Diagnostics};
pub use ir::{
    Ir, IrPreview, IrResult, IrStage, TextureBinding, TextureSource, UniformField, UniformLayout, UniformTy, build_ir,
};
pub use model::{
    Asset, AssetKind, BlendMode, Edge, Frame, MaterialKind, Meta, Node, NodeId, PbrOutputMode, PbrSettings, Preview,
    PreviewMode, Shading, SpriteSettings, UnlitSettings,
};
pub use preview::{PreviewSlot, PreviewSource, ShaderOutputDto, albedo_expr, converts_to_albedo};
pub use registry::{
    Builtin, CurveLookup, EmitCtx, Env, FnPort, FunctionLookup, FunctionSig, HiddenTexture, Hotkey, HotkeyKey,
    MaterialParam, NodeAction, NodeDef, NodeInfo, NodeOp, NodeRole, Operand, ParamDef, ParamKind, ParamType, PinDef,
    Registry, SigCtx, Signature, Stage, TextureLookup,
};
pub use texture::{TextureAsset, TextureSettings};
pub use types::{Conversion, PinType, conversion};
pub use value::Value;

/// Version of the generator written into autogen headers.
pub const GENERATOR_VERSION: u32 = 1;
