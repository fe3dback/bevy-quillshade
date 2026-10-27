# Changelog

## Unreleased (shader-editor fork)

### Changed (breaking)
- `WireDropped::onto` (and the same field on the new `EdgesDropped`): the
  port under the pointer at a drop away from any candidate (one the wire may
  not connect to, or the edges may not move to), so an app can say why
  without reading `HoverMap`. A new public field: code building
  `WireDropped` by struct literal must add it.

### Added
- Moving edges between ports: `GraphEdit::MoveEdges { from, to, edges }` re-points
  edges to another port of the same direction, keeping their entities, and
  reports `GraphChange::EdgesMoved`. `GraphQuery::check_move` and the
  `MoveCheck` observer event (`allow()` / `reject()`) decide, like
  `GraphQuery::check_connection` and `ConnectionCheck` for connections;
  `RejectReason::OtherDirection` refuses a move between an input and an output.
- `CanvasInteraction::move_wires_keys` (empty by default): with one held, a drag
  from a connected port lifts all its edges (`MovingEdges`) instead of dragging
  a new wire. Their loose ends follow the pointer and snap to the ports they may
  move to; a drop there requests one `MoveEdges`, a drop elsewhere reports
  `EdgesDropped` and, like a cancel, puts them back. Nothing is edited before
  the drop. The default style keeps their colour and draws them like the
  dragged wire.
- `PortAnchor::new`, for ports that are never laid out (headless tests).
- `CanvasInteraction::snap` (`None` by default): pointer drags put the grabbed
  node on the nearest point of a grid of that spacing at every step, and move
  the rest of the selection by the same delta.
- `style::PortHoverStyle { scale, lighten }` on a canvas or a port: a
  `PortHighlight` port the mouse is over (Bevy's `Hovered`, now required by
  `PortHighlight`), also under a dragged wire, is drawn lighter and at least
  `scale` times its size. Without it, ports look as before.
- `style::kit::PORT_SIZE` and `PORT_BORDER`: the size and border of the
  `kit::port` dot.
- `NodeFrame`: a node that carries the nodes lying entirely inside it (decided
  when a drag starts) when it is dragged with the pointer or the keyboard, in
  the same `MoveNodes` edits. The `comment_frames` example uses it instead of
  an `EditApplied` observer.

## 0.5.0 - 2026-10-13

### Changed (breaking)
- `NoodleDefaultStylePlugin` no longer adds Bevy's `CursorIconPlugin`, so it
  can be added before or after `FeathersPlugins`. Apps without Feathers add
  `CursorIconPlugin` themselves for hover and wire-drag cursors; without it
  the graph still works, with the default cursor.
- `style::kit` returns BSN scenes instead of bundles, composed with Bevy's
  `bsn!` macro: `commands.spawn_scene(bsn! { @kit::node(at) ChildOf(canvas)
  Children [ @kit::title("Add") -- @kit::row("a", Port::input(NUMBER), BLUE) ] })`.
  Patch single fields over the defaults (`@kit::node(at) Node { min_width:
  px(240) }`) and put any content, such as feathers controls, between the
  rows: the node pads it in line with them, so `kit::PADDING` is gone.
  `kit::row` and `kit::dot` take a `Port`, replacing `input`, `input_with`,
  `output`, `input_dot` and `output_dot`. `default_style` now enables Bevy's
  `bevy_scene`. `SelectedBorderColor` and `PortColor` have defaults, so
  scenes can patch them.

### Added
- `GraphQuery::source_of(node, index)`: the output feeding a node's input,
  the usual first step of evaluating it. The examples use it in place of
  their own `inputs_of(..).nth(i)` / `peers_of(..).next()` walks.
- `geometry_nodes` example: Blender-style nodes whose wires carry lists of
  points, including a node that queries entities already in the world,
  re-evaluated every frame. The README explains that evaluating a graph is
  the app's job.
- `shader_world` example: the graph compiles into a WGSL shader that
  raymarches a landscape you fly over. Rewiring regenerates the shader;
  sliders are uniforms, so they reshape it live.

## 0.4.0 - 2026-10-11

### Changed (breaking)
- Requires Bevy 0.20 and Rust 1.97.1. Pointer events are Bevy's flat
  `PointerPress`, `PointerDrag`, … events; the default style's shaders are
  WESL.
- Canvases spawn their own `CanvasContent` child (found with
  `GraphQuery::content_of`), and nodes spawned as children of a canvas move
  into it: `commands.spawn((kit::node(at), ChildOf(canvas)))`.
- Edges have no parent: their ports alone decide which graph they are in,
  so nothing re-parents them when nodes move between graphs. `edges_in`
  lists the edges leaving a canvas's outputs. Pointer events on an edge
  bubble to the window, not the canvas: observe them on the edge or
  app-wide (panning and scrolling from an edge still work).
- Selection is not a graph edit: `GraphEdit::Select` is gone. Use
  `commands.select(canvas, items, mode)` (or `world.select`), read it with
  `GraphQuery::selected_in` and `selection_with`, and react to Bevy's
  `Selected` being added or removed.
