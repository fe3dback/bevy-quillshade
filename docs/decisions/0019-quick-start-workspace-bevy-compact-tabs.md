# 0019. Help → Quick Start, Bevy from the game's workspace, compact tabs with short names

**Context.** QA feedback: the hook-up instructions were only on the welcome screen (gone once a project is open); the
generated `crates/shaders/Cargo.toml` always pinned `bevy = { version = "0.20", … }`, which duplicates (and may
diverge from) a game that declares Bevy once in `[workspace.dependencies]`; tabs showed the full project-relative
path and, once there were many, did not fit the bar.

**Decision.**
- Help → **Quick Start** (`Dialog::QuickStart`, `ui::welcome::quick_start`): five steps from an existing game crate to
  a material on screen (New / Open Project, create and save, the `Cargo.toml` snippet, `ShadersPlugin` and a
  constructor, hot reload with `file_watcher`). The welcome screen keeps its shorter snippet.
- `Project::bevy_dep` reads the game's root `Cargo.toml` on every regeneration: if it has
  `[workspace.dependencies] bevy`, or itself takes `bevy = { workspace = true }` (the game is a member of an outer
  workspace), the generated crate gets `bevy = { workspace = true, features = [..] }`
  (`shader_codegen_rust::BevyDep::Workspace`), otherwise the old pinned dependency. The feature list the generated code
  needs is kept in both forms: features are additive, so it is harmless when the workspace already enables them and
  keeps the crate compiling when the workspace declares Bevy with `default-features = false`. `default-features =
  false` is not repeated for the workspace form (cargo ignores it there). Editing only the root `Cargo.toml` does not
  trigger a regeneration; the next one picks it up. The editor still never writes the root `Cargo.toml` (W105).
- Tabs show `Tab::short_title` — the file name without directories and extensions (`rock_wet`) — and the full
  `Tab::title` when several open tabs share a short title (a material and its generated WESL both are `rock_wet`, so
  both show their paths; `tab::labels`). The full path is the tab's tooltip. Tabs are compact (14 px icon, 12 px
  label, a small close icon instead of a tool button) and the bar wraps them onto more rows.

**Consequences.** The window title still uses the full path. The close cross is an `IconId::Close` icon because the
UI font has no `×` glyph; `IconId::Help` (icon-rs `HELP`) marks Quick Start in the menu.
