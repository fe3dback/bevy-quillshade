# Manual UI checklist (spec §15, §17)

Automatically covered by `crates/shader_editor_app/tests/ui_smoke.rs` (creating a node, connecting
through bevy_noodle, rejecting an incompatible connection, undo, save, move) and
`state::tests` (creating/renaming/moving/deleting assets, auto-connect from the palette,
the prompt on close). Before a release, walk through by hand on `examples/sample_game`:

- [ ] Without arguments the last project opens; a directory without `Cargo.toml`/`assets/` — a message with the reason.
- [ ] Browser: `+Pbr`, `+Unlit`, `+2D`, `+Fn`, `+Folder` in the selected folder; RMB → Rename / Move / Delete (with confirmation, autogen is deleted); dragging an asset onto a folder moves it; "Show generated WESL/Rust" opens the file.
- [ ] Tabs: several assets, `*` on unsaved ones, Ctrl+W / × / MMB closes, a dirty tab asks Save/Discard/Cancel.
- [ ] Canvas: pan (MMB), zoom (wheel), box select, node drag (Ctrl+Z reverts it in one step), compatible-port highlighting while dragging a wire, a wire onto an incompatible port → a message with E002/E003 in the log.
- [ ] Palette: Tab and RMB on the canvas; search `*`, `+`, `lerp`, `texture sample`; arrows + Enter; a wire released into empty space → palette with compatible nodes only and auto-connect.
- [ ] Hotkeys: `1`/`2`/`3`/`4`/`S`/`T`/`M`/`A`/`L` + LMB, Ctrl+C/V/X/D, Delete, F, Home, C (frame), Ctrl+L (auto-layout).
- [ ] Reroute: double click on a wire; RMB on a wire deletes it.
- [ ] Comment frame: dragging the frame moves the nodes inside; the title is edited in the properties.
- [ ] Minimap: click/drag moves the view.
- [ ] Properties: node parameters (numbers, vectors, color, bool, enum, pin type, text, Custom code), input default values, asset blend mode / two-sided / shading; changes show on the nodes immediately.
- [ ] Log: errors/warnings appear live, nodes are outlined; clicking a line opens the asset and focuses the node.
- [ ] Ctrl+S: save and regenerate; with `examples/sample_game` running the picture updates without a restart.
- [ ] Changing a graph file externally (git checkout): a clean tab reloads, a dirty one asks Reload / Keep mine.
- [ ] Closing the window with unsaved changes — a prompt; window size and tabs are restored on the next launch.
