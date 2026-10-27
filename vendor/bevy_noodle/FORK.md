# bevy_noodle — local fork

This directory is a **local fork** of [bevy_noodle](https://github.com/gold-silver-copper/bevy_noodle), vendored from
the published crate **0.5.0** (crates.io, upstream commit `211f5df175755d9c73b1da9457f1802008f69951`). License: MIT
(`LICENSE`, unchanged).

The shader editor forked it because some canvas gestures cannot be built cleanly on the 0.5 public API; gesture and
canvas behaviour is implemented here at library level instead of as workarounds in `shader_editor_app`. The crate keeps
its name `bevy_noodle`, so code using it needs no changes. It is a member of the shader-editor workspace (root
`Cargo.toml`: `bevy_noodle = { path = "vendor/bevy_noodle", ... }`), so the workspace's `cargo fmt`,
`cargo clippy --workspace --all-targets -- -D warnings` and `cargo test --workspace` cover it, examples and tests
included. It is not published (`publish = false`).

Its own `clippy.toml` (strict: no `unwrap` / `expect` / `panic` / indexing outside tests, Bevy's panicking entity and
resource methods disallowed) still applies to the code in this directory — keep fork changes in that style.

## Changes from upstream

Every change to this directory appends an entry here (what and why).

### 0.5.0 vendoring

- Copied verbatim from the registry package: `src/`, `tests/`, `examples/`, `LICENSE`, `README.md`, `CHANGELOG.md`,
  `clippy.toml`. Not copied: the registry's `Cargo.lock` (the workspace lock file is used), `.cargo_vcs_info.json`,
  `.cargo-ok`.
- `Cargo.toml` is the upstream `Cargo.toml.orig` (not the registry-normalised manifest) with two edits:
  - removed `[profile.dev.package."*"] opt-level = 3`: profiles of a non-root workspace member are ignored by Cargo
    (with a warning). The root `Cargo.toml` keeps the fork optimised in dev builds through
    `[profile.dev.package.bevy_noodle]`, as before when it came from crates.io;
  - added `publish = false`.
- All `.rs` files were reformatted by the workspace `rustfmt.toml` (`max_width = 120`, `use_small_heuristics = "Max"`);
  whitespace / line breaks only, no code change.
- No lint fixes were needed: the crate passes `clippy -D warnings` as published. No tests or examples were dropped or
  gated; every example already declares its `required-features`, and the workspace build enables `default_style` and
  `scene`, so all of them are compiled and linted.

### Move-wires gesture (shader editor QA round 2, Task 11)

Why: the editor's Ctrl+drag moves a pin's wires to another pin of the same direction (UE "move wires"). On the 0.5 API
the drag could only be drawn as a new wire from the pin, and hiding / redrawing it needed overrides of the geometry,
style and port-highlight systems (see the Task 10 report). It is a library-level gesture now. With the new option
unset, behaviour is identical to upstream.

- `CanvasInteraction::move_wires_keys: Vec<KeyCode>` (default empty). Held when a drag starts on a connected port
  (input or output), the drag spawns `(MovingEdges { from, edges, pointer }, WireOf(canvas))` instead of a
  `PendingWire`: all edges of the port are *lifted*, with no edit and no `GraphEdit::Disconnect`. The key is read at
  drag start only. It takes precedence over `detach_wires`; on an unconnected port the drag is a normal new wire.
- `MovingEdges` (`components.rs`) shares the dragged-wire machinery: one per canvas through `WireOf` / `DraggedWire`
  (`GraphQuery::wire_of`), `WireTarget`, `WireCandidates`, snapping in `on_drag`, `snap_on_focus`, the crosshair
  cursor, `highlight_ports` (candidates highlighted, not dimmed), and the cancel paths (pointer cancel, the
  `CanvasKeyboard` cancel key, its connect key), which despawn it and so put the edges back.
- Candidates (`interaction::mark_move_candidates`, queued by the component's `on_add` hook like `mark_candidates`):
  ports of `from`'s direction other than `from`, by `GraphQuery::check_move` (new, `query.rs`, returning `EdgeMove`:
  missing ports, other canvas, other direction (new `RejectReason::OtherDirection`), nothing to move and an edge
  joining a node to itself are hard errors; types, already connected and a full `to` are the overridable verdict;
  a `Replace` port drops its oldest edges), then the new observer event `MoveCheck { canvas, from, to, edges, refused }`
  with `allow()` / `reject()` (`edit.rs`), mirroring `ConnectionCheck`. `ConnectionCheck` is not asked for moves.
- Geometry (`geometry.rs`): `update_edge_geometry` stays the only writer. A lifted edge's end at `from` is the
  `WireTarget` port's end (with its tangent) or the pointer with `from`'s own tangent, and the geometry's port there
  is the target or `None`; the far end stays. Lifted edges are recomputed every frame of the gesture, all edges when
  one ends (`RemovedComponents<MovingEdges>`, read every run so its cursor never lags). Its change filter also
  gained `Changed<EdgeSource>` / `Changed<EdgeTarget>`, so edges re-pointed by an edit get new geometry.
- Drawing (`style/mod.rs`, `draw_edges`): an edge takes its colour from its own `EdgeSource` / `EdgeTarget` ports
  (for an unlifted edge these are the geometry's ports, so nothing changes), so a lifted edge keeps its colour; lifted
  edges have no hover width, no `EdgeHitbox`, and the dragged wire's z.
- Drop (`on_drag_end`): onto a candidate, one `GraphEdit::MoveEdges { from, to, edges }` (new, through the normal
  pipeline: `MoveCheck`, then `EditRequested`, so an app can veto or translate it); applied, it re-points each edge
  (same entity) and reports `GraphChange::EdgesMoved { edges, from, to }` after `Disconnected` for edges it replaces.
  Anywhere else: the new event + message `EdgesDropped { canvas, from, edges, position }`, and the edges go back.
- `PortAnchor::new(node, offset, position)`: a public constructor, so headless tests can give ports (and so edges) a
  geometry; the library still overwrites it whenever it measures a port.
- Tests: `tests/interaction.rs` (lift an input / an output, no pending wire, geometry follows the pointer, snapping
  only to accepted candidates, one move request on a drop, drop elsewhere / pointer cancel restore, keys unbound by
  default, unconnected port), `tests/edits.rs` (`MoveEdges` by code), `tests/keyboard.rs` (cancel key), and a unit
  test in `style/mod.rs` (look of lifted edges). `tests/edits.rs`'s `kind` helper gained the new `GraphChange` arm.
- Docs: crate docs, README ("Move edges"), CHANGELOG (Unreleased).

### Grid snap and frames (shader editor QA round 2, Task 12)

Why: the editor snapped node drags by rewriting every `EditRequested` `MoveNodes` step of a pointer drag, with its
own drag-start state (a resource reset on `PointerDragStart`, on the final step and on any other edit), and carried a
comment frame's nodes by re-emitting each applied frame move with a custom `EditOrigin` (the upstream
`comment_frames` example's pattern). Both belong to the drag gesture. With the new options unset, behaviour is
identical to upstream.

- `CanvasInteraction::snap: Option<f32>` (default `None`). A pointer node drag puts the grabbed node (the
  `NodePosition`, its top-left corner) on the grid point nearest to where the pointer took it from its position at
  drag start, at every step and the final one; the other moved nodes take the same delta, keeping their layout.
  `DragProgress::total` stays the pointer's movement. Ties round away from zero, never `-0.0`; a non-positive or
  non-finite spacing does not snap. Keyboard nudges and code moves are not snapped.
- `NodeFrame` (`components.rs`, prelude): a frame node. When a drag of it starts (pointer), or it is nudged
  (`CanvasKeyboard` arrows), the nodes of its canvas whose laid-out rect (`NodePosition` + `ComputedNode`) lies
  entirely inside its rect, other than the moved ones, are added to the same `MoveNodes` edits. Membership is
  decided once per pointer drag, so a frame dragged over a node does not pick it up (the old per-step pattern
  did); nested frames are carried like any node.
- Drag state: a private `NodeDrag { lead, start, carried }` on the canvas, inserted by `on_drag_start` for every node
  drag (also unsnapped ones), read by `on_drag` / `on_drag_end`, removed by the drag end and `on_cancel`; a new drag
  start replaces it, so an abandoned drag leaves nothing behind. `on_drag_start` now selects an unselected grabbed
  node and records the moved set as before.
- `interaction::frame_contents` / `NodeLayout` (crate-private) are shared by the pointer and keyboard paths.
- Example `comment_frames.rs` uses `NodeFrame` instead of its `EditApplied` carry observer and custom origin.
- Tests: `tests/interaction.rs` (`snap_is_off_by_default`, `snapped_drags_put_the_grabbed_node_on_the_grid`,
  `each_drag_snaps_from_where_the_node_is_when_it_starts`, `dragging_a_frame_carries_the_nodes_inside_it`,
  `frames_carry_what_was_inside_when_the_drag_started`, `a_snapped_frame_moves_its_contents_by_its_own_delta`),
  `tests/keyboard.rs` (`arrows_move_a_frame_with_the_nodes_inside_it`).
- Docs: README ("Snap", "Frames"), CHANGELOG (Unreleased).

### Port hover look and port size constants (shader editor QA round 2, Task 12)

Why: the editor drew its pin hover as a halo child over each port, written after `NoodleSystems::Render` from the
colours and scale `highlight_ports` had just set, sized from copies of `kit::port`'s private size and border, with
its own `PointerOver` / `PointerOut` marker and a system confirming the marker on rebuilt ports against `HoverMap`.
The hover belongs to the port highlight. With no `PortHoverStyle`, ports look exactly as upstream.

- `style::PortHoverStyle { scale: f32, lighten: f32 }` (default 1.3 / 0.15; prelude), on a canvas for all its ports or
  on a port overriding it. `highlight_ports` applies it on top of the other states to a port whose `Hovered` is true:
  fill and border `lighter(lighten)` (alpha kept, so a dimmed port stays dim), scale `max(state scale, scale)` (a drop
  target's 1.4 stays). Wires trim to the scaled rim as for any scaled port. The canvas is looked up only for hovered
  ports.
- `PortHighlight` now requires Bevy's `Hovered` (`bevy_picking`, kept for the mouse from `HoverMap` every frame, also
  while a button is held, so during wire drags too). Without a hover style it is read by nothing. An app replacing a
  port under the pointer can insert `Hovered(true)` on the new entity; picking corrects it on the next frame.
- `style::kit::PORT_SIZE` (12) and `PORT_BORDER` (1.5) are public; `kit::port` uses them.
- Tests: `style::tests::hovered_ports_are_lighter_and_at_least_the_hover_scale`,
  `style::tests::hovering_changes_nothing_without_a_hover_style`,
  `style::kit::tests::ports_are_port_size_with_a_port_border`.
- Docs: README ("Style"), CHANGELOG (Unreleased), the style module table.

### The port under a refused drop (shader editor QA round 2, Task 12)

Why: a wire dropped on a port it may not connect to ends as `WireDropped`, which said nothing about that port, so the
editor read `HoverMap` itself to say why the connection was refused (and, since Task 11, why lifted edges did not
move). The library knows the hover at the drop.

- `WireDropped::onto: Option<Entity>` and `EdgesDropped::onto: Option<Entity>`: at the drop (`on_drag_end`, from
  `HoverMap` for the dragging pointer), the first hovered port of the same canvas other than `from`; `None` on empty
  canvas or over the source port. `WireDropped` stays `Copy`.
- Tests: `tests/interaction.rs` (`wires_dropped_on_a_refused_port_name_it`,
  `lifted_edges_dropped_on_a_refused_port_name_it`).
- Docs: the events' docs, CHANGELOG (Unreleased).
