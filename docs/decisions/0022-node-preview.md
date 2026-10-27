# 0022. Node preview: storage, `ShaderOutputDto`, IR

**Context.** QA feedback asks for an exclusive node preview like UE's "Start Previewing Node" (spec §5 of the QA
design): the output of one node is shown on the material instead of what the MaterialOutput node computes, in the
running game through WESL hot reload. This entry covers the model, the IR and the WESL templates per mode; the
editor part is in the "Editor" section at the end.

**Decision.**
- **Storage on `Asset`.** `Asset::preview: Preview { node: Option<NodeId>, mode: PreviewMode }`, serialized after
  `meta` only when it is not the default (`node: None`, `mode: Unlit`), so existing files and `FORMAT_VERSION` 1 are
  unchanged. It lives on the asset (not on `MaterialKind` or `Meta`) because it names a node, like edges do. The CLI
  `regen` / `check` read it like the editor, so they produce what the editor showed. Function assets ignore it
  silently.
- **Dangling ids.** `Asset::normalize` (run on load and save) keeps `preview.node` even when the node is missing, so a
  dangling id read from a file reaches `build_ir` and is reported (W107 "preview ignored: node `…` does not exist",
  spec §5). Editor edits call `Asset::clear_dangling_preview` after `normalize` (`Doc::edit_merge`): deleting the
  previewed node turns the preview off (the mode stays), and undo brings both back.
- **Modes.** `PreviewMode::Unlit` (default; masks are the most common case), `Lit`, `Full`; RON names `Unlit` /
  `Lit` / `Full`, labels `unlit` / `lit` / `full` in messages and the UI.
- **`ShaderOutputDto`** (`shader_graph_core::preview`) maps a `PreviewSlot` (Albedo, Metallic, Specular, Roughness,
  Emissive, Opacity, Normal, AmbientOcclusion) to an output pin id of the previewed node. `NodeDef::preview_output`
  builds it from the node's *resolved* signature (types of polymorphic pins follow the connected inputs); the default
  `ShaderOutputDto::main_output` puts the first output into Albedo when `converts_to_albedo(ty)`, otherwise the DTO
  is empty and the node is not previewable. Nodes may override it to fill more slots; the PBR struct pin maps
  1:1 onto the slots without changing the trait (decision 0023: a slot names `PreviewSource { pin, field }`). World Position Offset and UV slots are intentionally not
  `PreviewSlot`s: a preview only replaces fragment outputs (the vertex stage keeps existing with a zero offset). `PreviewSlot::output_pin(kind)` names the material output pin of a
  slot: Pbr3d has all of them (`base_color` for Albedo), Unlit3d `color` / `opacity`, Sprite2d only `color`.
- **Albedo conversion** (`preview::albedo_expr`, spec §5): Float → `(v, v, v)`, Float2 → `vec3<f32>(xy, 0.0)`,
  Float3 as is, Float4/Color → `.rgb`, Int → float; Bool and Texture2D have none. The Float2 rule is preview-only:
  the edge conversion table (`types::conversion`) still rejects Float2 → Float3.
- **IR (`build_ir`).** If `preview.node` is set, a missing node ("does not exist"), a node of an unknown kind
  ("cannot be resolved") or the MaterialOutput node ("the material output node cannot be previewed") gives `W107`
  "preview ignored: …" and no preview. Otherwise the previewed node and its upstream are *added* to the Fragment
  live set: the normal live sets stay, so every parameter, texture and the vertex stage are still collected and the generated Rust
  (types, `Params` layout, bindings, vertex shader) does not change when preview is toggled — only `.wesl` changes
  and the game hot-reloads it. Being live, the previewed upstream gets no W101. After emission the DTO is built from
  the node's resolved signature; if it yields no albedo (e.g. the node outputs Bool or a texture), the IR is built
  again without the preview — so the extra upstream leaves no trace — with `W107` "preview ignored: `…` has no output
  that converts to a color". Otherwise `W107` "preview active on this node (mode …); the material output is
  bypassed" is reported at the node and the Fragment `outputs` are replaced: Albedo's operand (`albedo_expr`) goes into
  `base_color` (Pbr3d) / `color` (Unlit3d) / `color` as `vec4<f32>(albedo, 1.0)` (Sprite2d, alpha forced to 1).
  `Full` also maps every other slot except Opacity into its output pin (converted to that pin's type; inactive pins skipped);
  `Lit` / `Unlit` use Albedo only. A vertex stage keeps existing with its offset set to `vec3<f32>(0.0)`.
  `uses_normal` follows the new outputs. `Ir::preview` is `Some(IrPreview { node, mode })`.
- **WESL per kind** (`shader_codegen_wesl::material`, spec §5 "Codegen per mode"). A preview adds a third header
  line `// preview of node {id} ({mode}): the material output is bypassed`. Alpha is forced to 1 in every kind; in `full`
  mode this wins over an Opacity slot of the DTO (`build_ir` skips that slot, so the previewed color stays fully visible).

  | Kind | `unlit` | `lit` | `full` |
  |---|---|---|---|
  | Pbr3d | `base_color = vec4(albedo, 1.0)`; `STANDARD_MATERIAL_FLAGS_UNLIT_BIT` set in `pbr_input.material.flags` before `alpha_discard`; forward writes `out.color = pbr_input.material.base_color` instead of `apply_pbr_lighting` (post-lighting processing kept); deferred keeps `deferred_output`, which honours the flag | `base_color = vec4(albedo, 1.0)`, normal lighting path | as `lit`, plus every other DTO slot except Opacity in its output pin |
  | Unlit3d | `return vec4<f32>(color, 1.0);` — no opacity, mask discard or blend-mode variants | same as `unlit` | same as `unlit` |
  | Sprite2d | `return {color};` (the IR built `vec4<f32>(albedo, 1.0)`), no `get_final_color` | same | same |

  The unlit bypass is done in WESL only, mirroring Bevy's `pbr.wesl` (which branches on the same flag), never via
  `StandardMaterial::unlit` in Rust, so the Rust side is untouched and the prepass/deferred variants still compile.
  Uniforms, textures and the vertex stage stay declared (the vertex offset is `vec3<f32>(0.0)`). The snapshot suite
  compiles all 9 kind × mode combinations (`tests/snapshots/wesl__preview__*`) in every forward/deferred variant.
- **W108** (`shader_project::regen`, materials only). When `asset.preview.node` is set, the preview IR is active
  (`ir.preview` is `Some`) and it has no errors, a second IR is built from a clone with `preview =
  Preview::default()`; if that one is error-free too and `shader_codegen_rust::material_module` differs between the
  two, `W108` "preview changes the generated Rust crate (extra parameters or builtins); rebuild the game, and again
  after turning preview off" is reported on the asset. The Rust crate is still generated from the preview IR (like
  the WESL), so the game binary and its shaders agree; the user just has to rebuild.
