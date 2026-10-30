//! The node palette after a wire drop and Named Routes.

use super::*;
use crate::graph_domain::MaterialGraph;

pub(super) const ROUTE_DECL: &str = "stdlib::NamedRoute";

/// The palette's entries for `link` as the canvas gets them ([`MaterialGraph::palette`]).
pub(super) fn palette_entries(e: &Editor, link: Option<&Link>) -> Vec<node_graph_base::PaletteEntry> {
    let pin = link.map(|l| node_graph_base::PinKey { node: l.node.clone(), pin: l.pin.clone(), output: l.from_output });
    let ctx = node_graph_base::GraphCtxRef { editor: e };
    <MaterialGraph as node_graph_base::GraphDomain>::palette(&ctx, e.doc().unwrap(), pin.as_ref())
}

/// The palette's rows for `link` and `query` as (kind, name), in the order it lists them (the base's ranking).
pub(super) fn palette_rows(e: &Editor, link: Option<&Link>, query: &str) -> Vec<(String, String)> {
    let entries = palette_entries(e, link);
    let rows = node_graph_base::palette::results(&entries, query);
    rows.into_iter().map(|i| (entries[i].id.clone(), entries[i].name.clone())).collect()
}

/// The palette's entries as they were before design §6.4: every node, the route usages, filtered by the wire.
pub(super) fn palette_before(e: &Editor, link: Option<&Link>) -> Vec<search::Entry> {
    let d = e.doc().unwrap();
    let mut entries =
        search::entries(&e.domains.material().registry, &e.domains.material().functions, d.asset.kind, &d.id.path);
    entries.extend(search::route_entries(&e.domains.material().registry, &d.asset));
    entries.sort_by(|a, b| (&a.category, &a.name).cmp(&(&b.category, &b.name)));
    if let Some(link) = link {
        let t = e.pin_type(&link.node, &link.pin, link.from_output).unwrap();
        let end = if link.from_output { WireEnd::FromOutput(t) } else { WireEnd::FromInput(t) };
        entries.retain(|x| {
            search::compatible_pin(
                &e.domains.material().registry,
                &e.domains.material().functions,
                d.asset.kind,
                &x.kind,
                end,
            )
            .is_some()
        });
    }
    entries
}

/// The rows as the palette listed them before design §6.4 (plain search order).
pub(super) fn palette_rows_before(e: &Editor, link: Option<&Link>, query: &str) -> Vec<(String, String)> {
    let entries = palette_before(e, link);
    search::search(&entries, query).into_iter().map(|x| (x.kind.clone(), x.name.clone())).collect()
}

pub(super) fn scalar_output(e: &Editor) -> Link {
    let param = node_of(e, "param::Scalar");
    assert_eq!(e.pin_type(&param, "out", true), Some(PinType::Float));
    Link { node: param, pin: "out".into(), from_output: true }
}

/// Design §6.4: after a wire from an output the existing Named Route declaration entry wins ties on the score (first
/// with an empty query or "route", below better matches such as DotProduct for "dot"); entries and filters are
/// unchanged.
#[test]
fn output_wire_drop_palette_starts_with_named_route_declaration() {
    let (_dir, mut e) = project();
    e.apply(Action::OpenAsset(AssetId::new(AssetType::Material, "env/rock_wet").to_ref()));
    let link = scalar_output(&e);
    assert_eq!(crate::ui::palette::entries_for(&e, Some(&link)), palette_before(&e, Some(&link)), "same entries");
    let rows = palette_rows(&e, Some(&link), "");
    assert_eq!(rows[0].0, ROUTE_DECL);
    let before = palette_rows_before(&e, Some(&link), "");
    assert_ne!(before[0].0, ROUTE_DECL, "it was not first before");
    let rest: Vec<_> = before.iter().filter(|r| r.0 != ROUTE_DECL).cloned().collect();
    assert_eq!(rows[1..], rest[..], "the others keep their order");
    assert_eq!(palette_rows(&e, Some(&link), "route")[0].0, ROUTE_DECL);
    // A tie-break after the score: better matches stay above it ("dot" is a keyword of Reroute and DotProduct, a
    // mere subsequence of "namedroute"), in the plain order.
    let dot = palette_rows(&e, Some(&link), "dot");
    assert_ne!(dot[0].0, ROUTE_DECL, "{dot:?}");
    let at = |kind: &str| dot.iter().position(|r| r.0 == kind).unwrap();
    assert!(at("stdlib::DotProduct") < at(ROUTE_DECL), "{dot:?}");
    assert_eq!(dot, palette_rows_before(&e, Some(&link), "dot"));
    let mul = palette_rows(&e, Some(&link), "mul");
    assert!(!mul.iter().any(|r| r.0 == ROUTE_DECL), "the query filters it out: {mul:?}");
    assert_eq!(mul, palette_rows_before(&e, Some(&link), "mul"));
}

