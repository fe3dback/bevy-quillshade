# 0034. QA round 2: Feathers widgets, View / Edit menus, number inputs, the bevy_noodle fork, graph and texture polish

**Context.** A QA pass after the editor-kinds round (decisions 0031–0033) found usability gaps (no typing into a
limited slider, no way to hide the log or the right sidebar, no Edit menu), hand-built look-alikes of Feathers 0.20
widgets, graph polish (snap step, the Ctrl+drag wire look, rename by double click, Preview mode and generated code in
the editor topbar), texture / image polish (icons, zoom in the image tab, an always-selected Easy preset), the parked
minor findings of the previous round, and a red Windows CI job (design
`docs/superpowers/specs/2026-10-27-qa-round-2-design.md`). None of it may change `FORMAT_VERSION`, the bytes of
existing asset files (`config.ron` may gain fields) or the generated WESL / Rust. Mid-round the user decided to fork
bevy_noodle locally, because the Ctrl+drag look could not be built cleanly on its public API.

**Decision.**

- **Line endings (Windows CI).** The Windows checkout turned LF into CRLF, so `include_str!`-embedded texts (the
  `baked_mod.rs` template, the stdlib `.wesl` modules) carried `\r\n` into tests, generated files, the byte-for-byte
  snapshots and the CI `regen` + `git diff` step.
  - `.gitattributes`: `* text=auto eol=lf` (LF on every OS), `*.png` / `*.ktx2` `binary`.
  - CR guards name the cause if it comes back: `shader_codegen_rust::baked::tests::template_has_lf_line_endings`,
    `shader_stdlib::tests::modules_have_lf_line_endings`.
  - CI runs `cargo test --workspace --no-fail-fast`, so one red run shows every failure.
  - The audit found one real Windows bug: `Project::open` canonicalises the root into a verbatim `\\?\C:\…` path,
    where Windows does not translate the `/` of our joined constants and asset paths.
    `layout::without_verbatim_prefix` drops the prefix (`C:\…`, `\\srv\share\…`); unlike `dunce` it does so even
    where the plain form means another path (reserved device names, trailing `.` / space), acceptable for a project
    directory. Verification is static: there is no Windows machine, the next push confirms it.
- **Cleanup of the parked minors** (design §2), in short: one `editors::fill_sidebar_with` (+ `SidebarKey::refresh`)
  for all six sidebars; `show_active_areas`, the editor statusbar and the menu_bar project text keyed on
  `(ui.log, ui.tabs, active, revision)`-style keys with no per-frame allocation (`UiRev::tabs` is the tabs
  generation: open, close, activate, rename); one `ui::plural`; `keys.rs` (`mods`, `no_command_mods`) below both `ui`
  and `editors`; `Doc::set_selection` (clears `enclosed_frames`); `Editor::plan_move_wires`, the pure dry run of a
  wire move that `move_wires` applies; `Editor::asset_diagnostics`; `wesl_lines` follows rename / delete; a
  `shown_if` on a pin type matches `Value::Type` or the type's label. The **selected node is white** (outline and the
  node's own `SelectedBorderColor`), like a selected comment frame, so it no longer reads as the warning colour.
- **Feathers containers and controls** (design §3, helpers in `ui/widgets.rs`):
  - Left sidebar: the project tree and the assets tree each in a `pane` (`pane_box`: `pane_header` with title and
    tools, `pane_body`). Right sidebar: one pane titled with the kind's `SIDEBAR_TITLE`, its content in `subpane`s
    (`subpane_box`), e.g. a material: Material, Preview, Description, Info, Hotkeys (*superseded by 0035:* no Info;
    Material, Preview, Hotkeys, Description); one node: Node, Parameters,
    Input defaults, Problems. Console/logs: a pane whose header holds the counts, Clear messages and the show / hide
    chevron. `group()` is unused. `section()`, `header()`, `header_row()` and `chip()` are gone.
  - `button_group(Vec<GroupButton>)`: joined `FeathersButton`s, corners Left / None / Right (All when single), 1 px
    gap, `ButtonVariant::selected`, a tooltip per button, `InteractionDisabled` for a disabled one (no `Activate`, no
    action; an icon caption dims to alpha 0.3, a selected one only to 0.9 so a disabled group still shows its choice,
    and `ui::icons::icon` fades its accent layer with it). Used by
    Easy / Advanced, the texture presets, R | G | B | All, the graph Preview mode group, the curve preview mode and
    the Quick Start step tabs.
  - Labelled bools are captioned `FeathersCheckbox`es (`widgets::checkbox`, `props::checkbox_row`): the label
    toggles too. The row has no label column: the box starts where the other rows' labels do (a note under it lines
    up with the caption). The uncaptioned `checkbox_w` stays only where the label is elsewhere (inline node editors,
    consts table cells).
  - Layout: the UI root needs `min_height: 0` as well as `min_width: 0`; the center column keeps
    `CENTER_MIN_W` = 400 px and the sidebars (`min_width: 0`) give way below it; buttons, groups and captioned
    checkboxes never shrink, and editor topbar rows (`editors::bar_row`) wrap whole items; panel scroll columns clip x.