- Connection rules live in `ConnectionCheck` observers (`allow()` /
  `reject()` the built-in verdict), asked by real connects and by previews
  alike (dragged wires, keyboard connections,
  `GraphWorldExt::preview_connection`). `EditRequested` fires only for edits
  about to apply, may rewrite them (a rewritten connect is checked again) or
  `reject()` them, and is the place for side effects. Marking a dragged
  wire's candidates is about six times faster.
- `GraphQuery::check_connection(canvas, a, b)` takes the canvas first and
  returns a `Connection { ports, replaces, refused }`. Port pairs are a
  `PortPair { output, input }` everywhere (`edge_ports`, `GraphChange`).
- Edits report what they changed as a `GraphChange`: `Connected { edge,
  ports }`, `Disconnected { edge, ports }`, `Moved { nodes, delta, drag }` or
  `Deleted { items }`. `EditApplied` carries it as `change` (instead of
  `edit`, `created` and `ports`), and `graph_edit` returns it.
- `GraphEdit::MoveNodes { nodes, delta, drag }`: build complete moves with
  `GraphEdit::move_nodes`; pointer drags add a `DragProgress { total,
  is_final }`, and `GraphChange::is_drag_step` tells undo stacks what to skip.
- A port's limit and what a full port does are one `Port::capacity`:
  `Capacity::Unlimited` (the default for outputs), `Capacity::Replace(n)`
  (the oldest edges make room; inputs default to one) or
  `Capacity::Refuse(n)`, with `n` a `NonZeroU32`. `Port::with_capacity`
  replaces `with_max_connections` and `when_full`, and `WhenFull` is gone.
- Zoom limits are `CanvasView::min_zoom`/`max_zoom`; `zoom_around` keeps to
  them.
- `GraphQuery` methods listing entities return iterators. `nodes_in` and
  `ports_of` walk only the graph they are asked about.
- Key settings are lists (empty unbinds): `CanvasKeyboard` has `move_left`
  … `move_down` and `zoom_in`/`zoom_out`, and `CanvasInteraction::zoom_keys`
  is `zoom_modifiers`. Both share one default for additive selection.
- `EdgeGeometry` is present only while both ends are laid out (no `valid`
  flag), with `output`/`input` ports; `EdgeGeometry::between` takes no ports
  (`with_ports` adds them).
- Pressing a node raises it with a `ZIndex` instead of reordering children;
  nodes with a negative `ZIndex` stay under.
- `EditRejected` and `WireDropped` are messages too, like `EditApplied`.
- `PortType`'s fields are private (`PortType::named`, `id`); it prints its
  name. `PortAnchor`'s fields are read-only (`GraphQuery::port_position`).
- Core modules are private: everything is at the crate root, and the
  prelude has what apps use. Snapshots are `SnapshotWorldExt` methods
  (`world.snapshot`, `snapshot_nodes`, `insert_snapshot`, `restore_snapshot`).
- `EdgeStyle::layer: EdgeLayer` is now `below_nodes: bool`. `FrameAll` and
  `NoodleSystems::Measure` are gone: ports are measured in Bevy's
  `UiSystems::PostLayout`.
- A dragged wire is `(PendingWire { from, pointer }, WireOf(canvas))`: a
  canvas has at most one (`DraggedWire`, or `GraphQuery::wire_of`), and
  spawning another replaces it. Its state lives on it: the ports it may
  connect to are its `WireCandidates` (set by the library, so custom
  bindings that make wires get them for free), and the one it snaps to is
  its `WireTarget(Option<Entity>)`. Ports no longer carry `WireCandidate`
  or `WireTarget` markers; `PendingWire::canvas` and `target` are gone.

### Added
- A runnable `quick_start` with two styled, connectable nodes, mirrored in
  the README and checked by rustdoc. The style documentation examples now
  run as doctests.
- CI checks library-only builds for every feature combination on Linux,
  Windows and macOS, the Rust 1.97.1 minimum on Linux, and release packaging.
- Hover cursors from Bevy's `EntityCursor` in the default style: `kit`
  nodes show a grab hand and ports a crosshair, which stays while a wire is
  dragged (`OverrideCursor`). `NoodleDefaultStylePlugin` adds Bevy's
  `CursorIconPlugin` if the app has not.
- Nodes and edges are Bevy `Selectable`s, so with an `AccessibilityNode`
  their `Selected` state reaches screen readers.
- Controls inside nodes: a press or drag that starts in a focusable control
  (anything with a `TabIndex`) belongs to the control, so sliders, text
  fields, color pickers and menus no longer select, raise or move the node.
- `scene::Transient`: snapshots leave out entities marked with it (and their
  descendants), for UI the app rebuilds from its own data.
- Keyboard use, opt-in per canvas: `NoodleKeyboardPlugin` and
  `CanvasKeyboard` (Tab focus, Enter, arrows, Space to connect, Escape), built
  on `bevy_input_focus`; a `FocusOutline` in the default style.
  Ctrl/Cmd+arrows pan the view and +/- zoom it. Example: `keyboard`.
- Examples: `comment_frames`, `reroute`, `type_conversion` (int and float
  ports with a converter), `minimap`, `auto_layout`.
