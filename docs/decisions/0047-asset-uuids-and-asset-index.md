# 0047: Asset UUIDs, `AssetIndex`, reference resolution, migration step v3

Design reference: `docs/data-domain-techdoc.md` §4.1–4.3 and §10 Phase 1 (decisions D4, D18, D19, D25, D26, D27,
D34), `AGENTS.md`. Extends decisions 0043 (domain backends: `DomainBackend` gains `index`, `plan` receives the index),
0044 (editor domain API: new `DomainState` hooks, `DocCtx.index`) and 0046 (Migration Tool: the first real step, v3,
with Material and Light migrators).

Supersedes in part (records 0001–0042 live in git history, see the README):

- the path-valued reference params of the old records 0015 (curves), 0017 (constant tables), 0024 (textures) and the
  gradient design §6 (gradient collections): the same params now hold the target's UUID;
- reference rewriting on rename / move (old design §14.4, `docs/techdoc.md` §9 browser "rename"): renaming or moving
  an asset no longer rewrites any reference, on disk or in memory.

## Context

Every editor source references other editor sources by path: graph params (`"curve": Str("common/pulse")`), function
node kinds (`function::common/tex_layer`), user node kinds (`user::soft_clamp`), the Light `cubemap: Some("sky")`.
Path identity has known failures (techdoc §4.1): a rename outside the editor (`git mv`, a file manager, an IDE) breaks
every reference and is indistinguishable from delete + create; a merge where one branch renames and the other adds a
reference to the old path breaks silently; an in-editor rename must rewrite every referrer in every domain; and the
Data domain (Phase 3) needs stable identities for rows and assets anyway, so paths would leave the project with two
identity mechanisms. UUID identity fixes all of these at the cost of duplicate UUIDs when a file is copied, which a
central `AssetIndex` detects.

Facts this record builds on:

- Editor-owned kinds: Material `.material.ron`, `.function.ron`, `.curve.ron`, `.consts.ron`, `.gradients.ron`,
  `.texture.ron`, user nodes `quillshade/material/.nodes/*.ron`; Light `.environment.ron`, `.curve.ron`,
  `.texture.ron`. `quillshade/project.ron` is not an asset. Game files under `assets/` stay path-addressed (D26),
  including the image a `.texture.ron` points at.
- All references in Phase 1 are intra-domain; cross-domain references arrive with Data (Phase 3).
- Paths are woven into code generation (`curve_fn_alias`, `gradient_ident`, `fixed_texture_ident`, function module
  paths, `BakedMaterials` fields / `IDS` / `by_path`, manifest keys, Light `Preset` / `Curve` enums). Techdoc §4.1
  keeps "generated Rust names derive from paths".
- Light codegen copies the source `Environment.format_version` into the generated `EnvironmentValues`.

## Decision

### 1. UUID type and generation (D4)

- Workspace dependency `uuid` 1.29 (`default-features = false`, features `std`, `v7`, `serde`), used by `project_core`
  only; backend and editor crates use it through `project_core`.
- `project_core::asset_uuid::AssetUuid(uuid::Uuid)`: `Default` / `NIL`, `new_v7()` (`Uuid::now_v7`),
  `parse(&str) -> Option<Self>` (hyphenated form; any case is accepted, lowercase is written), `is_v7()`, `as_u128()`,
  `Display`, `Ord`, serde as the hyphenated lowercase string. `is_uuid_shaped(&str)` recognizes the 8-4-4-4-12 hex
  shape; such a string can never be a valid asset path, because `check_path` forbids `-`.
- **`AssetUuid::legacy(source_rel)`**: the deterministic UUID that the v3 migration gives an asset without one. It is a
  well-formed v7 built from a fixed timestamp constant and 80 bits of FNV-1a-64 (twice) over
  `"quillshade:v3\0" + source_rel`. Reasons: an interrupted v3 run in which a referrer was already rewritten but its
  target was not must give the target the same UUID on rerun (a random v7 in `collect` would leave dangling
  references); the dry run predicts the real run exactly; fixtures and snapshots use the same function, so
  `make_sample` reproduces the migrated sample byte for byte. Assets created in the editor get a random `new_v7()`.

### 2. Source field

- A top-level `id: "<uuid>"`, serialized right after `format_version`, in every editor-owned kind: graph `Asset`,
  `Curve`, `GlobalConsts`, `GradientCollection`, `TextureAsset`, `Environment`, user nodes (§16).
