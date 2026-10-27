# 0016. Main menu, per-view toolbar, browser and tab interactions

**Context.** The toolbar mixed project commands (Open, Save, Save all, Regenerate) with graph commands (Auto-layout,
Comment, Fit) that do nothing on a curve or text tab; the browser had a row of `+Pbr`/`+Unlit`/`+2D`/`+Fn`/`+Curve`/
`+Folder` buttons, opened assets on a single click and highlighted both the current folder and the active asset;
tabs could only be closed one by one; inline color editors on nodes repeated the R/G/B/A fields of Properties next
to the picker; the accent colour was blue and defined twice (the tint of line icons' accent layer, icon-rs's
`Palette::DARK` for solid icons).

**Decision.**
- Toolbar = main menu + undo/redo + the active view's actions. `ui::MainMenu` buttons (`File`, `Assets`) open an
  `open_icon_menu` popup below themselves: File — Open Project, Close Project, Save, Save All, Quit; Assets — the
  browser's "New …" items (`browser::create_items`) and Regenerate. Menu labels may carry a hotkey after `\t`
  (right-aligned, dim); an empty label is a separator (`dialog::menu_separator`). After a separator the
  `ui::ViewTools` container is rebuilt per active tab (`view_tools`): graph — Auto-layout / Comment / Fit; curve —
  Select all / Delete; generated file — Copy / Source / Open externally.
- `Action::CloseProject(Unsaved)` closes the project (asks when tabs are dirty: Close without saving / Save all and
  close); its tabs are stored in the user config first, so they come back when it is opened again.
- Browser: create only through RMB menus (folder rows, and the empty space below the tree = root folder) or the
  Assets menu. Double click opens an asset; only the active tab's asset is highlighted (folders never are; the
  "current folder" for new assets is still set by clicking/RMB on a folder, the dialog shows the prefilled path).
  Unsaved assets get a small accent dot after the name instead of ` *`.
- Tab RMB menu: Close / Close All / Close Others / Close to the Left / Close to the Right / Close Unmodified
  (`Action::CloseTabs { which, at, unsaved }`). Dirty tabs among those being closed raise one dialog (Cancel /
  Discard / Save); the active tab stays active, or its nearest kept right neighbour takes over.
- Ctrl+LMB on a pin removes every wire at it (`Action::DisconnectPin`, outputs included); on a wire, removes the
  wire. bevy_noodle starts wires only on drag start, so a click on a port with Ctrl does not begin a connection.
  *Superseded by 0032:* Alt+LMB (UE) instead of Ctrl+LMB; Ctrl+drag from a pin now moves its wires.
- Inline Float3/Float4/Color editors on nodes are the color picker alone; exact component values are edited in
  Properties (supersedes that part of decision 0012).
- Curve view drags (supersedes the Shift/Ctrl toggle of decision 0015): X / Y held lock the movement to that axis
  (Blender-like), Shift held snaps the grabbed point to the 0.1 grid (the whole selection moves by the same delta;
  coordinates that land on the grid are set exactly, avoiding `0.39999998`). Shift+click toggles the selection.
  Ctrl is not used, so Ctrl+X / Ctrl+Y (cut / redo) never fire during a drag. Preview mode and "Show generated
  WESL" live only in the view header; Properties show the selected point and the hotkeys, one per line.
- One accent colour: `ui::ACCENT_RGB` / `ui::ACCENT` (orange `#fb923c`). The line icons' accent layer is tinted with
  it and solid icons are recoloured with `Palette { accent, ..Palette::DARK }`. SVG sources keep icon-rs's
  `#2563eb` accent slot (it is replaced at render time).

**Consequences.** Commands that used to be one click away (Save, Regenerate) are now in menus — their hotkeys stay.
The active-row highlight (`ROW_ACTIVE`) remains blue: it marks selection, not the accent. The `menu` demo hook shows
the File menu, `tab_menu` the tab menu.
