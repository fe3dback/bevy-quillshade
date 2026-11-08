//! The project-wide `AssetIndex` (ADR 0047 §7–8, techdoc §4.2, D19): every editor-owned asset of every domain by
//! UUID, the references between them, duplicate UUIDs (a copied file) and files without an id; plus the D27 trailing
//! path comments of the source writers (ADR 0047 §5).
//!
//! Domains contribute a [`DomainIndex`] each (`regen::DomainBackend::index`, built with their normal loaders); the
//! index itself does no I/O, so tests feed it parts directly. Bevy-free; `BTreeMap` / `BTreeSet` only (D7).

use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::asset_ref::AssetRef;
use crate::asset_uuid::{AssetUuid, is_uuid_shaped};
use crate::diag::Diagnostic;
use crate::domain::DomainId;
use crate::layout::Project;

/// A sub-item of an asset, referenced as `(asset uuid, local id)` (a constant of a `.consts.ron`, a gradient slot) or
/// by its own `uuid` (an enum or variant of a `.features.ron`).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct SubItem {
    pub id: u64,
    pub uuid: Option<AssetUuid>,
    pub label: String,
}

impl SubItem {
    pub fn new(id: u64, label: impl Into<String>) -> Self {
        SubItem { id, uuid: None, label: label.into() }
    }

    pub fn with_uuid(uuid: AssetUuid, label: impl Into<String>) -> Self {
        SubItem { id: 0, uuid: Some(uuid), label: label.into() }
    }
}

/// One reference stored in a source file.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct IndexedRef {
    pub to: AssetUuid,
    /// The local id of a sub-item of the target.
    pub sub: Option<u64>,
    /// Where the reference is stored: a node id, a field name.
    pub location: Option<String>,
}

/// One current-format source file (one that does not load is not listed).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexedAsset {
    pub asset: AssetRef,
    /// The file's `id`, or why it has none, phrased to follow the file name in the AE004 text ("has a malformed
    /// id"). A nil or non-v7 `Ok` id counts as missing too.
    pub id: Result<AssetUuid, String>,
    pub sub_items: Vec<SubItem>,
    pub refs: Vec<IndexedRef>,
}

/// A domain's contribution to the [`AssetIndex`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DomainIndex {
    pub domain: DomainId,
    pub assets: Vec<IndexedAsset>,
}

impl DomainIndex {
    /// A domain with no indexed asset.
    pub fn empty(domain: DomainId) -> DomainIndex {
        DomainIndex { domain, assets: Vec::new() }
    }
}

/// The asset a UUID names (the winner of a duplicate).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssetEntry {
    pub id: AssetUuid,
    pub asset: AssetRef,
    pub sub_items: Vec<SubItem>,
}

impl AssetEntry {
    /// `<domain>.<suffix>` (`material.curve`, `light.environment`); `<domain>.node` for a kind without a suffix (user
    /// nodes, ADR 0047 §16).
    pub fn kind_id(&self) -> String {
        let suffix = if self.asset.suffix.is_empty() { "node" } else { self.asset.suffix };
        format!("{}.{}", self.asset.domain, suffix)
    }
}

/// Source of the D27 labels: a UUID's domain and domain-relative path. Implemented by [`AssetIndex`] and by a plain
/// map (the migration's facts).
pub trait RefNames {
    fn name_of(&self, id: AssetUuid) -> Option<(DomainId, &str)>;
}

impl RefNames for BTreeMap<AssetUuid, (DomainId, String)> {
    fn name_of(&self, id: AssetUuid) -> Option<(DomainId, &str)> {
        self.get(&id).map(|(d, p)| (*d, p.as_str()))
    }
}

/// Every editor-owned asset of the project by UUID, with forward and reverse reference edges.
#[derive(Debug, Clone, Default)]
pub struct AssetIndex {
    parts: BTreeMap<DomainId, Vec<IndexedAsset>>,
    /// UUID → the winner of a duplicate.
    by_id: BTreeMap<AssetUuid, AssetEntry>,
    /// Sub-item UUID → `(domain, "<asset_path>::<sub_label>")` for D27 path comments.
    sub_by_id: BTreeMap<AssetUuid, (DomainId, String)>,
    /// Sub-item UUID → owning asset UUID (winner of duplicates).
    sub_owner: BTreeMap<AssetUuid, AssetUuid>,
    /// Every file with a valid id, duplicate losers included.
    by_ref: BTreeMap<AssetRef, AssetUuid>,
    /// Forward edges, keyed by file: duplicate files share a UUID but are different referrers.
    refs_from: BTreeMap<AssetRef, BTreeSet<IndexedRef>>,
    /// Reverse edges: the files referencing a UUID.
    refs_to: BTreeMap<AssetUuid, BTreeSet<AssetRef>>,
    /// Files sharing a UUID, sorted by `source_rel`; `[0]` is the winner.
    duplicates: BTreeMap<AssetUuid, Vec<AssetRef>>,
    /// Current-format files without a valid UUIDv7 id, and why.
    missing_ids: BTreeMap<AssetRef, String>,
    generation: u64,
}