- **Widget audit** (design §3; every other hand-built widget checked against Feathers 0.20):

  - **Replaced:**
    - the main menu (File / Edit / View / Assets / Help): `FeathersMenu` + `FeathersMenuToolButton` +
      `FeathersMenuPopup` / `FeathersMenuItem` / `FeathersMenuDivider` (`ui::main_menu`; `fill_main_menus` refills the
      popups when spawned and when a View flag changes);
    - context menus (tab, tree row, node, pin, curve, browser): a `FeathersMenuPopup` under a zero-size anchor at the
      cursor (`dialog::open_menu_entries`; Feathers has no container for a menu at a point); signatures kept;
    - the curve preview Catmull-Rom / Simple buttons: `button_group`;
    - the About link, the Welcome recent-project rows, the editor statusbar "N errors · M warnings" link and the tab
      close cross (`Button` + `HoverRow`): `FeathersButton` / `FeathersToolButton` `Plain`;
    - number sliders: `FeathersNumberInput` (below).
  - **Kept**, with the reason:
    - tabs bar tabs: Feathers has no tab widget (`bevy_ui_widgets` tabs are headless only);
    - tree rows and their caret: no tree widget; the whole row toggles / opens / drags, so
      `FeathersDisclosureToggle` would be a second, focusable toggle inside it, drawn with Feathers' PNG chevron
      instead of our icons;
    - log rows: a row is an action (focus the node), not a selection; `FeathersListRow` needs a `ListBox`, which takes
      the keyboard (Space / Enter / arrows next to our Space hotkey) and has taller rows;
    - palette, tooltip, canvas, curve plot, node previews: no Feathers equivalent;
    - `labeled` / `hotkey_row`, consts table rows / cells, `bar_row` / `sidebar_body`, Quick Start code blocks: layout,
      not widgets (the table cells are Feathers widgets);
    - `scroll_column`: `ScrollArea` is `bevy_ui_widgets`; a `FeathersScrollbar` would be a new feature, not a
      replacement;
    - `HoverRow`: for the kept rows (tabs, tree, log, palette); no button uses it any more.

  The menus keep icons (and their slot when any item has one), right-aligned dim `\t` hotkey hints, separators,
  disabled items with a reason tooltip and Esc, and gain Up / Down / Home / End / Enter / Space and an above-flip when
  there is no room below. A transparent full-window barrier (`dialog::menu_barrier`, z 94 under the popup's 95;
  `main_menu_barrier` while a main popup is visible) makes an outside click only close the menu, as before; the
  popup stops `AcquireFocus` so a press on its padding or a divider keeps it open. Because a Feathers menu needs the
  keyboard focus, only one menu is open at a time and its first item has the focus. Labels use the Feathers font,
  hints stay mono. The tooltip shows the innermost of several hovered tooltip widgets (a tab's close cross over the
  tab's path). Not covered: Feathers' own select lists and the colour picker popover have no barrier, so an outside
  click there also acts on what it hits (as before this round).
- **Number inputs with soft limits** replace every number slider (design §3): inline ranged fields on nodes
  (`Inline::Ranged`, from a pin `range` or `param::Scalar` min / max, only when `min < max`), the comment Hue, the
  consts `f32` row, and sidebar Float rows that have a range (the same soft limit as on the node).
  - `FeathersNumberInput` + `SoftLimit`; `HardLimit` only for a real bound (the Hue, 0..360). Dragging stays inside
    the soft range; a click without movement edits the text, and a typed value outside the soft range is kept, not
    clamped. The consts `f32` row's soft range is `slider_range(value)` and widens (`sync_fields` re-inserts
    `SoftLimit` with the value) when a typed value leaves it.
  - Precision: 2 for ranged fields (the former sliders), 3 for unranged fields and vector components (as before), 0
    for the Hue, 3 for the consts row. Feathers derives the drag speed from the precision.
  - A drag is one undo step: non-final `ValueChange`s merge, `is_final` ends the step (the old `SetComponent` path).
  - Every press on a number input focuses its text (`PointerFocusPlugin` → `AcquireFocus` → the text's `TabIndex`),
    a scrub included. `main_input::typing_into`: a focused text that is `TextReadWriteMode::Static` (Feathers' "not
    editing") counts as typing only while LMB is held, so hotkeys are off during a scrub and back after it, and a
    scrubbed field does not defer the canvas rebuild. Clicked into, the field is `Editable` and takes every key. A
    number input that Tab reaches from a field while its text is `Static` (it was scrubbed, or lost the focus, before)
    is made `Editable` with its text selected by `keep_focus_on_fields`, like a fresh one: otherwise typing would
    neither edit it nor be gated, and the keys would fire hotkeys (*superseded by 0035:* `tab_into_number_input` does
    this for a Tab from any widget). `Static` as "not editing" is a Feathers convention;
    re-check it on a Bevy upgrade.
  - Colour sliders (`FeathersColorSlider` in the colour picker) stay.
- **Keyboard focus** (`main_input::keep_focus_on_fields`, an observer on `FocusGained`). *Superseded by 0035* (the
  first two points): focus is Bevy's default, `keep_focus_on_fields` is gone, Tab reaches buttons and Enter / Space
  press them.
  - A `Button` / `Checkbox` never keeps the focus (any cause but `Auto`), unless it is inside an open popup
    (`MenuPopup` / `Popover`, the `Popups` system param). Otherwise Space / Enter re-activated the last clicked button
    and Tab walked a focus ring over the buttons. So **Tab to a button + Enter is gone**; Tab moves the focus only from
    one field to the next while typing.
  - Bevy 0.20 focuses a click with `FocusCause::Navigated` too (`acquire_focus_tab_index`), so "this came from Tab" is
    told by the focus ring (`InputFocusVisible`: Tab shows it, a press hides it): a non-button focus is dropped only
    when the ring is on and the user was not typing. Clicked non-button `TabIndex` widgets (a field, the colour plane,
    a list) keep the focus with the ring hidden; Bevy's initial focus on the window is kept (harmless).
  - `gate_keyboard` runs before `InputFocusSystems::Dispatch`, so `KeyboardGate::typing` describes the focus the key
    goes to. The gate also covers an open context menu (`MenuRoot`) and focus inside a popup. A Tab-focused field
    gets no key of the same frame's batch (dispatch fixes the target per batch).
- **menu_bar Edit and View, View settings** (design §4):
  - Edit: Undo (`Ctrl+Z`), Redo (`Ctrl+Shift+Z`); the icon-only Undo / Redo buttons stay. View: "Show log" (`` ` ``)
    and "Show properties" (Tab), a check icon while on, none while off. *Superseded by 0035:* View lists Zen mode
    (`` ` ``), Show project (Ctrl+1), Show console (Ctrl+2), Show properties (Ctrl+3).
  - `UserConfig::show_log` / `show_properties` in `config.ron`, default `true`; the struct's `#[serde(default)]` loads
    old files with both shown. `Action::ToggleLog` / `ToggleProperties` flip the flag and `save_config()` at once.
  - The layout keys on the flags themselves (no `UiRev` counter): `log::show_log` gives the hidden log's `LogPane`
    height Auto, hides the pane body, rounds the header and flips its chevron; `show_active_areas` (key `(active,
    kind, show_properties)`) hides the right sidebar and the editor content takes the width.
  - Hotkeys in `MainCtx` (`on_plain`): `` ` `` toggles the log, Tab the properties (Shift+Tab too: Shift is allowed on
    plain keys); Ctrl+Tab stays Next tab (the combination consumes Tab). *Superseded by 0035:* Ctrl+1/2/3 and `` ` ``
    Zen mode; Tab is Bevy's focus navigation. The graph palette moves from Tab to **Space**
    (user decision); RMB on empty canvas still opens it.
- **Graph editor topbar** (design §5.2, §5.4): a dynamic part (`GraphTopbarDynamic`, `redraw_graph_topbar` keyed on
  `(active, AssetId, is_material, preview.mode, previewing)`): for materials the Preview mode `button_group` Unlit /
  Lit / Full (`IconId::Unlit`, `Pbr`, new `Layers`), pushing the same `SetValue(PropTarget::PreviewMode)` the old
  sidebar select did (one undo step, the same regen), disabled with `InteractionDisabled` while no live node is
  previewed (a dangling id counts as none) but still showing the stored mode (the selected icon stays bright, the
  tooltips name their mode: "Preview: Lit - preview a node first (V)"); then right-aligned icon-only "Show
  generated WESL" (`IconId::Code`, materials and functions) and "Show generated Rust" (new `IconId::Crate` =
  `icon_rs::PART`: icon-rs has no Rust or package glyph; materials only; *superseded by 0035:* our "rs" glyph,
  `IconId::Rust`). No RON / source button (user decision). The
  sidebar's Preview → Mode row is gone; the "No node previewed - select a node and press V" hint stays.
- **Snap 10** (design §5.1): `doc::GRID_SNAP` = 10. Old positions stay until touched (no migration). The snap tests
  were re-based with inputs that still tell a restarted snap from a continued one.
- **Rename by double click** (design §5.5):
  - `Editor::rename_target(node)`: `NodeLabel` for Constant / Constant2–4Vector (labelled or not), `Param name` for
    every `param::*`, `FunctionInput`, `FunctionOutput` and a Named Route declaration (its usages refer to it by node
    id, so they follow). `Editor::header_action` is the priority list: rename dialog, else open the referenced asset,
    else nothing. A Named Route usage's header does nothing.
  - Header vs body: a `canvas::NodeHeader` marker on the title entity; a double click whose path from the original
    target up to the node crosses it is a header click. The body keeps opening the referenced asset; inline fields
    keep their clicks.
  - Comment title: the frame's title is a pickable full-width strip (`FrameTitleStrip`, min height 20 px; it was
    not pickable before); its double click opens the dialog for `FrameTitle`. Drags from the strip still move the
    frame.
  - `Action::TextDialog(PropTarget)` opens a `Dialog::Input` ("Rename node" / "Name" or "Comment" / "Title",
    prefilled, `AutoFocus`, so the typing gate holds every hotkey). `Submit::SetText(target)` applies
    `set_value(.., merge: false)`, the sidebar's own edit (one undo step, the same validation and diagnostics), and
    bumps `ui.props` so the sidebar shows the new text. No inline validation. `Action::Submit` trims the text, so an
    empty or blank Constant label clears it (`""`, which is "no label") and **a comment title loses leading /
    trailing whitespace**, which the sidebar's Title field keeps.