#[test]
fn input_wire_drop_palette_unchanged() {
    let (_dir, mut e) = project();
    e.apply(Action::OpenAsset(AssetId::new(AssetType::Material, "env/rock_wet").to_ref()));
    let out = node_of(&e, "output::Pbr3d");
    let link = Link { node: out, pin: "roughness".into(), from_output: false };
    assert_eq!(crate::ui::palette::entries_for(&e, Some(&link)), palette_before(&e, Some(&link)));
    for q in ["", "route", "mul"] {
        assert_eq!(palette_rows(&e, Some(&link), q), palette_rows_before(&e, Some(&link), q), "query {q:?}");
    }
    assert!(palette_rows(&e, Some(&link), "").iter().skip(1).any(|r| r.0 == ROUTE_DECL), "listed, not first");
}

#[test]
fn palette_without_link_unchanged() {
    let (_dir, mut e) = project();
    e.apply(Action::OpenAsset(AssetId::new(AssetType::Material, "env/rock_wet").to_ref()));
    assert_eq!(crate::ui::palette::entries_for(&e, None), palette_before(&e, None));
    for q in ["", "route", "mul"] {
        assert_eq!(palette_rows(&e, None, q), palette_rows_before(&e, None, q), "query {q:?}");
    }
    assert_ne!(palette_rows(&e, None, "")[0].0, ROUTE_DECL);
}

/// Design §6.4: a Named Route declaration picked in the palette (from an output wire or not) is added, wired from
/// the output in one undo step, and its rename dialog opens; cancelling keeps the default name.
#[test]
fn palette_named_route_declaration_opens_rename() {
    let (_dir, mut e) = project();
    e.apply(Action::OpenAsset(AssetId::new(AssetType::Material, "env/rock_wet").to_ref()));
    let output = scalar_output(&e);
    let pick = |e: &mut Editor, link: Option<&Link>, kind: &str| -> NodeId {
        let entries = palette_entries(e, link);
        let entry = entries.iter().find(|x| x.id == kind).unwrap().clone();
        let before: BTreeSet<NodeId> = e.doc().unwrap().asset.nodes.keys().cloned().collect();
        e.apply((entry.pick)(bevy::math::Vec2::new(123.0, 47.0)));
        let d = e.doc().unwrap();
        let new: Vec<_> = d.asset.nodes.keys().filter(|k| !before.contains(*k)).cloned().collect();
        assert_eq!(new.len(), 1, "{new:?}");
        new[0].clone()
    };
    let rename_open = |e: &Editor, route: &str| {
        let target = GraphProp::Param { node: route.to_string(), key: "name".into() };
        matches!(&e.dialog, Some(Dialog::Input { submit: Submit::SetText(t), initial, .. })
            if *t == target && initial == "route")
    };

    // From an output wire.
    let route = pick(&mut e, Some(&output), ROUTE_DECL);
    let d = e.doc().unwrap();
    assert_eq!(d.asset.nodes[&route].pos, (120.0, 50.0), "at the drop point, snapped");
    let edge = d.asset.edge_into(&route, "in").expect("wired from the output");
    assert_eq!(edge.from, (output.node.clone(), "out".to_string()));
    assert!(rename_open(&e, &route), "{:?}", e.dialog);
    let target = GraphProp::Param { node: route.clone(), key: "name".into() };
    // Cancel: the node keeps its default name.
    e.apply(Action::CloseDialog);
    assert_eq!(shown(&e, &target), Some(Value::Str("route".into())));
    // One undo step removes node and wire.
    e.apply(Action::Tab(TabAction::Undo));
    assert!(!e.doc().unwrap().asset.nodes.contains_key(&route));
    assert!(!e.is_dirty());

    // Without a link (Space / RMB): the dialog opens too; submitting renames it (its own undo step).
    let route = pick(&mut e, None, ROUTE_DECL);
    assert!(rename_open(&e, &route), "{:?}", e.dialog);
    let target = GraphProp::Param { node: route.clone(), key: "name".into() };
    e.apply(Action::Submit("wet".into()));
    assert_eq!(e.dialog, None);
    assert_eq!(shown(&e, &target), Some(Value::Str("wet".into())));
    e.apply(Action::Tab(TabAction::Undo));
    assert_eq!(shown(&e, &target), Some(Value::Str("route".into())));
    e.apply(Action::Tab(TabAction::Undo));
    assert!(!e.doc().unwrap().asset.nodes.contains_key(&route));

    // Other nodes open no dialog.
    pick(&mut e, Some(&output), "stdlib::Multiply");
    assert_eq!(e.dialog, None);
}

