# 0046: Migration Tool and the project version

Design reference: `docs/data-domain-techdoc.md` §4.4 and §10 Phase 0 (decisions D28, D38, D39), `AGENTS.md`. Extends
decisions 0043 (project layout, `project.ron`) and 0044/0045 (editor domain API, the built-in project domain);
supersedes in part 0043 (the `format_version` key of `project.ron` and "a missing `project.ron` means defaults").

## Context

Phase 1 of the data-domain roadmap changes every editor source format (asset UUIDs). That needs a project-wide,
resumable, domain-owned migration framework before any format changes. Today `quillshade/project.ron` carries
`format_version: 1`, nothing compares it with what the editor supports, and per-file `format_version`s are only checked
on load ("newer than supported" errors). This record fixes the "decide in ADR" points of techdoc §4.4 and the details
the techdoc leaves to the implementation.

## Decision

### Project version (D28)

- `project_core::version::ProjectVersion(u32)` (ordered, `Display` as `v<N>`, serde-transparent) is the single global
  project version. `ProjectVersion::CURRENT` is the version this build writes and opens; Phase 0 sets it to **2**.
  Every later format change adds one step and raises `CURRENT` by one.
- **Rename (decided): `format_version` → `project_version` in `quillshade/project.ron`.** "Format version" stays the
  name of the per-file markers; the project file gets an unambiguous name. Version 1 is, by definition, the only
  version spelled `format_version`; the v2 step performs the rename (see below). `ProjectSettings.format_version`
  becomes `ProjectSettings.project_version: ProjectVersion`; new projects (`ProjectSettings::default`,
  `Project::ensure_layout`) write `CURRENT`.
- **Probe.** `project_core::version::probe_project_version(text)` reads only the version, struct-name agnostic:
  `project_version` if present, else the legacy `format_version`; both present or neither present is an error. So
  are the values no project can have: `format_version` with a value other than 1 (v1 is the only version spelled that
  way, so `format_version: 2` is not "v2 already" but a settings error with a message saying so) and version 0. Any
  other value is accepted by the probe; the caller compares it with `CURRENT`.
- **Open flow** (`Project::open`, after the existing directory checks):
  - `== CURRENT` → parse the full settings and open as today;
  - `< CURRENT` → `OpenError::NeedsMigration { from, to }` (`to = CURRENT`);
  - `> CURRENT` → `OpenError::TooNew { found, supported }`;
  - unreadable / malformed / no version key → `OpenError::Settings` as today.
  `Project::reload_settings` (watcher) applies the same check and keeps the old settings on error.