- Read tolerantly (`serde(default)`, skipped when nil). A current-format file whose `id` is missing (nil) or a
  well-formed UUID of another version is **AE004**. An `id` string that is not UUID-shaped at all is a **load error**,
  not AE004: the file does not parse (`E009` Material, `LE009` Light, "`<text>` is not a UUID (8-4-4-4-12 hex
  digits)"), so it is not in the index and its referrers show AE002.
- Per-kind `format_version`s go **1 → 2** in the v3 step; they are the per-file idempotency markers (0046). Each
  domain's table: project v1, v2 → format 1; v3 → format 2.

### 3. Reference encoding (per kind)

The slots stay where they are; their values become UUID strings, each line with a D27 path comment:

- graph node kind of a function: `kind: "function::<uuid>", // common/rock_layer`;
- graph node kind of a user node: `kind: "user::<uuid>", // .nodes/soft_clamp`;
- params of `ParamKind::{Curve, GlobalConsts, Texture, GradientCollection}`: `Str("<uuid>")`; `@internal/*` and `""`
  are unchanged;
- sub-items (`GlobalConst`, `GradientSlot`) keep their separate `Int` local-id params, which together with the asset
  UUID form the `(asset uuid, local id)` pair of §4.1 with no format change for the local id;
- Light: `cubemap: Some("<uuid>"), // sky`.

### 4. In-memory representation: UUIDs in sources and documents, paths only at one resolve seam

- Documents hold exactly what the source holds (UUIDs). `shader_project::resolve` provides
  `resolve_asset(&Asset, &AssetIndex, &Registry) -> Asset` (UUID → current path in function kinds and reference
  params; anything that is not a known UUID — a path, `@internal/*`, a dangling UUID — passes through unchanged) and
  `resolve_set(&AssetSet, &AssetIndex, &Registry) -> AssetSet` (the registry recognizes function and user-node
  kinds). Light has the equivalent for `cubemap` (`LightAssets::resolve`).
- Everything that **reads** references uses the resolved view: `plan_set` / `regenerate_set` (before `DepGraph`, IR
  and codegen, so generated code is byte-identical to the path form), Light `plan_assets`, and the editors (titles,
  pin signatures, previews, "open referenced asset").
- Everything that **writes** references (pickers, drag-and-drop, palette, function insertion, Light cube-map picker)
  stores the target's UUID taken from the `AssetIndex`.
- Consequence: rename / move, inside or outside the editor, needs no rewrite anywhere — not on disk, not in open
  documents, not in undo history; only the D27 comment changes on the next save. Dangling UUIDs surface through the
  domains' existing "does not exist" checks plus AE002.
- Rejected: paths in memory, UUIDs only on disk (translate at load / save). Cheaper in the editor, but every save
  needs a correct index to map path → UUID, open documents and their undo history keep stale paths after an external
  rename (requiring index-delta "follow" machinery for dirty documents), and an in-editor rename would still edit
  documents.
- Transition property: because the resolver passes paths through, every stage works on both path (v2) and UUID (v3)
  sources, which keeps the tree green until the format flip.

### 5. Trailing path comments (D27)

- `project_core::asset_index::annotate_refs(text, names: &dyn RefNames, from: DomainId) -> String` post-processes
  serializer output (which has no comments): every line containing a UUID-shaped token, except the root `id` line,
  gets ` // <label>`. Label = the domain-relative path without suffix (`textures/rock_albedo`), prefixed `<domain>:`
  when the target is in another domain, `(missing)` when the UUID is unknown. `RefNames` is implemented by
  `AssetIndex` and by a UUID → (domain, path) map (used by migration).
- Writers of kinds that store references (graph save, environment save) annotate; the others do not. Hand-written
  comments are not preserved (D27).
- `rewrite_root_id(text, new) -> Option<String>` replaces the root `id` line (quick fixes).

### 6. Minting

- Model constructors and `Default` leave the id nil (equality-based tests stay stable). Every writer calls
  `ensure_id()` on the model in place before writing (it mints when the id is not a valid v7: nil or another version) (`TabDoc::save` takes `&mut self`), so the first save of a new
  asset (every "New …" template, texture-from-game-file, HDR converter output, quick start) assigns `new_v7()` and
  later saves keep it. A document opened from a file without an id (AE004) gets one on its first save.
