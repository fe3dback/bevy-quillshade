# Inline Pin Editors Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Edit unconnected input defaults and constant/parameter values directly on canvas nodes (UE5-style),
with an RMB pin menu that sets the type of polymorphic inputs.

**Architecture:** Canvas node scenes (`canvas::node_scene`) get Feathers widgets in pin rows. The widgets carry the
existing `ui::props::PropField`, so the existing `ValueChange` observers in `props.rs` turn them into
`Action::SetComponent`/`SetValue` — no new edit path. The canvas is rebuilt from the model after the edit (rebuild is
already suspended while a mouse button is held, so a slider drag is not torn down). Color values open a popup picker
built like `ui::dialog::open_menu`. Polymorphic pin type is stored as the type of the inline `Value` in
`Node::inputs` — `SigCtx::poly_type` already resolves from it, so the file format does not change.

**Tech Stack:** Bevy 0.20 UI + Feathers (`FeathersSlider`, `FeathersNumberInput`, `FeathersColorSwatch`,
`FeathersColorPlane`, `FeathersColorSlider`, `FeathersCheckbox`), bevy_noodle 0.5 (unchanged).

**Spec:** agreed in chat on 2026-10-25 (no separate design doc). Summary:
- Unconnected input shows an editor in its row; the editor disappears when a wire is connected.
- Float with known range → slider; Float without range and Int → number input (as in Properties);
  Float2 → X/Y number inputs; Float3/Float4/Color → swatch + R/G/B(/A) number inputs, swatch click opens a popup
  picker (hue/saturation plane + lightness slider + alpha slider for 4-component values); Bool → checkbox.
- ScalarParameter: slider for `default` with range `min..max` params; Vector/ColorParameter: swatch + fields;
  BoolParameter: checkbox; Constant nodes: editor for `value`.
- RMB on an input pin → context menu: for polymorphic inputs "Float/Float2/Float3/Float4" type choice;
  "Reset to default"; "Disconnect" when connected.
- No changes to external crates (bevy_noodle, Feathers). What cannot be done without them is reported, not hacked.

## Global Constraints

- Do not modify bevy_noodle or Bevy/Feathers sources; workarounds only in this repo (`fixes.rs` for Feathers bugs).
- No egui; UI is Bevy UI + Feathers via `bsn!`.
- File format stays at `format_version` 1; no migration.
- Codegen output must not change (snapshots and `examples/sample_game` regen stay byte-identical).
- English-only comments/docs; `spec §N` references where applicable; rustfmt `max_width = 120`.
- `cargo clippy --workspace --all-targets -- -D warnings` and `cargo test --workspace` pass.
- Do not commit (the user commits); the working tree already holds unrelated uncommitted work — don't revert it.

## Review Focus

1. Slider dragged while the canvas is zoomed in/out: the value must follow the pointer, the node must not move.
   (Manual: screenshot/scripted drag; record the outcome — if Feathers mis-maps under `UiTransform` scale, report.)
2. Typing into an inline number field, then Delete / Ctrl+Z: goes to the field, not to the graph
   (hotkeys already check `InputFocus` in `ui/input.rs`; verify by hand).
3. One slider drag = one undo step (merge until `is_final`) — test in Task 3.
4. Node selected in Properties while edited on canvas: Properties shows the new value after release — test in Task 3.
5. Connecting a wire removes the pin's editor; disconnecting brings it back with the stored value — test in Task 2.

---

### Task 1: Pin value range in the registry

**Files:**
- Modify: `crates/shader_graph_core/src/registry.rs:54-107` (`PinDef`)
- Modify: `crates/shader_stdlib/src/nodes/math.rs` (Lerp), `crates/shader_stdlib/src/nodes/utility.rs` (Fresnel
  `base`, Desaturation `fraction`)
- Test: `crates/shader_stdlib` existing test module (or `tests/` file next to other stdlib tests)

**Interfaces:**
- Produces: `PinDef::range: Option<(f32, f32)>` (default `None`), builder `PinDef::range(self, min: f32, max: f32) -> Self`.

- [ ] **Step 1: Failing test** `lerp_alpha_has_unit_range`: build the stdlib registry, compute the Lerp signature
  with nothing connected, assert `sig.input("alpha").unwrap().range == Some((0.0, 1.0))` and
  `sig.input("a").unwrap().range == None`.
- [ ] **Step 2:** `cargo test -p shader_stdlib lerp_alpha_has_unit_range` → FAIL (no field).
- [ ] **Step 3:** Add the field + builder; set `.range(0.0, 1.0)` on Lerp `alpha`, Fresnel `base`, Desaturation
  `fraction`. Output-node pins are `implicit` (no inline value) — leave them.
