# Data Domain & Project Identity — Technical Design and Roadmap

Status: **approved design; Phases 0–1 implemented (ADR 0046, ADR 0047)**, Phases 2–5 not implemented. Owner decisions
are final unless the owner reopens them.
Origin: research session of 2026-10-31 (decision IDs `D1`–`D40` below are referenced by later ADRs and tasks).

---

## 0. How to Use This Document

This document is the single source of truth for a multi-phase effort:

1. a **Migration Tool** (project versioning and domain-owned file migrations),
2. **UUID identity** for every editor asset plus a cross-domain **`AssetIndex`**,
3. a shared generated **runtime module** (`quillshade::common`) for curves and gradients,
4. a new **Data domain** (UE DataTable–style DTO schemas, collections, singletons) with codegen, baking, hot-reload,
   and an editor.

Rules for planning and implementation agents:

- **Planning agents** decompose one phase from §10 into tasks. A task must name the decision IDs (`D#`) and sections
  (`§N`) it implements, its files/crates, and its acceptance checks. Do not re-decide anything listed in §3; if a
  task uncovers a real contradiction, stop and escalate to the owner instead of picking a side silently.
- **Each phase starts with its ADR** (`docs/decisions/NNNN-*.md`, next free numbers after 0045; add it to
  `docs/decisions/README.md`). The ADR condenses the relevant part of this document and records task-level choices
  marked **"decide in ADR"** here.
- Follow `AGENTS.md` without exception: English-only code and docs, dedicated git branch per phase, sequential
  stages, lead-agent review after each stage, `BTreeMap`/`BTreeSet` determinism, never hand-edit generated files
  (regenerate with `cargo run -- regen examples/sample_game`), full verification list (§12) before finishing a stage.
- Region names (mainbar, secondbar, left sidebar, editor topbar, editor content, editor statusbar, right sidebar,
  console/logs) are used exactly as defined in `AGENTS.md` §4.

---

## 1. Summary

**Bevy Quillshade** compiles human-authored source assets (`quillshade/<domain>/…`) into a generated umbrella crate
(`crates/quillshade`) plus baked runtime assets (`assets/quillshade/<domain>/…`) that hot-reload in the game. Today it
has the `material` and `light` domains and the built-in `project` domain; assets are identified by **path**.

This effort adds a **Data domain**: users define DTO schemas (structs, enums/unions) with typed fields, then author
content as **collections** (tables of rows of one DTO type) or **singletons** (one named DTO instance). The editor
generates strongly typed Rust (`quillshade::data::*`) and bakes validated data into compact binary files that the game
hot-reloads through the Bevy asset system. Rows are addressed by UUIDv7. References from data to other editor assets
(materials, curves, gradients, textures, light environments) require stable cross-domain identity, so all editor
assets move from path identity to **UUID identity**, which in turn requires a **project migration** framework.

### Goals
- Arbitrary DTO schemas with a rich, deterministic type system (§7.2).
- Collections (many per DTO type) and named singletons.
- O(1) row lookup by UUID, ordered iteration, typed soft references, no panics on bad data.
- Hot-reload of **data values** without recompiling the game.
- Editor-owned, git-friendly RON sources; validation with clear diagnostics.
- Stable identity for all editor assets; rename/move never breaks references.
- A reusable migration framework for this and all future format changes.

### Non-goals (explicit)
- Hand-editing Data sources outside the editor (D3). Sources are not designed for humans.
- Hot-reload of **schema** changes (D1): schema changes require rebuilding the game.
- Composite / override tables, DLC/mod layering (D22).
- Tags / gameplay tags (D12).
- A localization tool (D11): `LocalizedString` is only a typed wrapper over `String` for now.
- Previews/thumbnails for game-asset references (D10).
- Named rows / row constants in the generated API for the MVP (D20, D29).
- Embedded databases (§6).

---

## 2. Glossary

| Term | Meaning |
| :--- | :--- |
| **Asset UUID** | UUIDv7 stored inside every editor-owned source file (`id: "…"`); the asset's identity. The path is only its *name*. |
| **Sub-item** | An addressable element inside an asset: a global constant in `.consts.ron`, a gradient slot in `.gradients.ron`. Identified by `(asset uuid, stable local id)`; both already carry numeric `id`s today (`GlobalConst(id: 1001…)`, `Slot(id: 1007…)`). |
| **`AssetIndex`** | Cross-domain index in `project_core`: UUID → (domain, kind, path, sub-item) plus forward and reverse reference edges. Detects duplicate UUIDs and dangling references. |
| **Project version** | Single global integer in `quillshade/project.ron` (`ProjectSettings.project_version`, today `2`; v1 spelled it `format_version`). Raised by every migration. |
| **Migration step** | Upgrade of the whole project from version `vN` to `vN+1`, executed by `project_migrate`, with file changes delegated to the owning domains. |
| **DTO** | A Data schema: `struct`-like (`.dto.ron`). Generates a Rust `struct`. |
| **Enum / union** | A Data schema of variants, optionally with payload fields (`.enum.ron`). Generates a Rust `enum`. A variant without fields is a plain enum value; with fields it is a tagged union (`FInstancedStruct` / CastleDB `Custom` analogue). |
| **Collection** | A table of rows of one DTO type (`.collection.ron`). Many collections may share a DTO type. |
| **Row** | One DTO instance in a collection, identified by a project-unique UUIDv7. Rows have **no names**. |
| **Singleton** | One named DTO instance (`.singleton.ron`), accessed by its path-derived name. |
| **Field UUID** | Identity of a schema field / enum variant / variant field. Row values are keyed by field UUID; the field *name* exists only in the schema for Rust identifiers. |
| **`SoftRef<T>`** | Data field type: reference to a row of DTO `T`, stored as the row UUID only. |
| **`DataRef<T>`** | Runtime handle to a row: resolves through the loaded collections; returns `Option`. |
| **`DATA_SCHEMA_<hash>`** | Structural hash of a DTO type (transitively including nested types). Embedded in generated Rust and in each baked file; a mismatch makes the game reject the file and keep the previous data. Analogue of Material's `SG_LAYOUT_<hash>`. |
| **Bake** | Writing validated, cleaned data as postcard binaries under `assets/quillshade/data/autogen/`. |
| **`quillshade::common`** | Generated umbrella module shared by domains at runtime: curve data and sampling, gradient data and OKLab sampling. |

