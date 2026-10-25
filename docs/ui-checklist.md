# Manual UI checklist (spec §15, §17)

Automatically covered by `crates/shader_editor_app/tests/ui_smoke.rs` (creating a node, connecting
through bevy_noodle, rejecting an incompatible connection, undo, save, move) and
`state::tests` (creating/renaming/moving/deleting assets, auto-connect from the palette,
the prompt on close, curve tab edits). `SHADER_EDITOR_INPUT=curve_drag|curve_add` with
`SHADER_EDITOR_OPEN=common/pulse.curve.ron` plays a point drag / double click on a curve. Before a release, walk through by hand on `examples/sample_game`:

- [+] Without arguments the last project opens; a directory without `Cargo.toml`/`assets/` — a message with the reason.
- [ ] Main menu (decision 0016): File opens Open Project / Close Project (asks when dirty; the tabs come back on reopen) / Save / Save All / Quit with right-aligned hotkeys; Assets creates every kind and regenerates; the toolbar after Undo/Redo follows the active tab (graph / curve / generated file).
- [~] Browser: double click opens an asset (a single click does not); only the active tab's asset is highlighted, folders never; unsaved assets show an orange dot; RMB on a folder or the empty space below the tree → New Pbr/Unlit/2D/Fn/Curve/Folder in that folder; RMB → Rename / Move / Delete (with confirmation, autogen is deleted); dragging an asset onto a folder moves it; "Show generated WESL/Rust" opens a read-only tab (code icon, `wesl`/`rust` watermark, highlighted, line numbers; wheel / Shift+wheel scroll; Ctrl+C and the Copy button copy the file; "Open in the default application" works; the tab refreshes after Ctrl+S of its asset; switching back to the graph tab keeps its camera).
- [ ] Icons: browser rows show caret + folder (open/closed) and the asset kind icon (fx / lit sphere / grey sphere / image); tabs, the toolbar and the menus show icons with an orange accent; Undo / Redo are icon-only, hovering a toolbar button shows a tooltip with its hotkey after ~0.5 s (hidden on click); the main view shows a faint `pbr3d`/`unlit3d`/`sprite2d`/`function` watermark bottom-right, above the graph (the minimap is bottom-left); icons stay sharp at a 125–200 % display scale.
- [+] Tabs: several assets, `*` on unsaved ones, Ctrl+W / × / MMB closes, a dirty tab asks Save/Discard/Cancel; RMB → Close / All / Others / Left / Right / Unmodified (one question for all dirty ones); generated-file tabs are restored on the next launch too.
- [~] Canvas: pan (RMB/MMB drag; releasing RMB after a pan opens no palette and deletes no wire), zoom (wheel), box select, node drag (Ctrl+Z reverts it in one step), compatible-port highlighting while dragging a wire, a wire onto an incompatible port → a message with E002/E003 in the log.
- [+] Palette: Tab and RMB on the canvas; search `*`, `+`, `lerp`, `texture sample`; arrows + Enter; a wire released into empty space → palette with compatible nodes only and auto-connect.
- [+] Hotkeys: `1`/`2`/`3`/`4`/`S`/`T`/`M`/`A`/`L` + LMB, Ctrl+C/V/X/D, Delete, F, Home, C (frame), Ctrl+L (auto-layout).
- [+] Reroute: double click on a wire; RMB on a wire deletes it. Ctrl+LMB on a wire deletes it, on a pin deletes all its wires (one Ctrl+Z).
- [ ] Named route: "Named Route" from the palette, then "Named Route Usage: {name}"; the usage takes the declaration's type; Route select in the properties; renaming the declaration renames the usages; a loop through a route is refused (E002).
- [ ] Inline editors: Lerp Alpha slider drags without moving the node and is one Ctrl+Z; a number field takes typed text and Delete/Ctrl+Z go to the field, not the graph; Float3/Float4/Color show only the color button, it opens the picker and Properties' R/G/B fields follow it after release; a wire hides the pin's editor; the node's selected Properties show the new value after release.
- [ ] Pin menu: RMB on Lerp A → Float3 turns A/B/out into Float3 (color editors), back to Float; Reset to default; Disconnect on a wired pin.
- [ ] Known: at zoom ≠ 1 a slider moves faster/slower than the cursor and the color picker opens off its button (Bevy 0.20, decision 0012).
- [+] Comment frame: dragging the frame moves the nodes inside; the title is edited in the properties.
- [+] Minimap: click/drag moves the view.
- [+] Properties: node parameters (numbers, vectors, color, bool, enum, pin type, text, Custom code), input default values, asset blend mode / two-sided / shading; changes show on the nodes immediately.
- [+] Log: errors/warnings appear live, nodes are outlined; clicking a line opens the asset and focuses the node.
- [+] Ctrl+S: save and regenerate; with `examples/sample_game` running the picture updates without a restart.
- [ ] Changing a graph file externally (git checkout): a clean tab reloads, a dirty one asks Reload / Keep mine.
- [ ] Curves (decision 0015): `+Curve` creates a ramp and opens it (spline icon, `curve` watermark); double click adds a point, drag moves the selection (one Ctrl+Z, points may pass each other; X / Y held lock the axis, Shift held snaps to 0.1), Shift+click toggles, Ctrl+A / Delete / Ctrl+C/V/D / arrows (Shift ×10) work, RMB on a point / empty space opens its menu; the header switches the preview between Catmull-Rom and Simple without marking the tab dirty; Properties edit the selected point's T / Value and list the hotkeys one per line; Ctrl+S regenerates `autogen/curves/…` and the running sample game updates `curve_bands`; a clean curve tab reloads on an external change.
- [ ] Sample Curve node: the Curve select lists the project's curves, "Open curve" opens it, the title shows the curve name; the node's mini plot follows the type, the inline T slider (a vertical line, gone when T is wired) and unsaved edits in the curve tab, and shows "n/a" without a curve; renaming the curve updates the node; deleting it gives E006 on the node.
- [~] Closing the window with unsaved changes — a prompt; window size and tabs are restored on the next launch.