- [ ] **Step 4:** Test passes; `cargo test -p shader_project --test snapshots` still passes unchanged.

### Task 2: Inline editors on canvas nodes

**Files:**
- Create: `crates/shader_editor_app/src/inline.rs` (widget choice + scenes)
- Modify: `crates/shader_editor_app/src/canvas.rs:168-260` (`pin_row`, `node_scene`), `src/lib.rs` (mod),
  `src/ui/props.rs` (`PropField` gets `pub inline: bool`; make `number`, `integer`, `vector`, `checkbox_w`
  `pub(crate)` or move shared builders to `inline.rs` — no duplication)
- Test: unit tests in `inline.rs`; `crates/shader_editor_app/tests/ui_smoke.rs`

**Interfaces:**
- Produces:
  - `pub enum Inline { Slider { min: f32, max: f32 }, Number, Integer, Vector(usize), Color(usize), Check }`
  - `pub fn inline_for_pin(ty: PinType, range: Option<(f32, f32)>) -> Option<Inline>` — `None` for Texture2D/Any.
  - `pub fn inline_for_param(kind: &str, params: &BTreeMap<String, Value>) -> Option<(String /*param key*/, Inline)>`
    — `param::Scalar` → `("default", Slider{min,max})` when `min < max` and both finite, else `Number`;
    `param::Vector` → `Vector(4)`; `param::Color` → `Color(4)`; `param::Bool` → `Check`;
    `stdlib::Constant*` → `"value"` with Number/Vector(n) by type; everything else `None`.
  - `pub fn inline_scene(target: PropTarget, inline: Inline, v: &Value) -> Box<dyn Scene>` — every interactive
    widget carries `PropField { target, comp, inline: true, .. }`; the color swatch carries `ColorPickerOpener(target)`
    (component defined here, consumed by Task 4).
- Consumes: `PinDef::range` (Task 1).

- [ ] **Step 1: Failing unit tests** in `inline.rs`:
  `inline_for_pin(Float, Some((0.,1.))) == Some(Slider{min:0.,max:1.})`; `(Float, None) → Number`;
  `(Int, None) → Integer`; `(Float2,_) → Vector(2)`; `(Float3,_) → Color(3)`; `(Float4,_) → Color(4)`;
  `(Color,_) → Color(4)`; `(Bool,_) → Check`; `(Texture2D,_) → None`.
  `inline_for_param("param::Scalar", {min:0.2,max:5.0}) == Some(("default", Slider{min:0.2,max:5.0}))`;
  with `min == max` → `Number`.
- [ ] **Step 2:** `cargo test -p shader_editor_app inline` → FAIL.
- [ ] **Step 3:** Implement. In `node_scene`: for each unconnected, non-`required`, non-`implicit` input whose
  `inline_for_pin` is `Some`, put the editor in the pin row after the label (drop the ` = value` suffix then);
  for param/constant nodes put the editor right under the title. Slider: `@FeathersSlider { @min, @max }`,
  `SliderValue`, `SliderPrecision(2)`, `on(slider_self_update)` as in noodle's `scene_builder_3d.rs`. Raise node
  `min_width` to fit a 4-field row (~220 px) only for nodes that show a vector/color editor.
- [ ] **Step 4: Smoke test** `inline_editor_follows_connection` in `ui_smoke.rs`: open `env/rock_wet`, add
  `stdlib::Lerp`, assert a `PropField` with `target == PropTarget::Input{node, pin:"alpha"}` exists; connect a
  `stdlib::Time` output to `alpha`; after two updates assert it is gone; `Action::Undo` → it is back.
  If Feathers templates need `FeathersPlugins` in the headless app, add the minimal plugin set to `app()`.
- [ ] **Step 5:** `cargo test -p shader_editor_app` passes.

### Task 3: Commit inline edits (undo step + Properties refresh)

**Files:**
- Modify: `crates/shader_editor_app/src/state.rs` (`Action`, `Editor::apply`), `src/ui/props.rs` (observers)
- Test: `state::tests`

**Interfaces:**
- Produces: `Action::CommitInline` — ends the current merge (`Doc::end_merge`) and bumps `ui.props`.
- Consumes: `PropField::inline` (Task 2).

- [ ] **Step 1: Failing test** `inline_drag_is_one_undo_step`: open `env/rock_wet`, add Lerp (id from the
  selection), apply three `SetComponent{target: Input{id,"alpha"}, comp:0, value: 0.2/0.4/0.6, is_final: false}`
  then `CommitInline`; assert `ui.props` grew, value is 0.6; one `Undo` → value back to default 0.5 (input map
  has no `alpha`).