---

## 3. Decision Log (final)

| ID | Decision |
| :--- | :--- |
| D1 | Hot-reload covers **data values only**. Schema changes require rebuilding the game; the editor shows "schema changed — rebuild required". Guard: `DATA_SCHEMA_<hash>`. |
| D2 | The game reads **only baked files** (`assets/quillshade/data/…`), never sources. The editor's watcher triggers auto-regen. |
| D3 | Data sources are editor-owned; no requirement to edit them by hand; human readability is not a goal. |
| D4 | All IDs are **UUIDv7**, generated and validated only by the editor. |
| D5 | Duplicate row UUIDs are an editor error; offending rows are highlighted and skipped. The winner is chosen by sorting (tie-break in §7.6). |
| D6 | Every field has a UUID and a mandatory default; generated structs implement `Default` from schema defaults. The form shows a per-field "reset to default" button (same UX as the Light environment form: accent tint when the value differs from default). |
| D7 | Deterministic collections everywhere, generated runtime API included: `BTreeMap` / `BTreeSet`. |
| D8 | Recursion through containers is allowed (e.g. `Array<Self>` for dialogue trees); data depth is limited (D33). Direct by-value cycles are forbidden. |
| D9 | Tagged unions (variant DTOs) are in the MVP. |
| D10 | Game-asset references (files under `assets/` not owned by the editor: audio, scenes, glTF, images, fonts…) are stored by path and exposed as `UntypedHandle`; no previews. |
| D11 | `LocalizedString` is a newtype over `String` in the MVP; no localization tooling. |
| D12 | No tags. |
| D13 | Fields have settings (constraints and UI hints) that drive editor widgets (e.g. `min`/`max` → slider with limits). |
| D14 | Material references are exposed as `UntypedHandle`. |
| D15 | Curves are baked into data as raw points (`CurveData`). Sampling code (simple, Catmull-Rom) moves to `quillshade::common`, and Light uses it too. |
| D16 | Light environment references are exposed as the `light::Preset` enum. |
| D17 | Gradients are baked into data as raw stops (`bevy::Color` + positions + midpoints); OKLab sampling lives in `quillshade::common`. |
| D18 | **All editor asset kinds in all domains get a UUID**; path references are replaced by UUID references. |
| D19 | An intermediate layer (`AssetIndex`) resolves and validates UUID references across domains. |
| D20 | MVP rows have no names; the game obtains rows by iteration or by UUID. |
| D21 | Every dereference returns `Option`; nothing panics on missing/invalid data. |
| D22 | No composite tables. |
| D23 | Singleton is a separate asset kind, edited as a single-DTO form, accessible from the game by name. |
| D24 | Several collections per DTO type are allowed; Rust gets a generated enum of the collections of each type. |
| D25 | DTO types, enums, collections, singletons are addressed by UUID (follows from D18). |
| D26 | Game files (no editor-owned source, e.g. `assets/textures/x.png` without `.texture.ron`) stay path-addressed. Only editor-owned assets have UUIDs. |
| D27 | Wherever a source stores a UUID reference, the writer appends a regenerated trailing comment with the target's path (e.g. `// textures/rock_albedo`). Hand-written comments are not preserved. |
| D28 | A single global **project version**. Opening a project older than the editor blocks work and opens the **Migration Tool**. |
| D29 | Primary runtime row handle: `DataRef<T> { uuid }`, resolved through the loaded collections. Rows have no names (unlike UE); schema fields do. |
| D30 | A Data field referencing an asset of a **disabled domain** is a **bake error**. |
| D31 | Dense storage: rows store every value. Changing a schema default affects only rows created afterwards (database semantics). |
| D32 | Baked format: **postcard**. Source format: text, **one file per collection** (and per DTO/enum/singleton). |
| D33 | Data recursion depth limit: **64**. |
| D34 | UUID identity confirmed for all editor assets (§4.1). |
| D35 | Curves from every domain (Light included) are baked raw into data, provided hot-reload keeps working (§5.3). |
| D36 | Source text format: **RON**. |
| D37 | Singleton group in the API: **`data.singletons`**. |
| D38 | Migration Tool is a separate crate `project_migrate`. **File migrations belong to domains**: the orchestrator asks each domain whether a file is fresh and delegates `migrate` to it. |
| D39 | The migrator has a `collect` pre-pass producing shared `MigrationFacts` (needed by cross-domain steps), and walks versions one at a time with a barrier. |
| D40 | Singletons are grouped by directory: `data.singletons.balance.game_balance()` for `quillshade/data/balance/game_balance.singleton.ron`. Collections symmetric: `data.collections.<dirs>.…`. |

---

## 4. Cross-Cutting Architecture

### 4.1 Asset identity (UUIDv7) — D4, D18, D25, D26, D27, D34

**Rationale (why UUID over path, independent of cost):**

| Criterion | Path identity | UUID identity |
| :--- | :--- | :--- |
| Rename/move in the editor | rewrite every reference in every domain | nothing changes |
| Rename outside the editor (`git mv`, file manager, IDE) | references break; rename is indistinguishable from delete + create | works; the UUID travels with the file content |
| Merge: branch A renames, branch B adds a reference to the old path | silently broken after merge | correct |
| Game hot-reload after renaming a referenced asset | key changes; data references dangle until full regen | key is stable |
| Uniqueness | guaranteed by the filesystem | duplicates possible when a file is copied → `AssetIndex` detects them |
| Identity mechanisms in the project | two (paths + Data row UUIDs) | one |

Precedent: Godot moved from paths to `uid://` in 4.x (UIDs in `.import`, `.uid` files since 4.4) because path references
broke on moves; Unreal's path + redirector model ("Fix Up Redirectors") is a known chronic pain; Unity's GUIDs are right
but live in sidecar `.meta` files — here the UUID lives **inside** the asset file, avoiding that problem.

