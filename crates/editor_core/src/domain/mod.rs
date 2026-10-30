//! Domains (design §6.1, §6.3): what each one registers ([`registrar`]), the list of them ([`list`]), the state
//! each keeps inside `Editor` ([`DomainState`]), its tools ([`ToolDef`], design §14.1) and the built-in project domain
//! ([`ProjectDomain`], design §14.2).

mod list;
mod project;
mod registrar;
mod state;
mod tool;

pub use list::DomainList;
pub use project::ProjectDomain;
pub use registrar::{
    AssetKindDef, AssetTemplate, DomainDef, DomainInfo, DomainRegistrar, EditorDomain, GameAssetsDef, Highlighter,
    ParseTabFn, QuickStartDef, SceneFn, StepFn, TabExistsFn, TreeAsset, TreeFolder,
};
pub use state::{DomainHooks, DomainHooksFor, DomainState, DomainStates};
pub use tool::{TOOL_KEY_PREFIX, ToolDef, ToolId};
