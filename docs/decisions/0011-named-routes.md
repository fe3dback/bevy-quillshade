# 0011. Named routes

**Context.** Not in the spec (§7 lists only Reroute). Long wires across a graph are hard to read; UE5 solves this with
Named Reroute Declaration/Usage nodes.

**Decision.** Two stdlib nodes in the Utility category:
- `stdlib::NamedRoute` (role `RouteDeclaration`) — `in` → `out` pass-through with a `name` parameter. The name is only
  for the editor (titles, palette, the route select); it does not have to be unique.
- `stdlib::NamedRouteUsage` (role `RouteUsage`) — outputs the declaration's input. The `route` parameter stores the
  declaration's **node id**, so renaming a route never breaks its usages.

The wire is "teleported" by an implicit edge declaration `out` → usage `in` (`shader_graph_core::ir::route_edges`).
It is never stored in the file; `build_ir` and the editor's typing/connection checks (`with_routes`) add it before
anything else looks at the edges. Types, cycles (E002) and codegen therefore work exactly as for a chain of Reroutes:
no new WESL is emitted, `out` aliases the declaration's input. A usage whose declaration is missing (deleted, or
pasted into another asset) reports E004.

**Editor.** The canvas does not draw the implicit wire and hides the usage's input pin. The palette offers one entry
"Named Route Usage: {name}" per declaration of the open asset (no generic usage entry); the properties panel switches
a usage between declarations. Copy/paste: a usage pasted together with its declaration points at the copy, a usage
pasted alone keeps pointing at the original.

**Consequences.** No format change (`format_version` stays 1): both nodes are ordinary nodes with parameters. The
per-node snapshot test skips both roles; `shader_project/tests/pipeline.rs` checks that a routed graph produces the
same output expression as a direct wire.
