# 0043: Multi-domain project layout and domain backends

Design reference: `docs/superpowers/specs/2026-10-28-multi-domain-editor-design.md` (§4, §6.5, §7, §8.3, §10).

## Context

The editor handled one domain, materials, with a layout hard-wired to it (`shaders/`, `assets/shaders/`,
`crates/shaders`, `ShadersPlugin`). More domains (Light, animation) need a shared backend and a layout that does not
name any one of them. Plan A moves the backend; Plans B (editor core, the kits' editor halves) and C (shell, Light
domain, docs) extend this decision.

## Decision

- **Layout.** Sources: `quillshade/<domain>/` (Material: `quillshade/material/**`, user nodes in `.nodes/`). Settings:
  `quillshade/project.ron` (`ProjectSettings { format_version: 1, header_date, domains: { "<id>": (enabled: bool) } }`;
  a missing entry means enabled, unknown keys are ignored). A missing `project.ron` means defaults; one that exists but
  cannot be read or parsed is an open error (`OpenError::Settings`: the CLI exits 2, the editor refuses the project),
  never silently replaced by defaults, which would re-enable disabled domains. The editor re-reads it when it changes
  on disk (a broken edit keeps the previous settings with a warning). Output: `assets/quillshade/<domain>/` (Material:
  `autogen`, `internal`, `gradients`). The game's Assets tree hides `assets/quillshade/`.
- **Umbrella crate.** One generated crate `crates/quillshade` (crate name `quillshade`). `project_core` writes its
  `Cargo.toml` (union of the domains' `CrateContribution`s) and `src/lib.rs`: `pub mod <id>;` for every domain,
  `QuillshadePlugin` adding `<id>::Quillshade{Domain}InternalPlugin` for enabled domains, `AUTOGEN_VERSION`. Material
  writes `src/material/` (`QuillshadeMaterialInternalPlugin`, `COMPILED_MANIFEST`, `BakedMaterials` / `BakedTextures`
  re-exports, `materials/**`, `baked/**`). The runtime manifest is `quillshade/material/autogen/baked.ron`. Games use
  `.add_plugins(quillshade::QuillshadePlugin)` and `quillshade::material::…`; the root `Cargo.toml` needs
  `members = ["crates/quillshade"]` and `quillshade = { path = "crates/quillshade" }` (W105 otherwise).
- **`project_core`.** Depends on no workspace crate and not on Bevy: `DomainId`, generic `Diagnostic` / `Level`,
  `ron_io`, `marker`, `writer`, `files`, `ident`, `layout`, `umbrella`, `regen`, `watch`, `game_files`. A domain
  implements `DomainBackend` (plan → `DomainPlan`, outputs as `DomainOutput`); `regen::run` plans every selected
  domain (all of them without `--domain`), whether enabled or not — `enabled` only decides whether `QuillshadePlugin`
  adds the domain's plugin — and `regen::commit` does the shared work (compare, E008 on foreign files, per-root
  orphan scan, atomic write; a debug assertion keeps every root of a domain under its own output or module directory).
  `shader_project` is the Material backend (`backend::MaterialBackend`, `layout::MaterialLayout`,
  `regen::{plan_set, regenerate_set}`); generated Rust refers to `crate::material::…`, WESL to
  `quillshade/material/autogen` (`shader_codegen_wesl::paths::AUTOGEN`).
- **Diagnostic codes (§8.3).** Material keeps E001-E014 and W101-W111 unprefixed. Core codes (E008, W105) keep their
  numbers.
- **Kits.** `curve_kit` (curve model and math) and `texture_kit` (texture model, sampler rules, `TextureShape`,
  `TexturePolicy { extensions, shape }`, `check_path`, `read_ktx2_header`, `shape_problem`) depend only on
  `project_core`. `shader_graph_core` re-exports them (`curve`, `texture`, `MATERIAL_TEXTURES`). The design's
  `code_prefix` on `TexturePolicy` was dropped: the kit returns messages, not diagnostics, so each domain maps kit
  messages to its own codes (Material: E011-E014, W109).
- **CLI.** `regen|check|diff <project> [--domain <id>]...`; `--compile` on regen and check. No `--domain` means all
  domains. An unknown id prints ``error: unknown domain `x` (known: material)``, exits 2 and writes nothing. The umbrella
  files are written regardless of `--domain`. Exit codes 0/1/2 and the `path:location:CODE: text` format are unchanged;
  paths are now `quillshade/material/…`.
- **No migration.** Old `shaders/` projects are neither migrated nor detected (owner decision).
- **CI.** The `lint` job also runs `cargo test --workspace` in `examples/sample_game` (no GPU or window needed; it
  exports the toolchain libdir because the sample uses `dynamic_linking`).

## Consequences

Adding a domain means a backend, a source directory and an umbrella module, not a new layout. Existing projects must
be moved by hand. Older records that name `crates/shaders` or `assets/shaders` (and the manifest path) are superseded
in part by this one; they are no longer in the tree.