**Rules:**
- Every editor-owned source file gets a top-level `id: "<uuidv7>"` field: `.material.ron`, `.function.ron`,
  `.consts.ron`, `.curve.ron` (material and light), `.gradients.ron`, `.texture.ron` (material and light),
  `.environment.ron`, user node definitions (`quillshade/material/.nodes/*.ron`), and all Data kinds.
  `quillshade/project.ron` is not an asset and has no UUID.
- Sub-items are referenced as `(asset uuid, local id)`; the existing numeric local ids (`GlobalConst.id`, `Slot.id`)
  stay and must remain stable and unique within their asset.
- References in sources become UUIDs, each with a D27 trailing path comment. Examples today that change:
  `"table": Str("common/tuning")`, `"collection": Str("common/terrain")`, `"curve": Str("common/pulse")`,
  `kind: "function::common/tex_layer"`, `user::<id>`, texture ids such as `"textures/rock_albedo"`, Light
  `cubemap: Some("sky")`. The exact per-kind encoding is **decided in the Phase 1 ADR**.
- Game files stay path-addressed (D26). A Texture asset (`.texture.ron`) is editor-owned and gets a UUID; the image file
  it points at keeps its path.
- **Generated Rust names still derive from paths** (`baked.env.rock_wet`, `data.singletons.balance.game_balance()`).
  Renaming a file changes the generated API (a compile error in the game, which is expected) but never breaks data
  references.
- Editor commands that create or duplicate assets always mint new UUIDs. Copying a file outside the editor creates a
  duplicate UUID, reported by `AssetIndex` with a quick fix "regenerate UUID for this asset" (references keep pointing
  at the winner; winner = lexicographically smaller path, deterministic).
- UUID generation uses the `uuid` crate (already in `Cargo.lock`, 1.29) with the `v7` feature, in editor/backend
  crates only. Generated game code only parses/serializes UUIDs (stored as `u128` in baked files).

### 4.2 `AssetIndex` — D19

Location: `project_core::asset_index` (Bevy-free).

```rust
pub struct AssetEntry {
    pub id: Uuid,                 // asset uuid
    pub domain: DomainId,
    pub kind: AssetKindId,        // e.g. "material.curve", "light.environment", "data.collection"
    pub path: String,             // source path relative to the project root
    pub sub_items: Vec<SubItem>,  // (local id, label) for consts / gradient slots
}
pub struct AssetRef { pub from: Uuid, pub to: Uuid, pub sub: Option<u64> }

pub struct AssetIndex {
    by_id: BTreeMap<Uuid, AssetEntry>,
    refs_from: BTreeMap<Uuid, BTreeSet<RefKey>>,   // forward edges
    refs_to: BTreeMap<Uuid, BTreeSet<Uuid>>,       // reverse edges ("who references me")
    duplicates: Vec<(Uuid, Vec<String>)>,
}
```

- Each domain contributes entries and forward references through a new `DomainBackend` method (e.g.
  `fn index(&self, project: &Project) -> DomainIndex`). Scanning reads only headers/ids/references, not full models.
- `regen::run` builds the index across **all compiled-in backends** before planning, and passes `&AssetIndex` into
  `plan` (signature change across `MaterialBackend`, `LightBackend`, the fake test backend, and the Data backend).
- Diagnostics from the index (duplicate UUIDs, dangling references, references into disabled domains) are core codes;
  the prefix/numbering is **decided in the Phase 1 ADR** (core codes today: `E008`, `W105`; Material owns unprefixed
  `E001`–`E014`, `W101`–`W112`; Light uses `LE`/`LW`). Suggested: `AE###`/`AW###`.
- The editor keeps an `AssetIndex` in shell state, rebuilt incrementally on watcher events and after saves.
- `BakedMaterials` gains `by_id: BTreeMap<Uuid, UntypedHandle>` (D7) next to `by_path` (Material Rust codegen).

### 4.3 Cross-domain regeneration (required by D15/D17/D35)

- Saving an asset in domain A must regenerate dependents in other domains (e.g. a curve edited in Material or Light
  must re-bake every Data collection/singleton embedding it). Use `AssetIndex::refs_to` to compute affected assets
  and schedule a scoped regen of their domains (Material already has `Scope::Affected` internally; generalize the
  idea at the `project_core::regen` level).
- Hot-reload stays intact: re-baked files are rewritten → Bevy reloads those assets.
- Accepted cost: a widely used curve re-bakes many files; postcard is fast enough for indie scale. Future alternative
  (not now): a shared baked curve/gradient table with `CurveRef(uuid)` in data — possible later without source changes.

### 4.4 Migration Tool — D28, D38, D39

**Versioning**
- `quillshade/project.ron`'s `format_version` becomes the global **project version** (rename to `project_version` is
  **decided in the Phase 0 ADR**; the migration itself must handle the rename if done).
- Per-file `format_version`s (`FORMAT_VERSION`, `CURVE_FORMAT_VERSION`, `ENVIRONMENT_FORMAT_VERSION`,
  `CONSTS_FORMAT_VERSION`, `GRADIENTS_FORMAT_VERSION`, `TEXTURE_FORMAT_VERSION`, …) remain and act as idempotency
  markers. Each domain owns a table `project version → per-kind file format_version`.
- Open flow (`project_core::layout`):
  - `== CURRENT` → open normally;
  - `< CURRENT` → new `OpenError::NeedsMigration { from, to }`; the editor blocks work and opens the Migration Tool;
    CLI `check`/`regen`/`diff` exit with code 2 and a hint to run `migrate`;
  - `> CURRENT` → new `OpenError::TooNew { found, supported }`; refuse to open.

**Crate `crates/project_migrate`** (depends on `project_core` only; domain crates depend on it; Bevy-free):