- **The local bevy_noodle fork** (`vendor/bevy_noodle`, user decision). The Ctrl+drag look on bevy_noodle 0.5's
  public API needed six workarounds: a transparent fake pending wire, a second per-frame writer of `EdgeGeometry`, a
  `set_changed` on `PortAnchor` to restore, a per-frame restyle of ports (same-direction ports are never candidates,
  so `highlight_ports` dims every one), per-edge style overrides saved and restored by hand, and a Ctrl latch racing
  the pending wire's spawn. So the crate is vendored from 0.5.0 (upstream commit `211f5df1…`) as a workspace member
  and path dependency with the same name, `publish = false`, its own strict `clippy.toml`, and kept optimised in dev
  builds by the root `[profile.dev.package.bevy_noodle]`.
  - Rule: canvas / gesture behaviour goes **into the fork**, not into app workarounds; hacks stay an anti-goal in
    both. Every fork change is appended to `vendor/bevy_noodle/FORK.md` ("Changes from upstream") with what and
    why. Each change is additive and off by default (empty `move_wires_keys`, `snap: None`, no `NodeFrame`, no
    `PortHoverStyle`), and every upstream test passes unchanged.
  - Upgrade: vendor the new upstream release and re-apply the `FORK.md` entries one by one, with their tests. The
    risk is that upstream's drag state machine, geometry or highlight systems change shape under the move-wires
    gesture; until then the fork does not follow upstream fixes by itself.
