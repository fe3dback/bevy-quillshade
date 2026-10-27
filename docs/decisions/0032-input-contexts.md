# 0032. Input contexts and UE5-style graph gestures

*The Ctrl+drag mechanism and the palette on Tab are superseded by [0034](0034-qa-round-2.md) (marked below).*

**Context.** All hotkeys used to live in one hand-written `ui::input::shortcuts` system. It had a `not_typing` run
condition, `ctrl_held` / `pressed_hotkey` helpers and `editor.doc().is_some()` / cursor-over-canvas guards for the
graph-only keys. The curve view had its own `nudge_keys` system. Most keys were pushed on every tab, and whatever
the active tab did not support was silently ignored. Graph mouse gestures followed bevy_noodle's defaults plus a few
Ctrl modifiers, unlike UE5's Material Editor, which the editor models (design
`docs/superpowers/specs/2026-10-26-editor-kinds-qa-design.md` §2.2).

**Decision.**

- **bevy_enhanced_input 0.27** (`default-features = false`) handles keyboard hotkeys (`editors::main_input`).
  - Every input context sits on one entity, `InputRoot`:
    - `MainCtx` (priority 2) is always active and evaluated first.
    - Each editor kind that has hotkeys adds one context: `GraphCtx`, `CurveCtx` or `TextCtx` (priority 1).
      `sync_input_context::<E>` activates it while the active tab is of that kind and deactivates it otherwise. It
      writes `ContextActivity` only when the state changes.
    - Consts, image and texture tabs have no kind context: only `MainCtx` works there.
  - A kind takes part through `editors::EditorInput` (`type Ctx`, `fn bindings()`) and the `EditorAreas::input(app)`
    hook, which calls `add_kind::<Self>` and registers the kind's observers. `InputContextsPlugin` visits the kinds
    through `for_each_kind`, so there is no second list of kinds. A new kind with hotkeys adds both to its
    `editors/<kind>.rs`, in addition to the recipe in decision 0031.
  - Every action is built with `hotkey::<A>()`: `ActionSettings { consume_input: true, require_reset: true }`.
    Handlers observe `On<Start<A>>` and push `state::Action`s. The shared edit actions (`EditCopy`, `EditCut`,
    `EditPaste`, `EditDuplicate`, `EditSelectAll`, `EditDelete`) have one observer each; `Editor` routes the
    resulting `Action` to the active tab, as before.
  - "Ctrl" in a binding always means Ctrl or Super: the `ctrl(k)` and `ctrl_shift(k)` helpers bind both.
- **Combinations beat plain keys.**
  - bevy_enhanced_input evaluates actions with more modifiers first, and `consume_input` hides their keys from the
    rest. So Ctrl+S hides S, Ctrl+Shift+S beats Ctrl+S, and Ctrl+V beats V.
  - An unbound combination must not fire the plain key either: Ctrl+F must not fit the view. Plain-key handlers
    (`on_plain`, convert, preview, comment, palette, curve nudge) therefore check `no_command_mods` and ignore the
    key while **Ctrl, Alt or Super** is held. Shift is allowed (Shift+arrows, Shift+numpad).
  - Consequence: AltGr, which Windows reports as Ctrl+AltRight, also blocks plain keys. This is acceptable because no
    plain graph or curve binding needs AltGr.
  - `BlockBy` was rejected. It needs a named "modifier held" action inside the same `bsn!`, and for Shift bindings it
    reads that action's state from the previous frame, so a combination pressed in a single frame would get through.
- **Typing gate.** `gate_keyboard` runs in `PreUpdate`, before `EnhancedInputSystems::Update`. It sets
  `ActionSources.keyboard = false` while the user types, for **every** context, `MainCtx` included. "Typing" means a
  dialog is open, the palette is open, or `InputFocus` is on an `EditableText`, which is the old `not_typing`. The
  widgets' own keys stay local to the widget: the palette list, Enter in a dialog, the consts add form and the
  texture path field. *Extended by 0034:* an open menu or popup also counts as typing, a scrubbed number field only
  while LMB is held, and the gate runs before `InputFocusSystems::Dispatch`.