- Rename keeps the id (Material's rename re-saves the same model; core's default is `fs::rename`). There is no
  "duplicate asset" command; any future one must reset the id.

### 7. `AssetIndex` (D19)

`project_core::asset_index`, Bevy-free, built only from `BTreeMap` / `BTreeSet` (D7), construction free of I/O:

- Domains contribute a `DomainIndex { domain, assets: Vec<IndexedAsset> }`; an `IndexedAsset` is the file's
  `AssetRef`, its id (or the reason it has none), its sub-items (`SubItem { id: u64, label }` for consts and gradient
  slots) and its references (`IndexedRef { to, sub, location }`, `location` = node id or field name). A file still in
  a pre-v3 format is not listed.
- The index holds `by_id` (UUID → `AssetEntry { id, asset: AssetRef, sub_items }`, the winner of a duplicate),
  `by_ref` (every indexed file, losers included), forward edges `refs_from`, reverse edges `refs_to`, `duplicates`
  (UUID → files sorted by `source_rel`, `[0]` is the winner), files with a missing id, and a `generation` counter.
- API: `build(parts)`, `replace_domain(part)`, `get(id)`, `id_of(&AssetRef)`, `refs_from`, `refs_to`,
  `affected(changed)` (the changed files plus their transitive referrers), `dangling()`, `duplicates()`,
  `assets_of(domain, suffix)` (pickers), `diagnostics(&Project)`, `generation()`.
- `AssetEntry` embeds the existing `project_core::AssetRef` (domain + path + suffix covers §4.2's `domain`, `kind`
  and `path`; kind id = `<domain>.<suffix>`, user nodes `material.node`).
- **Deviations from the techdoc §4.2 sketch:** forward edges are keyed by **file** (`AssetRef`), not by UUID — two
  duplicate files share a UUID but are different referrers. Domains build their part with their normal loaders
  (whole files, not "headers only"): graph references are only recognizable through the node registry; parsing is
  cheap at indie scale.

### 8. Core diagnostic codes

- `AE001` duplicate asset UUID — error at every non-winner; winner = lexicographically smaller `source_rel`;
  references resolve to the winner; never blocks generation of either file.
- `AE002` dangling reference — error at the referrer; `location` = node id / field.
- `AE003` reference into a disabled domain (D30) — implemented and unit-tested now, first real use in Phase 3.
- `AE004` current-format asset without a valid UUIDv7 `id` — error, "save it in the editor to assign one" (user nodes:
  the Assign UUID quick fix, §16).
- `AW###` is reserved for index warnings.
- Diagnostic `domain` = the asset's domain (console filtering), `asset` = its `source_rel`. Domain checks keep their
  own "does not exist" errors (they carry canvas locations), so a dangling reference shows AE002 plus the domain's
  error — accepted.

### 9. `DomainBackend` and headless regeneration

- `DomainBackend` gains `fn index(&self, project: &Project) -> DomainIndex` (required; every backend, test backends
  included). `plan` becomes `plan(&self, project, index: &AssetIndex, compile)`. New `plan_affected(project, index,
  affected: &BTreeSet<AssetRef>, compile)`, defaulting to the full `plan` (Material: `Scope::Affected`).
- `regen::run` builds the index over **all** compiled-in backends — enabled or not, so AE003 is detectable — passes
  it to `plan`, and reports `index.diagnostics()` for the selected enabled domains right after W105.
- New `regen::run_affected(project, backends, changed, compile, dry_run)` (techdoc §4.3): `index.affected(changed)`
  grouped by domain → `plan_affected` per domain (no orphan scan) → one `commit`.
- `shader_project::RegenOptions` gains `index: Option<&AssetIndex>`; `None` makes Material build its own part
  (headless convenience for tests and `regenerate`).

### 10. Editor shell index

- `Editor.asset_index`, rebuilt from `DomainList::backends()` at the start of `Editor::rescan` (so domain rescans
  resolve against it), and refreshed per domain (`replace_domain`) after every successful save, create, rename,
  delete and quick fix; watcher events already rescan.
- `DocCtx` gains `index: &AssetIndex` (writers annotate with it).
- Core merges `index.diagnostics()` into the console, the active diagnostics and the mainbar error count.
- Cross-domain follow-up: after a save, every other domain with assets in `index.affected(saved)` gets the new hook
  `DomainState::regenerate_affected(editor, &[AssetRef])` (default: `generate`).