- **Move-wires gesture** (design §5.3; supersedes the mechanism of decision 0032, not its semantics):
  - Fork API: `CanvasInteraction::move_wires_keys` (the editor: Ctrl and Super, left and right). Held at drag start
    on a **connected** port it spawns `MovingEdges { from, edges, pointer }` instead of a `PendingWire`: all the
    port's edges are lifted, with no edit and no new wire; the key is latched at drag start (releasing Ctrl mid-drag
    changes nothing). `update_edge_geometry` stays the only geometry writer: the lifted ends follow the pointer
    (with `from`'s tangent) or snap to the `WireTarget` port; the far ends stay. Lifted edges keep their colour (taken
    from their own ports), are drawn above the nodes, and have no hover width or hitbox. Candidates are ports of the
    same direction, checked by `GraphQuery::check_move` (`EdgeMove`, `RejectReason::OtherDirection`) and the observer
    event `MoveCheck` (`allow()` / `reject()`); `highlight_ports` treats them like wire candidates. A drop on a
    candidate requests one `GraphEdit::MoveEdges` (applied: `GraphChange::EdgesMoved`, edges re-pointed in place);
    anywhere else, `EdgesDropped` and the edges are back. A pointer cancel or the keyboard cancel key also puts them
    back. `PortAnchor::new` exists for headless tests.
  - App: `canvas::check_move` answers `MoveCheck` with `Editor::plan_move_wires(..).is_ok()`, the sole authority (all
    or nothing, the same per-wire `connection_error` check as the move itself), cached per (from pin, to pin) in
    `MoveVerdicts` (cleared when a gesture starts: the model does not change during one, and a drop is planned again
    when applied). `canvas::request_move_wires` vetoes every
    `MoveEdges` in `EditRequested` and pushes one `Action::MoveWires` instead (one undo step; the model is the source
    of truth, decision 0008). `idle()` counts a `MovingEdges`, so the canvas is not rebuilt mid-gesture.
  - A drop on a pin the move is refused for logs `plan_move_wires`' reason ("Wires not moved: …",
    `ui::input::lifted_wires_dropped`, the pin from `EdgesDropped::onto`); a drop on empty canvas restores silently
    and opens no palette. A Ctrl+drag from an **unconnected** pin is a normal new wire, whose drop on empty canvas
    now opens the palette like any wire drop (before, Ctrl suppressed that).
  - **Esc** (`GraphCtx` `CancelWire`, no modifier filter since Ctrl is held) cancels any wire drag, a lift or a new
    wire, by despawning `GraphQuery::wire_of(canvas)`, as the fork's own cancel does; a later release does nothing.
    The typing gate keeps Esc for the palette, dialogs and fields.
