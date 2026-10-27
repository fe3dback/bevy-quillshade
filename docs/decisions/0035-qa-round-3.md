# 0035. QA round 3: Bevy's default focus, panel toggles and Zen mode, hold C, pin reach, menu button state, icons

**Context.** A QA pass after round 2 (decision 0034) found:
- a main menu button that stayed active after its menu closed;
- keyboard focus logic that fought Bevy's default (Tab never reached a button, Enter never pressed one);
- no way to hide the left sidebar;
- a comment frame around the whole graph when nothing was selected, added on a plain C tap;
- wires that connected only when dropped exactly on a 12 px dot;
- no quick way to name a wire with a Named Route;
- a generic cube on the Rust button and a busy Texture icon.

The design is `docs/superpowers/specs/2026-10-27-qa-round-3-design.md`, with the user's corrections to §6.3 (pin size)
and §6.4 (no new palette entry). As in round 2, nothing may change `FORMAT_VERSION`, the bytes of existing asset files
or the generated WESL / Rust (`config.ron` may gain fields). Canvas behaviour goes into the bevy_noodle fork
(decision 0034's rule).

**Decision.**

- **Main menu button state** (design §1). Feathers 0.20 shows a menu button as active through `Pressed`. A
  `FeathersMenuToolButton` (`ActivateOnPress`) gets it on the press that opens the menu and loses it only on a release
  over itself. Our click barrier (decision 0034) is under the pointer by the time of that release, so it took the
  release: the button stayed active after the menu closed, and its next press was ignored (every second click on a menu
  name did nothing).
  - Fix: `ui::main_menu_barrier`, which already follows the main popups' visibility, keeps each main menu button's
    `Pressed` equal to its popup's visibility. Every close path (outside click, Esc, an item, another menu) ends in the
    popup going `Hidden`, so this one place covers them all. Feathers' own style systems restyle the button.
  - Context menus are not affected: they open on click, after the release, and have no menu button. Feathers selects
    have no barrier.
  - On a Bevy / Feathers upgrade: drop the sync if Feathers gains a real "open" state for menu buttons, or if picking
    starts routing the release to the pressed entity (pointer capture).
- **Keyboard focus is Bevy's default** (design §2). Tab / Shift+Tab walk every `TabIndex` widget under the root
  `TabGroup`, buttons, checkboxes, selects and fields included, with the focus ring on; Enter / Space press a focused
  button (`bevy_ui_widgets::button_on_key_event`). A click focuses the clicked `TabIndex` widget with the ring off.
  `keep_focus_on_fields` and the Tab / Shift+Tab bindings of `ToggleProperties` are removed.
  - A click on the canvas clears the focus without our help: the canvas has no `TabIndex`, so Bevy's `AcquireFocus`
    bubbles up to the window, which clears `InputFocus` (`on_window_acquire_focus_clear`). So after a toolbar button
    click, Space presses that button; after a canvas click, Space opens the palette. Never both.
  - Kept or added on top of the default, each only adding to it:
    - `main_input::tab_into_number_input`: when Tab focuses a used Feathers number input whose text is `Static`, the
      text becomes `Editable` and all selected. Feathers leaves it `Static` after a scrub, Enter or focus loss and does
      not change that on focus, so typing would otherwise neither edit the field nor be held back from the hotkeys. It
      never moves the focus. Tab is told from a click by the focus ring (`InputFocusVisible`), because a click also
      focuses with `FocusCause::Navigated`. It now works from any widget, not only from a field.
    - `main_input::PRESS_KEYS` (Space, Enter): the widget wins. While an enabled focused `Button` / `Checkbox` has one
      of them down, `Typing::now` holds the keyboard back from the hotkeys and re-arms them on release. Bevy's widget
      stops the `FocusedInput` propagation, but bevy_enhanced_input reads `ButtonInput` directly and has no public
      per-key consume. The rule covers exactly the keys `bevy_ui_widgets` presses with; other keys (Ctrl+S, Delete)
      still reach the hotkeys while a button has the focus. `InputSync` now also runs `.after(InputSystems)`, so the
      gate sees a key in the frame it goes down.
    - `main_input::drop_hidden_focus` (PostUpdate, after visibility propagation): a focused entity whose
      `InheritedVisibility` goes from visible to hidden loses the focus. Bevy clears the focus only on despawn, so a
      button hidden by a panel toggle, Zen mode or a tab switch would keep it, and Space / Enter would make an edit
      nobody sees. Only the transition counts, not "hidden now": a popup's items can hold the focus while the popover
      is still hidden for a frame.
    - The palette's `TabGroup { modal: true }` on `PaletteRoot`, so Tab stays inside it. Feathers dialogs
      (`ModalDialog`, made modal by `ModalDialogPlugin`) and menu popups (`MenuPopup` requires `TabGroup::modal()`)
      are already modal in Bevy.
    - Areas, strips, the left sidebar and the console also get `Visibility::Hidden` next to `Display::None`
      (`ui::areas::display`, written only on change): Bevy's tab navigation skips only `InheritedVisibility`-hidden
      entities, so a `Display::None` widget would still be reachable by Tab.
    - The typing gate (`gate_keyboard`, `Typing::now`, `typing_into`) is unchanged: Bevy's text input does not stop the
      propagation of typed keys, so the gate is what keeps the hotkeys quiet.
  - The one exception: **Ctrl+Tab** (`NextTab`) clears `InputFocus` and the ring. Bevy's `handle_tab_navigation`
    ignores modifiers, so Ctrl+Tab would also move the focus, and the tab switch replaces the areas the focus was in.
    The `next_tab` observer sets a `DropFocus` flag and `drop_focus` clears the focus in PreUpdate after both
    `InputFocusSystems::Dispatch` and `EnhancedInputSystems::Trigger` (the two are not ordered against each other, so
    clearing in the observer could be undone by a later dispatch).
  - Known side effects of the default, kept:
    - after a main menu or select pick, the focus returns to its button (Bevy's `MenuAction::FocusRoot`), so Space /
      Enter reopen that menu or re-press the button until a canvas click;
    - the first Tab after start logs a Bevy `WARN` ("No tab group found for currently focused entity"): Bevy's initial
      focus is the window, which has no tab group;
    - disabled buttons (the Preview group without a previewed node) are tabbable; Space / Enter do nothing on them, and
      `PRESS_KEYS` excludes `InteractionDisabled`, so Space there opens the palette.
- **View: panel toggles and Zen mode** (design §3).

  | Key | Action | Toggles | `UserConfig` field |
  |---|---|---|---|
  | Ctrl+1 | `ToggleLeftSidebar` | left sidebar (project + assets trees) | `show_left_sidebar` (new, default `true`) |
  | Ctrl+2 | `ToggleLog` | console/logs messages | `show_log` |
  | Ctrl+3 | `ToggleProperties` | right sidebar | `show_properties` |
  | `` ` `` | `ToggleZen` | Zen mode | `zen_mode` (new, default `false`) |

  - `MainCtx` binds `ctrl(Digit1/2/3)` with `on_combo` ("Ctrl" = Ctrl or Super) and `plain([Backquote])` with
    `on_plain`, so Ctrl+` / Alt+` / Super+` do nothing. All four are gated while typing. Each flips its flag and saves
    the config at once; an old `config.ron` loads with the defaults (struct-level `#[serde(default)]`).
  - **Zen priority.** `zen_mode` is stored apart and never rewrites the panel flags. `UserConfig::panel_visible(Panel)`
    = `!zen_mode && own flag` is the single place the layout asks (`ui::show_left_sidebar`, `log::show_log`,
    `areas::show_active_areas`). So Ctrl+1/2/3 in Zen mode still flip their flag (the View menu check mark follows)
    while the panel stays hidden, and turning Zen off shows exactly the panels whose flags are on.
  - `Panel::Console` (the strip with its header) is hidden only by Zen; `Panel::Log` (the messages) by Zen or by
    `show_log` off. So "Show console" off keeps the header strip as before, and Zen removes it. Zen keeps the
    menu_bar, tabs bar, editor topbar and editor statusbar.
  - Hidden panels take no space (`Display::None`) and are skipped by Tab (`Visibility::Hidden`).
  - View menu, in order: "Zen mode `", a separator, "Show project Ctrl+1", "Show console Ctrl+2", "Show properties
    Ctrl+3", each with its own check mark (also while Zen hides the panel). There is no platform Ctrl label helper:
    the hints are literal `Ctrl+…`, like every other hint.
  - The "hold 1–4 and click" node drop needed no change: `hotkey_node` already ignores keys while Ctrl / Alt / Super is
    held. Releasing Ctrl while 1 is still down, then clicking, drops a Constant: that is "hold 1 + click".
- **Graph right sidebar and editor statusbar** (design §5): the "Info" subpane is gone; Description is the last
  subpane (material: Material, Preview, Hotkeys, Description; function: Function, Hotkeys, Description). The editor
  statusbar shows `N nodes · N wires · N comments · N WESL lines`, so nothing is lost.
- **Comment with nothing selected** (design §6.1). `Doc::add_frame(title, nodes, at)` skips `bounds` for an empty
  set and makes a 400x300 frame (`EMPTY_FRAME_SIZE`) at `snap(at)`, with no nodes, in one edit (one undo step; it used
  to be two). `bounds(∅)` still means "all nodes", which `paste` relies on, and stays that way.
  - `Action::AddComment(None)` is the visible canvas centre: `editors::graph::comment_at_view_centre` (before
    `apply_actions`) fills it in from `canvas::view_center`. The topbar Comment button sends `None`. Hold C sends the
    cursor's graph position, or `None` when the cursor is off the canvas (ruling: the view centre, not graph (0, 0),
    which may be off screen). Headless (no canvas) `None` is the origin.
- **Hold C** (design §6.2). `main_input::held_hotkey::<A>(bindings, secs)` is `hotkey::<A>` plus a one-shot
  bevy_enhanced_input `Hold`; `AddComment` uses it with `COMMENT_HOLD_SECS` = 0.5. A tap does nothing; a hold fires
  once per press, while the key is still down.
  - Handlers of a held hotkey observe `Fire`, not `Start`: bevy_enhanced_input sends `Start` at the press
    (None → Ongoing) and `Fire` when the hold is reached.
  - The Ctrl+C release trap: consumption is per frame, so after Ctrl+C, letting go of Ctrl with C still down re-`Start`s
    the plain C action (this already added a comment before this round). `comment_press` (on `Start`) arms a
    `CommentKey` only when C was `just_pressed` with no Ctrl / Alt / Super, and `comment_at_cursor` (on `Fire`) acts
    only if armed. Any future held plain-key hotkey that shares its key with a Ctrl combo needs the same arming.
  - Tooltip "Add a comment frame (hold C)"; the sidebar's Hotkeys line says the same.
- **Pin reach and pin size** (design §6.3; fork entry "Nearest candidate within a radius, port physical / visual
  size" in `vendor/bevy_noodle/FORK.md`). Upstream 0.5 has no snap distance for wire drags: the target was only the
  candidate port picking hovers.
  - `CanvasInteraction::wire_target_radius: Option<f32>` (default `None` = upstream), in logical window px; the editor
    sets 40 (`canvas::WIRE_TARGET_RADIUS`, `attach_canvas`). While a new wire or lifted wires are dragged, `on_drag`
    makes the nearest `WireCandidates` port whose centre (`geometry::endpoint`) lies within the radius the
    `WireTarget` (ties by entity). Only candidates count, so `ConnectionCheck` / `MoveCheck` decide and a refused pin
    never takes the snap. With none within the radius, the candidate under the pointer, as upstream. The radius is
    turned into graph units through the pointer's own mapping, so it is constant on screen at any zoom.
  - Ruling: a refused port under the pointer (its box in `HoverMap`, not `from`) blocks the snap, so a drop there
    still reports why through `WireDropped::onto` / `EdgesDropped::onto`. Pointing at a pin is intent for that pin;
    silently wiring a neighbour would surprise.
  - During such a drag the hover look (`PortHoverStyle`) marks only the target, so exactly one pin looks hovered.
  - Rejected alternative: a larger invisible pickable area per port (decision 0033 found it steals hits from wires and
    is no drop target). The lookup only writes `WireTarget`; geometry snapping, highlighting and the drop are as
    before. Without a drag, hover and press stay on the port's box.
  - User decision, replacing "2x dots": `kit::PortSize { physical, visual }` (default: both `kit::PORT_SIZE`) and
    `kit::port_sized(port, color, size)`. Physical is the port node: picking, drags and drops, the wire anchor and the
    row's room. Visual is only the drawn circle, a non-pickable `style::PortDot` child with the border scaled to it;
    at equal sizes the port draws itself as upstream. The editor uses `canvas::PIN_PHYSICAL` = 24 and
    `canvas::PIN_VISUAL` = 16.
  - Rulings: wires end at the drawn dot (`trim_to_ports` reads the `PortDot`'s size and scales), since a gap would look
    like a glitch; hover, target and candidate scale grow only the dot, so the hit area stays the physical box.
  - Node sizes derive from `PIN_PHYSICAL`: pin rows are 24 px tall (nodes grow 2 px per row), the label padding is
    `10 + (PIN_PHYSICAL - PORT_SIZE) / 2`, and a Reroute is a 2:1 pill of the physical height (48x24, no 12 px
    exception), leaving one physical width of body to grab it by.
  - The radius counts logical window px: under a `UiScale` ≠ 1 it would not scale with the dots (the editor uses 1).
- **Named Route Declaration first after an output wire drop** (design §6.4, user correction: no new entry, no new
  filter). `ui::palette::results_for` uses `search::search_indices_leading`: after a wire from an output, the existing
  "Named Route" declaration entry sorts first when it matches. Entries, the wire-type filter and the query filter are
  unchanged, so the entry is hidden when the query does not match it and when its `in` cannot take the wire's type.
  - Ruling: the lead is a tie-break after the score, not a pin to the top. Empty query and `route`: Named Route first;
    `dot`: Reroute and DotProduct (keyword `dot`, score 100) stay above it.
  - It used to sort late because the list is ordered by `(category, name)` and "Utility" sorts late, not because of
    its score.
  - Ruling: every Named Route declaration added from the palette opens its rename dialog (`Action::AddNodeAndRename`
    → `rename_target` → `TextDialog`, the header double-click's dialog), since a route always needs a name. Cancel
    keeps "route"; the rename is its own undo step.
  - Adding a node from a wire is now one undo step for every node (`Editor::connect_checked(.., into_last_step)`). The
    old `undo(); redo(); connect()` "merge" recorded two steps.
  - The Enter-repeat guard: `ui::dialog::submit_on_enter` ignores key repeat, so an Enter still held from the palette
    pick does not submit the dialog it opened. An input dialog submits only on a fresh Enter press.
  - A wire from an input, and a palette opened any other way, keep the plain search order.
- **Icons** (design §4).
  - "Show generated Rust" uses our own line icon `icons/rust_rs.svg` (`IconId::Rust`, was `IconId::Crate` =
    `icon_rs::PART`): the lowercase letters "rs" as stroked paths, ink only, stroke 1.75. The WESL button keeps
    `IconId::Code`.
  - Asset kind icons come from icon-rs (user request): Texture assets use `copper-zone` (`icon_rs::COPPER_ZONE`, our
    `icons/texture.svg` is deleted), Global Consts tables `configurations` (`icon_rs::CONFIGURATIONS`, was
    `custom-table`).

**Superseded.**
- Decision 0034:
  - the "Keyboard focus" bullets "a `Button` / `Checkbox` never keeps the focus" and "Tab moves the focus only from one
    field to the next" (`keep_focus_on_fields` is gone);
  - `keep_focus_on_fields` making a Tab-reached number input editable (now `tab_into_number_input`, from any widget);
  - the View hotkeys `` ` `` (log) and Tab / Shift+Tab (properties): now Ctrl+1/2/3 and `` ` `` Zen mode;
  - the material sidebar's Info subpane and its order;
  - the hovered port's 1.3× hit area and "the larger hit area of decision 0033 is still not built": the hit area is
    the physical box and never scales;
  - the Rust button's `IconId::Crate` cube and the solid checkered Texture tile.
- Decision 0032: "Tab toggles the right sidebar" in its before → after table.

**Consequences.**
- Tests:
  - menu button: `main_menu_button_resets_on_every_close`;
  - focus: `tab_reaches_buttons`, `enter_and_space_press_focused_button`, `space_opens_palette_after_canvas_click`,
    `tab_stays_inside_dialog`, `tab_stays_inside_palette`, `ctrl_tab_moves_no_focus`, `tab_skips_inactive_areas`,
    `hiding_a_panel_drops_its_focus`, `focus_is_never_dropped`, `tab_does_not_toggle_properties`;
  - view: `view_flags_round_trip`, `old_config_shows_all_panels_without_zen`, `zen_folds_into_panel_visibility`,
    `zen_hides_panels_and_restores_flags`, `ctrl_digits_toggle_panels`, `backquote_toggles_zen`,
    `view_keys_ignored_while_typing`, `view_menu_marks_follow_flags`, `zen_hides_whole_console`,
    `hidden_left_sidebar_takes_no_space`, `ctrl_1_does_not_arm_constant_drop`;
  - graph: `add_comment_without_selection_is_default_size_at_cursor`, `add_comment_with_selection_wraps_it`,
    `graph_status_counts_wires_and_comments`, `graph_sidebar_has_no_info_and_description_is_last`,
    `comment_button_adds_frame_at_view_centre`, `tap_c_adds_no_comment`, `hold_c_adds_one_comment`,
    `ctrl_c_held_never_adds_comment`, `hold_c_while_typing_adds_nothing`;
  - palette: `output_wire_drop_palette_starts_with_named_route_declaration`, `input_wire_drop_palette_unchanged`,
    `palette_without_link_unchanged`, `palette_named_route_declaration_opens_rename`,
    `search::tests::lead_entry_sorts_first_when_it_matches`, `dialog_enter_ignores_key_repeat`;
  - pins: `wire_drop_near_pin_connects`, `pins_have_physical_and_visual_sizes`, `pin_hover_highlights_the_dot`, the
    fork tests listed in `FORK.md`.
- Debug hooks: `SHADER_EDITOR_INPUT=tab[:N]` (logs where each Tab puts the focus), `menu_click` / `menu_close`
  (+ `SHADER_EDITOR_INPUT_MENU`), `SHADER_EDITOR_INPUT_AT=to:<dx>,<dy>` for wire drags (offset from the target pin);
  the wire hold log prints the snapped target.
- Kept as known: a main menu button keeps Feathers' pressed token while open (Feathers 0.20 has no "open" token);
  with the 24 px boxes and candidate scale, adjacent input dots nearly touch at low zoom; the "rs" glyph's s-arcs are
  plain ellipse halves (invisible at 16 px).