### 11. Duplicate-UUID quick fix

- `Action::RegenerateUuid(AssetRef)`: like Rename, a dirty tab of the asset is saved first; the file gets a fresh
  `new_v7()` through `DomainState::set_asset_uuid` (§16), the project is rescanned (clean tabs reload through the
  usual disk-change path), and the log says `Regenerated the UUID of <file>`.
- Entry points: `FocusDiagnostic` on an `AE001` opens a Confirm dialog — "`<loser>` has the same UUID as `<winner>`
  (a copied file?). References keep pointing at `<winner>`." — with **Cancel** / **Open** / **Regenerate UUID**; the
  Project tree row menu offers "Regenerate UUID" (existing chrome icon) on a duplicate loser.
- Screenshot automation: `SHADER_EDITOR_DEMO=duplicate_uuid` (frame 150 focuses the first AE001) and
  `duplicate_uuid_fix` (the same, then presses the dialog's Regenerate UUID button on frame 170).

### 12. `BakedMaterials::by_id`

Generated `pub by_id: BTreeMap<Uuid, UntypedHandle>` (`bevy::asset::uuid::Uuid`, no new game dependency), filled in
`reserve` from `Uuid::from_u128` literals, plus `get_by_id::<M>(Uuid) -> Option<Handle<M>>`. Materials without a valid
id are left out; `by_path` stays. `BakedTextures::by_id` is deferred to Phase 3 (§4.2 names only materials; Data's
`Texture` field type will need it).

### 13. Rename, move, delete

- Rename and move only move the file; no reference is rewritten on disk or in memory. `TabDoc::rename` still
  retargets the renamed asset's own tab and its generated-file tabs.
- Delete keeps today's behavior, matched by UUID: Material clears texture, gradient collection and constant table
  references; Light clears `cubemap`. Curves and functions stay dangling (existing errors plus AE002). The techdoc does
  not specify delete; this keeps user-visible behavior unchanged.

### 14. Migration step v3

- Title `"Assign asset UUIDs and reference assets by UUID"`. Migrators: `shader_project::migrate::MaterialMigrator`
  and `light_domain::backend::migrate::LightMigrator`, registered through `EditorDomain::migrator()` (0046).
- Per domain: `files` = the domain's scan (Material lists `.nodes/*.ron` explicitly). In `examples/sample_game` the
  step rewrites 33 files: 28 Material (13 materials, 5 functions, 2 curves, 1 constant table, 1 gradient collection,
  5 Texture assets, 1 user node), 4 Light and `quillshade/project.ron`. `collect` publishes
  `"<suffix>:<path>" → uuid` (user nodes: `"user:<ident>" → uuid`), the existing id if present, else
  `AssetUuid::legacy(source_rel)`; `is_fresh(v3)` = per-kind `format_version >= 2`; `migrate(v3)` parses with the
  current DTO (format 1 text, id defaulting to nil) — or a frozen DTO where the current one has diverged (user nodes,
  techdoc §4.4) — sets `id` and `format_version: 2`, rewrites every path reference found in the facts to its UUID
  (unknown paths stay: they were already dangling) and writes through the canonical writer plus `annotate_refs`
  (labels from the facts).
- Generated files are regenerated by the orchestrator's caller (0046); the output must be identical.

### 15. Light codegen decoupling

Generated `EnvironmentValues.format_version` gets the runtime struct's own constant instead of the source file's
`format_version`, so the per-kind bump does not change generated output.

### 16. User nodes (owner decision: they follow §4.1)

- **Schema v2**: `UserNode(format_version: 2, id: "<uuid>", ident: "soft_clamp", name: "SoftClamp", …)`. The old
  code-identifier field `id` is renamed **`ident`** (`name` is already the display name). `ident` stays the WESL
  function name (`sg_user_<ident>`) and must be unique among user nodes: a duplicate is a new E009 "duplicate user
  node ident" at the later file in path order (today a duplicate silently overwrites the registry entry). v1 = no
  `format_version` key (for user nodes only, a missing key probes as 1).
- **Writer**: `UserNode` gains `Serialize` and a canonical `to_ron()` (struct-named, 2-space); `template` is a string
  and survives, hand-written comments do not (D27). Used by the migrator and the UUID quick fixes.
- **References**: graphs store `user::<uuid>` with a D27 comment. The resolve seam maps `user::<uuid>` →
  `user::<ident>`; the registry keeps registering user nodes as `user::<ident>`, so IR and WESL are unchanged (the
  `ident` plays the role a path plays for the other kinds).
- **Location**: user nodes are `quillshade/material/.nodes/*.ron` only — the folder is scanned **non-recursively**
  (registry, index part and migrator alike); a file in a subfolder of `.nodes` is not a user node. Renaming a file
  inside `.nodes` keeps every reference valid.
- **Index identity**: user-node files have no kind suffix, so `AssetRef` allows an empty suffix (`source_rel` =
  `quillshade/material/.nodes/<name>.ron`, path `.nodes/<name>`, kind id `material.node`, D27 label
  `.nodes/<name>`). Scans that skip hidden folders keep doing so; the Material index part and migrator list
  `.nodes/*.ron` explicitly.
- **Hand-written new nodes (D4)**: authors write `format_version: 2`, `ident`, … and no `id`. The file loads and
  registers, but graphs can reference it only once it has a UUID; the index reports AE004 and the palette lists it
  disabled ("needs a UUID (AE004)"). Quick fix **Assign UUID** (`Action::AssignUuid(AssetRef)`, from
  `FocusDiagnostic` on AE004 with a Confirm dialog "Assign a UUID to `<file>`? The file is rewritten in the canonical
  format (comments are not kept).", and from the tree row menu where a row exists) writes a `new_v7()` through the
  domain's writer. A `.nodes` file without `format_version` in a v3 project is a load error (E009: "written for an
  older editor; run `shader-editor migrate` or add `format_version: 2`").
- **Quick-fix plumbing (all kinds)**: Regenerate UUID (AE001) and Assign UUID (AE004) both go through a new
  `DomainState::set_asset_uuid(editor, r, AssetUuid) -> Result<(), String>`. The default rewrites the root `id` line
  textually (`rewrite_root_id`, failing cleanly when there is none); Material overrides it with load → set →
  canonical save for every kind (needed for hand-formatted `.nodes` files).

## Consequences

- Sources are less readable: references are UUIDs. D27 comments mitigate this and are regenerated on every save.
- Rename and move are free, inside and outside the editor; a merge of a rename with a new reference stays correct.
  Generated Rust names still follow paths, so renaming changes the game-facing API (a compile error in the game, as
  intended) but never breaks references.
- Generated code does not change through the flip: the resolve seam feeds codegen the same paths as before.
- Copying a file outside the editor creates a duplicate UUID: AE001 with a Regenerate UUID quick fix; references keep
  pointing at the winner.
- User nodes become editor-owned assets: they get UUIDs, their code identifier is renamed `ident`, and a
  hand-written node needs Assign UUID (AE004) before graphs can reference it.
- Project version 3 (`ProjectVersion::CURRENT = 3`); v2 projects open into the Migration Tool. Per-kind formats 2
  are unreadable by older editors.
- Known limits: the index parses whole files; a dangling reference shows two diagnostics (AE002 + the domain's);
  `BakedTextures::by_id` is deferred to Phase 3; delete semantics (clearing) are kept, not redesigned. The
  cross-domain follow-up regeneration does not run after a rename, delete or UUID quick fix (and after a delete
  `affected()` no longer finds the referrers); Phase 1 has no cross-domain references, so this is deferred to Phase 3
  (capture `refs_to(old id)` in `before_delete`).

## History

The transition keeps every intermediate commit green, in this order:

1. `project_core` foundations (`AssetUuid`, `AssetIndex`, the D27 annotator, the backend API) with no format change.
2. A pass-through resolver in each domain (Material, user nodes, Light) and ids in the models, so path and UUID
   sources both work.
3. Per-domain v3 migrators, tested in isolation (golden files, interruption, end-to-end on a sample copy with no
   generated changes).
4. Editor index, core diagnostics and quick fixes; cross-domain affected regeneration; editor read side (resolved
   view), then write side (UUIDs stored by pickers, drops and the palette).
5. The flip: `CURRENT = 3`, per-kind formats 2, migrators registered, and `examples/sample_game` migrated by the
   Migration Tool, not by hand.
6. Rename / move without reference rewriting, delete by UUID; then `BakedMaterials::by_id`.