- **Other canvas workarounds moved into the fork** (with the options unset the fork behaves as upstream):
  - Grid snap: `CanvasInteraction::snap` (`Some(GRID_SNAP)`). The **lead is the grabbed node** (before: the first
    selected node, not necessarily the one under the pointer); it lands on `snap(start + total)` at every step, the
    others take its delta. The drag state (`NodeDrag`) is per gesture: a new drag start or a pointer cancel ends it.
    Placement snapping of new / pasted nodes stays in the model (`Editor::add_node` / `paste`). Replaces
    `snap_drag` / `SnapDrag` / `restart_snap_drag`.
  - Frame carry: `NodeFrame` on comment frames. The nodes whose laid-out rect lies inside the frame **when the drag
    starts** are added to the same `MoveNodes` edits (one edit per step, one undo step); a frame dragged over a node
    no longer picks it up, **nested frames are carried** too (the old re-emit carried only nodes), and keyboard nudges
    of a frame carry its contents. Replaces `carry_frame_contents` / the `CARRIED` origin.
  - Pin hover: `style::PortHoverStyle { scale: 1.3, lighten: 0.15 }` on the canvas; `PortHighlight` requires
    `Hovered`, which picking keeps up to date (also during drags). The **port dot itself grows** (at least 1.3×, a
    drop target keeps 1.4) and lightens, alpha kept; wires end at the larger rim, and the hovered port's hit area is
    1.3× (a small hysteresis; *superseded by 0035:* the hit area is the port's physical box and never scales, only
    the drawn `PortDot` grows). `rebuild_canvas` puts `Hovered(true)` on a rebuilt pin's new port (only the app knows
    it replaces the old one); picking corrects it a frame later. `kit::PORT_SIZE` / `PORT_BORDER` are public.
    Replaces `PinHalo`, `pin_halos`, `PinHovered`, `CarriedHover`, `confirm_carried_hover`. The larger hit area of
    decision 0033 is still not built (*superseded by 0035:* `kit::PortSize` physical 24 / visual 16 and the 40 px
    wire reach).
  - `WireDropped::onto` / `EdgesDropped::onto`: the port hovered at a refused drop. The plain-drag "Connection
    refused: …" notice and the Ctrl-drop message read it; the app no longer reads `HoverMap`.
  - Kept in the app, as they work around nothing in noodle: `RightPress` (RMB click vs pan, 3 px slop),
    `edge_click` (Alt / RMB click deletes a wire, double click adds a reroute: UE policy) and the frame title strip.
    `node_outlines` and noodle's `outline_focus` do not conflict (editor nodes never take keyboard focus).