/// Generation source: process-wide, so an index rebuilt from scratch never repeats an earlier index's generation.
static GENERATION: AtomicU64 = AtomicU64::new(1);

impl AssetIndex {
    pub fn build(parts: Vec<DomainIndex>) -> AssetIndex {
        let mut index = AssetIndex::default();
        for part in parts {
            index.parts.insert(part.domain, part.assets);
        }
        index.rebuild();
        index
    }

    /// Replaces one domain's assets (after a save, create, rename or delete in it), keeping every other domain's.
    pub fn replace_domain(&mut self, part: DomainIndex) {
        self.parts.insert(part.domain, part.assets);
        self.rebuild();
    }

    fn rebuild(&mut self) {
        let parts = std::mem::take(&mut self.parts);
        *self = AssetIndex { generation: GENERATION.fetch_add(1, Ordering::Relaxed), ..AssetIndex::default() };
        let mut files: BTreeMap<AssetUuid, Vec<&IndexedAsset>> = BTreeMap::new();
        for a in parts.values().flatten() {
            let missing = match &a.id {
                Ok(id) if id.is_v7() => {
                    files.entry(*id).or_default().push(a);
                    None
                }
                Ok(id) if id.is_nil() => Some("has no `id`".to_string()),
                Ok(id) => Some(format!("has the `id` {id}, which is not a UUIDv7")),
                Err(why) => Some(why.clone()),
            };
            if let Some(why) = missing {
                self.missing_ids.insert(a.asset.clone(), why);
            }
            for r in &a.refs {
                self.refs_from.entry(a.asset.clone()).or_default().insert(r.clone());
                self.refs_to.entry(r.to).or_default().insert(a.asset.clone());
            }
        }
        for (id, mut same) in files {
            same.sort_by_key(|a| a.asset.source_rel());
            for a in &same {
                self.by_ref.insert(a.asset.clone(), id);
            }
            if same.len() > 1 {
                self.duplicates.insert(id, same.iter().map(|a| a.asset.clone()).collect());
            }
            let winner = same[0];
            for sub in &winner.sub_items {
                if let Some(sub_uuid) = sub.uuid {
                    self.sub_by_id
                        .insert(sub_uuid, (winner.asset.domain, format!("{}::{}", winner.asset.path, sub.label)));
                    self.sub_owner.insert(sub_uuid, id);
                }
            }
            self.by_id.insert(id, AssetEntry { id, asset: winner.asset.clone(), sub_items: winner.sub_items.clone() });
        }
        self.parts = parts;
    }

    fn target_entry(&self, id: AssetUuid) -> Option<&AssetEntry> {
        self.by_id.get(&id).or_else(|| self.sub_owner.get(&id).and_then(|owner| self.by_id.get(owner)))
    }