#[test]
fn named_routes_in_the_editor() {
    let (_dir, mut e) = project();
    e.apply(Action::OpenAsset(AssetId::new(AssetType::Material, "env/rock_wet").to_ref()));
    e.apply(MaterialAction::AddNode {
        kind: "stdlib::NamedRoute".into(),
        pos: (0.0, 0.0),
        link: None,
        params: Vec::new(),
    });
    let decl = e.doc().unwrap().selection.first().unwrap().clone();

    // The palette offers one usage entry per declaration, preset to it.
    let entries = search::route_entries(&e.domains.material().registry, &e.doc().unwrap().asset);
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].name, "Named Route Usage: route");
    let params = entries[0].params.clone();
    e.apply(MaterialAction::AddNode { kind: entries[0].kind.clone(), pos: (0.0, 0.0), link: None, params });
    let usage = e.doc().unwrap().selection.first().unwrap().clone();
    let d = e.doc().unwrap();
    assert_eq!(d.asset.nodes[&usage].params["route"], Value::Str(decl.clone()));
    assert_eq!(
        crate::doc::node_title(&e.domains.material().registry, &e.domains.material().functions, &d.asset, &usage),
        "route (Named Route Usage)"
    );

    // usage → declaration would close a loop through the route.
    let err = connection_error(
        &d.asset,
        (&usage, "out"),
        (&decl, "in"),
        &e.domains.material().registry,
        &e.domains.material().functions,
        &d.id.path,
    );
    assert!(err.unwrap().contains("E002"));

    // Copied with its declaration into the same graph, the declaration becomes a usage too and both keep the
    // original (decision 0038; a duplicated declaration in another graph is followed, see `doc` tests).
    e.apply(GraphEdit::Select([decl.clone(), usage.clone()].into()));
    e.apply(Action::Tab(TabAction::Duplicate));
    let d = e.doc().unwrap();
    assert_eq!(d.selection.len(), 2);
    for id in &d.selection {
        assert_eq!(d.asset.nodes[id].params["route"], Value::Str(decl.clone()));
    }
    e.apply(GraphEdit::Select([usage.clone()].into()));
    e.apply(Action::Tab(TabAction::Duplicate));
    let d = e.doc().unwrap();
    let copy = d.selection.first().unwrap();
    assert_eq!(d.asset.nodes[copy].params["route"], Value::Str(decl));
}