- **Image and texture tabs** (design §6):
  - Shared preview `ui/image_preview.rs` (`ImagePreviewPlugin`, `doc_kind::PreviewDoc` on `ImageDoc` and
    `TextureDoc`): wheel zoom around the pointer, drag pan, double-click fit, zoom label, R | G | B | All, Tile 3x3,
    the checkerboard; moved out of `texture_view.rs`, not copied. `Action::PreviewChannel` / `PropTarget::PreviewTiled`
    replace `TextureChannel` / `TextureTiled`. Channel and tiling are view state (not saved, not undoable, never
    dirty). The camera is per tab, keyed by `(ViewKind, image path)`, so **a changed image path resets the zoom**;
    other tabs' cameras are stashed and pruned when their tab closes. An image tab tiles with a Repeat sampler (a game
    image has no sampler and the default clamps).
  - **Default preset = no sampler** (no new field): `FilterPreset::Default` first (Default, Pixel art, UI, 3D,
    Ground) ↔ `TextureSettings::sampler = None`. `FilterPreset::of` matches on the `Option`, so a *stored* sampler
    equal to `ImagePlugin::default()` is custom, not Default. `fits_easy == of().is_some()`. Easy is disabled
    (tooltip "Settings match no preset - edit them in Advanced") and `SetEasy(true)` is a no-op while it does not fit;
    switching never rewrites the settings; undo / redo / reload to a custom sampler leaves Easy
    (`TextureDoc::after_history`). The texture topbar is keyed on `(layout, fits_easy)`, apart from the form.
  - **Texture icon**: icon-rs has nothing that reads as a texture (appearance, patterns, layers… reviewed), so
    `icons/texture.svg` is ours (a Solid rounded tile with a 3x3 checker; *superseded by 0035:* icon-rs `copper-zone`,
    the SVG is deleted). It marks Texture assets, Texture tabs, the
    virtual Internal rows (they open a Texture tab) and "Create Texture Asset"; game images and image tabs keep
    `IconId::Image`.
  - Tooltips use " - ", not an em dash: the UI font has no U+2014.
- **Consts editor**: the constant count is only in the editor statusbar (design §6.4): neither the right sidebar nor
  the editor topbar (only the table's path) repeats it; how values are used is the sidebar's Usage subpane. The `f32`
  row is one number input (above).

**Superseded.**
- Decision 0032: the Ctrl+drag *mechanism* (bevy_noodle draws a new wire, `wire_dropped` reads `HoverMap` and turns
  a drop on a same-direction pin into `MoveWires` while Ctrl is held at drop time) and "a Ctrl drop on empty canvas
  does nothing" for unconnected pins; the graph palette on Tab (now Space; Tab toggles the right sidebar, *superseded
  by 0035:* Tab is focus navigation); the typing
  gate's definition (it now also covers menus and popups). The semantics of `MoveWires` are unchanged.
- Decision 0033: the 5-unit grid (now 10), the snap lead and its `snap_drag` mechanism, the `CARRIED` frame carry,
  the `PinHalo` hover, and the selected yellow (now white).

