# 0018. Welcome screen, About, New project, config location

**Context.** Without a project the editor showed an empty window and the Open Project dialog; there was no way to
see the version or where the project lives, and no hint how to hook the generated crate into a game. The user config
lived in `$XDG_CONFIG_HOME/shader-editor/` (decision 0009), with `%APPDATA%` taking part on every OS.

**Decision.**
- Config: `<OS config dir>/fe3dback/bevy-shader-editor/config.ron` (still RON, written atomically). The OS config
  dir comes from environment variables first, then the platform's usual place under the home directory
  (`std::env::home_dir`): Windows `%APPDATA%` → `<home>\AppData\Roaming`; macOS `$HOME/Library/Application Support`;
  others `$XDG_CONFIG_HOME` → `$HOME/.config` → `<home>/.config` (`config::os_config_dir`, pure and tested per OS).
  The directory is created at startup. Recent projects are stored as `recent_projects` (full canonical roots, 10).
  The old `shader-editor/config.ron` is not migrated. Supersedes the location part of 0009.
- Welcome screen (`ui::welcome`): an overlay over everything below the main menu while `Editor::project` is `None`:
  the big logo, name, version, disclaimer, recent projects (click opens), Open Project / New Project and the
  `Cargo.toml` / `ShadersPlugin` snippet. Startup no longer opens the Open Project dialog: the CLI argument or the
  most recent project is opened through `Action::OpenProject` (a failure shows the error dialog over the welcome
  screen).
- New project (`Project::create`) sets up an existing game crate — the directory must have `Cargo.toml`; `assets/`
  and `shaders/` are created. It does not scaffold a Bevy game and still never edits `Cargo.toml` (W105).
- Main menu: the logo on the far left, **Help** → Project Page, About. About is `Dialog::About` (logo at 128 px,
  version, link, author, license, disclaimer). Name, version (`EDITOR_VERSION`, also `--version`), URL, author,
  license and disclaimer are constants in `shader_editor_app` — CI rewrites `EDITOR_VERSION` on release. It is not the
  generator version in autogen headers (`GENERATOR_VERSION`, the crate version), which would change codegen.
- Logos (`ui::logo`) are the PNGs from the repository's `assets/`, compiled in with `include_bytes!` (the binary needs
  no asset directory) and box-filtered down by powers of two to the shown physical size; 32 px and below use
  `logo_sm.png`.

- Folder picker: Open / New Project open the system folder picker (`rfd`: xdg-desktop-portal on Linux, the native
  dialogs on Windows/macOS) on a background thread right away, plus a Browse... button in the dialog; the choice
  submits the dialog (`Action::FolderPicked`, ignored once the dialog is closed). The text field stays as the fallback
  (no portal, cancelled picker). Bevy's `system_clipboard` feature is on, so the field takes Ctrl+V from other
  applications (and Copy reaches them) — without it Bevy 0.20's clipboard is in-app only.

**Consequences.** URLs open through `external::open` like files (`Action::OpenUrl`). The `about` / `welcome` demo
hooks show the dialog / close the project before the screenshot (`open_project` the Open project dialog; screenshot
runs never start the system picker).