- `kit::input_dot`/`kit::output_dot`: port rows without labels, about a
  fifth fewer UI entities per node and no text to lay out. The `stress`
  example switches to them with T.
- `tools/audit_inputs`: runs the examples and checks every text field's
  focus and cursor.

### Fixed
- Wires drawn above nodes no longer intercept presses on focusable controls
  or their children. Graph node backgrounds remain pickable for edges.
- Failed snapshot writes remove partial entities, including generated
  content of nested canvases. Restore keeps the old graph until the
  replacement has been inserted successfully.
- The input audit fails on crashes, timeouts and missing results, and keeps
  stdout and stderr logs for diagnosis.
- Edges of an outer graph are pickable where they pass over a nested canvas.
- New wires and selection boxes show in the frame they appear, not the next.
- Restoring or inserting a snapshot no longer leaves children it left out
  listed in their parent's `Children`.
- The `FocusOutline` also appears when focus becomes visible later.
- Ending a dragged wire no longer clears the candidates of a wire dragged
  on another canvas.
- The library and examples no longer panic: no `unwrap`, `expect`, indexing
  or `panic!`, and none of Bevy's methods that panic (or queue a failing
  command) when an entity or resource is gone, enforced by clippy lints.
  Loading a `save_load` model whose edges name a missing node or port
  reports an error.
- The plugins add the resources their systems read (`UiScale`, `HoverMap`,
  `PointerMap`, `ButtonInput<KeyCode>`, `InputFocus`, `InputFocusVisible`),
  so headless apps no longer panic without Bevy's UI and picking plugins.
- `CanvasView::zoom_around` no longer panics when `min_zoom > max_zoom`.
- Pointer positions are right under `UiScale`, and drags follow the
  pointer inside a nested canvas whose outer canvas is zoomed.
- A cancelled pointer drops its dragged wire and selection box, and a wire
  goes with the port it started at.
- Deleting a node reports the edges of nodes inside it as disconnected.
- Unparenting a node disconnects its edges to the graph it left.
- Keys typed into a control inside a node (a text field) no longer zoom,
  pan or cancel.
- Pinch zoom respects `InteractionDisabled`.
- Edges that lose their `EdgeStyle` are hidden and no longer pickable.
- Additive (Shift) box selection shrinks back to what was selected before.

### Examples
- styled, editor, save_load, subgraph, scene_builder_3d, type_conversion,
  keyboard and comment_frames edit every value in the node, with Bevy's
  feathers controls (number fields, sliders, color pickers, a dropdown) or
  `EditableText` with Bevy's `TextInput`, and show results live. Feathers
  number fields can be dragged to scrub their value; the ring count in
  `scene_builder_3d` has a `HardLimit`. `save_load` also saves the graph
  model alone (a few hundred bytes instead of a full snapshot).
- Text fields show their cursor and selection: `math_graph` and
  `comment_frames` give theirs a `TextCursorStyle`, and the feathers examples
  work around two `bevy_feathers` 0.19 issues (`examples/feathers_fixes`):
  fields spawned after startup got a near-invisible cursor, and a press on a
  field's frame selected the node instead of focusing the field.

## 0.3.0

A smaller, Bevy-native core: you build and style the nodes and edges, the
library handles the graph.

### Added
- Pickable edges: `EdgeHitbox` gives edges `Pointer` events; clicking selects
  them, and `GraphEdit::Delete` removes selected nodes and edges together.
- `scene` feature: `DynamicWorld` snapshots of a graph or of selected nodes
  (`snapshot`, `snapshot_nodes`, `insert`, `restore`) for undo, copy and
  paste, and saving.
- `EdgeStyle`: gradients, dashes, animated flow, layer (above or below
  nodes), hover and selection highlights, trimming at port rims.
- `kit::canvas()`, and flatter kit nodes (two fewer entities each).
- `EditApplied::ports`, `GraphQuery::edges_in`, `EditResult`.
- Examples: `edge_styles`, `subgraph`, `scene_builder_3d`, `editor`,
  `save_load`, `stress`.

### Changed
- Interaction is opt-in per canvas (`CanvasInteraction`), with configurable
  buttons and keys; no built-in key bindings.
- `GraphEdit::DeleteNodes { nodes }` → `Delete { items }`;
  `Select { nodes, .. }` → `Select { items, .. }`; `nodes_of` → `nodes_in`.
- Edges are children of the canvas content and follow their ports into other
  graphs; `Edge` is a marker.
- `NodePosition` is optional: without it, your layout places the node.
- Wire snapping and pinch zoom use Bevy's `HoverMap`; edge hit testing uses
  `bevy_math` curves.

### Removed
- `NoodleKeyBindingsPlugin`, `CanvasKeymap`, the action events, the node
  finder popup, `CanvasWantsInput`, `KitTheme`, `kit::body`.

### Fixed
- `NodeDragHandle` is enforced for the whole drag.
- The interaction plugin no longer panics without a picking backend.
- Edits no longer scan the whole canvas, so large graphs stay fast.

## 0.2.0
Headless, entity-based redesign.

## 0.1.0
First release.