```rust
pub trait DomainMigrator {
    fn domain(&self) -> DomainId;
    /// Source files this domain owns (under `quillshade/<domain>/`).
    fn files(&self, project: &RawProject) -> Vec<PathBuf>;
    /// `false` = the file is stale for project version `target`.
    fn is_fresh(&self, file: &Path, text: &str, target: ProjectVersion) -> Result<bool, MigrationError>;
    /// Optional pre-pass over all of this domain's files: publish facts other domains need at `target`
    /// (e.g. `path -> uuid` for the UUID step). Default: nothing.
    fn collect(&self, _file: &Path, _text: &str, _target: ProjectVersion, _facts: &mut MigrationFacts) {}
    /// Upgrade one stale file by exactly one project version, to `target`.
    fn migrate(&self, file: &Path, text: &str, target: ProjectVersion, facts: &MigrationFacts)
        -> Result<String, MigrationError>;
}
```

`RawProject` is a minimal project handle that does **not** require a current-version `project.ron` (opening via
`Project::open` would fail with `NeedsMigration`).

**Orchestrator**
```text
for v in project_version+1 ..= CURRENT:          // barrier: whole project reaches v before v+1
    facts = {}
    for each domain, each file: domain.collect(file, v, &mut facts)
    stale = [(domain, file) | !domain.is_fresh(file, v)]      // listed in UI / --dry-run
    for (domain, file) in stale: write_atomic(file, domain.migrate(file, v, &facts))
    write project.ron: project version = v                    // last; a crash => rerun is idempotent
then: full regen (generated files are never migrated, they are regenerated)
```

**Rules**
- A domain keeps its step code (e.g. `light_domain::migrate::v2`). A step may use the domain's current DTOs **only
  while it is the latest step**; when a later version changes those DTOs, the older step is moved to `ron::Value` or a
  frozen copy of the old structs **inside the domain**.
- `project.ron` is owned by the built-in `project` domain's migrator.
- Files of a domain not compiled into the editor are not migrated; the orchestrator reports a warning. Disabled
  domains (`enabled: false`) are migrated like enabled ones.
- Writes go through `project_core::writer` (temp file + rename per file).
- Registration happens in `src/main.rs` next to the `DomainBackend`s (e.g. `fn migrator(&self) ->
  Option<&dyn DomainMigrator>` on the backend, or a parallel list — **decide in Phase 0 ADR**).
- **UI**: modal Migration Tool (editor_core): current → target version, steps with titles, stale files grouped by
  domain, warning about uncommitted git changes (recommend committing first), dry-run, progress, log, result. The rest
  of the editor is unavailable until migration finishes. Uses existing dialog/widget infrastructure.
- **CLI**: `shader-editor migrate <project> [--dry-run]` on the same engine (exit codes consistent with the CLI: 0 ok,
  1 migration errors, 2 usage/open errors).
- **Tests**: per domain step — golden before → after, and "fresh after migrate"; orchestrator — interruption midway
  plus rerun yields the same result, a second run changes nothing, full chain `v1 → CURRENT` on an old fixture.

### 4.5 Shared runtime module `quillshade::common` — D15, D17

- Generated by the umbrella writer (`project_core::umbrella`) as `crates/quillshade/src/common/` (`curve.rs`,
  `gradient.rs`, `mod.rs`); always present regardless of enabled domains.
- `CurveData { interp: Interp, points: Vec<(f32, f32)> }` with `sample(t)` (Simple and Catmull-Rom, identical to
  `curve_kit::Curve::sample`).
- `GradientData { stops: Vec<GradientStop { t, color: Color, mid: Option<f32> }> }` with `sample(t) -> Color` using
  OKLab interpolation identical to `shader_graph_core::gradient::Slot::sample`.
- **Single source of truth**: the pure math (f32-only, Bevy-free) lives in one template file compiled into the editor
  side (`include!` from `project_core` or a tiny kit) **and** emitted verbatim into the umbrella; Bevy wrappers
  (`Color`) exist only in generated code. Parity tests compare editor and generated sampling on fixed inputs.
- The Light runtime template (`crates/light_domain/src/backend/templates/light_mod.rs`) drops its own curve sampling
  and uses `crate::common`. Verify with sample-game screenshots (`SAMPLE_SCREENSHOT`) before/after.

---

## 5. Data Domain — Overview

### 5.1 Layout

| What | Where |
| :--- | :--- |
| Sources | `quillshade/data/**`: `*.dto.ron`, `*.enum.ron`, `*.collection.ron`, `*.singleton.ron` |
| Generated Rust | `crates/quillshade/src/data/**` (`mod.rs`, types, collections, singletons, runtime) |
| Baked data | `assets/quillshade/data/autogen/<path>.<kind>.qsd` (one file per collection / singleton; extension **decided in Phase 3 ADR**) |
| Crate | `crates/data_domain` with `backend/` (Bevy-free, always built) and `editor/` (feature `editor`), mirroring `light_domain` |
| Domain id | `data` (`DomainId::new("data")`), plugin `QuillshadeDataInternalPlugin` via `CrateContribution` |
| Diagnostics | `DE###` (errors), `DW###` (warnings) |

### 5.2 Pipeline
1. Load sources (RON). A parse failure (corruption, git conflict markers) is an error for that asset; the editor
   refuses to open it.