- **Missing `project.ron` (decided, replaces 0043's "a missing `project.ron` means defaults").** The version of a
  project without `quillshade/project.ron` depends on what `quillshade/` holds:
  - `quillshade/` missing, or holding no non-hidden entries (names starting with `.` are ignored) → a **new project at
    `CURRENT`**; `ensure_layout` writes the file with `project_version: CURRENT` as before;
  - `quillshade/` holding any non-hidden entry besides `project.ron` → a **v1 project** (every project made before
    this decision had no other way to exist) → `OpenError::NeedsMigration { from: v1, to: CURRENT }`.
  The same rule applies in `Project::open`, `RawProject::open` and the CLI, so a hand-made or CLI-only v1 project (the
  CLI never calls `ensure_layout`) cannot be opened as current and then stamped `CURRENT`, which would skip every
  later step for good. Rejected: keeping "missing means `CURRENT`" and making `project.ron` mandatory in the README —
  a silent, permanent miss for exactly the projects the README calls valid.
- Per-file `format_version`s (`FORMAT_VERSION`, `CURVE_FORMAT_VERSION`, …) stay as idempotency markers; each domain
  owns its own `project version → per-kind format_version` table when it first adds a step.

### Crate `project_migrate` (D38)

Bevy-free, depends on `project_core` (plus `serde`, `ron`, `thiserror`); domain crates depend on it from their
Bevy-free backend modules.

- `RawProject { root, version }`: opened by `RawProject::open(root)` with the same `Cargo.toml` / `assets/` checks as
  `Project::open` and the version probe, but **no** version comparison (so it opens old and current projects; a
  too-new project is reported by the orchestrator as `MigrationError::TooNew`). Path helpers mirror `Project`
  (`quillshade_dir`, `domain_dir`, `settings_path`, `rel`).
- `DomainMigrator` exactly as in techdoc §4.4 (`domain`, `files`, `is_fresh`, `collect` with a default no-op,
  `migrate`), plus `Send + Sync`. `files` returns absolute paths; the orchestrator sorts them.
- `MigrationFacts`: a deterministic string store namespaced by the publishing domain
  (`BTreeMap<(DomainId, String), String>`, `insert(domain, key, value)`, `get(domain, key)`, `iter(domain)`), reset
  for every step. Phase 1 publishes `path → uuid` through it; it may grow typed helpers then.
- `MigrationError` (`thiserror`): open/probe failures, `TooNew`, I/O with the path, and a domain failure
  `{ domain, path, message }`.
- **Steps table**: `project_migrate::STEPS: &[StepInfo { to, title }]`, contiguous from v2, the last `to` equal to
  `ProjectVersion::CURRENT` (unit test). Titles are what the UI and `--dry-run` show. Phase 0 has one step:
  `v2 — "Rename format_version to project_version in project.ron"`.
- **Built-in project migrator** (`ProjectMigrator`, domain `project`, D38 "project.ron is owned by the built-in
  project domain"): lives in `project_migrate`, is always appended by the orchestrator itself (no domain registers
  it) and always runs **last** in a step. `files` = `[quillshade/project.ron]`, also when the file is missing (a v1
  project without it, see above); `is_fresh(v)` = probed version `>= v` and no legacy `format_version` key (a
  missing file is v1, never fresh);
  `migrate(v)` rewrites the version to `v` (for `v == 2` it also renames the key), preserving every other setting, or
  **creates** the file from `ProjectSettings::default()` with `project_version: v` when it is missing; either way it is
  written in the canonical `project.ron` text format (`project_core::layout` settings writer: struct names, 2-space
  indent, `\n`, final newline). For steps after v2 it is a pure version bump. Comments and keys `ProjectSettings` does
  not know are not kept: the first rewrite of a `project.ron` that existed before the run is reported as
  `MigrationEvent::Note("project.ron rewritten: comments and unknown keys are not kept")`, which the CLI prints
  (`note: …`) and the modal logs.

### Orchestrator (D39)

`Migrator::new(project: RawProject, migrators: &[&dyn DomainMigrator], compiled: &[DomainId])`:

```text
if version > CURRENT: TooNew;  if version == CURRENT: nothing to do
for v in version+1 ..= target:                      // barrier: the whole project reaches v before v+1
    facts = {}
    for each domain (by id, project last), each file (sorted): collect(file, text, v, &mut facts)
    stale = [(domain, file) | !is_fresh(file, text, v)]      // all computed before any write of this step
    for (domain, file) in stale: write_atomic(file, migrate(file, text, v, &facts))   // one temp+rename per file
    // project.ron is the project migrator's stale file, always last: the step's commit point
```

- `target` is `CURRENT`; `with_target(v)` exists for tests (synthetic multi-step chains with a test domain).
- **Dry run = in-memory simulation.** `plan()` runs the same loop over an in-memory overlay of file texts and writes
  nothing, so later steps see earlier steps' output; it returns `MigrationPlan { from, to, steps: [PlannedStep { to,
  title, stale: [(domain, rel_path)] }], warnings }`. The UI and `--dry-run` list exactly this.
- `run(observer)` performs the writes and reports `MigrationEvent`s (`StepStarted`, `FileMigrated`, `StepFinished`,
  `Warning`) to an `FnMut` observer (the editor forwards them over a channel; the CLI prints them). It returns a
  `MigrationReport` (the plan actually executed and the written files).
- **Failure and resume.** A failing `is_fresh` aborts the step before it writes anything; a failing `migrate` aborts
  after the files already written in that step. In both cases `project.ron` is not bumped, so a rerun restarts that
  step; files already migrated are fresh and skipped (per-file idempotency). Tests prove interruption + rerun equals a
  clean run and that a second run changes nothing.
- **Uncompiled domains.** A directory `quillshade/<dir>/` whose name is not in `compiled` (and not hidden) yields a
  warning "domain `<dir>` is not compiled into this editor; its files are not migrated". A compiled domain without a
  migrator has no steps: nothing to warn. Disabled domains (`enabled: false`) are migrated like enabled ones.
- **Generated files are never migrated**: after a successful real run the caller runs a full regen (all backends, all
  domains) as part of the run — the CLI before it exits, the editor on the run thread before it reports the result
  — so quitting right after a migration never leaves stale generated files behind (the next open sees a current
  project and regenerates nothing). `project_migrate` does not depend on the backends.
- **Git check.** `project_migrate::git_state(root) -> GitState { Clean, Dirty { changed }, NotARepo, Unavailable }`
  runs `git -C <root> status --porcelain -- .` (only the project directory counts); any failure to run git is
  `Unavailable`. It never blocks; the UI and CLI only warn ("commit first so the migration can be reviewed and
  reverted").

### Registration (decided: on `EditorDomain`)

- `editor_core::domain::EditorDomain` gains `fn migrator(&self) -> Option<Box<dyn project_migrate::DomainMigrator>>`
  with a default of `None`, next to `backend()`. `DomainList::migrators()` collects them for the editor;
  `src/main.rs`'s CLI collects them from the same `domains()` list it already takes the backends from — one
  registration list for GUI and CLI. The built-in `ProjectDomain` returns `None` (its migrator is built into the
  orchestrator).
- Rejected: `DomainBackend::migrator()` — `DomainBackend` lives in `project_core`, which `project_migrate` depends on
  (D38), so it would need a dependency cycle or the trait in `project_core`. Rejected: a separate parallel list in
  `src/main.rs` — a second list to keep in sync with `domains()`.
- A domain keeps its step code in its Bevy-free backend (e.g. `light_domain::backend::migrate::v3`); the rules of
  techdoc §4.4 about frozen DTOs apply. Phase 0 adds no Material or Light migrator (their formats do not change).

### CLI

- `shader-editor migrate <project> [--dry-run]` on the same engine. Output: `project v1 -> v2`, then per step
  `step v2: <title>` and one `  migrate <domain> <rel path>` line per stale file; warnings on stderr (`warning: …`),
  including the git warning (real runs only). A real run then regenerates everything and prints the regen changes
  like `regen` does.
- Exit codes: `0` success or already up to date (`project is up to date (v2)`); `1` migration error or error
  diagnostics from the regen; `2` usage / open errors, including `TooNew` and "not a project".
- `check` / `regen` / `diff` on an old project: `error: project version v1 is older than this editor (v2)` plus
  `hint: run \`shader-editor migrate <path>\``, exit `2`. A too-new project: error, exit `2`.

