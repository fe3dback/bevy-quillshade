//! The editor state (design §6.3) and its command dispatch: the [`editor::Editor`] resource, applying actions
//! (`apply.rs`), the core flows on assets and folders (`assets.rs`), the project and its tabs (`tabs.rs`), the active
//! domain (`domain_switch.rs`), the project watcher (`watch.rs`) and core's defaults for the domains' asset kinds
//! ([`kinds`]).

mod apply;
mod assets;
mod domain_switch;
pub mod editor;
pub mod kinds;
mod tabs;
mod watch;

#[cfg(test)]
mod tests;
