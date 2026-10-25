# 0017. Global constant tables and the Global* nodes

**Context.** Several materials often share tuning values (a wind strength, a palette colour, a quality switch). With
parameters every material carries its own copy, and a constant node is local to one graph. Requirements: a new
asset kind holding a table of named constants (`f32`, `vec2`, `vec3`, `vec4`, `bool`, `compileFlag`) edited in a
form + table view; nodes that read a constant of any table by name; values copied into the generated WESL as
literals (the tables exist only in the editor and affect compile time); `bool` behaves as `f32` (`1.0` / `0.0`),
`compileFlag` is a real `bool` usable only by logic nodes; changing a constant's type finds its usages, asks, and
clears the references (`GlobalConstVec2("my_const")` becomes `GlobalConstVec2("")`; the request said such a node outputs
zero, changed to 1.0 — see Nodes).

**Decision.**
- **File** `shaders/<path>.consts.ron`: `GlobalConsts(format_version: 1, name: "...", consts: [GlobalConst(name:
  "wind", ty: F32, value: Float(0.5)), ...])`, its own model type `shader_graph_core::consts::GlobalConsts` with its
  own `format_version` probe (like curves, decision 0015). Values are normalized to their type on load/save
  (`F32` → `Float`, `VecN` → `FloatN`, `Bool`/`CompileFlag` → `Bool`). `AssetType::Consts` joins `AssetId`;
  `AssetSet` keeps tables in their own map (`consts`). The file name wins over `name`.
- **Names** are identifiers (`[a-z][a-z0-9_]*`, not keywords) and unique in the whole project: nodes store only the
  name, not the table. The editor refuses to add a name that exists in any table (saved or open); a hand-edited
  duplicate is E005 at every table after the first one that defines it (path order), and only the first counts.
- **Nodes** (Constants category): `GlobalConstF32` (Float), `GlobalConstVec2/3/4` (Float2/3/4), `GlobalBool`
  (Float: `1.0f` / `0.0f`), `GlobalCompileFlag` (Bool: `true` / `false`). Parameter `const`
  (`ParamKind::GlobalConst(type)`, the constant's name as `Value::Str`); Properties offer the constants of that type
  from every table. Emission aliases the output to the value's WESL literal (no `let`), exactly like the Constant
  nodes. An empty reference outputs 1.0 in every component (`true` for `GlobalCompileFlag`) and reports W106 (new
  code, a warning); a new constant also starts at 1.0 / `true`. Not zero: a cleared reference or a fresh constant
  feeding a divisor or `log` made naga reject the shader (`1f / 0f`, `log(0f)` are const-folded into errors), which
  live validation does not catch (it does not compile) — only saving did, with E010. An unknown name is E006;
  a constant of another type is E003. Since `Bool` pins never convert to numbers, `compileFlag` reaches only Bool
  inputs (If (Bool), And, Or, Not, Bool function inputs) — the "logic only" rule needs no extra check.
- **Codegen / deps**: tables generate nothing (no WESL module, no Rust). `Env::consts` (`ConstLookup`) resolves names
  for the IR builder; `const_refs` lists the references of a graph; `DepGraph` makes a graph depend on the table
  that defines each referenced name, so saving a table regenerates its users (their WESL text changes, Bevy hot
  reloads them). `regen::wesl_file` / `module_of` return `Option` (`None` for tables).
- **Editor**: `Tab::Consts(ConstsDoc)` / `ViewKind::Consts` (steps of decision 0014). `ConstsDoc` keeps the table
  with the shared model-level history (`History<GlobalConsts>`) plus the add-form draft (name, type, error; editor
  only, not undoable). The view: a header (path), the form (name field, type select, Add), the table (name, type
  select, value editor, delete button; the form's Add is the only one, the toolbar has none). Value editors: `f32` —
  number field (any value) plus a slider over `0..1`, or the power of ten around the value when it is outside
  (`5` → `0..10`, `-37.5` → `-100..100`; a typed value widens it); `vecN` — colour picker plus R/G(/B/A)
  number fields (any value can be typed); `bool` / `compileFlag` — checkbox. Edits reuse `PropField` and the
  `SetValue`/`SetComponent` path (`PropTarget::Const*`), so slider drags and picker drags are one undo step each.
  The view is rebuilt only on structural changes (`ConstsDoc::layout`: add, delete, type change, undo/redo, reload),
  never while a value is being dragged or typed; number fields and the picker are kept in step by a sync system.
- **Type change / delete**: usages = nodes referencing the name in every graph (open documents as edited, the rest
  from disk), counted only for names the table actually defines (a hand-made duplicate in a later table has no
  readers of its own). None → a plain undoable edit (the value is converted, e.g. `vec3` → `f32` keeps `x`).
  Otherwise a confirmation lists the graphs and node counts; on OK the table is edited **and saved** (so the table
  on disk and its readers never disagree) and every reference is cleared: open graphs are edited in memory (a clean
  one is saved right away, a dirty one stays dirty), closed ones are rewritten on disk, then the project
  regenerates. Undo / redo in the table go through the same check (`ConstsAction::History`): a step that removes or
  retypes a constant that nodes read asks first; clearing in other graphs is undone in those graphs, not by the
  table's undo. Deleting a table file (browser) asks the same way for the constants only it defines, then clears
  their readers. Renaming a constant is not supported (delete and add again); renaming or moving a table file changes
  nothing for its users.
- **Node preview**: Global* nodes show the value read-only under the title, from the open table tab if there is one
  (unsaved edits show at once), redrawn by its own system like the curve preview: `f32` — the number; `vecN` — a
  colour square from the components (linear RGB, missing ones 0, alpha ignored); `bool` / `compileFlag` — a 32×32
  square with `0` / `1` and a small check mark, green when on, grey when off; "n/a" without a resolvable constant.
  The title shows the constant's name.
- **Browser / menu**: `New Global Consts...` (icon-rs `custom-table`), watermark `consts`; no "Show generated WESL"
  for tables.

**Consequences.** A fourth asset kind; a table edit rewrites the WESL of every user (unlike curves, whose data lives
in their own module) — acceptable for a handful of shaders and what "compile-time constants" means. Values are not
uniforms: changing one needs a regeneration (save), not a material property change at runtime.
