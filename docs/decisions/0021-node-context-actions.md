# 0021. Node context actions: menu rows, hotkeys, "Convert to"

**Context.** QA feedback asks for a node context menu (RMB on a node) and for turning Add / Subtract / Multiply /
Divide into each other without rewiring (spec §1–§2 of the QA design). Node-specific items must be described where
the node is defined, but `shader_graph_core` and `shader_stdlib` must stay free of Bevy.

**Decision.**
- **Actions are data on `NodeDef`.** `NodeDef::context_actions()` (default empty) returns `NodeAction { label, hotkey:
  Option<Hotkey>, op: NodeOp }`. `NodeOp` is a small closed enum the editor knows how to apply (`ConvertTo(kind)` for
  now — *superseded by 0026:* `NodeOp::Convert(Conversion { kind, params, unique_name, pins })`, used by Convert to
  Parameter); `Hotkey { shift, key: HotkeyKey }` names a logical key (`Plus`, `Minus`, `Star`, `Slash`, `Letter(c)`),
  and `Hotkey::label()` gives the menu text (`Shift +`, `V`). Stdlib nodes attach actions with the builder's
  `.action(..)`. The four arithmetic nodes offer "Convert to" the other three with `Shift +`, `Shift -`, `Shift *`,
  `Shift /`; converting to the node's own kind is not offered.
- **Menu rows** (`Editor::node_menu`) reuse the `open_icon_menu` convention: `"label\tHotkey"` shows the hotkey
  right-aligned, `""` is a separator. Order: the node's own actions, a separator (only when there are some), then
  the generic selection actions `Copy\tCtrl+C`, `Duplicate\tCtrl+D`, `Delete\tDel` (the existing `Action::Copy`,
  `Duplicate`, `DeleteSelection`). A node of an unknown kind gets only the generic rows.
- **RMB on a node body** opens the menu only for a click (`RightPress::is_click`; a moving RMB press pans) and not
  when the click started on a pin (pins keep their own menu, decision 0012). A node that is not selected is
  selected first, so the generic rows act on it; RMB on empty canvas still opens the palette.
- **Selection semantics.** A convert from the menu of a selected node acts on every selected node offering that
  same op, from the menu of an unselected node on that node alone. A hotkey acts on the selection with no menu
  open: selected nodes are grouped by the target kind of their action bound to that key, one
  `Action::ConvertNodes` per kind (`Editor::hotkey_actions`). The preview key `V` (decision 0022) acts only when
  exactly one node is selected.
- **Convert** (`Action::ConvertNodes { nodes, kind }`) changes `kind` in place in one `Doc::edit` (one undo step):
  id, position, params, inline input values and wires stay; wires and inline values of pins that the node's new
  signature lacks are dropped. For the four arithmetic nodes (pins `a`, `b`, `out`) nothing is dropped.
  *Update (decision 0026):* the action is `Action::ConvertNodes { nodes, conv: Conversion }`; a `Conversion` with
  an empty param list behaves as described here, a non-empty one copies only the listed params (renamed), can
  generate a unique parameter name and rename output pins.
- **Key mapping** (`ui::input::pressed_hotkey`): `Shift`+`Equal` / `Minus` / `Digit8` / `Slash` (the main row, as
  on a US layout) and `NumpadAdd` / `NumpadSubtract` / `NumpadMultiply` / `NumpadDivide` with or without Shift all
  map to the `Shift` hotkeys. Like the other shortcuts they are ignored while a text field has the focus or a
  dialog / the palette is open (`not_typing`). bevy_noodle's keyboard zoom (`Equal` / `Minus`) is not enabled in
  the editor, so there is no clash.
  *Update (decision 0032):* the mapping is the graph editor's input context (`editors::graph`, actions
  `ConvertPlus` … `ConvertSlash`, `PreviewNode`); the keys are hidden from it while typing by
  `editors::main_input::gate_keyboard`, and Ctrl / Alt / Super held blocks them.
- **`V` toggles on release.** Holding `V` + LMB on empty canvas already drops a Vector parameter (`hotkey_node`), so
  `V` cannot act on press: `ui::input::PreviewKey` remembers the press, `canvas_click` marks it when it drops a node
  while `V` is held, and the release toggles only when nothing was placed meanwhile.

**Consequences.**
- A new node action is added in the node's definition (plus a `NodeOp` variant if the editor needs to learn a new
  operation); the menu and hotkey dispatch pick it up without editor changes for existing ops.
- The physical keys behind `HotkeyKey` follow the US layout for the main row; numpad keys work on any layout.
- Converting a node to a kind with other pins silently drops the affected wires and values (undo restores them).

## Constant labels

`stdlib::Constant`, `Constant2Vector`, `Constant3Vector` and `Constant4Vector` nodes may carry an optional
`label` (`Node::label`, omitted from the RON when empty). It is editor-only: the node title shows the label instead of
the value when it is non-empty, and the Properties panel has a "Label" row (`PropTarget::NodeLabel`, merged edits).
The IR and both generators never read it, so labelling a node does not change generated files (covered by the
`label_does_not_change_codegen` pipeline test). `stdlib::Global*` nodes are unaffected: they already show the
constant's name.