2. Validate against `AssetIndex` and schemas (§7.6). Invalid rows are skipped, invalid assets are not baked.
3. Generate Rust for all valid schemas; compute `DATA_SCHEMA_<hash>` per type.
4. Bake each valid collection/singleton to postcard (§7.7).
5. Commit through `project_core::regen::commit` (orphan scan in the domain's roots, atomic batch).

### 5.3 Hot-reload semantics
- Value edits → re-bake → Bevy reloads the file → runtime swaps the collection; `DataRef`s see new values.
- Removed rows resolve to `None`; added rows appear in iteration.
- Schema edits → new `DATA_SCHEMA_<hash>` in baked files → the running game rejects them (warning, keeps previous
  data) until rebuilt; the editor shows "schema changed — rebuild required".
- Edits in other domains that are embedded in data (curves, gradients) re-bake dependents (§4.3).

---

## 6. Storage Format Research (decided: RON sources, postcard baked)

| Format | Kind | Verdict |
| :--- | :--- | :--- |
| RON | text | **Sources** (D36): consistent with the project, diff/merge in git, review-friendly. |
| JSON | text | Rejected for sources: inconsistent with the rest of the project. |
| **postcard 1.x** | binary, serde | **Baked** (D32): stable, documented wire format; compact; fast; actively maintained. Not self-describing, which is fine because `DATA_SCHEMA_<hash>` guarantees layout match. |
| bitcode | binary | Rejected: wire format may change across major versions → editor and game versions could diverge. |
| rkyv | zero-copy | Rejected: heavy codegen (`Archived*` types) with little gain, since data is materialized into owned structs anyway. |
| bincode | binary | Rejected: unmaintained (RUSTSEC-2025-0141; 3.0.0 is a `compile_error!` sentinel). |
| MessagePack / CBOR | binary, self-describing | Rejected: larger/slower than postcard; schema tolerance unnecessary given the hash guard. |
| SQLite / redb / fjall / native_db | embedded DB | Rejected for both sides. Runtime: Bevy `AssetReader` yields bytes (wasm, Android APK, `embedded://`, packed assets) while a DB wants a file path/mmap, and hot-reload is per asset file. Sources: a binary single file cannot be diffed or merged in git; locks/WAL vs `git checkout`. Queries/transactions are not needed: everything is in memory with generated indexes. |

Source layout S1 (one file per collection, D32) is chosen; known downside: two people appending rows concurrently get
a git conflict at the end of the file (resolve by keeping both).

---

## 7. Data Domain — Detailed Specification

### 7.1 Asset kinds

| Kind | File | Editor view (`ViewKindId`) | Purpose |
| :--- | :--- | :--- | :--- |
| DTO | `*.dto.ron` | `data.dto` | Struct schema: ordered fields. |
| Enum / union | `*.enum.ron` | `data.enum` | Variants, each with optional payload fields. |
| Collection | `*.collection.ron` | `data.collection` | Rows of one DTO type. |
| Singleton | `*.singleton.ron` | `data.singleton` | One named DTO instance. |

Merging plain enums and unions into one `.enum.ron` kind is the proposed default; confirm or split in the Phase 3 ADR.

### 7.2 Type system

**Scalars:** `bool`, `i8`/`i16`/`i32`/`i64`, `u8`/`u16`/`u32`/`u64`, `f32`, `f64`, `String`, `LocalizedString` (D11),
`Vec2`/`Vec3`/`Vec4`, `IVec2`/`IVec3`, `UVec2`/`UVec3`, `Quat` (edited as Euler degrees), `Color` (stored linear,
edited with the existing color widget), `Duration` (seconds, `f32`), `Range<f32>`, `Range<i32>` (min/max).

**Composites:**
- `Option<T>`
- `Array<T>` → `Vec<T>`; `FixedArray<T, N>` → `[T; N]`
- `Set<K>` → `BTreeSet<K>` (D7)
- `Map<K, V>` → `BTreeMap<K, V>` (D7)
- `Dto(uuid)` — inline struct by value
- `Enum(uuid)` — enum or tagged union by value

**Key types** for `Set`/`Map`: integers, `bool`, `String`, plain enums (no payload), `SoftRef<T>`. Floats, vectors,
colors, and payload unions are forbidden as keys (no `Eq`/`Ord`).

**References:**

| Field type | Stored in source/bake | Runtime type | Notes |
| :--- | :--- | :--- | :--- |
| `SoftRef(dto uuid)` | row UUID | `SoftRef<T>` → resolve to `Option<&T>` | Row UUID is project-unique, so no table id (multi-collection safe). Picker lists rows of every collection of `T`. |
| `Material` | material asset UUID | `UntypedHandle` | D14; resolved via `BakedMaterials::by_id` in the apply system (not in the `AssetLoader`). |
| `Texture` | texture asset UUID | `Handle<Image>` | Through Material's `BakedTextures` (keeps Texture asset sampler settings). |
| `Curve` | curve asset UUID (any domain) | `CurveData` (baked raw) | D15/D35. |
| `Gradient` | (gradients asset UUID, slot id) | `GradientData` (baked raw) | D17. |
| `LightEnvironment` | environment asset UUID | `light::Preset` | D16; compile-time dependency on `crate::light`. |
| `GameAsset(kind)` | path under `assets/` | `UntypedHandle` | D10/D26. `kind` ∈ {Image, Audio, Scene, Gltf, Font, …} so the runtime can call the typed `load::<A>()` and erase it; exact list **decided in Phase 3 ADR**. |

**Cycles and depth:**
- By-value cycles (`A { b: B }`, `B { a: A }`, or `A { a: A }`) are a schema error (DFS over the type graph).
- Recursion through `Array`/`Option`/`Map`/`Set` is allowed (D8); generated code must box where required
  (`Option<Box<T>>` for direct optional self-reference).
- Data nesting depth is limited to **64** (D33), enforced by the editor; baked files never exceed it.
- Cycles through `SoftRef` are always allowed.

### 7.3 Field settings (D13)
Per field, optional, type-dependent: `min`, `max`, `step`, `slider: bool`, `multiline: bool` (strings), `max_len`,
`doc` (tooltip), `required` (non-empty string / `Some`), `unique_in_collection: bool`, `hidden_in_list: bool`, and
display hints for colors/vectors. Settings drive the widget choice in forms (slider when `min` and `max` are both set
and `slider` is on; `number_ranged` otherwise) and produce validation diagnostics.

### 7.4 Defaults (D6, D31)
- Each field declares a default value in the schema; generated types implement `Default` with those values (not
  `#[derive(Default)]` unless every field default equals the Rust default).
- Storage is dense: new rows copy current defaults; existing rows never change when a default changes.
- "Reset to default" copies the current schema default into the field; the reset icon is accent-tinted when the value
  differs from the default (mirror `light_domain::editor::form` `EnvironmentResetIcon`).

### 7.5 Source file formats (illustrative; exact field names decided in Phase 3 ADR)

```ron
// quillshade/data/library/book.dto.ron
Dto(
  format_version: 1,
  id: "0192f3a1-7c2e-7d4b-9a10-3c5e8f0b1a22",
  doc: "A book in the library.",
  fields: [
    Field(
      id: "0192f3a1-7c2e-7d4b-9a10-3c5e8f0b1a23",
      name: "title",
      ty: LocalizedString,
      default: Str(""),
    ),
    Field(
      id: "0192f3a1-7c2e-7d4b-9a10-3c5e8f0b1a24",
      name: "pages",
      ty: U32,
      default: U32(100),
      settings: (min: Some(1.0), max: Some(5000.0), slider: true),
    ),
    Field(
      id: "0192f3a1-7c2e-7d4b-9a10-3c5e8f0b1a25",
      name: "author",
      ty: SoftRef("0192f3a0-1111-7aaa-8bbb-000000000001"), // library/author
      default: None,
    ),
  ],
)
```

```ron
// quillshade/data/library/books_core.collection.ron
Collection(
  format_version: 1,
  id: "0192f3a2-0000-7000-8000-000000000010",
  dto: "0192f3a1-7c2e-7d4b-9a10-3c5e8f0b1a22", // library/book
  rows: [ // sorted by row id (UUIDv7 = creation order)
    Row(
      id: "0192f3a2-0000-7000-8000-000000000011",
      values: {
        "0192f3a1-7c2e-7d4b-9a10-3c5e8f0b1a23": Str("Dune"), // title
        "0192f3a1-7c2e-7d4b-9a10-3c5e8f0b1a24": U32(412), // pages
        "0192f3a1-7c2e-7d4b-9a10-3c5e8f0b1a25": Ref("0192f3a2-0000-7000-8000-000000000099"), // author
      },
    ),
  ],
)
```

```ron
// quillshade/data/balance/game_balance.singleton.ron
Singleton(
  format_version: 1,
  id: "0192f3a3-0000-7000-8000-000000000001",
  dto: "0192f3a3-0000-7000-8000-0000000000aa", // balance/game_balance_dto
  values: { /* field uuid -> value, same encoding as rows */ },
)
```

Writer rules: deterministic formatting, one value per line, rows sorted by UUID, map keys sorted (D7), trailing D27
comments regenerated on every save (field names for value keys, paths for references).

### 7.6 Validation (diagnostic codes allocated in the Phase 3 ADR)
Errors (asset or row is excluded from bake):
- RON parse error / unsupported `format_version`.
- Duplicate asset UUID (core, from `AssetIndex`); duplicate row UUID anywhere in the project (D5).
  **Winner**: sort by `(row uuid, collection uuid, collection path, row position)`; the first is kept, the others are
  skipped and reported. (Identical UUIDs cannot be ordered by UUID alone, hence the tie-break.)
- Invalid identifier for type/field/variant names (`project_core::ident`), duplicate field names in a schema.
- By-value type cycle; unknown type UUID; forbidden key type.
- Value type mismatch; missing field (dense storage requires all fields — the editor fills defaults on schema change);
  unknown field UUID in a row (kept in the source until the next save, then dropped with a warning — never silently).
- Dangling `SoftRef` / asset reference; reference into a disabled domain (D30); wrong target kind.
- Data depth > 64 (D33); constraint violations marked as errors (`required`, `unique_in_collection`).
Warnings: soft constraint violations (`min`/`max` out of range if configured as warnings), empty collections, unused
schemas.

Schema-change handling in the editor: added field → fill rows with the default; removed field → drop values on next
save with a warning; type change → convert where lossless (e.g. `i32 → f32`), else reset to default with a per-row
warning; delete + re-create of a field yields a new UUID (old values are not restored).

### 7.7 Baked format (D32)
Per collection/singleton file:
```text
magic      : b"QSDATA\0\0"     (8 bytes)
version    : u32 LE            (baked container format version)
schema     : u64 LE            (DATA_SCHEMA hash of the DTO type)
dto        : u128 LE           (DTO type uuid)
payload    : postcard(BakedCollection<T> { rows: Vec<(u128, T)> })   // or BakedSingleton<T> { value: T }
```
- Rows are already validated, deduplicated, depth-checked, and sorted by UUID.
- Embedded curves/gradients are raw values; asset references are stored as UUIDs (materials, environments, textures)
  or paths (game assets) and resolved at apply time.
- Runtime loader: a custom `AssetLoader` per kind; header mismatch or decode error → keep the previous data and log a
  warning (never panic).
- Whether a compiled-in fallback copy is embedded (like Material/Light `COMPILED_MANIFEST`) is **decided in Phase 3
  ADR**; default proposal: no embedding, data becomes available after loading, signalled by a `DataReady` message
  and per-collection load state.

### 7.8 Generated Rust API (sketch; names final in Phase 3 ADR)

```rust
// crates/quillshade/src/data/… (generated)
pub enum BookCollection { BooksCore, BooksDlc }                    // D24: collections of type `Book`
pub enum Effect { Damage { amount: f32 }, Heal { amount: f32 } }   // D9 union

#[derive(Clone, Debug, PartialEq)]
pub struct Book {
    pub title: LocalizedString,
    pub pages: u32,
    pub author: SoftRef<Author>,
    pub cover: Option<UntypedHandle>,          // GameAsset(Image)
    pub material: Option<UntypedHandle>,       // Material (D14)
    pub fade: CurveData,                       // crate::common (D15)
    pub genres: BTreeSet<Genre>,
    pub effects: Vec<Effect>,
}
impl Default for Book { /* schema defaults (D6) */ }

fn system(data: Data) {                                            // `Data` is a SystemParam
    for (id, book) in data.collections.library.books(BookCollection::BooksCore) { /* uuid order */ }
    let book: Option<&Book> = data.book(id);                       // global O(1) across all collections of Book
    let author: Option<&Author> = book.and_then(|b| data.get(&b.author));
    let r: DataRef<Book> = DataRef::new(id);                       // store in components; resolve each use
    let cfg: Option<&GameBalance> = data.singletons.balance.game_balance();   // D37, D40
}
```

- Storage per collection: rows in UUID order plus a `BTreeMap<Uuid, usize>` index (or `HashMap` *only* if it never
  feeds ordered output — D7 prefers `BTreeMap`); a global per-type index across collections.
- `DataRef<T>` is the primary handle (D29); everything returns `Option` (D21).
- A `DataChanged<T>` message (or per-collection change events) after each reload so systems can react.
- **Bevy labeled sub-assets are not used for rows.** Finding from `bevy_asset-0.20.0/src/server/info.rs`
  (`process_asset_load`, `should_reload`, `remove_dependents_and_labels`): on reload, labels that disappeared are not
  removed from `Assets<T>` and no `Removed` event is sent, so a live handle keeps returning stale data — violating
  D21; also every reload sends `Modified` for every row. Exposing `file#<uuid>` sub-assets may be revisited later as a
  convenience with that caveat documented.
- Dependencies of the generated `data` module: `crate::common` always; `crate::light` only if any schema uses
  `LightEnvironment`; `crate::material` for material/texture resolution. A disabled target domain is a bake error
  (D30), so generated code never references a removed module.
- `CrateContribution` for Data adds `serde` and `postcard` dependencies (and `uuid` with `serde` if UUIDs are typed as
  `Uuid` rather than `u128`; **decide in Phase 3 ADR**).

### 7.9 Editor (Data domain GUI, `data_domain::editor`)
Built on `editor_core` (`EditorDomain`, `DomainRegistrar`, `TabDoc`, `EditorAreas`, undo history, `ui::props`
widgets: `number`, `number_ranged`, `integer`, `text_w`, `text_area`, `checkbox`, `select_values`, `vector_fields`,
color input).
- **Left sidebar**: project tree of `quillshade/data/**`; `Assets` menu → New DTO / Enum / Collection / Singleton /
  folder; the assets tree shows game assets (needed for `GameAsset` pickers).
- **DTO / enum editor** (`data.dto`, `data.enum`): ordered field list (add, remove, reorder, rename), type picker
  (nested types, containers, references), default value editor, settings editor; enum variants with payload fields.
  The editor statusbar warns "schema changed — rebuild required" after structural edits.
- **Collection editor** (`data.collection`): editor content = virtualized row list (UUID short form, first string
  field as label, search, filter by errors) + property form of the selected row; the right sidebar = problems and
  hotkeys. Row commands: add (defaults), duplicate (new UUID), delete, copy/paste values. A spreadsheet/grid view for
  scalar columns is a later enhancement (Phase 5).
- **Singleton editor** (`data.singleton`): the same property form for one instance.
- **Property form**: widgets from field type + settings (D13), reset-to-default per field (D6), nested structs as
  collapsible groups, arrays/maps/sets with add/remove/reorder, union variant switcher (keeps payload values per
  variant while editing), reference pickers backed by `AssetIndex` (rows of `T`, materials, curves, gradients,
  textures, environments, game assets by kind) with a D27-style path label.
- **Errors**: invalid rows/fields highlighted inline; dangling references show the missing UUID; duplicates offer
  "regenerate UUID".
- **Watcher**: external changes to `quillshade/data/**` (e.g. git checkout) reload open documents (dirty-tab conflict
  handling as in other domains) and trigger regen.
- Icons follow `AGENTS.md` icon rules (`DATA_ICONS` list, `assert_icons_rasterize`).
- Screenshot automation: extend `SHADER_EDITOR_DOMAIN` to accept `data`, `SHADER_EDITOR_OPEN` for data assets.

---

## 8. Repository Changes Overview

| Area | Change |
| :--- | :--- |
| `crates/project_core` | `asset_index`, `DomainBackend::index`, `plan(&Project, &AssetIndex, compile)`, `OpenError::{NeedsMigration, TooNew}`, project version, umbrella emits `common/`, cross-domain affected regen. |
| `crates/project_migrate` (new) | `DomainMigrator`, `MigrationFacts`, `RawProject`, orchestrator, dry-run report. |
| `crates/shader_graph_core`, `shader_codegen_*`, `shader_project`, `material_domain` | asset UUIDs, UUID references in graphs/consts/gradients/functions/user nodes, D27 comments, `BakedMaterials::by_id`, Material migrator, index contribution. |
| `crates/curve_kit`, `crates/texture_kit` | UUID field in models; shared curve math source. |
| `crates/light_domain` | UUIDs, UUID references (`cubemap`), Light migrator, runtime uses `crate::common`. |
| `crates/editor_core` | Migration Tool modal, open-flow blocking, `AssetIndex` in shell state, cross-domain regen trigger. |
| `crates/data_domain` (new) | backend (model, validate, codegen, bake, migrator, index) + editor feature. |
| `src/main.rs` | register `DataBackend`/`DataDomain`, migrators; CLI `migrate`. |
| `examples/sample_game` | migrated sources; sample Data assets (`crates/data_domain/examples/make_sample.rs`); game code using `quillshade::data`. |
| `docs/` | ADRs per phase, `docs/techdoc.md` updates, `README.md`, `AGENTS.md` (glossary, file tree, domains, verification command 3 gains `data_domain`). |

---

## 9. Risks and Known Limitations

- **Scope**: the Data domain is effectively a second product (CastleDB-class tool). The editor UI is the largest cost
  (virtualized lists, nested forms, pickers, undo); keep the grid view out of the MVP.
- **Phase 1 is invasive**: every domain's model, references, codegen, tests, snapshots, and the sample game change;
  material/light sources become less readable (mitigated by D27 comments).
- **Copied files duplicate UUIDs** (any domain); detection and quick fix are mandatory.
- **Schema changes need a game rebuild**; designers must see this clearly.
- **All data in memory**; fine for indie scale (tens of thousands of rows), documented as a limitation.
- **Concurrent row appends** in S1 files conflict in git (resolvable by keeping both sides).
- **Cross-domain re-bake fan-out** for widely used curves/gradients (§4.3).
- **Stack depth**: serde recursion with deep data — bounded by D33.
- **Parity** between editor and generated curve/gradient sampling — enforced by tests (§4.5).

---

## 10. Roadmap

Each phase: its own branch and ADR, sequential stages, lead review per stage, full §12 verification, sample game
regenerated and checked. Later phases depend on earlier ones in order.

### Phase 0 — Migration Tool (D28, D38, D39; §4.4)
- `crates/project_migrate`: trait, facts, `RawProject`, orchestrator, dry-run report, atomic writes.
- Project version handling and `OpenError::{NeedsMigration, TooNew}` in `project_core`; CLI exit codes.
- Built-in `project` domain migrator; one trivial step (version bump only) to exercise the machinery.
- Editor: blocking Migration Tool modal; CLI `shader-editor migrate <project> [--dry-run]`.
- **Done when**: an older fixture project opens into the modal, migrates, and then opens normally; interruption +
  rerun and double-run tests pass; CLI dry-run lists stale files; ADR merged.

### Phase 1 — Asset UUIDs + `AssetIndex` (D4, D18, D19, D25–D27, D34; §4.1–4.3)
- UUID field on every editor asset kind; sub-item refs as `(uuid, local id)`; UUID references with D27 comments in all
  sources; writers updated.
- `AssetIndex` with forward/reverse edges, duplicate and dangling detection, core diagnostics; `DomainBackend::index`;
  `plan` receives the index; editor shell keeps it updated; "regenerate UUID" quick fix.
- Generic cross-domain affected regen at `project_core` level.
- `BakedMaterials::by_id`.
- Migration step (first real one): `collect` path→uuid facts, assign UUIDs, rewrite references, per domain.
- Editor rename/move no longer rewrites references (only the path comment updates on next save).
- **Done when**: `examples/sample_game` is migrated by the tool (not by hand) and regenerates identically in behavior;
  renaming a file outside the editor keeps references valid; a copied file is reported as a duplicate; snapshot tests
  updated through regeneration; ADR merged.

### Phase 2 — `quillshade::common` runtime (D15, D17; §4.5)
- Umbrella emits `common/` (curve + gradient data and sampling); single-source math with parity tests.
- Light runtime switched to `crate::common`; sample-game screenshots before/after show no regression.
- **Done when**: parity tests pass; Light hot-reload of curves still works; ADR merged.

### Phase 3 — Data backend (headless) (D1–D9, D13, D14, D16, D20–D24, D29–D33, D35–D37, D40; §5, §7.1–7.8)
- `crates/data_domain` backend: models for the four kinds, RON load/save with deterministic writer, validation and
  codes, schema hashing, Rust codegen (types, `Default`, collections enums, `Data` SystemParam, `DataRef`, `SoftRef`,
  singletons tree, change messages), postcard bake, runtime loaders, migrator stub, index contribution.
- `make_sample` writing sample Data assets; the sample game reads them (iteration, lookup, soft refs, singleton,
  curve/gradient sampling, material/texture/game-asset handles, light preset).
- CLI `check`/`regen`/`diff` cover the Data domain; snapshot tests for generated code.
- **Done when**: regen produces compiling generated code in the sample workspace; hot-reload of a value change is
  visible in the running sample game; a schema change is rejected at runtime with a warning; duplicate/dangling/
  disabled-domain cases produce the specified diagnostics and never panic; ADR merged.

### Phase 4 — Data editor (D6, D13, D23, D24; §7.9)
- `DataDomain` registration, tree/menus/templates/icons; DTO/enum editor; collection editor (virtualized list +
  property form); singleton editor; reset-to-default; settings-driven widgets; undo; watcher handling; rebuild-required
  notice; screenshot automation hooks.
- **Done when**: a user can create a schema, a collection, and a singleton entirely in the GUI, see them in the
  running sample game via hot-reload, and every error case is visible inline; `docs/ui-checklist.md` extended; ADR
  merged.

### Phase 5 — Cross-domain pickers, unions, recursion polish
- Reference pickers for all asset kinds via `AssetIndex`; union variant editing UX; recursive structures (dialogue
  tree example) with depth enforcement; optional grid view for scalar columns.
- **Done when**: the sample includes a recursive dialogue tree and a union-typed effects list editable in the GUI;
  pickers cover every reference type in §7.2.

---

## 11. Prior Art Notes

- **UE DataTable**: row key is a human `FName`; renaming a row silently breaks `FDataTableRowHandle`s; renaming or
  retyping struct fields drops values unless `CoreRedirects` are used; tables load fully; binary `.uasset`.
  Quillshade answers: UUID rows (D4), field UUIDs (D6), migrations (§4.4), text sources (D36).
- **UE CompositeDataTable** (layering) — out of scope (D22). **UE Data Registry** (id-based indirection, streaming)
  — motivates UUID identity; streaming out of scope. **UE `FInstancedStruct`** — motivates unions (D9).
  **UE PrimaryDataAsset** — motivates singletons (D23).
- **CastleDB** (Dead Cells, Northgard): closest analogue — typed sheets, `Ref`, `Enum`, `List`, `Custom` unions.
- **Unity ScriptableObject / GUID in `.meta`** — duplicate GUIDs on file copy; we store UUIDs in-file and detect
  duplicates. **Godot Resources / `uid://`** — precedent for moving from paths to UIDs.
- **Bevy 0.20**: `Handle::Uuid` / `AssetId::Uuid` exist; labeled sub-asset reload behavior documented in §7.8.

---

## 12. Verification (every stage)

From `AGENTS.md` §5, all must pass:
```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo clippy -p curve_kit -p texture_kit -p light_domain -p data_domain --no-default-features --all-targets -- -D warnings
cargo test --workspace --no-fail-fast
cargo run -- check examples/sample_game --compile
cargo run -- diff examples/sample_game
cd examples/sample_game
CARGO_TARGET_DIR=/home/neo/code/fe3dback/bevy-quillshade/target/sample cargo clippy --workspace --all-targets -- -D warnings
CARGO_TARGET_DIR=/home/neo/code/fe3dback/bevy-quillshade/target/sample cargo test --workspace
```
Plus, where relevant: editor screenshots before/after with `magick compare` (UI changes), sample-game screenshots
(`SAMPLE_SCREENSHOT`) for runtime changes, migration golden/idempotency tests, curve/gradient parity tests, and
generated-code snapshot tests. (`data_domain` joins the feature-gating check once it exists; `project_migrate` is
Bevy-free and covered by the workspace commands.)
