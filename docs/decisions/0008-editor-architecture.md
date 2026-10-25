# 0008. Editor architecture

**Decision.**
- **The model is the source of truth** (`shader_graph_core::Asset` inside `doc::Doc`). The bevy_noodle canvas is rebuilt from the model when the document revision changes (only when no gesture is active: no wire drag and no mouse button held). User gestures arrive as `EditApplied` with `EditOrigin::Interaction` and are turned into an `Action`; edits the canvas makes itself during a rebuild are ignored.
- **Undo/redo/copy/paste are model-level**, not bevy_noodle scene snapshots: the history stores copies of `Asset` (limit of 200 steps), consecutive edits of the same field (typing text, dragging a number) are merged into one step, and creating a node from the palette together with its auto-connection is one step as well.
- **Connection checks** — a custom `ConnectionCheck`: bevy_noodle ports have `PortType::ANY`, and compatibility is decided by the rules of spec §7.2 (`doc::connection_error`, cycles E002, types E003). That is why compatible-port highlighting during a wire drag accounts for polymorphic nodes.
- Type conversions are not inserted as separate nodes: an allowed conversion (broadcast, truncation, Color↔Float4…) is performed by codegen directly on the edge; truncation produces W104.
- Node movement is committed as one step without rebuilding the canvas. Comment frames carry the nodes lying inside them in the same gesture.
- The UI is entirely Bevy UI + Feathers (`bsn!`), no egui. Workarounds for known Feathers 0.20 issues are copied from the bevy_noodle examples (`fixes.rs`). Vendoring bevy_noodle was not needed.
- File paths are typed in as text (plus a list of recent projects): a native folder picker (rfd) is not used, to avoid pulling in portal/zbus dependencies.

**Consequences.** The editor logic (`state::Editor`) is tested without a window; the canvas smoke tests drive the real bevy_noodle without rendering (`crates/shader_editor_app/tests/ui_smoke.rs`).