    /// Changes whenever the index is built or a domain replaced; never repeats within the process.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn get(&self, id: AssetUuid) -> Option<&AssetEntry> {
        self.by_id.get(&id)
    }

    /// The file's own id (a duplicate loser's too); `None` when it is not indexed or has no valid id.
    pub fn id_of(&self, r: &AssetRef) -> Option<AssetUuid> {
        self.by_ref.get(r).copied()
    }

    /// The id references to the file store and resolve to it by: its own id when it is the winner of that id;
    /// `None` for a duplicate loser (references to its id name the winner, ADR 0047 §8), a file not indexed or one
    /// without a valid id. What a picker stores and what a delete clears.
    pub fn ref_id_of(&self, r: &AssetRef) -> Option<AssetUuid> {
        self.id_of(r).filter(|id| self.by_id.get(id).is_some_and(|e| &e.asset == r))
    }

    /// The references stored in a file.
    pub fn refs_from(&self, r: &AssetRef) -> impl Iterator<Item = &IndexedRef> {
        self.refs_from.get(r).into_iter().flatten()
    }

    /// The files referencing a UUID.
    pub fn refs_to(&self, id: AssetUuid) -> impl Iterator<Item = &AssetRef> {
        self.refs_to.get(&id).into_iter().flatten()
    }

    /// `changed` plus every file referencing one of them, transitively and across domains (techdoc §4.3). A
    /// duplicate loser has no referrers: references resolve to the winner.
    pub fn affected(&self, changed: &BTreeSet<AssetRef>) -> BTreeSet<AssetRef> {
        let mut out = changed.clone();
        let mut todo: Vec<AssetRef> = changed.iter().cloned().collect();
        while let Some(r) = todo.pop() {
            let Some(id) = self.ref_id_of(&r) else { continue };
            for referrer in self.refs_to(id) {
                if out.insert(referrer.clone()) {
                    todo.push(referrer.clone());
                }
            }
            if let Some(entry) = self.by_id.get(&id) {
                for sub in &entry.sub_items {
                    if let Some(sub_uuid) = sub.uuid {
                        for referrer in self.refs_to(sub_uuid) {
                            if out.insert(referrer.clone()) {
                                todo.push(referrer.clone());
                            }
                        }
                    }
                }
            }
        }
        out
    }

    /// References to a UUID no asset has, with their referrer, in file order.
    pub fn dangling(&self) -> Vec<(AssetRef, IndexedRef)> {
        self.refs_from
            .iter()
            .flat_map(|(from, refs)| refs.iter().map(move |r| (from, r)))
            .filter(|(_, r)| self.target_entry(r.to).is_none())
            .map(|(from, r)| (from.clone(), r.clone()))
            .collect()
    }

    /// UUIDs shared by several files, each with its files sorted by `source_rel` (`[0]` is the winner).
    pub fn duplicates(&self) -> impl Iterator<Item = (AssetUuid, &[AssetRef])> {
        self.duplicates.iter().map(|(id, files)| (*id, files.as_slice()))
    }

    /// Current-format files without a valid UUIDv7 id, with the reason.
    pub fn missing_ids(&self) -> impl Iterator<Item = (&AssetRef, &str)> {
        self.missing_ids.iter().map(|(r, why)| (r, why.as_str()))
    }

    /// The assets of a kind in path order, duplicate losers left out: what a picker lists.
    pub fn assets_of(&self, domain: DomainId, suffix: &str) -> impl Iterator<Item = &AssetEntry> {
        self.by_ref
            .iter()
            .filter(move |(r, _)| r.domain == domain && r.suffix == suffix)
            .filter_map(|(r, id)| self.by_id.get(id).filter(|e| &e.asset == r))
    }

    /// The core index diagnostics (ADR 0047 §8) of the enabled domains' files, file by file in path order: `AE001`
    /// at a duplicate loser, `AE004` for a missing id, `AE002` / `AE003` per reference. Each carries the file's
    /// domain and `source_rel`; reference ones their `location`.
    pub fn diagnostics(&self, project: &Project) -> Vec<Diagnostic> {
        let mut winner_of: BTreeMap<&AssetRef, &AssetRef> = BTreeMap::new();
        for files in self.duplicates.values() {
            for loser in &files[1..] {
                winner_of.insert(loser, &files[0]);
            }
        }
        let files: BTreeSet<&AssetRef> =
            self.by_ref.keys().chain(self.missing_ids.keys()).chain(self.refs_from.keys()).collect();
        let mut out = Vec::new();
        for file in files.into_iter().filter(|f| project.domain_enabled(f.domain)) {
            let at = |d: Diagnostic| d.in_domain(file.domain).in_asset(file.source_rel());
            if let Some(winner) = winner_of.get(file) {
                out.push(at(Diagnostic::error(
                    "AE001",
                    format!(
                        "`{file}` has the same UUID as `{winner}` (a copied file?); references keep pointing at \
                         `{winner}`"
                    ),
                )));
            }
            if let Some(why) = self.missing_ids.get(file) {
                out.push(at(Diagnostic::error(
                    "AE004",
                    format!("`{file}` {why}; save it in the editor to assign one"),
                )));
            }
            for r in self.refs_from(file) {
                let d = match self.target_entry(r.to) {
                    None => Diagnostic::error("AE002", format!("references the UUID {}, which no asset has", r.to)),
                    Some(target) if !project.domain_enabled(target.asset.domain) => Diagnostic::error(
                        "AE003",
                        format!("references `{}` of the disabled domain `{}`", target.asset, target.asset.domain),
                    ),
                    Some(_) => continue,
                };
                out.push(match &r.location {
                    Some(loc) => at(d).at(loc.clone()),
                    None => at(d),
                });
            }
        }
        out
    }
}

