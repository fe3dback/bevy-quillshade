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

### Nearest candidate within a radius, port physical / visual size (shader editor QA round 3, Task 3)

Why: a wire had to be dropped exactly on a 12 px dot to connect: the target was only the candidate port picking
hovers. The user asked for a reach (UE-like): the nearest pin the wire may end on, within 40 screen px, takes it; and
for ports easier to hit without large dots: a port's box (hit area) apart from its drawn dot. Upstream 0.5 has no snap
distance for wire drags (only the hovered port) and one fixed dot size. With the new options unset, behaviour is
identical to upstream.

- `CanvasInteraction::wire_target_radius: Option<f32>` (default `None`), in logical window pixels. In `on_drag`, for
  a `PendingWire` or `MovingEdges`: the `WireCandidates` (so `ConnectionCheck` / `MoveCheck` have decided: a refused
  port never takes it) whose centres (`geometry::endpoint`: node position + measured offset, as the wire geometry
  uses) lie within the radius of the pointer, nearest first (ties by entity, so the choice is stable), become the
  `WireTarget`; with none within it, the candidate under the pointer, as upstream (a port larger than the radius at a
  high zoom still takes the wire). With the pointer on a port of the canvas it may not end on (its laid-out box, in
  `HoverMap`; not `from`), nothing snaps, so a drop there reports that port as without a radius (`WireDropped::onto`
  / `EdgesDropped::onto`). The radius is turned into graph units at the pointer through the same mapping as the
  pointer itself (`Ctx::graph_per_pixel`: zoom, outer transforms, `UiScale`), so it is the same on screen at any
  zoom. A non-positive or non-finite radius is ignored. The drop (`on_drag_end`) is unchanged: it connects / moves to
  the `WireTarget`. Picking is untouched: without a drag, hover and presses stay on the port's box.
- `highlight_ports`: on a canvas with a usable radius, while its wire is dragged, the `PortHoverStyle` look goes to
  the wire's target (also when it is not under the pointer) and only to it; elsewhere, and on other canvases, hover
  works as before.
- `style::kit::PortSize { physical, visual }` (default: both `PORT_SIZE`) and
  `style::kit::port_sized(port, color, size)`. `physical` is the port's node: picking (hover, press, drags, drops),
  the wire anchor (centre) and the room in its row; it stays centred on the node's border and
  its row. `visual` is only the drawn dot: when it differs, the port node is undrawn (no fill, no border) and a child
  `style::PortDot` (prelude; `Pickable` ignoring it, absolute, centred, border `PORT_BORDER * visual / PORT_SIZE`)
  draws it. `highlight_ports` colours and scales the `PortDot` child when there is one (the port's box, its hit area,
  keeps its size while hovered or targeted), the port otherwise. `draw_edges` trims wires (`trim_to_ports`) to the
  dot's rim (its size × its scale × the port's scale), so they touch the drawn dot. At equal sizes nothing changes:
  `kit::port` is `port_sized(.., PortSize::default())`, the same `Node` drawing itself, no child. A kit function
  rather than a canvas component: ports are scenes built before they join a canvas. The private `PORT_RADIUS`
  constant is gone.
- `WireCandidates`' field is `pub(crate)` (a style unit test builds one).
- Tests: `tests/interaction.rs` (`wire_target_radius_is_off_by_default`,
  `wires_take_a_candidate_within_the_radius_in_screen_pixels_at_any_zoom` (35 px diagonally at zoom 1, 0.5, 2:
  target, drop connects), `wires_beyond_the_radius_do_not_snap` (45 px diagonally: off-target `WireDropped`),
  `the_nearest_candidate_wins` (10 px over 25 px, the farther one hovered),
  `a_refused_port_under_the_pointer_blocks_the_snap` (on a refused port 5 px away, a candidate 30 px away: no target,
  the drop names the refused port), `a_refused_port_near_the_pointer_never_takes_the_target` (the same off its box:
  the candidate), `lifted_edges_take_a_candidate_within_the_radius` (zoom 2, diagonal, snapped geometry, one
  `MoveEdges`), `without_a_radius_only_the_hovered_port_is_the_target`),
  `style::tests::with_a_target_radius_the_hover_look_goes_to_the_wire_target`,
  `style::tests::a_port_dot_takes_the_look` (the dot is coloured and scaled, the port's box keeps scale 1),
  `style::tests::wires_end_at_the_drawn_dot` (12 at a 24 px port, 8 at its 16 px dot, 10.4 at that dot hovered),
  `style::kit::tests::the_default_port_size_is_the_upstream_port`,
  `style::kit::tests::a_port_is_its_physical_box_and_draws_a_visual_dot_inside_it`.
- Docs: README ("Reach", "Style"), CHANGELOG (Unreleased), the style module table, the field's, `PortHoverStyle`'s,
  `PortSize`'s and `PortDot`'s docs.

### Doc examples compile only (shader editor CI)

- The two runnable doc examples (`style::kit` module docs, `style::EdgeStyle`) are `no_run`, like the README's. In the
  editor's workspace Bevy is built with `dynamic_linking`, so a doctest binary links `libstd` dynamically; rustdoc runs
  merged doctests from a temp dir without the toolchain's library path, and on CI they failed to start
  (`libstd-….so: cannot open shared object file`). `no_run` still compiles them against the API.