- **Held keys after a context switch.** bevy_enhanced_input's `require_reset` only acts on *deactivation*: the
  bindings of a context that turns inactive stay ignored while held. On *activation* it does nothing, so a context
  that was active before would fire a key held during a tab switch at once.
  - When `sync_input_context` activates a context, or when typing ends, it re-inserts that context's `Binding`
    components. The library's `Binding` `on_insert` hook re-arms its `FirstActivation` flag, and the key is ignored
    until released.
  - This relies on library internals: `FirstActivation` is `pub(crate)`, and we depend on the documented behaviour
    "a new binding ignores inputs that are already active".
  - The guarding test is `held_key_not_fired_after_tab_switch` in `tests/input_contexts.rs`. **Re-check it on every
    bevy_enhanced_input upgrade.**
- **Mouse gestures** stay picking observers on bevy_noodle entities (`ui/input.rs`, `ui/curve_view.rs`). They read
  modifiers through `main_input::mods()` (`ModKeys::pressed`; since moved to `keys::mods()`) instead of hand-written
  key lists.
- **Graph gestures follow UE5:**
  - **Alt+LMB click on a pin** removes all its wires (`DisconnectPin`). **Alt+LMB click on a wire** removes the wire.
    RMB click on a wire still removes it. Ctrl+LMB no longer removes anything; on a wire it only toggles the wire's
    selection (bevy_noodle's additive select).
  - **A plain LMB drag from a connected input starts a new wire** (`CanvasInteraction.detach_wires = false`).
    Dropping it on an output connects as usual, and the connect replaces the input's old wire in one step: the model
    connect replaces the edge, and the canvas translation drops the "disconnected" change it reports first.
  - **A Ctrl+LMB drag from a pin moves its wires** (`Action::MoveWires`, `Editor::move_wires`).
    - bevy_noodle draws the wire as usual. A pin of the same direction is never a wire candidate, so a drop there
      arrives as `WireDropped` with no target. While Ctrl (or Super) is held, `wire_dropped` turns a drop on another
      pin of the same direction into `MoveWires`. *Superseded by 0034:* the drag is the bevy_noodle fork's move-wires
      gesture: Ctrl is latched at drag start, the pin's wires are lifted and follow the cursor (no new wire), they
      snap only to pins `plan_move_wires` accepts, and the drop's `GraphEdit::MoveEdges` becomes the same
      `MoveWires`; `wire_dropped` no longer reads Ctrl or `HoverMap`. The rules below are unchanged.
    - From an input, its single wire `src → pin` becomes `src → to`; a wire already into `to` is replaced.
    - From an output, every wire `pin → dst` becomes `to → dst`.
    - All or nothing:
      - `to` of the other direction: nothing changes, and a warning names the pin.
      - `to` equal to the source pin: nothing changes.
      - Any rewired wire refused by `doc::connection_error` (type, attributes or cycle): nothing changes, and the
        warning names the first failing wire. The wires are checked one by one against the graph without the old
        wires, each accepted wire added before the next is checked.
      - Otherwise the move is one edit, so one undo step.
    - Dropping on empty canvas with Ctrl held does nothing, and no palette opens. *Superseded by 0034* for an
      unconnected pin: a Ctrl+drag from it is a normal new wire, and its drop on empty canvas opens the palette.
      Lifted wires dropped on empty canvas go back, with no palette.
    - Dropping on a pin of the other direction is not part of this gesture. bevy_noodle connects a compatible pin as
      in a plain drag, and an incompatible one reports why.
  - **Hold a key + LMB on empty canvas** drops a node, now with UE's D, O, E, N, P and I as well. `NODE_HOTKEYS`
    lists only kinds that exist; UE's B (BumpOffset) has no stdlib node and is skipped. The `hotkey_nodes_exist` test
    resolves every entry in the registry. Like every plain key, the held key places nothing while Ctrl, Alt or Super
    is held (`hotkey_node` checks `no_command_mods`; Shift is allowed), so Ctrl+D held + a click is not a Divide.
  - Ctrl+W stays "close tab"; UE's Ctrl+W (duplicate) is not adopted, and Ctrl+D duplicates.

**Before → after.** "Ctrl" = Ctrl or Super throughout, before and after.

Global (`MainCtx`, every tab):

| Keys | Before | After |
|---|---|---|
| Ctrl+S / Ctrl+Shift+S | save / save all | same |
| Ctrl+Z / Ctrl+Y, Ctrl+Shift+Z | undo / redo (the active tab's history) | same |
| Ctrl+W | close tab | same (UE's duplicate not adopted) |
| Ctrl+Tab | next tab | same; Tab pressed again while Ctrl stays held goes on cycling |
| Ctrl+O / Ctrl+Q | open project / quit | same |
| Any key while typing (text field focused, palette or dialog open) | ignored (`not_typing`) | ignored (keyboard gate, all contexts); a key still held when typing ends does nothing until released |
| A key held while switching tabs | did not fire again (keys acted on `just_pressed`) | same: ignored until released (re-armed bindings, see above) |

Graph (`GraphCtx`, graph tabs only):

| Keys / gesture | Before | After |
|---|---|---|
| Ctrl+C / X / V / D / A | pushed on every tab | graph only; Ctrl+V pastes at the cursor |
| Ctrl+L | auto-layout, pushed on every tab | graph only |
| Delete, Backspace | delete selection, every tab | graph and curve only |
| F / Home | fit selection / all, every tab | graph only |
| C | comment frame, every tab | graph only |
| Tab | palette at the cursor, if over the canvas | same, graph only (*superseded by 0034:* the palette is on Space; Tab toggles the right sidebar on every tab; *superseded by 0035:* Tab is Bevy's focus navigation) |
| Shift + `=` `-` `8` `/`, numpad `+ - * /` | convert the selected nodes | same |
| V | exclusive preview on release | same |
| Alt + a plain key (Alt+F) | fired the plain key | nothing |
| Ctrl + an unbound key (Ctrl+F) | nothing | nothing |
| Hold key + LMB on empty canvas | `1`–`4` `S` `V` `T` `M` `A` `L` `U` `R` | the same, plus `D` Divide, `O` OneMinus, `E` Power, `N` Normalize, `P` Panner, `I` If; nothing while Ctrl / Alt / Super is held |
| LMB click on a pin with Ctrl | remove all its wires | nothing |
| LMB click on a pin with Alt | nothing | remove all its wires |
| LMB click on a wire with Ctrl | remove it (and toggle its selection) | toggle its selection only |
| LMB click on a wire with Alt | select it | remove it |
| RMB click on a wire | remove it | same |
| Double click on a wire | reroute point | same |
| LMB drag from a connected input | picks up and moves its wire (bevy_noodle `detach_wires`) | starts a new wire; connecting replaces the old one |
| Ctrl+LMB drag from a pin onto a pin of the same direction | as a plain drag | moves the wire (input) / all wires (output) there, all or nothing, one undo step (*0034:* the wires follow the cursor; Esc cancels) |
| Wire dropped on empty canvas | palette filtered by type | same; with Ctrl held: nothing (*superseded by 0034:* lifted wires go back; a new wire from an unconnected pin opens the palette) |
| RMB click on a pin / a node / the canvas | pin menu / node menu / palette | same |

Curve (`CurveCtx`, curve tabs only):

| Keys | Before | After |
|---|---|---|
| Ctrl+A / D / C / X / V | through the global shortcuts | curve context (same actions) |
| Delete, Backspace | through the global shortcuts | curve context |
| Arrows (Shift: ×10) | `nudge_keys` system in `curve_view.rs`, with a selection | curve context `Nudge`, with a selection |
| F / Home / C / Tab, Ctrl+L | pushed, no effect | not bound |
| Mouse (double click, drag, X / Y / Shift held, Shift+click, RMB menus) | unchanged | unchanged (modifiers via `mods()`) |

Text (`TextCtx`, generated-file tabs): Ctrl+C copies the file, as before. Shift+wheel scrolls horizontally, as before
(through `mods()`). Graph-only keys no longer reach the tab.

Consts / image / texture: no kind context, so only the global keys work. Enter in the consts add form and in the
texture path field stays a widget observer, unchanged. Graph and curve keys pressed there used to be pushed and
ignored; now they are not bound at all.

**Consequences.**
- Hotkeys need no "is this my tab" guards: the active context decides.
- `ctrl_held`, `pressed_hotkey`, `not_typing`, `shortcuts` and `nudge_keys` are gone.
- Tests:
  - `tests/input_contexts.rs` drives `KeyboardInput` messages through the real input path: Ctrl+S without S, unbound
    combinations, typing, held keys across tab switches, curve nudge, node hotkeys.
  - `state::tests` covers `MoveWires` (moves, one undo step, rejected moves leave the graph untouched) and connect
    replacing an input's wire.
  - `ui::input::tests::hotkey_nodes_exist`.
- Scripted GUI check: `SHADER_EDITOR_INPUT=ctrl_drag_wire|wire_drag|alt_click` with `SHADER_EDITOR_INPUT_PIN` /
  `SHADER_EDITOR_INPUT_TO` (`demo.rs`). It logs the wires at both pins before and after.