**Consequences.**
- Tests:
  - View: `view_flags_default_and_round_trip`, `toggle_log_and_properties_persist`,
    `backquote_toggles_log_tab_toggles_properties`, `view_keys_gated_while_typing`, `hidden_log_keeps_header`,
    `hidden_properties_hides_right_sidebar`, `view_menu_check_marks_follow_config`;
  - focus: `focus_stays_on_fields_and_open_popups`, `click_into_field_keeps_focus`,
    `tab_and_shift_tab_toggle_properties_and_move_no_focus`, `open_menus_and_popups_block_hotkeys`;
  - number inputs: `typed_number_beyond_soft_limit_is_kept`, `scrub_drag_gates_hotkeys_only_while_held`,
    `number_drag_is_one_undo_step`, `number_precision_by_kind`;
  - menus: `context_menu_is_a_feathers_menu_that_only_closes_on_outside_click`, `context_menu_item_click_and_esc`,
    `main_menu_is_a_feathers_menu_with_check_marks`;
  - move-wires: `ctrl_drag_*`, `escape_cancels_wire_drags`, `plain_wire_dropped_on_refused_pin_says_why`, and the
    fork's `tests/interaction.rs`;
  - rename: `rename_target_kinds`, `rename_dialog_*`, `header_dblclick_opens_rename_or_reference`,
    `frame_title_dblclick_opens_dialog`, `typing_in_rename_dialog_fires_no_hotkey`;
  - textures: `default_preset_is_no_sampler`, `easy_preset_round_trips_through_file`,
    `history_and_reload_leave_easy_for_custom_sampler`, `image_tab_zooms_and_fits`;
  - snap 10 (`snap_rounds_to_ten`) and the CR guards.
- Rewritten tests: the snap / frame tests drive real pointer drags now that snapping lives in the gesture (injected
  `MoveNodes` edits are not snapped any more); in `snap_drag_restarts_per_gesture` the step "any other edit ends the
  drag" became "a pointer cancel ends it". The pin hover tests check the port's own scale and colour and set
  `Hovered` directly, as picking would. `port_size_matches_noodle` is gone (the constants are the fork's).
- Debug hooks: `SHADER_EDITOR_DEMO=menu[:<name>]`, `game_file_menu[:2]`; `SHADER_EDITOR_INPUT=ctrl_wire_hold`,
  `type_field`, `frame_dblclick`, `node_dblclick` with `SHADER_EDITOR_INPUT_AT=header`,
  `SHADER_EDITOR_INPUT_AT=<dx>,<dy>` for wire drags, `hover` with `SHADER_EDITOR_INPUT_TIP=<prefix>`, `texture_zoom`
  on image tabs too. `slider` drags over the field's on-screen width (scaled by the canvas zoom; it used the layout
  width and missed the field at fit zoom), takes an inline field only when it lies wholly inside the visible canvas,
  and logs its node's position: on rock_wet it scrubs `n05.default` 0.3 → 0.36 at fit zoom and `n06.default`
  1 → 8 at zoom 1, the node staying put.
- Parked, not done in this round:
  - Icons: the Rust button is a generic cube (done in 0035); the Sprite2d material icon is the game image's picture.
  - At the 800 px minimum window the consts table clips its Value column; wrapped texture topbar rows sit tight on the
    view.
  - Code: `log::show_log` has 10 parameters; unreachable Bool arms in `props::value_widget` /
    `texture_view::field_widget`; double padding of `pane_body` + `scroll_column`; the image tab calls
    `GameImageCache::get_channel` (a file check) every frame; `redraw_graph_topbar` clones the asset id every frame.
  - The rebuild's `Hovered` carry ignores the asset: switching between two graphs with the same pin key can show the
    hover on the other graph's pin for one frame.
  - Fork: no fork test for carrying nested frames or for the zero-size layout fallback; `move_nodes` carries a
    `too_many_arguments` allow.
  - Test harness caveats: the ui_smoke `click` helper registers `Window` so events bubble, `menu_app` forces every
    entity visible, gesture tests skip hit testing, headless text has no font layout.
  - Number-field scrubbing at canvas zoom ≠ 1 still runs at the wrong speed, and the colour picker opens off its
    button (Bevy 0.20, decision 0012).
  - Frame membership for Delete (`Frame::nodes`) is still creation-time data (decision 0033).
  - No Windows run: the line-ending and path fixes are verified only by the CR guards and the audit.
