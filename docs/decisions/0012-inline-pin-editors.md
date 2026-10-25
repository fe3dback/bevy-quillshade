# 0012. Inline value editors on nodes

**Context.** Values (unconnected input defaults, constants, parameter defaults) could only be edited in the
Properties panel. UE5's Material Editor edits them on the node, and lets the user pick the type of a polymorphic
input from the pin's context menu. bevy_noodle nodes can hold any Bevy UI, so Feathers widgets fit inside them.

**Decision.**
- Every unconnected, non-`required`, non-`implicit` input shows an editor in its pin row; a wire hides it. The editor
  follows the pin type (`inline::inline_for_pin`): Float with a range → `FeathersSlider`; Float without one / Int →
  number field; Float2 → X/Y fields; Float3/Float4/Color → `FeathersColorInput` (button + popup picker with wheel,
  RGB/HSL, alpha, recent colors) plus R/G/B(/A) fields; Bool → checkbox. Constants and parameters edit their value
  under the title (`inline_for_param`); ScalarParameter's slider uses its `Slider Min/Max`.
- Ranges are a new optional `PinDef::range` (editor-only, not enforced in codegen). Set on Lerp `alpha`,
  Fresnel `base`, Desaturation `fraction`. Output-node pins have built-in fallbacks (`implicit`) and no editor.
- Editors carry the Properties panel's `PropField`, so edits go through the same `ValueChange` observers and
  `Action::SetComponent`/`SetValue`. An observer tells a canvas widget by `GraphQuery::node_of`; a finished canvas
  edit (`is_final`) pushes `Action::CommitInline` — ends undo merging (one drag = one undo step) and redraws
  Properties. Colors are stored linear (spec §7.1), the picker converts with `Color::LinearRgba`; the Properties
  swatch now does the same.
- The canvas is still rebuilt from the model, but not while text is typed into a field inside a node or a node's
  popup (the color picker) is open — the rebuild would despawn them (`canvas::inline_editing`). Meanwhile
  `sync_inline_fields` keeps the node's number fields in step with the model. Focused sliders don't defer the
  rebuild (they keep focus after a click); their drags are covered by the pressed-button check.
- RMB on an input pin (`Editor::pin_menu`): for polymorphic pins while none of the node's polymorphic pins is wired,
  "Float/Float2/Float3/Float4" (`Action::SetPinType`); "Reset to default" (`Action::ResetInput`); "Disconnect" on a
  wired pin. The type is stored as the type of the inline values in `Node::inputs` — `SigCtx::poly_type` already
  resolves from them — so the file format is unchanged. All unconnected polymorphic inputs are rewritten, since the
  common type is the widest one and changing one pin could not narrow it.

**Known limitations (Bevy/Feathers 0.20, not worked around).**
- Slider drag at canvas zoom ≠ 1: `bevy_ui_widgets` `emit_slider_drag_value_change` maps the drag distance with
  `UiGlobalTransform::transform_vector2` (multiplies by the zoom) where the inverse is needed, so the value moves
  zoom² times as fast as the cursor relative to the track. Same scaling for scrubbing a Feathers number field (no
  visual mapping there, only speed). Options: an upstream one-line fix (`transform.try_inverse()`), a
  `[patch.crates-io]` fork, or an own slider widget in this repo.
- Color picker at zoom ≠ 1: the `FeathersColorInput` popover lives inside the scaled canvas; `bevy_ui_widgets`
  places popovers from unscaled sizes around the scaled anchor, so it opens shrunk/grown and off its button (about
  100 px at fit zoom ≈ 0.75). Options: the same upstream fix in `popover.rs`, or an own picker popup spawned outside
  the canvas (closer to UE's separate picker window; Feathers' plane/slider/wheel widgets can be reused there).
- `FeathersColorInput` always shows an alpha slider; for Float3 values alpha is ignored.
- Feathers number fields print every digit they are given; fields show values rounded to 3 decimals
  (`state::display_value`), the model keeps full precision: a field sending its rounded text back on focus loss is
  not an edit (`Editor::set_component`).
- Inline values are edited in the pin's resolved type (`Editor::current_value` coerces the stored value), so a
  scalar stored on Lerp A becomes Float3 once a Float3 is wired into B and A is edited.

**Consequences.** No codegen or format change (snapshots and the sample game regenerate byte-identically). Headless
UI tests need the Feathers resources the widgets' observers read (`UiTheme`, `InputFocus`, `UnitsRegistry`, `Font`,
`Image` assets) — see `tests/ui_smoke.rs::app`.
