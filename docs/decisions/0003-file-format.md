# 0003. RON schema and migrations

**Decision.**
- The file is a serialized `Asset { format_version, kind, name, nodes, edges, comments, meta }` with struct names (`PrettyConfig::struct_names`), 2-space indent, nodes in a `BTreeMap` by id — stable order and readable diffs.
- A node id is a lowercase ULID (`[a-z0-9]+`, validated, E009); fixtures use `n01…`.
- Pin and parameter values equal to the defaults are stored only if set explicitly; `inputs` is omitted when empty.
- Loading first reads only `format_version` (`VersionProbe`). A newer version than supported is a clear error; older versions will migrate through an "old struct → new struct" chain in `Asset::from_ron`. The current version is 1.
- The asset name is taken from the file name (the file wins over the `name` field).

**Consequences.** Renaming a file outside the editor does not break the asset; the editor updates `function::path` references in other assets itself (`ops::rename`).