impl RefNames for AssetIndex {
    fn name_of(&self, id: AssetUuid) -> Option<(DomainId, &str)> {
        self.get(id)
            .map(|e| (e.asset.domain, e.asset.path.as_str()))
            .or_else(|| self.sub_by_id.get(&id).map(|(d, p)| (*d, p.as_str())))
    }
}

/// Appends the D27 path comment to every line of `text` (serializer output, one value per line) that holds a
/// UUID-shaped token, except `id` lines (`id: "<uuid>"`, the root asset id or a sub-item/row id): ` // <label>` with
/// label = the target's domain-relative path without suffix, prefixed `<domain>:` when it is not in `from`,
/// `(missing)` when unknown; several tokens on a line are joined with `, `. A `//` comment already on such a line
/// (outside strings) is replaced, so annotating annotated text changes nothing.
pub fn annotate_refs(text: &str, names: &dyn RefNames, from: DomainId) -> String {
    let mut out = String::with_capacity(text.len() + 64);
    for line in text.split_inclusive('\n') {
        let (body, eol) = match line.strip_suffix('\n') {
            Some(b) => b.strip_suffix('\r').map_or((b, "\n"), |b| (b, "\r\n")),
            None => (line, ""),
        };
        let code = &body[..comment_start(body).unwrap_or(body.len())];
        let tokens = uuid_tokens(code);
        if is_id_line(code) || tokens.is_empty() {
            out.push_str(line);
            continue;
        }
        let labels: Vec<String> = tokens
            .iter()
            .map(|t| match AssetUuid::parse(t).and_then(|id| names.name_of(id)) {
                Some((d, path)) if d == from => path.to_string(),
                Some((d, path)) => format!("{d}:{path}"),
                None => "(missing)".into(),
            })
            .collect();
        out.push_str(code.trim_end());
        out.push_str(" // ");
        out.push_str(&labels.join(", "));
        out.push_str(eol);
    }
    out
}

/// `text` with the UUID of its root `id` line (see [`annotate_refs`]) replaced by `new`; `None` when it has none.
pub fn rewrite_root_id(text: &str, new: AssetUuid) -> Option<String> {
    let root = root_id_line(text)?;
    let mut out = String::with_capacity(text.len());
    for (i, line) in text.split_inclusive('\n').enumerate() {
        if i == root {
            let start = line.find('"')? + 1;
            out.push_str(&line[..start]);
            out.push_str(&new.to_string());
            out.push_str(&line[start + 36..]);
        } else {
            out.push_str(line);
        }
    }
    Some(out)
}

/// Whether `line` reads `id: "<uuid>"` (any indentation, an optional trailing `,` and comment).
fn is_id_line(line: &str) -> bool {
    line.trim_start()
        .strip_prefix("id:")
        .map(str::trim_start)
        .and_then(|v| v.strip_prefix('"'))
        .is_some_and(|v| v.get(..36).is_some_and(is_uuid_shaped) && v[36..].starts_with('"'))
}

/// Index of the first line that reads `id: "<uuid>"` (any indentation, an optional trailing `,` and comment).
fn root_id_line(text: &str) -> Option<usize> {
    text.split_inclusive('\n').position(is_id_line)
}

/// Byte offset of a `//` comment outside RON string literals.
fn comment_start(line: &str) -> Option<usize> {
    let b = line.as_bytes();
    let (mut in_str, mut i) = (false, 0);
    while i < b.len() {
        match b[i] {
            b'\\' if in_str => i += 1,
            b'"' => in_str = !in_str,
            b'/' if !in_str && b.get(i + 1) == Some(&b'/') => return Some(i),
            _ => {}
        }
        i += 1;
    }
    None
}

