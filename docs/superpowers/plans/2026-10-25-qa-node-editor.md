# QA node editor follow-ups Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Node context menus with hotkeys, Add/Sub/Mul/Div conversion, editor-only constant labels, always-prefixed
identifiers in generated WESL/Rust (reserved words allowed in names), and an exclusive per-node preview with
`unlit`/`lit`/`full` modes.

**Architecture:** Identifier prefixes are applied at emission time from helpers in `shader_graph_core::ident` and
`shader_codegen_wesl::paths`; file contents (pin ids, names) stay unprefixed. Node actions are plain data on
`NodeDef` (core stays Bevy-free); the editor turns them into menu rows and hotkeys. Preview is stored on `Asset`,
resolved in `build_ir` (previewed node's main output → `ShaderOutputDto` → replaces the material output values), and
rendered by small mode switches in the WESL generator; the editor regenerates one asset from memory when the
preview changes.

**Tech Stack:** Rust ≥ 1.97.1, Bevy 0.20 UI + Feathers (`bsn!`), bevy_noodle 0.5, wesl 0.6 + naga 30, ron, insta.

**Spec:** `docs/superpowers/specs/2026-10-25-qa-node-editor-design.md` (read it before every task; section numbers
below refer to it).

## Global Constraints

- Follow `CLAUDE.md`: English-only docs/comments, `spec §N` / decision references in comments, README kept current,
  new decisions in `docs/decisions/NNNN-*.md` + index (next free numbers: 0020, 0021, 0022).
- Never hand-edit autogen output (`examples/sample_game/assets/shaders/autogen/**`, `examples/sample_game/crates/shaders/**`,
  `crates/shader_project/tests/snapshots/*.snap`) — regenerate (`INSTA_UPDATE=always cargo test -p shader_project --test snapshots`,
  `cargo run -- regen examples/sample_game`).
- Codegen stays deterministic (byte-identical output for the same graph).
- Never run `nix` commands; plain `cargo`. `cargo fmt --all` (max_width 120) and
  `cargo clippy --workspace --all-targets -- -D warnings` must pass at the end of every task.
- Prefixes (decided): material parameter `inp_`, function name `fun_`, function input `inp_`, function output
  `out_`; path components: directory `dir_`, material file `mat_`, function file `fun_`, curve file `crv_`
  (builder's call per spec §4, fixed here). Rust type names (`RockWetMaterial`, `RockWetExt`, `RockWetParams`,
  `…Out`) stay unprefixed.
- Custom node inputs and user node (`shaders/.nodes`) inputs keep the reserved-word check and stay unprefixed.
- Toggling preview must not change the generated Rust unless the previewed node needs a parameter/builtin the
  material output does not use (then W108).
- Graph file format: new optional fields only (`Node::label`, `Asset::preview`), `#[serde(default)]` +
  `skip_serializing_if`; `FORMAT_VERSION` stays 1.

## Review Focus

1. Preview toggled on a graph that has an error elsewhere (e.g. an unconnected required input) → the in-memory
   regen reports it in the log, the previous WESL stays on disk, nothing panics. Test in Task 8.
2. Deleting the previewed node, or undoing the toggle → `Asset::preview.node` becomes `None` and the WESL is
   regenerated back to the normal output. Tests in Tasks 6 and 8.
3. Converting nodes keeps wires and stored inline inputs, is one undo step, and with a mixed selection
   (Add + Lerp) converts only the nodes offering the action. Test in Task 4.
4. Names that are Rust/WGSL keywords everywhere at once (material `kw/loop/type`, parameter `self`,
   function `kw/fn` with input `in` and outputs `out`/`let`, material `kw/loop` next to directory `kw/loop/`, a
   top-level directory `curves`) generate, compile with naga in all variants and pass clippy in the sample game. Tests in Task 2.
5. Previewing a node whose upstream uses a parameter that the material output does not → WESL compiles, Rust gains
   the parameter, W108 is reported. Test in Task 7.

---

### Task 1: Identifier checks and prefix helpers

**Files:**
- Modify: `crates/shader_graph_core/src/ident.rs`
- Modify: `crates/shader_stdlib/src/nodes/utility.rs:373` (Custom inputs), `crates/shader_stdlib/src/user.rs:57-59`
- Test: `crates/shader_graph_core/src/ident.rs` (`mod tests`)

**Interfaces:**
- Produces (all in `shader_graph_core::ident`):
  - `pub fn check_ident(name: &str) -> Result<(), String>` — shape only: `[a-z][a-z0-9_]*`, no `__`, no trailing `_`.
  - `pub fn check_code_ident(name: &str) -> Result<(), String>` — `check_ident` + the existing `RESERVED` list +
    `RESERVED_PREFIXES` (today's `check_ident` behaviour). For names embedded verbatim in user WGSL.
  - `pub fn input_ident(name: &str) -> String` (`inp_{name}`), `pub fn output_ident(name: &str) -> String`
    (`out_{name}`), `pub fn function_ident(name: &str) -> String` (`fun_{name}`), `pub fn dir_ident(name: &str) -> String`
    (`dir_{name}`), `pub fn material_file_ident(name: &str) -> String` (`mat_{name}`),
    `pub fn curve_file_ident(name: &str) -> String` (`crv_{name}`).

- [ ] **Step 1: Write the failing test** — replace `idents()`:

```rust
#[test]
fn idents() {
    for ok in ["rock_wet", "a1", "fn", "type", "loop", "self", "vec3", "n_foo", "params"] {
        assert!(check_ident(ok).is_ok(), "{ok}");
    }
    for bad in ["Rock", "1a", "a-b", "a__b", "a_", ""] {
        assert!(check_ident(bad).is_err(), "{bad}");
    }
    for bad in ["fn", "vec3", "loop", "n_foo", "_x"] {
        assert!(check_code_ident(bad).is_err(), "{bad}");
    }
    assert!(check_code_ident("strength").is_ok());
    assert_eq!(input_ident("type"), "inp_type");
    assert_eq!(output_ident("out"), "out_out");
    assert_eq!(function_ident("fn"), "fun_fn");
    assert_eq!((dir_ident("loop"), material_file_ident("type"), curve_file_ident("pulse")),
        ("dir_loop".into(), "mat_type".into(), "crv_pulse".into()));
    assert_eq!(to_snake_case("Base Tint"), "base_tint");
    assert_eq!(to_snake_case("albedoMap"), "albedo_map");
    assert_eq!(to_pascal_case("rock_wet"), "RockWet");
}
```

- [ ] **Step 2: Run it, expect a compile failure** — `cargo test -p shader_graph_core ident` → `cannot find function check_code_ident`.
- [ ] **Step 3: Implement** the functions above in `ident.rs`; update the module doc (spec §4 / decision 0020).
  Switch the Custom input check (`utility.rs:373`) and user node id/input checks (`user.rs:57,59`) to
  `check_code_ident`. All other callers (`assets.rs`, `ops.rs`, `regen.rs`, `ir.rs`, editor `state.rs`) keep
  `check_ident` and thereby lose the reserved-word rule.
- [ ] **Step 4: Run** `cargo test -p shader_graph_core ident` → PASS. Run `cargo test -p shader_stdlib` → PASS.
- [ ] **Step 5: Commit** — `git commit -am "ident: shape-only names, strict check for code-embedded names"`.

---

### Task 2: Prefixed identifiers in generated WESL and Rust

**Files:**
- Modify: `crates/shader_codegen_wesl/src/paths.rs` (module paths), `crates/shader_codegen_wesl/src/lib.rs`
  (`write_bindings`, `function`), `crates/shader_graph_core/src/ir.rs` (`UniformLayout::compute` field names, drop
  the "parameter clashes with a function name" E005), `crates/shader_graph_core/src/registry.rs` (`FunctionSig` doc)
- Modify: `crates/shader_stdlib/src/nodes/constants.rs` (param nodes emit `params.inp_x`, textures `inp_x`/`inp_x_sampler`),
  `crates/shader_stdlib/src/nodes/output.rs` (FunctionInput alias), `crates/shader_stdlib/src/nodes/utility.rs`
  (`function_call_emit`: `fun_` call, `.out_` fields)
- Modify: `crates/shader_codegen_rust/src/lib.rs` (fields, `new` args, setters, docs, `crate_files` module tree and
  re-exports), `crates/shader_project/src/regen.rs` (`rust_file`), `crates/shader_project/src/assets.rs` (remove
  `module_conflicts` and the reserved `shaders/{stdlib,functions,curves}/` check in `validate_path`, and their call
  in `regen.rs`)
- Modify: `crates/shader_project/src/fixtures.rs` (new `keyword_names()` + `keyword_fn()` in `all()`)
- Test: `crates/shader_codegen_wesl/src/paths.rs` tests, `crates/shader_project/tests/pipeline.rs`, snapshots
- Docs: `docs/decisions/0020-identifier-prefixes.md` + index, README (generated API, file layout, limitations)

**Interfaces:**
- Consumes: Task 1 helpers.
- Produces (`shader_codegen_wesl::paths`):
  - `pub fn material_components(path: &str) -> Vec<String>` — `env/rock_wet` → `["dir_env", "mat_rock_wet"]`.
  - `material_module` / `function_module` / `curve_module` keep signatures; they now return prefixed components
    (`["shaders","autogen","dir_env","mat_rock_wet"]`, `[…,"functions","dir_common","fun_wet_mask"]`,
    `[…,"curves","dir_common","crv_pulse"]`). `stdlib_module` unchanged.
- Emitted names: uniform field `inp_wetness` (padding stays `_padN`), texture `inp_albedo` + `inp_albedo_sampler`,
  WESL fn `fun_wet_mask(inp_a: …, b_uv: …)`, result struct `WetMaskOut { out_mask: f32 }`; Rust field/arg
  `inp_wetness`, setter `set_inp_wetness`, file `src/materials/dir_env/mat_rock_wet.rs`, `pub mod dir_env;`,
  root `pub use dir_env::mat_rock_wet::{RockWetExt, RockWetMaterial, RockWetParams};`. Pin ids, `MaterialParam::name`,
  `FnPort::name`, `IrStage::outputs` keys stay raw (unprefixed).

- [ ] **Step 1: Write failing tests**
  - In `paths.rs` tests: `assert_eq!(material_module("loop/type"), v("shaders::autogen::dir_loop::mat_type"))`,
    `assert_eq!(function_module("common/fn"), v("shaders::autogen::functions::dir_common::fun_fn"))`,
    `assert_eq!(curve_module("pulse"), v("shaders::autogen::curves::crv_pulse"))`; adapt `relative_imports`
    expectations to prefixed paths (e.g. `super::super::stdlib::math` from `shaders::autogen::dir_env::mat_rock_wet`).
  - In `pipeline.rs`: `keyword_names_generate_and_compile` — save `fixtures::all()` plus a material `kw/loop`
    (Unlit3d, constant color) next to the fixture's `kw/loop/` directory and a material `curves/tint` (top-level
    dir `curves`);
    `regenerate` with `compile_check: BevySources::discover().as_ref()`; assert no error diagnostics; assert
    `p.root.join("assets/shaders/autogen/dir_kw/dir_loop/mat_type.wesl")` exists and contains `params.inp_self` and
    `fun_fn(`; assert `p.crate_dir().join("src/materials/dir_kw/dir_loop/mat_type.rs")` contains `pub inp_self: f32`
    and `pub fn set_inp_self(`. Replace `materials_may_not_live_in_reserved_curves_dir` with this coverage (delete it).
  - Fixtures (`fixtures.rs`): `keyword_fn()` at `kw/fn` — FunctionInput `in` (Float), FunctionOutputs `out`
    (`in * 2`, via `stdlib::Multiply`) and `let` (`in`); `keyword_names()` at `kw/loop/type` — Unlit3d,
    `param::Scalar` named `self` (default 0.5), `param::Color` named `mesh`, a call of `function::kw/fn` with
    `in ← self`, color = `mesh.rgb * out` (Multiply), opacity unused. Add both to `all()`.
- [ ] **Step 2: Run** `cargo test -p shader_codegen_wesl paths` and `cargo test -p shader_project --test pipeline keyword_names`
  → FAIL (old paths / E005 reserved names).
- [ ] **Step 3: Implement** the prefixes in the files listed above, using only the Task 1 helpers (no literal
  `"inp_"` outside `ident.rs`). `rust_file` and `crate_files` both build paths from `material_components`, so
  generated text tabs (`tab.rs`, `state.rs` `ShowFile`) follow automatically. The function-name uniqueness check
  (E006 "functions … have the same name") stays: two `fun_x` imports would still clash.
- [ ] **Step 4: Run** `cargo test -p shader_codegen_wesl` and `cargo test -p shader_project --test pipeline` → PASS
  (fix other pipeline assertions that hard-code `autogen/env/…` paths to the new ones).
- [ ] **Step 5: Accept snapshots** — `INSTA_UPDATE=always cargo test -p shader_project --test snapshots`, then
  `cargo test -p shader_project --test snapshots` → PASS without the "Bevy sources … skipped" warning. Skim
  `git diff --stat crates/shader_project/tests/snapshots` (renamed identifiers only).
- [ ] **Step 6: Regenerate the sample** — `git status` clean for `examples/`, then
  `cargo run -p shader_project --example make_sample -- examples/sample_game`, `cargo run -- regen examples/sample_game`
  (old unprefixed files are deleted as orphans), `cargo run -- check examples/sample_game --compile` → exit 0.
  `examples/sample_game/src/main.rs` uses only type names and positional constructors — verify it still builds:
  `cd examples/sample_game && CARGO_TARGET_DIR=../../target/sample cargo clippy --workspace --all-targets -- -D warnings` → PASS.
- [ ] **Step 7: Docs** — write `docs/decisions/0020-identifier-prefixes.md` (table of prefixes, why static
  prefixes, Rust API break, Custom/user-node exception, removed checks: reserved words, reserved generated prefixes
  for asset/param names, material-vs-directory clash, reserved `stdlib/functions/curves` dirs) and index it;
  README: generated API examples (`inp_` fields/args/setters), autogen layout, Limitations (remove the reserved-word
  limitation, keep the Custom/user-node one).
- [ ] **Step 8: Commit** — `git add -A && git commit -m "codegen: prefix user identifiers in WESL and Rust (decision 0020)"`.

---

### Task 3: Node context actions in the registry

**Files:**
- Modify: `crates/shader_graph_core/src/registry.rs` (types + `NodeDef::context_actions`), `crates/shader_graph_core/src/lib.rs` (re-exports if the crate re-exports registry types)
- Modify: `crates/shader_stdlib/src/def.rs` (`DefBuilder::action`, `Def::actions`), `crates/shader_stdlib/src/nodes/math.rs`
- Test: `crates/shader_stdlib/src/nodes/math.rs` (`#[cfg(test)] mod tests`, add if absent)

**Interfaces:**
- Produces (`shader_graph_core::registry`):

```rust
/// Key of a node action's hotkey; the editor maps it to physical keys (decision 0021).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HotkeyKey { Plus, Minus, Star, Slash, Letter(char) }

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Hotkey { pub shift: bool, pub key: HotkeyKey }

impl Hotkey {
    pub const fn shift(key: HotkeyKey) -> Self;
    pub const fn plain(key: HotkeyKey) -> Self;
    /// Menu text: `Shift +`, `Shift -`, `Shift *`, `Shift /`, `V`.
    pub fn label(&self) -> String;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NodeOp { ConvertTo(String) } // target node kind

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeAction { pub label: String, pub hotkey: Option<Hotkey>, pub op: NodeOp }

// in trait NodeDef:
/// Extra items of the node's context menu (decision 0021).
fn context_actions(&self) -> Vec<NodeAction> { Vec::new() }
```

- `DefBuilder::action(self, a: NodeAction) -> Self`.

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn arithmetic_nodes_convert_into_each_other() {
    let r = crate::registry();
    let a = r.get("stdlib::Add").unwrap().context_actions();
    let ops: Vec<(&str, String)> = a.iter().map(|x| match &x.op {
        NodeOp::ConvertTo(k) => (k.as_str(), x.hotkey.unwrap().label()),
    }).collect();
    assert_eq!(ops, vec![
        ("stdlib::Subtract", "Shift -".to_string()),
        ("stdlib::Multiply", "Shift *".to_string()),
        ("stdlib::Divide", "Shift /".to_string()),
    ]);
    assert_eq!(a[0].label, "Convert to Subtract");
    assert_eq!(r.get("stdlib::Divide").unwrap().context_actions().len(), 3);
    assert!(r.get("stdlib::Power").unwrap().context_actions().is_empty());
}
```

- [ ] **Step 2: Run** `cargo test -p shader_stdlib arithmetic_nodes_convert` → FAIL (no `context_actions`).
- [ ] **Step 3: Implement** the types, the default trait method, `Def` field + builder method, and in `math.rs`
  attach to each of Add/Subtract/Multiply/Divide the three others in the order Add, Subtract, Multiply, Divide
  (Add ↔ `Plus`, Subtract ↔ `Minus`, Multiply ↔ `Star`, Divide ↔ `Slash`, all `shift: true`).
- [ ] **Step 4: Run** `cargo test -p shader_stdlib` → PASS.
- [ ] **Step 5: Commit** — `git commit -am "registry: node context actions; arithmetic convert-to"`.

---

### Task 4: Editor node menu, hotkeys and Convert

**Files:**
- Modify: `crates/shader_editor_app/src/state.rs` (`Action::ConvertNodes`, `Editor::node_menu`, `Editor::hotkey_actions`, `convert_nodes`, tests)
- Modify: `crates/shader_editor_app/src/ui/input.rs` (node RMB observer, `pressed_hotkey`, `PreviewKey`)
- Docs: `docs/decisions/0021-node-context-actions.md` + index; README hotkeys/editor section

**Interfaces:**
- Consumes: Task 3 types.
- Produces:
  - `Action::ConvertNodes { nodes: Vec<NodeId>, kind: String }` — sets `kind` of each node in one `Doc::edit`
    (one undo step), keeps id/pos/params/inputs, then drops edges and stored `inputs` whose pin id is missing from
    the node's new signature (`node_signature`).
  - `Editor::node_menu(&self, node: &str) -> Vec<(String, Action)>` — rows use the existing `"label\tHotkey"`
    convention of `open_icon_menu`; `""` is a separator. Order: the node's `context_actions` (as `ConvertNodes` on
    the selection's nodes offering that op if `node` is selected, else on `[node]`), separator, (Task 8 inserts
    preview here), `Copy\tCtrl+C`, `Duplicate\tCtrl+D`, `Delete\tDel` (existing selection actions). Empty for
    unknown kinds except the generic rows.
  - `Editor::hotkey_actions(&self, key: Hotkey) -> Vec<Action>` — for the active graph's selection: group selected
    nodes by the `ConvertTo` target of their action with that hotkey → one `ConvertNodes` per target kind.
    (Task 8 adds `V`.)
  - `input.rs`: `fn pressed_hotkey(keys: &ButtonInput<KeyCode>) -> Option<Hotkey>` — `just_pressed` with Shift held:
    `Equal`/`NumpadAdd` → Plus, `Minus`/`NumpadSubtract` → Minus, `Digit8`/`NumpadMultiply` → Star,
    `Slash`/`NumpadDivide` → Slash; numpad keys also count without Shift. Called from `shortcuts` (already gated by
    `not_typing`) in the non-Ctrl branch; every returned action is pushed.
  - `input.rs`: `#[derive(Resource, Default)] pub(crate) struct PreviewKey { held: bool, placed: bool }` with
    `fn press(&mut self)`, `fn placed(&mut self)`, `fn release(&mut self) -> bool` (true = toggle: was held and
    nothing was placed). `canvas_click` calls `placed()` when it drops a hotkey node while `KeyV` is pressed. Wiring
    the release to an action happens in Task 8.
  - Node RMB: new observer `node_click(click: On<PointerClick>, …)` — only when `click.event_target()` has a
    `NodeKey`, the original target is not a `PinKey`, the button is Secondary and `RightPress::is_click`; if the node
    is not selected push `Action::Select({node})` first, then `open_menu(&mut commands, pos, editor.node_menu(node))`.

- [ ] **Step 1: Write failing tests** in `state.rs` `mod tests`:

```rust
#[test]
fn convert_keeps_wires_and_inputs_in_one_undo_step() {
    let (_dir, mut e) = project();
    e.apply(Action::OpenAsset(AssetId::new(AssetType::Material, "env/rock_wet")));
    e.apply(Action::AddNode { kind: "stdlib::Add".into(), pos: (-700.0, 0.0), link: None, params: Vec::new() });
    let add = e.doc().unwrap().asset.nodes.iter().find(|(_, n)| n.kind == "stdlib::Add").unwrap().0.clone();
    e.apply(Action::SetValue { target: PropTarget::Input { node: add.clone(), pin: "b".into() }, value: Value::Float(3.0), merge: false });
    let lerp = add_lerp(&mut e);
    e.apply(Action::Connect { from: (add.clone(), "out".into()), to: (lerp.clone(), "a".into()) });
    e.apply(Action::Select([add.clone(), lerp.clone()].into()));
    let acts = e.hotkey_actions(Hotkey::shift(HotkeyKey::Slash));
    assert_eq!(acts.len(), 1);
    for a in acts { e.apply(a); }
    let d = e.doc().unwrap();
    assert_eq!(d.asset.nodes[&add].kind, "stdlib::Divide");
    assert_eq!(d.asset.nodes[&lerp].kind, "stdlib::Lerp");
    assert_eq!(d.asset.nodes[&add].inputs.get("b"), Some(&Value::Float(3.0)));
    assert!(d.asset.edge_into(&lerp, "a").is_some());
    e.apply(Action::Undo);
    assert_eq!(e.doc().unwrap().asset.nodes[&add].kind, "stdlib::Add");
}

#[test]
fn node_menu_lists_convert_with_hotkeys_and_generic_rows() {
    let (_dir, mut e) = project();
    e.apply(Action::OpenAsset(AssetId::new(AssetType::Material, "env/rock_wet")));
    e.apply(Action::AddNode { kind: "stdlib::Multiply".into(), pos: (0.0, 0.0), link: None, params: Vec::new() });
    let m = e.doc().unwrap().asset.nodes.iter().find(|(_, n)| n.kind == "stdlib::Multiply").unwrap().0.clone();
    let labels: Vec<String> = e.node_menu(&m).into_iter().map(|r| r.0).collect();
    assert!(labels.contains(&"Convert to Add\tShift +".to_string()));
    assert!(labels.contains(&"Convert to Divide\tShift /".to_string()));
    assert!(!labels.iter().any(|l| l.starts_with("Convert to Multiply")));
    assert!(labels.contains(&"Delete\tDel".to_string()));
}
```

  In `input.rs` add `#[cfg(test)] mod tests`: `pressed_hotkey` with a `ButtonInput<KeyCode>` where `ShiftLeft` and
  `Equal` are pressed → `Some(Hotkey::shift(HotkeyKey::Plus))`; only `Equal` → `None`; only `NumpadDivide` →
  `Some(Hotkey::shift(HotkeyKey::Slash))`. `PreviewKey`: press → release ⇒ `true`; press → placed → release ⇒ `false`;
  release without press ⇒ `false`.
- [ ] **Step 2: Run** `cargo test -p shader_editor_app convert_keeps node_menu_lists pressed_hotkey preview_key` → FAIL.
- [ ] **Step 3: Implement** the interfaces above. If `Hotkey`/`HotkeyKey` are not re-exported at crate root, import
  them from `shader_graph_core::registry` in tests.
- [ ] **Step 4: Run** `cargo test -p shader_editor_app` → PASS (incl. `tests/ui_smoke.rs`).
- [ ] **Step 5: Docs** — `docs/decisions/0021-node-context-actions.md`: actions as data on `NodeDef`, menu row
  format, selection semantics (convert = all selected offering it; `V` = exactly one), key mapping incl. numpad,
  `V` toggles on release unless a hotkey node was placed meanwhile; index it. README: node RMB menu,
  `Shift + - * /`.
- [ ] **Step 6: Commit** — `git add -A && git commit -m "editor: node context menu, hotkeys, convert arithmetic nodes (decision 0021)"`.

---

### Task 5: Editor-only label of constant nodes

**Files:**
- Modify: `crates/shader_graph_core/src/model.rs` (`Node::label`)
- Modify: `crates/shader_editor_app/src/canvas.rs` (`node_title`), `crates/shader_editor_app/src/ui/props.rs`
  (`node_rows`), `crates/shader_editor_app/src/state.rs` (`PropTarget::NodeLabel`, `SetValue`, prop value getter)
- Test: `crates/shader_graph_core/src/model.rs` tests, `crates/shader_editor_app/src/state.rs` tests,
  `crates/shader_project/tests/pipeline.rs`
- Docs: append a "Constant labels" section to `docs/decisions/0021-node-context-actions.md`; README file format

**Interfaces:**
- Produces: `pub label: String` on `Node` with `#[serde(default, skip_serializing_if = "String::is_empty")]`
  (after `inputs`); `Node::new` sets it empty. `PropTarget::NodeLabel(NodeId)` — free text, merged edits
  (`edit_merge`, like `Description`). Label row ("Label", `text_w`) only for kinds `stdlib::Constant`,
  `stdlib::Constant2Vector`, `stdlib::Constant3Vector`, `stdlib::Constant4Vector`, shown above "Parameters".
  `node_title`: those kinds show the label when non-empty, else the value as today.

- [ ] **Step 1: Write failing tests**
  - model: a `Node` with `label: "rim width".into()` survives `Asset::to_ron` → `Asset::from_ron`; a node with an
    empty label serializes without a `label:` key.
  - pipeline: `label_does_not_change_codegen` — regenerate `fixtures::all()`, read `rock_wet`'s WESL; set `label`
    on every `stdlib::Constant*` node of the saved asset, save, regenerate; WESL and Rust byte-identical.
    (If `rock_wet` has no constant, use a fixture that does, or add one constant to the test graph.)
  - state: `constant_label_replaces_title` — add `stdlib::Constant`, `SetValue { target: PropTarget::NodeLabel(id),
    value: Value::Str("rim".into()), merge: true }` → `canvas::node_title(&e, &doc.asset, &id) == "rim"`; empty
    string → title is the value again.
- [ ] **Step 2: Run** the three tests → FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** `cargo test --workspace` → PASS.
- [ ] **Step 5: Commit** — `git add -A && git commit -m "editor: editor-only labels for constant nodes"`.

---

### Task 6: Preview model and IR

**Files:**
- Modify: `crates/shader_graph_core/src/model.rs` (`PreviewMode`, `Preview`, `Asset::preview`, `normalize`)
- Create: `crates/shader_graph_core/src/preview.rs` (`PreviewSlot`, `ShaderOutputDto`), register in `lib.rs`
- Modify: `crates/shader_graph_core/src/registry.rs` (`NodeDef::preview_output`), `crates/shader_graph_core/src/diag.rs` (`W107`, `W108` docs; both warnings), `crates/shader_graph_core/src/ir.rs`
- Test: `crates/shader_graph_core/src/ir.rs` tests (or `crates/shader_graph_core/tests/` if IR tests live there — follow existing placement), `crates/shader_graph_core/src/preview.rs` tests
- Docs: `docs/decisions/0022-node-preview.md` (model + IR part) + index

**Interfaces:**
- Produces (`shader_graph_core::model`):

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum PreviewMode { #[default] Unlit, Lit, Full }

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Preview { pub node: Option<NodeId>, pub mode: PreviewMode }
impl Preview { pub fn is_default(&self) -> bool; }

// Asset, after `meta`:
#[serde(default, skip_serializing_if = "Preview::is_default")]
pub preview: Preview,
```

  `Asset::normalize` clears `preview.node` when that node no longer exists.
- Produces (`shader_graph_core::preview`):

```rust
/// Material output a previewed node can feed (spec §5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum PreviewSlot { Albedo, Metallic, Specular, Roughness, Emissive, Opacity, Normal, AmbientOcclusion }
impl PreviewSlot {
    /// Input pin of the material output node for this slot (`base_color` / `color` for Albedo), if the kind has it.
    pub fn output_pin(self, kind: &MaterialKind) -> Option<&'static str>;
}

/// What a previewed node feeds: slot → the node's own output pin id.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ShaderOutputDto { pub slots: BTreeMap<PreviewSlot, String> }
impl ShaderOutputDto {
    /// Default: the first output → Albedo, if its type converts to Float3; otherwise empty (not previewable).
    pub fn main_output(sig: &Signature) -> Self;
    pub fn is_empty(&self) -> bool;
}

// in trait NodeDef:
/// Outputs fed into the material when this node is previewed (decision 0022).
fn preview_output(&self, sig: &Signature) -> ShaderOutputDto { ShaderOutputDto::main_output(sig) }
```

- Produces (`shader_graph_core::ir`): `pub struct IrPreview { pub node: NodeId, pub mode: PreviewMode }`,
  `Ir::preview: Option<IrPreview>`.
- `build_ir` behaviour (materials only; function assets ignore `preview` silently):
  1. If `asset.preview.node` is `Some(p)`: `p` unknown, the MaterialOutput node, or `preview_output(sig0)` empty →
     `W107` "preview ignored: …" at `p` (if it exists), no preview. Otherwise `W107` "preview active (mode)" at `p`.
  2. After the live sets (step 5), add `p` and its upstream to the Fragment live set (the normal live sets stay,
     so parameters, textures and the vertex stage are still collected — Rust stays the same).
  3. Keep the Fragment stage's per-pin types after emission; build the new Fragment `outputs`: Albedo's operand
     converted with `conversion(from, Float3)`, written to `base_color` (Pbr3d) / `color` (Unlit3d) /
     `color` as `vec4<f32>({albedo}, 1.0)` (Sprite2d). `Full` also maps every other slot through
     `PreviewSlot::output_pin` with `conversion(from, that pin's type)`; `Lit`/`Unlit` use Albedo only.
  4. If a vertex stage exists, set its `WPO_PIN` output to `vec3<f32>(0.0)` (keep the key so the vertex shader the
     Rust side declares still exists).
  5. `uses_normal` is computed from the new outputs; `Ir::preview = Some(IrPreview { node: p, mode })`.

- [ ] **Step 1: Write failing tests**
  - `preview.rs`: `main_output` on a signature with outputs `[rgb: Float3, r: Float]` → `{Albedo: "rgb"}`;
    first output `Bool` → empty; `Texture2D` → empty.
  - model: `Preview` round-trips through RON; a default `Preview` is not serialized; `normalize` after removing the
    previewed node → `preview.node == None`, `mode` kept.
  - ir: `preview_replaces_outputs` — Pbr3d graph: `stdlib::Constant` (0.25) → `stdlib::Multiply.a`, Multiply.out →
    nothing; `param::Scalar` `rough` → output `roughness`; MaterialOutput `base_color` ← another constant;
    `preview = Preview { node: Some(mul), mode: Unlit }`. Assert: `ir.main.outputs.keys()` == `["base_color"]`,
    `ir.main.outputs["base_color"]` is `vec3<f32>(<mul expr>)`-shaped (contains the multiply's var name),
    `ir.params` still contains `rough`, a `W107` diagnostic at `mul`, `ir.preview.unwrap().mode == PreviewMode::Unlit`.
  - ir: `preview_of_bool_or_missing_node_is_ignored` — preview on a `stdlib::BoolSelect`'s bool input source
    (e.g. `param::Bool`) and on id `"zzz"` → outputs unchanged, `ir.preview == None`, W107 present.
  - ir: `preview_keeps_vertex_stage` — Unlit3d graph with WPO connected; preview on a constant → `ir.vertex` is
    `Some` with `outputs[WPO_PIN] == "vec3<f32>(0.0)"`.
- [ ] **Step 2: Run** `cargo test -p shader_graph_core preview` → FAIL.
- [ ] **Step 3: Implement** as described. Diagnostic texts: `W107` "preview active on this node (mode unlit|lit|full);
  the material output is bypassed", "preview ignored: node `{id}` does not exist", "preview ignored: `{name}` has no
  output that converts to a color". `W108` doc only (emitted in Task 7).
- [ ] **Step 4: Run** `cargo test -p shader_graph_core` → PASS; `cargo test --workspace` → PASS (no codegen change
  without preview: snapshots unchanged).
- [ ] **Step 5: Docs** — `docs/decisions/0022-node-preview.md`: storage on `Asset`, modes, `ShaderOutputDto` and
  `NodeDef::preview_output` (future PBR DTO pin maps 1:1), live-set union rule and why (Rust stability), W107.
  README: file format (`preview`), diagnostics table (W107, W108).
- [ ] **Step 6: Commit** — `git add -A && git commit -m "core: node preview model and IR (decision 0022)"`.

---

### Task 7: Preview in WESL codegen, W108, snapshots

**Files:**
- Modify: `crates/shader_codegen_wesl/src/lib.rs` (`material`, `pbr_fragment`, `unlit_fragment`, `sprite_fragment`)
- Modify: `crates/shader_project/src/regen.rs` (W108)
- Test: `crates/shader_project/tests/snapshots.rs` (`preview_modes_generate_and_compile`), `crates/shader_project/tests/pipeline.rs` (`preview_needing_extra_param_warns_w108`)
- Docs: extend `docs/decisions/0022-node-preview.md` (codegen per kind)

**Interfaces:**
- Consumes: `Ir::preview` (Task 6).
- WESL output when `ir.preview` is `Some(p)`:
  - Third header line `// preview of node {p.node} ({mode}): the material output is bypassed`.
  - Pbr3d: base color line writes alpha `1.0` instead of `pbr_input.material.base_color.a`. `Unlit` mode
    additionally: import `bevy_pbr::render::pbr_types::STANDARD_MATERIAL_FLAGS_UNLIT_BIT`, set
    `pbr_input.material.flags = pbr_input.material.flags | STANDARD_MATERIAL_FLAGS_UNLIT_BIT;` before
    `alpha_discard`, and in the forward branch write `out.color = pbr_input.material.base_color;` instead of
    `apply_pbr_lighting` (as Bevy's `pbr.wesl` does for unlit; deferred keeps `deferred_output`, which honours the flag).
    `Lit`/`Full`: lighting path unchanged.
  - Unlit3d (all modes): after the graph lines `return vec4<f32>({color}, 1.0);` — no opacity, mask discard or
    blend-mode variants.
  - Sprite2d (all modes): `return {color};` (the IR already built `vec4<f32>(albedo, 1.0)`), no `get_final_color`.
- W108 (`regen.rs`, materials only, when `asset.preview.node.is_some()` and both IRs are error-free): build a second
  IR from a clone with `preview = Preview::default()`; if `shader_codegen_rust::material_module` differs between
  them → `W108` "preview changes the generated Rust crate (extra parameters or builtins); rebuild the game, and again
  after turning preview off". Rust is generated from the preview IR (as today, the asset's IR).

- [ ] **Step 1: Write failing tests**
  - snapshots: `preview_modes_generate_and_compile` — for each material kind (Pbr3d, Unlit3d, Sprite2d) × mode
    (Unlit, Lit, Full) save a material `preview/{kind}_{mode}` built with `GraphBuilder`: `stdlib::TextureCoordinate`
    `out` wired straight into `stdlib::Lerp.alpha` (Float2 → Float truncation, W104 is fine), Lerp `a`/`b` left at
    their defaults, Lerp → output color; plus a `param::Scalar` `rough` into Pbr `roughness`; for Unlit3d also a
    constant into WPO. Set `preview = Some(lerp)` with the mode. `regenerate` with compile check, assert no errors,
    `snapshot("wesl__preview__{kind}_{mode}", …)`, and for each kind assert the Rust module equals the one generated
    with preview cleared (regenerate twice, compare `rust_file` contents).
  - pipeline: `preview_needing_extra_param_warns_w108` — material where a `param::Scalar` `extra` feeds a Multiply
    that is not connected to the output; preview on the Multiply; regenerate: a W108 on the asset, the Rust module
    contains `inp_extra`, WESL contains `params.inp_extra`; clear preview, regenerate: no W108, Rust has no `inp_extra`.
- [ ] **Step 2: Run** `cargo test -p shader_project --test snapshots preview_modes` → FAIL (no snapshots / outputs not
  bypassed). Run the pipeline test → FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Accept and verify** — `INSTA_UPDATE=always cargo test -p shader_project --test snapshots`; read the 9
  new `.snap` files (preview header line present, lighting bypass only in Pbr unlit, `return vec4<f32>(…, 1.0)` for
  Unlit3d); `cargo test -p shader_project` → PASS without the "skipped" warning; old snapshots unchanged
  (`git status` shows only new files).
- [ ] **Step 5: Docs** — decision 0022: per-kind codegen table, alpha forced to 1, W108 rule.
- [ ] **Step 6: Commit** — `git add -A && git commit -m "codegen: preview modes in WESL, W108"`.

---

### Task 8: Preview in the editor

**Files:**
- Modify: `crates/shader_editor_app/src/state.rs` (`Action::TogglePreview`, `PropTarget::PreviewMode`, preview sync
  in `apply`, `regenerate_from_memory`, `regenerate_from_disk`, `node_menu` / `hotkey_actions` additions, `remove_tabs` hook, tests)
- Modify: `crates/shader_editor_app/src/doc.rs` (`Doc::regen_from_memory: bool`, cleared in `mark_saved`)
- Modify: `crates/shader_editor_app/src/ui/input.rs` (`V` via `PreviewKey`), `crates/shader_editor_app/src/ui/props.rs`
  (Preview section in `asset_rows`), `crates/shader_editor_app/src/canvas.rs` (highlight), `crates/shader_editor_app/src/tab.rs`
  (title suffix), `crates/shader_editor_app/src/ui/icons.rs` (`IconId::Warning` → `icon_rs::WARNING_FILLED`, add to `ALL`)
- Docs: decision 0022 (editor part), README (hotkey `V`, Properties preview select), `docs/ui-checklist.md`

**Interfaces:**
- Consumes: Tasks 4 and 6.
- Produces:
  - `Action::TogglePreview(NodeId)` — on the active graph doc (material only): sets `preview.node` to the node, or
    `None` if it already is (one `Doc::edit`, undoable). `PropTarget::PreviewMode` — `select_w` with
    `["Unlit", "Lit", "Full"]` in a "Preview" section of `asset_rows` (materials only), plus a dim text naming the
    previewed node title or "No node previewed — select a node and press V".
  - Preview sync: in `Editor::apply`, remember `(doc id, asset.preview.clone())` of the active graph doc before the
    action; if after it the same doc's `preview` differs, call `regenerate_from_memory(active)`. This covers toggle,
    mode change, undo/redo, deleting the previewed node.
  - `fn regenerate_from_memory(&mut self, i: usize) -> bool` — `regenerate_set(project, &set, &RegenOptions { scope:
    Scope::Affected([id].into()), dry_run: false, compile_check: self.bevy.as_ref() })` where `set` = `self.disk`
    with only this doc's asset replaced; sets `regen_from_memory = true`; log `Preview: regenerated {source_rel}` or
    a warning with the error count (files kept); then `validate()` and `reload_text_tabs()`.
  - `fn regenerate_from_disk(&mut self, id: &AssetId)` — same with `self.disk` unchanged; called from `remove_tabs`
    for closed graph docs with `regen_from_memory && is_dirty()`.
  - `node_menu`: inserts `"Exclusive preview\tV"` (or `"Stop preview\tV"` when it is the previewed node) →
    `TogglePreview(node)` for material docs; for a node whose `preview_output` is empty (or the MaterialOutput) the
    row is `"Exclusive preview (not available)"` → `Action::Notify("… has no output that converts to a color")`.
    Not shown in function graphs.
  - `hotkey_actions(Hotkey::plain(HotkeyKey::Letter('v')))` → `[TogglePreview(n)]` iff exactly one node is selected
    and it is previewable in a material doc.
  - `input.rs`: in `shortcuts` (non-Ctrl branch): `just_pressed(KeyV)` → `preview_key.press()`;
    `just_released(KeyV)` and `preview_key.release()` → push `editor.hotkey_actions(V)`.
  - Canvas (`node_scene`): previewed node → border `px(3)`, `SelectedBorderColor { normal: ACCENT, selected: Color::WHITE }`,
    title row with `@icon(IconId::Warning, 16.0, ACCENT)` before the title text.
  - Tab title: graph tabs with `preview.node.is_some()` get suffix ` (preview)`.

- [ ] **Step 1: Write failing tests** (`state.rs` tests)

```rust
#[test]
fn preview_toggle_regenerates_from_memory_and_discard_restores() {
    let (dir, mut e) = project();
    let id = AssetId::new(AssetType::Material, "env/rock_wet");
    e.apply(Action::RegenAll);
    let wesl = wesl_file(e.project.as_ref().unwrap(), &id).unwrap();
    let normal = std::fs::read_to_string(&wesl).unwrap();
    let lerp = add_lerp(&mut e);
    e.apply(Action::Select([lerp.clone()].into()));
    for a in e.hotkey_actions(Hotkey::plain(HotkeyKey::Letter('v'))) { e.apply(a); }
    assert_eq!(e.doc().unwrap().asset.preview.node.as_deref(), Some(lerp.as_str()));
    let previewed = std::fs::read_to_string(&wesl).unwrap();
    assert!(previewed.contains("// preview of node"));
    assert!(!std::fs::read_to_string(dir.path().join("shaders/env/rock_wet.material.ron")).unwrap().contains("preview"));
    e.apply(Action::Undo);
    assert!(e.doc().unwrap().asset.preview.node.is_none());
    assert!(!std::fs::read_to_string(&wesl).unwrap().contains("// preview of node"));
    for a in e.hotkey_actions(Hotkey::plain(HotkeyKey::Letter('v'))) { e.apply(a); }
    e.apply(Action::DiscardTab(e.active.unwrap()));
    assert_eq!(std::fs::read_to_string(&wesl).unwrap(), normal);
}

#[test]
fn preview_with_errors_elsewhere_keeps_file_and_warns() {
    // rock_wet + an added stdlib::TextureSample whose required `tex` stays unconnected, wired into the Lerp:
    // toggling preview on the Lerp logs a warning, the WESL on disk is unchanged, no panic.
}

#[test]
fn deleting_previewed_node_clears_preview() {
    // TogglePreview(lerp), Select({lerp}), DeleteSelection → preview.node is None and the WESL has no preview header.
}
```

  (Write the two sketched tests fully in the same style; `Select` + `DeleteSelection` are existing actions.)
  Use `PreviewMode` too: `SetValue { target: PropTarget::PreviewMode, value: Value::Str("Lit".into()), merge: false }`
  while previewing → `asset.preview.mode == PreviewMode::Lit` and the WESL header says `(lit)`.
- [ ] **Step 2: Run** `cargo test -p shader_editor_app preview` → FAIL.
- [ ] **Step 3: Implement.** Icons: follow CLAUDE.md "Add an icon-rs icon" (variant, `source()` mapping to
  `icon_rs::WARNING_FILLED`, append to `ALL`, bump its length).
- [ ] **Step 4: Run** `cargo test -p shader_editor_app` → PASS.
- [ ] **Step 5: Visual check** — `SHADER_EDITOR_SCREENSHOT=$SCRATCH/preview.png SHADER_EDITOR_OPEN=env/rock_wet.material.ron cargo run -- examples/sample_game`
  after temporarily setting `preview` in a scratch copy of the sample project (copy `examples/sample_game` to the
  scratchpad, edit the copy's `rock_wet.material.ron` to add `preview: (node: Some("<a node id>"), mode: Unlit)`,
  run the editor on the copy). Inspect the PNG: thick orange border + warning icon on that node, tab title with
  `(preview)`. Do not commit the copy.
- [ ] **Step 6: Docs** — decision 0022 editor part (in-memory regen, undo/close semantics, `V` on release);
  README (hotkey `V`, Properties → Preview); `docs/ui-checklist.md`: node RMB menu, Convert via menu and
  `Shift + - * /`, constant Label, preview toggle via `V` and menu, highlight, mode select, discard restores.
- [ ] **Step 7: Commit** — `git add -A && git commit -m "editor: exclusive node preview (decision 0022)"`.

---

### Task 9: Final verification

- [ ] **Step 1:** `cargo fmt --all --check` → no output.
- [ ] **Step 2:** `cargo clippy --workspace --all-targets -- -D warnings` → PASS.
- [ ] **Step 3:** `cargo test --workspace` → PASS; confirm the snapshot suite did not print the "Bevy sources … skipped" warning.
- [ ] **Step 4:** `cargo run -- regen examples/sample_game` → no changes printed; `git status examples/` clean.
  `cargo run -- check examples/sample_game --compile` → exit 0, no W107/W108 (the sample has no preview).
- [ ] **Step 5:** `cd examples/sample_game && CARGO_TARGET_DIR=../../target/sample cargo clippy --workspace --all-targets -- -D warnings` → PASS.
- [ ] **Step 6:** Grep that no literal prefix leaked outside the helpers: `grep -rn '"inp_\|"fun_\|"out_\|"dir_\|"mat_\|"crv_' crates --include=*.rs`
  → only `ident.rs` (and tests).
- [ ] **Step 7:** README review against the spec's "Docs & bookkeeping" list; commit any fix-ups.
