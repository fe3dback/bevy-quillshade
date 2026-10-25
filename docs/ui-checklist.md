# Manual UI checklist (spec §15, §17)

Automatically covered by `crates/shader_editor_app/tests/ui_smoke.rs` (creating a node, connecting
through bevy_noodle, rejecting an incompatible connection, undo, save, move) and
`state::tests` (creating/renaming/moving/deleting assets, auto-connect from the palette,
the prompt on close). Before a release, walk through by hand on `examples/sample_game`:

- [+] Without arguments the last project opens; a directory without `Cargo.toml`/`assets/` — a message with the reason.
- [~] Browser: `+Pbr`, `+Unlit`, `+2D`, `+Fn`, `+Folder` in the selected folder; RMB → Rename / Move / Delete (with confirmation, autogen is deleted); dragging an asset onto a folder moves it; "Show generated WESL/Rust" opens the file.
- [ ] Icons: browser rows show caret + folder (open/closed) and the asset kind icon (fx / lit sphere / grey sphere / image); tabs, the toolbar, the `+…` buttons and the browser RMB menus show icons; Save / Save all / Undo / Redo are icon-only, hovering a toolbar button shows a tooltip with its hotkey after ~0.5 s (hidden on click); the main view shows a faint `pbr3d`/`unlit3d`/`sprite2d`/`function` watermark bottom-left, above the graph; icons stay sharp at a 125–200 % display scale.
- [+] Tabs: several assets, `*` on unsaved ones, Ctrl+W / × / MMB closes, a dirty tab asks Save/Discard/Cancel.
- [~] Canvas: pan (RMB/MMB drag; releasing RMB after a pan opens no palette and deletes no wire), zoom (wheel), box select, node drag (Ctrl+Z reverts it in one step), compatible-port highlighting while dragging a wire, a wire onto an incompatible port → a message with E002/E003 in the log.
- [+] Palette: Tab and RMB on the canvas; search `*`, `+`, `lerp`, `texture sample`; arrows + Enter; a wire released into empty space → palette with compatible nodes only and auto-connect.
- [+] Hotkeys: `1`/`2`/`3`/`4`/`S`/`T`/`M`/`A`/`L` + LMB, Ctrl+C/V/X/D, Delete, F, Home, C (frame), Ctrl+L (auto-layout).
- [+] Reroute: double click on a wire; RMB on a wire deletes it.
- [ ] Named route: "Named Route" from the palette, then "Named Route Usage: {name}"; the usage takes the declaration's type; Route select in the properties; renaming the declaration renames the usages; a loop through a route is refused (E002).
- [ ] Inline editors: Lerp Alpha slider drags without moving the node and is one Ctrl+Z; a number field takes typed text and Delete/Ctrl+Z go to the field, not the graph; the color button opens the picker and the node's R/G/B fields follow it live; a wire hides the pin's editor; the node's selected Properties show the new value after release.
- [ ] Pin menu: RMB on Lerp A → Float3 turns A/B/out into Float3 (color editors), back to Float; Reset to default; Disconnect on a wired pin.
- [ ] Known: at zoom ≠ 1 a slider moves faster/slower than the cursor and the color picker opens off its button (Bevy 0.20, decision 0012).
- [+] Comment frame: dragging the frame moves the nodes inside; the title is edited in the properties.
- [+] Minimap: click/drag moves the view.
- [+] Properties: node parameters (numbers, vectors, color, bool, enum, pin type, text, Custom code), input default values, asset blend mode / two-sided / shading; changes show on the nodes immediately.
- [+] Log: errors/warnings appear live, nodes are outlined; clicking a line opens the asset and focuses the node.
- [+] Ctrl+S: save and regenerate; with `examples/sample_game` running the picture updates without a restart.
- [ ] Changing a graph file externally (git checkout): a clean tab reloads, a dirty one asks Reload / Keep mine.
- [~] Closing the window with unsaved changes — a prompt; window size and tabs are restored on the next launch.