/// The UUID-shaped tokens of `s` that are not part of a longer run of hex digits and dashes.
fn uuid_tokens(s: &str) -> Vec<&str> {
    let b = s.as_bytes();
    let part = |c: u8| c.is_ascii_hexdigit() || c == b'-';
    let mut out = Vec::new();
    let mut i = 0;
    while i + 36 <= b.len() {
        let bounded = (i == 0 || !part(b[i - 1])) && b.get(i + 36).is_none_or(|&c| !part(c));
        if bounded && s.is_char_boundary(i) && s.is_char_boundary(i + 36) && is_uuid_shaped(&s[i..i + 36]) {
            out.push(&s[i..i + 36]);
            i += 36;
        } else {
            i += 1;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::DomainEntry;

    const MAT: DomainId = DomainId::new("material");
    const LIGHT: DomainId = DomainId::new("light");

    fn r(domain: DomainId, path: &str, suffix: &'static str) -> AssetRef {
        AssetRef { domain, path: path.into(), suffix }
    }

    fn id(n: u128) -> AssetUuid {
        // A v7 with the version / variant bits set: `xxxxxxxx-xxxx-7xxx-8xxx-xxxxxxxxxxxx`.
        AssetUuid::from_u128(0x0190_0000_0000_7000_8000_0000_0000_0000 | n)
    }

    fn asset(r: AssetRef, n: u128, refs: &[(u128, Option<u64>, &str)]) -> IndexedAsset {
        IndexedAsset {
            asset: r,
            id: Ok(id(n)),
            sub_items: Vec::new(),
            refs: refs.iter().map(|&(to, sub, at)| IndexedRef { to: id(to), sub, location: Some(at.into()) }).collect(),
        }
    }

    fn project() -> (tempfile::TempDir, Project) {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[workspace]\nmembers = [\"crates/*\"]\n").unwrap();
        std::fs::create_dir(dir.path().join("assets")).unwrap();
        let p = Project::open(dir.path()).unwrap();
        (dir, p)
    }

    /// material: `common/pulse` curve (1, two sub-items), `env/rock` material (2) using 1 and the light texture 3;
    /// light: `sky` texture (3), `day` environment (4) using 3.
    fn sample() -> AssetIndex {
        let mut pulse = asset(r(MAT, "common/pulse", "curve"), 1, &[]);
        pulse.sub_items = vec![SubItem::new(0, "a"), SubItem::new(7, "b")];
        AssetIndex::build(vec![
            DomainIndex {
                domain: MAT,
                assets: vec![pulse, asset(r(MAT, "env/rock", "material"), 2, &[(1, None, "n01"), (3, None, "n02")])],
            },
            DomainIndex {
                domain: LIGHT,
                assets: vec![
                    asset(r(LIGHT, "sky", "texture"), 3, &[]),
                    asset(r(LIGHT, "day", "environment"), 4, &[(3, None, "cubemap")]),
                ],
            },
        ])
    }

    #[test]
    fn build_and_lookup() {
        let ix = sample();
        let e = ix.get(id(1)).unwrap();
        assert_eq!(
            (e.asset.path.as_str(), e.kind_id(), e.sub_items.len()),
            ("common/pulse", "material.curve".into(), 2)
        );
        assert_eq!(ix.id_of(&r(LIGHT, "day", "environment")), Some(id(4)));
        assert_eq!(ix.id_of(&r(LIGHT, "nope", "environment")), None);
        assert_eq!(ix.get(id(9)), None);
        let refs: Vec<_> = ix.refs_from(&r(MAT, "env/rock", "material")).map(|x| (x.to, x.location.clone())).collect();
        assert_eq!(refs, [(id(1), Some("n01".into())), (id(3), Some("n02".into()))]);
        let to_sky: Vec<_> = ix.refs_to(id(3)).collect();
        assert_eq!(to_sky, [&r(LIGHT, "day", "environment"), &r(MAT, "env/rock", "material")]);
        assert_eq!(ix.refs_to(id(2)).count(), 0);
        let curves: Vec<_> = ix.assets_of(MAT, "curve").map(|e| e.id).collect();
        assert_eq!(curves, [id(1)]);
        assert_eq!(ix.assets_of(LIGHT, "curve").count(), 0);
        assert!(ix.dangling().is_empty() && ix.duplicates().next().is_none() && ix.missing_ids().next().is_none());
        assert_ne!(ix.generation(), sample().generation(), "every build has its own generation");
        assert_eq!(AssetIndex::default().get(id(1)), None);
    }

    #[test]
    fn duplicates_winner_is_smaller_source_rel() {
        // `b/copy` is listed first, but `a/orig` sorts first by `source_rel`.
        let ix = AssetIndex::build(vec![DomainIndex {
            domain: MAT,
            assets: vec![
                asset(r(MAT, "b/copy", "curve"), 1, &[]),
                asset(r(MAT, "a/orig", "curve"), 1, &[]),
                asset(r(MAT, "user", "material"), 2, &[(1, None, "n01")]),
            ],
        }]);
        assert_eq!(ix.get(id(1)).unwrap().asset, r(MAT, "a/orig", "curve"));
        let dups: Vec<_> = ix.duplicates().collect();
        assert_eq!(dups, [(id(1), &[r(MAT, "a/orig", "curve"), r(MAT, "b/copy", "curve")][..])]);
        // Both files keep their own id; the picker lists the winner only.
        assert_eq!(ix.id_of(&r(MAT, "b/copy", "curve")), Some(id(1)));
        assert_eq!(ix.assets_of(MAT, "curve").map(|e| e.asset.path.as_str()).collect::<Vec<_>>(), ["a/orig"]);
        // References resolve to the winner: only it has referrers.
        let affected = ix.affected(&BTreeSet::from([r(MAT, "b/copy", "curve")]));
        assert_eq!(affected, BTreeSet::from([r(MAT, "b/copy", "curve")]));
        let affected = ix.affected(&BTreeSet::from([r(MAT, "a/orig", "curve")]));
        assert!(affected.contains(&r(MAT, "user", "material")));
    }

    #[test]
    fn dangling_and_disabled_domain() {
        let (_d, mut p) = project();
        let ix = AssetIndex::build(vec![
            DomainIndex {
                domain: MAT,
                assets: vec![asset(r(MAT, "m", "material"), 1, &[(9, Some(3), "n04"), (2, None, "n05")])],
            },
            DomainIndex { domain: LIGHT, assets: vec![asset(r(LIGHT, "sky", "texture"), 2, &[])] },
        ]);
        assert_eq!(
            ix.dangling(),
            [(r(MAT, "m", "material"), IndexedRef { to: id(9), sub: Some(3), location: Some("n04".into()) })]
        );
        let codes = |p: &Project| ix.diagnostics(p).into_iter().map(|d| d.code).collect::<Vec<_>>();
        assert_eq!(codes(&p), ["AE002"]);
        p.settings.domains.insert(LIGHT.as_str().into(), DomainEntry { enabled: false });
        assert_eq!(codes(&p), ["AE003", "AE002"], "in reference order (by target UUID)");
        // A disabled domain's own files report nothing.
        p.settings.domains.insert(MAT.as_str().into(), DomainEntry { enabled: false });
        assert!(codes(&p).is_empty());
    }

    #[test]
    fn affected_is_transitive_and_cross_domain() {
        // light sky (3) <- material env/rock (2) <- material env/wrap (5); light day (4) -> sky.
        let mut ix = sample();
        ix.replace_domain(DomainIndex {
            domain: MAT,
            assets: vec![
                asset(r(MAT, "common/pulse", "curve"), 1, &[]),
                asset(r(MAT, "env/rock", "material"), 2, &[(1, None, "n01"), (3, None, "n02")]),
                asset(r(MAT, "env/wrap", "function"), 5, &[(2, None, "n09")]),
            ],
        });
        let sky = r(LIGHT, "sky", "texture");
        let got = ix.affected(&BTreeSet::from([sky.clone()]));
        let want = BTreeSet::from([
            sky,
            r(LIGHT, "day", "environment"),
            r(MAT, "env/rock", "material"),
            r(MAT, "env/wrap", "function"),
        ]);
        assert_eq!(got, want);
        // Leaves stay alone; an unindexed file is kept as given.
        let wrap = r(MAT, "env/wrap", "function");
        assert_eq!(ix.affected(&BTreeSet::from([wrap.clone()])), BTreeSet::from([wrap]));
        let gone = r(MAT, "gone", "curve");
        assert_eq!(ix.affected(&BTreeSet::from([gone.clone()])), BTreeSet::from([gone]));
    }

    #[test]
    fn replace_domain_keeps_others() {
        let mut ix = sample();
        let before = ix.generation();
        ix.replace_domain(DomainIndex::empty(MAT));
        assert_ne!(ix.generation(), before);
        assert_eq!(ix.get(id(1)), None);
        assert_eq!(ix.get(id(3)).unwrap().asset, r(LIGHT, "sky", "texture"));
        // The removed material's edge into Light is gone; Light's own edge stays.
        assert_eq!(ix.refs_to(id(3)).collect::<Vec<_>>(), [&r(LIGHT, "day", "environment")]);
        assert_eq!(ix.refs_from(&r(MAT, "env/rock", "material")).count(), 0);
    }

    #[test]
    fn ae_codes_and_locations() {
        let (_d, p) = project();
        let no_id = IndexedAsset {
            asset: r(MAT, "c/no_id", "curve"),
            id: Ok(AssetUuid::NIL),
            sub_items: Vec::new(),
            refs: vec![IndexedRef { to: id(8), sub: None, location: None }],
        };
        let v4 = IndexedAsset {
            id: Ok(AssetUuid::parse("6ba7b810-9dad-41d1-80b4-00c04fd430c8").unwrap()),
            ..asset(r(MAT, "c/v4", "curve"), 0, &[])
        };
        let bad = IndexedAsset { id: Err("has a malformed `id`".into()), ..asset(r(LIGHT, "bad", "curve"), 0, &[]) };
        let ix = AssetIndex::build(vec![
            DomainIndex {
                domain: MAT,
                assets: vec![
                    asset(r(MAT, "a", "curve"), 1, &[]),
                    asset(r(MAT, "b", "curve"), 1, &[]),
                    asset(r(MAT, "m", "material"), 2, &[(7, None, "n03")]),
                    no_id,
                    v4,
                ],
            },
            DomainIndex { domain: LIGHT, assets: vec![bad] },
        ]);
        let got: Vec<_> = ix
            .diagnostics(&p)
            .into_iter()
            .map(|d| (d.is_error(), d.code, d.domain.unwrap(), d.asset.unwrap(), d.location))
            .collect();
        let row =
            |code: &str, d, a: &str, l: Option<&str>| (true, code.to_string(), d, a.to_string(), l.map(String::from));
        assert_eq!(
            got,
            [
                row("AE004", LIGHT, "quillshade/light/bad.curve.ron", None),
                row("AE001", MAT, "quillshade/material/b.curve.ron", None),
                row("AE004", MAT, "quillshade/material/c/no_id.curve.ron", None),
                row("AE002", MAT, "quillshade/material/c/no_id.curve.ron", None),
                row("AE004", MAT, "quillshade/material/c/v4.curve.ron", None),
                row("AE002", MAT, "quillshade/material/m.material.ron", Some("n03")),
            ]
        );
        let text: Vec<_> = ix.diagnostics(&p).into_iter().map(|d| d.message).collect();
        assert!(text[1].contains("`quillshade/material/a.curve.ron`"), "{}", text[1]);
        assert!(text[2].contains("has no `id`") && text[4].contains("is not a UUIDv7"), "{text:?}");
        assert!(text[0].contains("`quillshade/light/bad.curve.ron` has a malformed `id`; save it"), "{}", text[0]);
        assert_eq!(ix.missing_ids().count(), 3);
    }

    fn names() -> BTreeMap<AssetUuid, (DomainId, String)> {
        BTreeMap::from([(id(1), (MAT, "textures/rock_albedo".into())), (id(2), (LIGHT, "sky".into()))])
    }

    #[test]
    fn annotate_appends_labels_skips_root_id_and_marks_missing() {
        let text = format!(
            "(\n  format_version: 2,\n  id: \"{root}\",\n  nodes: {{\n    \"n01\": (\n      kind: \"function::{a}\",\n      \
             params: {{\n        \"texture\": Str(\"{a}\"),\n        \"cube\": Str(\"{b}\"),\n        \"gone\": \
             Str(\"{c}\"),\n        \"path\": Str(\"common/pulse\"),\n        \"long\": Str(\"x{a}0\"),\n      }},\n    \
             ),\n  }},\n)\n",
            root = id(5),
            a = id(1),
            b = id(2),
            c = id(9),
        );
        let out = annotate_refs(&text, &names(), MAT);
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines[2], format!("  id: \"{}\",", id(5)), "the root id is not annotated");
        assert_eq!(lines[5], format!("      kind: \"function::{}\", // textures/rock_albedo", id(1)));
        assert_eq!(lines[7], format!("        \"texture\": Str(\"{}\"), // textures/rock_albedo", id(1)));
        assert_eq!(lines[8], format!("        \"cube\": Str(\"{}\"), // light:sky", id(2)));
        assert_eq!(lines[9], format!("        \"gone\": Str(\"{}\"), // (missing)", id(9)));
        assert_eq!(lines[10], "        \"path\": Str(\"common/pulse\"),");
        assert_eq!(lines[11], format!("        \"long\": Str(\"x{}0\"),", id(1)), "not a whole token");
        assert_eq!(out.lines().count(), text.lines().count());
        assert!(out.ends_with(")\n"));
        // Several tokens on a line; another domain's view of the same names.
        let two = format!("  pair: (\"{}\", \"{}\"),", id(1), id(2));
        assert_eq!(annotate_refs(&two, &names(), LIGHT), format!("{two} // material:textures/rock_albedo, sky"));
    }

    #[test]
    fn annotate_is_idempotent_on_serializer_output() {
        #[derive(serde::Serialize)]
        struct Doc {
            format_version: u32,
            id: AssetUuid,
            cubemap: Option<String>,
            refs: Vec<String>,
            note: String,
        }
        let doc = Doc {
            format_version: 2,
            id: id(5),
            cubemap: Some(id(2).to_string()),
            refs: vec![id(1).to_string(), id(9).to_string()],
            note: format!("see // {} \"quoted\"", id(1)),
        };
        let pretty = ron::ser::PrettyConfig::new().indentor("  ".to_string());
        let text = ron::ser::to_string_pretty(&doc, pretty).unwrap();
        let once = annotate_refs(&text, &names(), LIGHT);
        assert_ne!(once, text);
        assert_eq!(annotate_refs(&once, &names(), LIGHT), once);
        // A changed name replaces the old label instead of adding a second one.
        let renamed = BTreeMap::from([(id(2), (LIGHT, "sky_v2".to_string()))]);
        let again = annotate_refs(&once, &renamed, LIGHT);
        assert!(again.contains("// sky_v2\n") && !again.contains("// sky\n"), "{again}");
        // The `//` inside the string is not a comment: the note's label is appended after it.
        assert!(
            once.contains(&format!("\"see // {} \\\"quoted\\\"\", // material:textures/rock_albedo", id(1))),
            "{once}"
        );
        // The annotated text still parses to the same value.
        #[derive(serde::Deserialize, PartialEq, Debug)]
        struct Back {
            id: AssetUuid,
            cubemap: Option<String>,
            note: String,
        }
        let back: Back = ron::Options::default().from_str(&once).unwrap();
        assert_eq!((back.id, back.cubemap, back.note), (doc.id, doc.cubemap, doc.note));
    }

    #[test]
    fn rewrite_root_id() {
        let text = format!(
            "Curve(\n  format_version: 2,\n  id: \"{}\",\n  table: Str(\"{}\"), // x\n)\n",
            id(5).to_string().to_uppercase(),
            id(1)
        );
        let new = AssetUuid::new_v7();
        let out = super::rewrite_root_id(&text, new).unwrap();
        assert_eq!(
            out,
            format!("Curve(\n  format_version: 2,\n  id: \"{new}\",\n  table: Str(\"{}\"), // x\n)\n", id(1))
        );
        // Nothing to rewrite: no id line, or an `id` that is no UUID (a constant's local id).
        assert_eq!(super::rewrite_root_id("(\n  format_version: 1,\n  consts: [(id: 3)],\n)\n", new), None);
        assert_eq!(super::rewrite_root_id("(\n  id: \"common/pulse\",\n)\n", new), None);
    }

    #[test]
    fn sub_item_uuid_refs_resolve_to_owning_asset_and_propagate_affected() {
        const DATA: DomainId = DomainId::new("data");
        let (_d, mut p) = project();
        p.settings.domains.insert(DATA.as_str().into(), DomainEntry { enabled: true });

        let mut books = asset(r(DATA, "books", "collection"), 10, &[]);
        books.sub_items = vec![SubItem::with_uuid(id(11), "The Shader Alchemist")];
        let authors = asset(r(DATA, "authors", "collection"), 20, &[(11, None, "row[0].favorite_book")]);

        let ix = AssetIndex::build(vec![DomainIndex { domain: DATA, assets: vec![books, authors] }]);
        assert!(ix.dangling().is_empty(), "reference to sub-item UUID should not be dangling");
        assert!(ix.diagnostics(&p).is_empty(), "reference to sub-item UUID should not emit AE002");
        assert_eq!(ix.name_of(id(11)), Some((DATA, "books::The Shader Alchemist")));

        let affected = ix.affected(&BTreeSet::from([r(DATA, "books", "collection")]));
        assert_eq!(
            affected,
            BTreeSet::from([r(DATA, "books", "collection"), r(DATA, "authors", "collection")]),
            "changing the collection that owns a referenced row UUID must mark referrers as affected"
        );
    }
}