### Editor (Migration Tool modal)

- `Editor::open_project` / `switch_project` (CLI argument, last project at startup, recent projects, Open dialog):
  `NeedsMigration` does not open the project; it starts a `MigrationSession` (`Editor::migration`) with the plan and
  the git state. `TooNew` shows the usual "Cannot open project" error dialog.
- While a session exists the modal covers the window and every other action is ignored (the rest of the editor is
  unavailable, D28). The modal (`editor_core::ui::migration`, a `FeathersDialog` redrawn from `Editor::migration`)
  shows the project path, `v1 -> v2`, the git line, the steps with titles, stale files grouped by domain, and buttons
  **Cancel** / **Dry run** / **Migrate**. While the migration runs on a background thread (events over a channel) a
  progress bar and the log follow it and every button is disabled (the primary one captioned "Migrating..."). After
  the migration the same thread regenerates every enabled domain headless (`project_core::regen::run` with the
  domains' backends, `DomainList::backends`; `DomainBackend` is `Send + Sync` for this), its changes and diagnostics
  in the log. The result: success → **Open project** (opens normally; nothing left to regenerate); failure (of the
  migration or of the regen) → the error in the body with **Cancel** / **Dry run** / **Retry** (Retry runs again and
  resumes where the failed run stopped; after a failed regen it only regenerates). Cancel leaves no project open.
- **Keyboard focus.** Feathers' modal focuses its last focusable child (Migrate / Retry) shortly after it spawns,
  and Enter / Space press a focused Feathers button past the editor's hotkey gate; the modal respawns on every
  change. So Cancel carries `AutoFocus` in Ready and Failed: a stray Enter never starts a migration.
- The modal's texts use ASCII (`->`, `:`, `...`): the editor's UI font has no glyphs for `→` or `—`.
- **Quit while a migration runs is deferred.** While the session is `Running`, Quit / ForceQuit / closing the window
  do not exit: the session records `quit_requested`, and the app quits once the run thread reports `Finished`
  (success or failure). A crash midway is safe (the next run resumes), but a deliberate quit should not leave a half
  step behind. Outside `Running` (plan shown, result shown) quitting works as usual.
- Built with the existing dialog / Feathers widget infrastructure. Screenshot automation (`SHADER_EDITOR_SCREENSHOT`
  with an older project as the CLI argument): without `SHADER_EDITOR_DEMO` the shot shows the modal as opened, with
  the plan; `SHADER_EDITOR_DEMO=migration_dry_run` presses **Dry run** and `migration_run` presses **Migrate** (frame
  150; the shot shows the log, resp. the result); `migration_open` presses **Migrate**, then **Open project** once the
  run is done, so the shot shows the migrated project opened normally.

## Consequences

- From this phase on, every format change is a numbered step in `STEPS` with domain-owned file migrations and tests
  (golden before → after, fresh after migrate). `examples/sample_game` is migrated by the tool, never by hand.
- `project.ron` files written by this editor are unreadable by older editors (they report a missing
  `format_version`); older projects are refused by the CLI until migrated (exit 2) and open into the Migration Tool
  in the GUI.
- `quillshade/project.ron` is no longer optional for a project with sources: one without it is v1 and is migrated
  (the project migrator creates the file); the README no longer calls it optional.
- Known limits: facts are string-typed; no Material/Light steps yet.
- `project.ron` replaced by an older one while its project is open (e.g. a git checkout) is not migrated on the fly:
  the watcher's `reload_settings` refuses it like any settings error, keeps the previous settings and logs a
  warning; the Migration Tool appears only when the project is opened again.

## History

- Phase 0 reuses three earlier commits for the project version, `project_migrate`, the CLI `migrate` command and the
  sample migrated by the tool. They land as **one** commit: their intermediate states fail the sample-game tests
  (the sample's `project.ron` and the editor's `CURRENT` must change together).
