# 0033. Graph editor QA: snapping, header colours, node emphasis, comment frames, pin hover, drops

**Context.** A QA round on the node graph editor (design `docs/superpowers/specs/2026-10-26-editor-kinds-qa-design.md`
§4) asked for grid snapping, header colours by name, thicker selection / preview borders, a pulsing preview node,
deleting comment frames together with their nodes, a comment colour, a fix for "Show WESL" in preview mode, a larger
pin hover area, a dimmer grid, a slider range for function inputs and dropping Function / Curve assets onto a graph.
None of it may change `FORMAT_VERSION`, the bytes of existing asset files or the generated WESL / Rust, and the canvas
stays rebuilt from the model (decision 0008). This records what was built and where it deviates from the design.

**Decision.**

- **Snap to grid** (design §4.1). `doc::GRID_SNAP` = 5 graph units. `doc::snap` rounds a position to the nearest grid
  point; ties round away from zero (`f32::round`: 12.5 → 15, -12.5 → -15), and `-0.0` becomes `0.0`.
  - **Drag.** bevy_noodle has no snapping, so the `snap_drag` observer on `EditRequested` rewrites every step of a
    pointer drag (`GraphEdit::MoveNodes` with `drag: Some(..)` and origin `Interaction`). The *lead* node is the first
    entity of the step, which is bevy_noodle's first selected node and **not necessarily the one under the pointer**.
    `snapped_delta` gives the delta that puts the lead on `snap(start + total)`. Every other moved node gets the same
    delta, so a multi-node drag snaps only the lead and keeps the layout of the rest; nodes loaded off-grid reach the
    grid once they lead a drag. The node visibly jumps from grid point to grid point while dragging, not on release.
    `SnapDrag` holds the lead and its start position. It is cleared on the final step, on every `PointerDragStart`
    (`restart_snap_drag`, so a cancelled touch drag leaves nothing behind) and on any other `Interaction` edit. Moves
    made by code (`drag: None`) and the frame-carry re-emits (`CARRIED`) are left alone.
  - **New positions.** Placing a node (`Editor::add_node`: palette, add-from-wire, drops) snaps its position. Paste and
    Duplicate move the first clipboard node (in id order) onto the grid and the others with it. A reroute point and a
    comment frame created at the cursor snap too.
  - **Float noise.** `doc::tidy` pulls a coordinate within `1e-3` of a grid point onto it. Paste and `commit_moves` use
    it, so a follower of a snapped lead is stored as `35.0`, not `34.999996`. Real off-grid positions are kept.
  - **Not snapped:** positions loaded from a file (never rewritten until touched), auto-layout, the raw
    `Doc::add_node` / `add_node_with` API used by tests.
  - **Frames.** A dragged frame is a canvas node: dragged alone it is the lead, so its origin snaps, and the nodes it
    carries move by the same delta. Dragged with nodes, it follows the lead's delta. Frame **size is not snapped**:
    the editor has no frame resize gesture, and `w` / `h` are set only on creation (400×300, or the selection's bounds
    plus a margin).