- [ ] **Step 2:** run → FAIL (no variant).
- [ ] **Step 3:** Add the variant. In `number_f32`/`number_i32`/`checkbox`: when `f.inline && ev.is_final`
  (checkbox: always when inline) push `Action::CommitInline` after the edit.
- [ ] **Step 4:** test passes.

### Task 4: Color picker popup

**Files:**
- Create: `crates/shader_editor_app/src/ui/color_picker.rs`
- Modify: `src/ui/mod.rs` (plugin registration), `src/lib.rs` (`SHADER_EDITOR_DEMO=color` opens it on the first
  color pin of the open asset, for screenshots)

**Interfaces:**
- Consumes: `ColorPickerOpener(PropTarget)` (Task 2), `Action::SetValue`, `Action::CommitInline` (Task 3).
- Produces: `pub fn open_color_picker(commands: &mut Commands, pos: Vec2, target: PropTarget, value: &Value)`.

- [ ] **Step 1:** `PointerClick` (Primary) on a `ColorPickerOpener` → `open_color_picker` at the pointer.
  Popup = full-window barrier (like `MenuRoot`) + panel with `@FeathersColorPlane::HueSaturation`
  (`min_height: px(120)`), `@FeathersColorSlider { @channel: ColorChannel::HslLightness }`, and for 4 components
  an alpha `@FeathersSlider {min:0,max:1}`. Plane/slider changes convert HSL→RGB (sRGB floats, as `props.rs`
  swatches use `Color::srgba`) keeping the original type (`Float3`/`Float4`/`Color`) and push
  `SetValue{merge:true}`; plane/slider colors update live (`ColorPlaneValue`, `SliderBaseColor`).
- [ ] **Step 2:** Barrier press or Escape closes the popup and pushes `Action::CommitInline`.
- [ ] **Step 3:** Screenshot: `SHADER_EDITOR_SCREENSHOT=<scratch>/color.png SHADER_EDITOR_OPEN=env/rock_wet.material.ron
  SHADER_EDITOR_DEMO=color cargo run -- examples/sample_game`; inspect the PNG.

### Task 5: RMB pin menu and polymorphic type

**Files:**
- Modify: `crates/shader_editor_app/src/state.rs` (`Action`), `src/ui/input.rs` (observer)
- Test: `state::tests`

**Interfaces:**
- Produces: `Action::SetPinType { node: NodeId, ty: PinType }`, `Action::ResetInput { node: NodeId, pin: String }`.

- [ ] **Step 1: Failing tests** `set_pin_type_resolves_poly`: Lerp added, `SetPinType{ty: Float3}` → signature
  inputs `a`,`b` and output are `Float3`, `alpha` stays `Float`; `SetPinType{ty: Float}` → back to `Float`
  (requires rewriting *all* unconnected poly inputs, otherwise the max-dim rule keeps Float3).
  `reset_input_removes_override`: `SetComponent` on alpha then `ResetInput` → `inputs` has no `alpha`; one undo
  step each.
- [ ] **Step 2:** run → FAIL.
- [ ] **Step 3:** `SetPinType`: one `edit` that, for every unconnected `poly` input of the node, stores
  `current.coerce(ty)`. `ResetInput`: remove the key. Both bump `ui.props`.
- [ ] **Step 4:** In `ui/input.rs` add `pin_click` (`On<PointerClick>`, Secondary, `RightPress::is_click`) on an
  entity with `PinKey { output: false }` → `open_menu` with: `Float/Float2/Float3/Float4` (current marked "✓ ",
  only when the pin is `poly` and no poly pin of the node is connected), `Reset to default` (when not connected),
  `Disconnect` (when connected). Output pins: no menu.
- [ ] **Step 5:** tests pass; screenshot with `SHADER_EDITOR_INPUT=rmb_click` adapted to a pin if cheap,
  otherwise check by hand in the GUI.

### Task 6: Verify, docs

- [ ] `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`.
- [ ] `cargo run -- regen examples/sample_game` → no changes; `git diff --stat examples/sample_game` empty
  apart from pre-existing edits.
- [ ] Screenshots of rock_wet with Lerp/Scalar/Color nodes at default zoom and zoomed; scripted `drag` on a slider
  if the input hooks allow targeting it; write down what works and what doesn't.
- [ ] `docs/decisions/0012-inline-pin-editors.md` (+ index): editors reuse `PropField`; poly type = inline value
  type; ranges on `PinDef`; known limitations found in testing. Update `README.md` (editor section: inline
  editing, RMB pin menu) and `docs/ui-checklist.md` (new manual checks = Review Focus 1, 2).
