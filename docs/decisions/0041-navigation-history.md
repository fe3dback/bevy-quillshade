# 0041. Navigation history: Back / Forward through tabs and graph camera views

**Context.** Big projects mean jumping between materials, functions, curves and generated files, and around large
graphs. The user wants browser-style **Back / Forward**: which tab was open and, on a graph tab, where the camera
looked. This includes the side buttons of their mouse, which they already use that way in browsers. It is pure UX and
read-only: it never changes files, assets, documents, undo / redo or dirty flags. The design is
`docs/superpowers/specs/2026-10-28-navigation-history-design.md`.

Non-goals: no camera history for the curve, image / texture preview or gradients views (their entries are "this tab"
only); no restored selection, text scroll position or sidebar state; no history list UI; a reopened tab goes where a new
tab goes (the end of the tabs bar), not to its old position.

**Decision.**

- **One flat list with a cursor** (`state/nav.rs`, `NavHistory`), like a browser, not a stack of tabs plus a camera
  stack per tab. An entry is `NavEntry { tab: TabKey, view: Option<NavView> }`: the stable tab identity (the key the
  user config stores open tabs under; tab indices shift) and, for a graph tab once its camera is known, its view.
  A new entry after Back truncates the forward part. At most `NAV_CAP` (100) entries; the oldest go first.
- **Amend vs push** (the VS Code model). A new entry is pushed only on a jump:
  - the active tab changes, by any path. `Editor::follow_active` runs at the end of every `Editor::apply` and compares
    `Editor::active_key()` with the current entry (`NavHistory::visit`, a no-op on the same tab). Hooking `activate`
    alone would miss closing the active tab (`remove_tabs` sets `active` directly), and the project restore activates
    every tab: `open_project` clears the history and records only the restored active tab;
  - the graph camera makes a large jump: the centre moved by more than `NAV_JUMP_SCREEN` (0.5) of the canvas's smaller
    side in screen px (graph distance × zoom), or the zoom changed by a factor ≥ `NAV_JUMP_ZOOM` (2). Fit and
    FocusDiagnostic get no flag of their own: a small Fit just amends.

  Otherwise a settled view amends the current entry (or sets its missing view). Switching away from a graph amends the
  entry the cursor last left (`amend_last_of`, tracked by `visit` / `step`) with the outgoing camera, settled or not, so
  Back returns there; "the nearest entry at or before the cursor" would pick the wrong entry after a Back (an A B A B
  history, where the outgoing tab sits after the cursor).
- **Centre + zoom, not the raw pan.** `NavView` is the graph-space point at the middle of the visible canvas and the
  zoom. `bevy_noodle::CanvasView::pan` is relative to the canvas's top-left corner, and `keep_center_on_resize` shifts
  it on every canvas resize (window resize, Ctrl+1 / Ctrl+3, Zen mode). With the centre a resize is not a camera
  change, and an entry recorded before a layout change still comes back centred.
- **App-side polling, not a fork event.** Many paths move the camera (the fork's pan / zoom gestures, MMB pan, minimap,
  Fit, the per-document restore on a tab switch) and none reports "settled". `canvas::report_settled_view` watches the
  shown graph's view and calls `NavHistory::note_view` once it has rested for `NAV_SETTLE_SECS` (0.3 s) with no
  gesture running and no camera request pending. It needs no bevy_noodle change and catches future paths too. It
  compares with the **current entry's view**, not with the last view it reported: after visiting another tab and
  coming back, the new `{tab, None}` entry must still get the restored view, which equals the last report. History
  writes bypass `Editor` change detection: nothing redraws or validates on them.
- **Back / Forward** (`Editor::navigate`, `NavHistory::step`) walk from the cursor to the next entry that opens:
  - an entry whose tab is open is just activated: an open tab is **never gone**, even if its file vanished on disk
    (the watcher never closes tabs, so the tab is still there, maybe with unsaved edits);
  - an entry of a closed tab whose target is gone (the same checks as restoring open tabs: asset on disk, game file,
    internal texture) is **skipped and dropped**, so Back never gets stuck on a deleted file;
  - an entry whose target exists but does not open (an asset that fails to load, a generated file not written yet) is
    **skipped and kept**: it may be fixed later. The load error goes to the console only: the error dialog
    `open_asset` raises is dismissed during the walk, otherwise it would stay up over the entry the walk reached and
    swallow the next Back (`navigate` is ignored while a dialog is open);
  - Back while no tab is active, or while the active tab is not the current entry's, first tries the **current entry
    itself**: the last tab closed keeps the cursor on its entry, so "oops, closed it" + Back reopens it;
  - the reached entry's view replaces the camera through `ViewRequest::Set { centre, zoom }`, which clears the queue
    first (a reopened graph tab queued a FitAll) and runs after `rebuild_canvas`'s per-document restore;
  - nothing in that direction: nothing happens and nothing is logged (a line per press would pile up with a mouse
    side button; the View menu items already show it dimmed). Navigation itself records nothing.
- **Housekeeping.** Renaming an asset rewrites the keys in the history (`NavHistory::rename`, `Generated` keys
  included); deleting one does nothing eagerly (its entries are dropped when reached). Opening or closing a project
  clears the history.
- **In memory only.** The history is never saved, not even in `config.ron`. A tab Back / Forward opens is saved as open
  by the usual `save_tabs_to_config`, as if opened from the tree.
- **Controls** (`MainCtx`, so they work on every editor kind and without a tab): `NavBack` / `NavForward` on **Ctrl+[ /
  Ctrl+]** (`ctrl()`: Super = Cmd on macOS; physical `BracketLeft` / `BracketRight`, so the Russian layout's `х` / `ъ`
  keys work too), the mouse side buttons `MouseButton::Back` / `Forward` (bevy_picking knows only Primary / Secondary /
  Middle, so no clash with canvas gestures) and `KeyCode::BrowserBack` / `BrowserForward`, which some mice and drivers
  send instead. `gate_keyboard` turns off only the keyboard source, so the handler `on_nav` itself drops the action
  while `KeyboardGate::typing` (dialog, palette, menu, popup, text field) or while the left, right or middle button is
  held (a gesture). The View menu lists `Back\tCtrl+[` / `Forward\tCtrl+]` between Zen mode and the panel toggles,
  dimmed with the reason when `NavHistory::can_back` / `can_forward` say there is nothing in that direction (validity
  is not checked); `fill_main_menus` keys on both, so the items re-enable as the history changes. The menu_bar has
  icon-only Back / Forward buttons (`IconId::Back` / `Forward`, icon-rs chevrons; `NavButton`) in place of the former
  Undo / Redo ones (user request; undo / redo stay in the Edit menu and on Ctrl+Z / Ctrl+Shift+Z): `sync_nav_buttons`
  dims them at the same ends (`InteractionDisabled` + the icon's alpha, written only on change). No editor topbar
  buttons.
  - Ctrl+[ / ] were the user's choice over Alt+Left / Alt+Right.
  - Ctrl+Alt+Left / Right were rejected: desktop environments use them to switch workspaces, so the editor would never
    see them.

**Consequences.**

- The camera history is a polled approximation: a view that never rests for 0.3 s (continuous panning into a tab
  switch) is still recorded through `amend_last_of`, but intermediate stops shorter than that are not.
- The thresholds (0.5 of the smaller side, zoom × 2, 0.3 s, 100 entries) are starting values, tuned by feel.
- The mouse side buttons and the Browser keys cannot be proven by the headless tests or the winit scripts on real
  hardware: `docs/ui-checklist.md` has a manual item (X11 and Wayland, Russian layout).