- **Header colour by name** (design §4.2). `canvas::header_color` takes the hue from `name_hue` (FNV-1a 32-bit of the
  name's UTF-8 bytes, `hash % 360`) at fixed saturation 0.45 and lightness 0.32 (`name_header_color`); the header
  text stays readable on the dark theme. Name sources:
  - `stdlib::NamedRoute` (the declaration): its `name` param.
  - `stdlib::NamedRouteUsage`: its declaration's name, so a declaration and its usages share a colour. A usage without
    a declaration keeps the category colour.
  - Constant nodes (`stdlib::Constant*`) with a non-empty `Node::label` (decision 0022): the label.
  - `stdlib::Global*` nodes: the constant's name (the `ParamKind::GlobalConst` param), not a label.
  - Anything else, and an empty name: the category colour, as before. Different names can share a hue (hashing into
    360 values).
- **Node emphasis** (design §4.3, §4.4). Drawn as a bevy_ui `Outline`, not as the node border: an outline changes no
  layout, so ports and wires do not move when a node is selected. Every node keeps a 1.5 px border.
  `canvas::node_outline` decides it, in this priority:
  - the previewed node (decision 0022): 6 px, offset 0, pulsing;
  - a selected node: 3 px, offset 0, bevy_noodle's selected yellow, or the error / warning colour if it has a
    diagnostic;
  - a node with a diagnostic only: 2 px, offset 2, error / warning colour (as before).

  The pulse (`pulse_color`) is `k = 0.5 - 0.5·cos(2πt / 1.2 s)` mixing orange `(255, 140, 0)` into dark red
  `(140, 20, 20)` in sRGB. `rebuild_canvas` tags the previewed node with `PreviewPulse`; the `node_outlines` system
  runs every frame but only writes the `Outline` component (`set_if_neq`), so nothing rebuilds the canvas. It uses
  `Time::elapsed_secs_wrapped`, which wraps after 3600 s, a multiple of the period, so there is no phase jump. Known
  limits: the selected yellow and the warning colour are close (they differ in width and offset); a previewed node
  that is also selected shows only the pulse.
- **Dimmer grid** (design §4.9). The editor canvas's `CanvasGrid` uses white at alpha 0.006 (minor lines) and 0.014
  (major lines), tuned by screenshot (`ui/mod.rs`).
- **Deleting comment frames with their nodes** (design §4.5). `Doc::enclosed_frames` marks the frames that Delete
  removes together with the node selection, in one edit (`Doc::delete_nodes_and_frames`), so one undo step brings
  everything back.
  - A frame is marked (`Action::SelectFrames`, from `canvas::sync_selection`) only if both hold:
    1. **Geometry:** while bevy_noodle's `SelectionBox` exists, the box (converted with `CanvasView::canvas_to_graph`)
       **fully contains** the frame's rect. bevy_noodle's box selects every frame it merely touches; without this
       check a box that crosses a frame and covers its member nodes would delete it, and a frame with `nodes: []`
       would always qualify. Once the box is gone, the marks stay as long as the frame stays selected.
    2. **Membership:** there is a node selection and every node in `Frame::nodes` is in it (`Doc::enclosed`).
  - `Frame::nodes` is set when the frame is created and **not updated when nodes move**. A node dragged out of a frame
    is still a member, so boxing the frame without that node keeps the frame; a node dragged into a frame is not a
    member. The rule errs towards keeping frames: nothing is deleted that the box did not enclose.
  - A marked frame with a member node that **survives the delete** is kept: output nodes are never deleted, so a
    frame boxed together with the material output stays around it while its other nodes go (still one undo step).
  - **Cut keeps enclosed frames**: the clipboard holds no frames, so Cut clears the marks before deleting. Only Delete
    removes frames. Copy, Duplicate, moving and the right sidebar keep working on the node selection only.
  - Any new node selection clears the marks (`Action::Select`, `SelectFrame`, adding / pasting / dropping a node, the
    jump to a diagnostic). Clicking a frame still selects only the frame. Ctrl+box marks only the frames inside the
    current box. Frames the box only touched stay selected on the canvas (white border) but are not deleted.
- **Comment hue** (design §4.6). Stored as a new optional field `Frame.hue: Option<f32>` (degrees), after `color`,
  skipped when `None`: existing files load and save byte-identical (`frame_without_hue_roundtrips_byte_identical`).
  `canvas::frame_color` gives `Color::hsl(hue, 0.55, 0.55)` when set; otherwise the stored `color` or the default
  look, as before. Saturation / lightness can be retuned later without touching files. The graph right sidebar of a
  selected frame has a **Hue** slider (0..360, whole degrees; unset starts at the default colour's hue) and a
  **Default colour** button that clears the field (`PropTarget::FrameHue`). A slider drag is one undo step. The
  `frame_colors` system recolours frame entities from the model while dragging, since the canvas is rebuilt only after
  the gesture.
- **"Show WESL" in preview mode** (design §4.7). Not a pipeline bug: the regenerated file on disk had the preview.
  The generated-file view (`ui/text_view.rs::redraw`) caches on `(rel, revision)`, and every newly opened `TextDoc`
  counted its revision from 0. A WESL tab closed and reopened after the file changed got the same key `(rel, 1)` and
  kept showing the old text. `TextDoc::next_revision` now hands out process-wide unique revisions, so the key never
  repeats across tab instances (`tab::tests::reopened_text_doc_gets_new_revision`). No codegen change.
- **Pin hover** (design §4.8). The **highlight is implemented; the larger hit area is skipped**.
  - A spike gave every port a transparent, hoverable 20×20 child.
    - **Starting a drag** from it works: the drag start bubbles to the port.
    - **Dropping** on it does not: bevy_noodle's `on_drag` sets `WireTarget` only from a *port entity itself* under
      the pointer, so a drop beside the 12 px dot leaves the target empty and ends as `WireDropped` (the palette opens).
    - **Connected pins break**: a hoverable child is always above its port, and bevy_noodle's `pick_edges` skips edge
      picking only when the topmost entity is a port. With the hit area on top, a wire ending at a connected pin stole
      the press at low zoom.
  - Making drops work would mean writing `WireTarget` ourselves against bevy_noodle's own per-frame update, a hack on
    its drag state machine. Enlarging the port itself would make the dot look 20 px (`highlight_ports` paints the
    whole port) and stop wires short (`draw_edges` trims by the port size). Per the design's no-hacks rule the hit area
    is skipped. A clean path would be upstream: a component letting a non-port entity stand for a port in `pick_edges`
    and `WireTarget`.
  - The highlight: `pin_hover` / `pin_unhover` (`PointerOver` / `PointerOut` observers) put `PinHovered` on the port.
    Each port has a `PinHalo` child exactly over its dot, `Pickable::IGNORE` and hidden unless hovered. `pin_halos`
    (PostUpdate, after bevy_noodle's render systems) shows it in the port's current colours lightened by 0.15, at
    least 1.3× the dot size (`pin_hover_look`). It writes only halo components, so bevy_noodle's own port look, wire
    trimming and picking are untouched. bevy_picking keeps hovering during a drag, so the highlight also shows on the
    pin under a dragged wire.
- **FunctionInput slider range** (design §4.10). `output::FunctionInput` has optional `min` / `max` params ("Slider
  Min" / "Slider Max", appended after `order`). `ParamDef::shown_if("type", Float)` + `ParamDef::visible` hide them in
  the right sidebar unless the input's `type` is Float. `ir::declared_function_sig` sets `FnPort.range` only for a
  Float input with both values finite and `min < max` (the `param::Scalar` rule); the call node's pin takes it
  (`PinDef::range`), so its inline editor becomes a slider. Node params store only edited values, so a FunctionInput
  without them stays byte-identical (`function_files_unchanged`). Once edited, a value stays in the file even when set
  back to 0, like every param. The call node's "Input defaults" in the right sidebar stays a number field. No codegen
  change.
- **Dropping Project assets onto a graph** (design §4.11), through the same path as images (`droppable.rs`,
  `Editor::drop_item`, decision 0026):
  - a Function asset adds a `function::<path>` call node at the drop point, also when dropped on a node;
  - a Curve asset dropped on a node with a `ParamKind::Curve` param (`stdlib::SampleCurve`) sets that param, otherwise
    it adds a `stdlib::SampleCurve` reading the curve;
  - each drop is one undo step; other asset kinds are not droppable.
  - Only a **direct self-call** is rejected (a function dropped into its own graph): nothing changes and a warning
    says so. The rule is `search::is_self_call`, shared with the palette. Indirect recursion (A calls B, dropping A
    into B) is not detected in the editor, as in the palette; regeneration reports it as E006.

**Consequences.**
- Graphs converge to the 5-unit grid as they are edited, without a migration; old files keep their positions.
- Frame membership remains creation-time data. Deriving it from geometry (or updating it on moves) would change the
  frame-delete rule and the frame-carry behaviour and is left for later.
- The pin hit area waits for bevy_noodle support; the halo is independent of it.
- Debug hooks for the scripted checks (`demo.rs`, `lib.rs`): `SHADER_EDITOR_INPUT=box_delete|box_delete_partial`
  (`SHADER_EDITOR_INPUT_FRAME`), `hover` with `SHADER_EDITOR_INPUT_PIN`, `wire_hold`, `drop_game_file` with a
  `shaders/<path>.<kind>.ron` Project asset, and `SHADER_EDITOR_SELECT=<comment frame id>`.