- **Diagnostics.** `W107` (warning) is reported while a preview is active so `check` / `regen` and CI logs show it,
  and when a stored preview is ignored. `W108` (warning) is reported when the preview changes the generated Rust
  after all — when the previewed upstream uses a parameter or builtin the material output does not.

**Consequences.**
- Previewing a node that is upstream of nothing still emits it (it joins the live set); nodes that only the
  material output uses are emitted too (dead code in preview WESL, deterministic and compiling).
- Errors anywhere in the Fragment live set (including the normal output's upstream) still block the asset while
  previewing.
- An active preview costs a second `build_ir` of the asset per regeneration (the W108 check).
- A rejected preview costs a second `build_ir` of the asset (only for a stored preview of a node that has no color-like output).

## Editor

- **Toggling.** `Action::TogglePreview(node)` acts on the active graph document, materials only: it sets
  `preview.node` to the node, or clears it when it already is the previewed one, in one `Doc::edit` (undoable, marks
  the tab unsaved). Previewability is decided from the editor's resolved `node_signature`: the node's
  `NodeDef::preview_output` must not be empty and the MaterialOutput node never is (`Editor::preview_refusal`).
  The previewed node itself can always be turned off (menu "Stop preview", `V`), even when its output no longer
  converts to a color (the preview is then ignored with W107).
  The node menu (decision 0021) gets `Exclusive preview\tV` / `Stop preview\tV` after the separator that follows the
  node's own actions, before Copy; a node that cannot be previewed gets `Exclusive preview (not available)`, which
  only logs "`…` has no output that converts to a color". Function graphs get no row.
- **`V` on release.** `Editor::hotkey_actions(Hotkey::plain(Letter('v')))` gives `TogglePreview` only when exactly one
  node is selected and it can be previewed. `ui::input::shortcuts` sends it when `V` is released and `PreviewKey`
  says no node was dropped with `V` + LMB meanwhile (decision 0021).
- **Mode.** `PropTarget::PreviewMode`: a select `Unlit` / `Lit` / `Full` in a "Preview" section of the material's
  Properties, with a dim line naming the previewed node (or "No node previewed - select a node and press V"; an
  ASCII dash because the UI font has no em dash). An ordinary undoable edit; the mode stays when no node is previewed.
- **In-memory regeneration** (spec §5 "Applying it"). `Editor::apply` remembers the active graph document's
  `preview` before each action; when that document's `preview` differs afterwards (toggle, mode, undo / redo,
  deleting the previewed node) it calls `regenerate_from_memory`: `regenerate_set` over `Editor::disk` with only this
  asset replaced by the in-memory document (its other unsaved edits included — accepted), `Scope::Affected({id})`,
  not a dry run, with the WESL compile check when Bevy sources are known. It goes through the normal writer (atomic,
  marker check, write-only-if-changed), so the Rust module is rewritten only if it really differs (W108, or other
  unsaved edits that change it). Errors in the asset keep its files and log a warning with their count. The
  document is flagged `Doc::regen_from_memory` when it is unsaved at that moment (a clean document generates what
  its file does; `mark_saved` clears the flag). Other edits while previewing do not
  regenerate; they follow the usual Ctrl+S. A mode change while no node is previewed (before and after) does not
  regenerate either. A full regeneration (Ctrl+S of another tab, Save All, Regen All) overlays flagged unsaved
  documents onto the disk set, so it keeps their in-memory output instead of silently reverting the preview.
- **Discarding.** Closing a flagged document that is still unsaved (`remove_tabs`: Discard, tab menu), Reload in
  the "File changed on disk" dialog (checked before `Doc::reload` clears the flag; the tab need not be active),
  closing the project without saving and Quit without saving call `regenerate_from_disk`, the same path with `Editor::disk`
  unchanged, so the generated WESL matches the file again. Until then a CLI `regen` would undo the preview (accepted).
- **Look.** The previewed node has a 3 px border in the accent color (white when selected) and
  `IconId::Warning` (icon-rs `WARNING_FILLED`) in the accent color before its title; the diagnostic outline (W107) is
  drawn around it as for any warning. Material tabs with a previewed node get the suffix ` (preview)` in the tab bar
  and the window title; `tab::labels` compares the names without it, so two `rock_wet` tabs still show full paths.
  *Superseded in part by 0033:* the border is now a 6 px outline pulsing between orange and dark red (selected
  nodes: a 3 px outline); the warning icon stays.
