//! Moving wires (decision 0032), comment frames, function calls, the graph statusbar.

use super::*;

pub(super) fn edges(e: &Editor) -> Vec<(NodeId, String, NodeId, String)> {
    let d = e.doc().unwrap();
    let mut v: Vec<_> =
        d.asset.edges.iter().map(|x| (x.from.0.clone(), x.from.1.clone(), x.to.0.clone(), x.to.1.clone())).collect();
    v.sort();
    v
}

pub(super) fn connect(e: &mut Editor, from: &NodeId, from_pin: &str, to: &NodeId, to_pin: &str) {
    e.apply(GraphEdit::Connect { from: (from.clone(), from_pin.into()), to: (to.clone(), to_pin.into()) });
    let source = e.doc().unwrap().asset.edge_into(to, to_pin).map(|x| x.from.clone());
    assert_eq!(source, Some((from.clone(), from_pin.into())));
}

pub(super) fn move_wires(node: &NodeId, pin: &str, output: bool, to: (&NodeId, &str)) -> Action {
    GraphEdit::MoveWires { node: node.clone(), pin: pin.into(), output, to: (to.0.clone(), to.1.into()) }.into()
}

#[test]
fn move_input_wire_to_another_input() {
    let (_dir, mut e) = project();
    e.apply(Action::OpenAsset(AssetId::new(AssetType::Material, "env/rock_wet").to_ref()));
    let (a, b, c) = (add(&mut e, "stdlib::Constant"), add(&mut e, "stdlib::Lerp"), add(&mut e, "stdlib::Lerp"));
    connect(&mut e, &a, "out", &b, "a");
    let before = edges(&e);
    e.apply(move_wires(&b, "a", false, (&c, "b")));
    let d = e.doc().unwrap();
    assert!(d.asset.edge_into(&b, "a").is_none());
    let moved = d.asset.edge_into(&c, "b").expect("moved");
    assert_eq!(moved.from, (a.clone(), "out".into()));
    assert_eq!(edges(&e).len(), before.len());
    e.apply(Action::Tab(TabAction::Undo));
    assert_eq!(edges(&e), before, "one undo step");
}

#[test]
fn move_output_wires_to_another_output() {
    let (_dir, mut e) = project();
    e.apply(Action::OpenAsset(AssetId::new(AssetType::Material, "env/rock_wet").to_ref()));
    let (a, d) = (add(&mut e, "stdlib::Constant"), add(&mut e, "stdlib::Constant"));
    let (b, c) = (add(&mut e, "stdlib::Lerp"), add(&mut e, "stdlib::Lerp"));
    connect(&mut e, &a, "out", &b, "a");
    connect(&mut e, &a, "out", &c, "b");
    let before = edges(&e);
    e.apply(move_wires(&a, "out", true, (&d, "out")));
    let asset = &e.doc().unwrap().asset;
    assert!(!asset.edges.iter().any(|x| x.from.0 == a), "{:?}", asset.edges);
    assert_eq!(asset.edge_into(&b, "a").unwrap().from, (d.clone(), "out".into()));
    assert_eq!(asset.edge_into(&c, "b").unwrap().from, (d.clone(), "out".into()));
    e.apply(Action::Tab(TabAction::Undo));
    assert_eq!(edges(&e), before, "one undo step");
}

#[test]
fn move_wires_rejects_incompatible() {
    let (_dir, mut e) = project();
    e.apply(Action::OpenAsset(AssetId::new(AssetType::Material, "env/rock_wet").to_ref()));
    let out = output_node(&e);
    let (v3, lerp) = (add(&mut e, "stdlib::Constant3Vector"), add(&mut e, "stdlib::Lerp"));
    let make = add(&mut e, "stdlib::MakeMaterialAttributesPbr");
    connect(&mut e, &v3, "out", &lerp, "a");
    connect(&mut e, &v3, "out", &out, "base_color");
    let (before, rev) = (edges(&e), e.doc().unwrap().revision);
    // The Lerp takes material attributes, `base_color` does not: nothing moves.
    e.apply(move_wires(&v3, "out", true, (&make, "attributes")));
    assert_eq!(edges(&e), before);
    assert_eq!(e.doc().unwrap().revision, rev, "no edit recorded");
    let last = e.messages.last().expect("a message");
    assert_eq!(last.level, Level::Warning);
    assert!(last.text.contains("base_color"), "names the failing wire: {}", last.text);
}

#[test]
fn move_wires_rejects_direction_mismatch() {
    let (_dir, mut e) = project();
    e.apply(Action::OpenAsset(AssetId::new(AssetType::Material, "env/rock_wet").to_ref()));
    let (a, b, c) = (add(&mut e, "stdlib::Constant"), add(&mut e, "stdlib::Lerp"), add(&mut e, "stdlib::Constant"));
    connect(&mut e, &a, "out", &b, "a");
    let (before, rev) = (edges(&e), e.doc().unwrap().revision);
    e.apply(move_wires(&b, "a", false, (&c, "out")));
    assert_eq!(edges(&e), before, "input wire onto an output");
    e.apply(move_wires(&a, "out", true, (&b, "b")));
    assert_eq!(edges(&e), before, "output wires onto an input");
    e.apply(move_wires(&b, "a", false, (&b, "a")));
    assert_eq!(edges(&e), before, "onto itself");
    assert_eq!(e.doc().unwrap().revision, rev);
}

/// A move that would close a loop changes nothing, also when only a later wire of an output does (Review Focus 4).
#[test]
fn move_wires_rejects_cycle() {
    let (_dir, mut e) = project();
    e.apply(Action::OpenAsset(AssetId::new(AssetType::Material, "env/rock_wet").to_ref()));
    let reject = |e: &mut Editor, action: Action, what: &str| {
        let (before, rev, n) = (edges(e), e.doc().unwrap().revision, e.messages.len());
        e.apply(action);
        assert_eq!(edges(e), before, "{what}");
        assert_eq!(e.doc().unwrap().revision, rev, "{what}: no edit recorded");
        assert_eq!(e.messages.len(), n + 1, "{what}: one message");
        let last = e.messages.last().unwrap();
        assert_eq!(last.level, Level::Warning, "{what}");
        assert!(last.text.contains("E002"), "{what}: {}", last.text);
    };

    // Input: l1 → l2 → l3.a; moving l3.a's wire (from l2) onto l1.b closes l1 → l2 → l1.
    let (c, l1, l2, l3) = (
        add(&mut e, "stdlib::Constant"),
        add(&mut e, "stdlib::Lerp"),
        add(&mut e, "stdlib::Lerp"),
        add(&mut e, "stdlib::Lerp"),
    );
    connect(&mut e, &c, "out", &l1, "a");
    connect(&mut e, &l1, "out", &l2, "a");
    connect(&mut e, &l2, "out", &l3, "a");
    reject(&mut e, move_wires(&l3, "a", false, (&l1, "b")), "input move");

    // Output: k.out feeds two Lerps; the one whose wire is checked second (edge order) feeds z. Moving k's wires onto
    // z.out: the first rewired wire is fine, the second closes second → z → second.
    let (k, p, q, z) = (
        add(&mut e, "stdlib::Constant"),
        add(&mut e, "stdlib::Lerp"),
        add(&mut e, "stdlib::Lerp"),
        add(&mut e, "stdlib::Lerp"),
    );
    connect(&mut e, &k, "out", &p, "a");
    connect(&mut e, &k, "out", &q, "a");
    let order: Vec<_> = e.doc().unwrap().asset.edges.iter().filter(|w| w.from.0 == k).map(|w| w.to.0.clone()).collect();
    assert_eq!(order.len(), 2);
    connect(&mut e, &order[1], "out", &z, "a");
    reject(&mut e, move_wires(&k, "out", true, (&z, "out")), "output move");
}

/// The wire an input move replaces is not part of the check: `Add.a` fed a Float4 takes a Float moved onto it.
#[test]
fn move_wires_validates_without_replaced_wire() {
    let (_dir, mut e) = project();
    e.apply(Action::OpenAsset(AssetId::new(AssetType::Material, "env/rock_wet").to_ref()));
    let (v4, sum) = (add(&mut e, "stdlib::Constant4Vector"), add(&mut e, "stdlib::Add"));
    let (f, lerp) = (add(&mut e, "stdlib::Constant"), add(&mut e, "stdlib::Lerp"));
    connect(&mut e, &v4, "out", &sum, "a");
    connect(&mut e, &f, "out", &lerp, "b");
    let before = edges(&e);
    let from = (lerp.clone(), "b".to_string());
    let to = (sum.clone(), "a".to_string());
    let planned = e.plan_move_wires(&from, false, &to).expect("the replaced Float4 wire is ignored");
    assert_eq!(planned.edge_into(&sum, "a").unwrap().from, (f.clone(), "out".into()));
    assert!(planned.edge_into(&lerp, "b").is_none());
    assert!(!planned.edges.iter().any(|x| x.from.0 == v4), "the Float4 wire is replaced");
    e.apply(move_wires(&lerp, "b", false, (&sum, "a")));
    assert_eq!(e.doc().unwrap().asset.edges, planned.edges, "applies the plan");
    e.apply(Action::Tab(TabAction::Undo));
    assert_eq!(edges(&e), before, "one undo step");
}

/// The dry run a Ctrl+drag repeats while it hovers pins changes nothing and reports nothing.
#[test]
fn plan_move_wires_is_pure() {
    let (_dir, mut e) = project();
    e.apply(Action::OpenAsset(AssetId::new(AssetType::Material, "env/rock_wet").to_ref()));
    let (a, b, c) = (add(&mut e, "stdlib::Constant"), add(&mut e, "stdlib::Lerp"), add(&mut e, "stdlib::Lerp"));
    connect(&mut e, &a, "out", &b, "a");
    let (before, rev, messages, props) = (edges(&e), e.doc().unwrap().revision, e.messages.len(), e.ui.props);
    let ok = e.plan_move_wires(&(b.clone(), "a".into()), false, &(c.clone(), "b".into()));
    assert!(ok.is_ok_and(|x| x.edge_into(&c, "b").is_some()));
    let refused = e.plan_move_wires(&(b.clone(), "a".into()), false, &(a.clone(), "out".into()));
    assert!(refused.is_err_and(|why| why.contains("is not an input")));
    assert_eq!(edges(&e), before);
    assert_eq!(e.doc().unwrap().revision, rev, "no revision bump");
    assert_eq!((e.messages.len(), e.ui.props), (messages, props), "no message, no redraw");
}

/// The statusbar's WESL line counts follow the asset: moved on rename, dropped on delete.
#[test]
fn wesl_lines_follow_delete_and_rename() {
    let (_dir, mut e) = project();
    e.regenerate();
    let id = AssetId::new(AssetType::Material, "env/rock_wet");
    assert!(e.domains.material().wesl_lines.contains_key(&id));
    e.apply(Action::MoveTo(id.to_ref(), "env2".into()));
    let moved = AssetId::new(AssetType::Material, "env2/rock_wet");
    assert!(!e.domains.material().wesl_lines.contains_key(&id), "the old id has no entry");
    assert!(e.domains.material().wesl_lines.contains_key(&moved), "the new id has one");
    e.apply(Action::Delete(moved.to_ref()));
    assert!(!e.domains.material().wesl_lines.contains_key(&moved), "dropped on delete");
}

/// The editor statusbar keys on `ui.tabs` instead of the tab's config key: another tab at the same index, or the
/// same tab renamed, must bump it.
#[test]
fn tabs_revision_follows_tab_identity() {
    let (_dir, mut e) = project();
    e.apply(Action::OpenAsset(AssetId::new(AssetType::Material, "env/rock_wet").to_ref()));
    let (active, rev) = (e.active, e.ui.tabs);
    e.apply(Action::CloseTab(0));
    e.apply(Action::OpenAsset(pulse().to_ref()));
    assert_eq!(e.active, active, "same index");
    assert_ne!(e.ui.tabs, rev, "another tab");
    let rev = e.ui.tabs;
    e.apply(Action::MoveTo(pulse().to_ref(), "moved".into()));
    assert_ne!(e.ui.tabs, rev, "renamed");
}

#[test]
fn connect_replaces_existing_input_edge() {
    let (_dir, mut e) = project();
    e.apply(Action::OpenAsset(AssetId::new(AssetType::Material, "env/rock_wet").to_ref()));
    let (a, b, c) = (add(&mut e, "stdlib::Constant"), add(&mut e, "stdlib::Lerp"), add(&mut e, "stdlib::Constant"));
    connect(&mut e, &a, "out", &b, "a");
    connect(&mut e, &c, "out", &b, "a");
    let into: Vec<_> = e.doc().unwrap().asset.edges.iter().filter(|x| x.to == (b.clone(), "a".into())).collect();
    assert_eq!(into.len(), 1);
    assert_eq!(into[0].from, (c.clone(), "out".into()));
}

/// Three new Constant nodes in rock_wet and a comment frame around the first `framed` of them; returns the node
/// ids and the frame id.
pub(super) fn framed_nodes(e: &mut Editor, framed: &[usize]) -> (Vec<NodeId>, String) {
    open_rock_wet(e);
    let mut ids = Vec::new();
    for i in 0..3 {
        let before: Vec<_> = e.doc().unwrap().asset.nodes.keys().cloned().collect();
        let pos = (-900.0 + 200.0 * i as f32, 600.0);
        e.apply(MaterialAction::AddNode { kind: "stdlib::Constant".into(), pos, link: None, params: Vec::new() });
        ids.push(e.doc().unwrap().asset.nodes.keys().find(|k| !before.contains(k)).unwrap().clone());
    }
    let frames: Vec<_> = e.doc().unwrap().asset.comments.iter().map(|f| f.id.clone()).collect();
    e.apply(GraphEdit::Select(framed.iter().map(|i| ids[*i].clone()).collect()));
    e.apply(GraphEdit::AddComment(None));
    let f = e.doc().unwrap().asset.comments.iter().find(|f| !frames.contains(&f.id)).unwrap().id.clone();
    (ids, f)
}

pub(super) fn has_frame(e: &Editor, f: &str) -> bool {
    e.doc().unwrap().asset.comments.iter().any(|c| c.id == f)
}

#[test]
fn deleting_nodes_with_enclosing_frame_removes_it() {
    let (_dir, mut e) = project();
    let (ids, f) = framed_nodes(&mut e, &[0, 1]);
    let (a, b) = (ids[0].clone(), ids[1].clone());
    e.apply(GraphEdit::Select([a.clone(), b.clone()].into()));
    e.apply(GraphEdit::SelectFrames([f.clone()].into()));
    assert_eq!(e.doc().unwrap().enclosed_frames, BTreeSet::from([f.clone()]));
    e.apply(Action::Tab(TabAction::DeleteSelection));
    let d = e.doc().unwrap();
    assert!(!d.asset.nodes.contains_key(&a) && !d.asset.nodes.contains_key(&b));
    assert!(!has_frame(&e, &f));
    assert!(e.doc().unwrap().enclosed_frames.is_empty());
    // One undo step brings back nodes and frame together (Review Focus 4).
    e.apply(Action::Tab(TabAction::Undo));
    let d = e.doc().unwrap();
    assert!(d.asset.nodes.contains_key(&a) && d.asset.nodes.contains_key(&b));
    assert!(has_frame(&e, &f));
    let frame = d.asset.comments.iter().find(|c| c.id == f).unwrap();
    let mut members = vec![a, b];
    members.sort();
    assert_eq!(frame.nodes, members);
}

#[test]
fn frame_kept_when_not_all_nodes_selected() {
    let (_dir, mut e) = project();
    let (ids, f) = framed_nodes(&mut e, &[0, 2]);
    let (a, b, c) = (ids[0].clone(), ids[1].clone(), ids[2].clone());
    e.apply(GraphEdit::Select([a.clone(), b.clone()].into()));
    e.apply(GraphEdit::SelectFrames([f.clone()].into()));
    assert!(e.doc().unwrap().enclosed_frames.is_empty(), "c of the frame is not selected");
    e.apply(Action::Tab(TabAction::DeleteSelection));
    let d = e.doc().unwrap();
    assert!(!d.asset.nodes.contains_key(&a) && !d.asset.nodes.contains_key(&b));
    assert!(d.asset.nodes.contains_key(&c));
    assert!(has_frame(&e, &f));
}

/// A marked frame whose member survives the delete (output nodes are never deleted) stays (decision 0033).
#[test]
fn frame_kept_when_a_member_survives_delete() {
    let (_dir, mut e) = project();
    open_rock_wet(&mut e);
    let (out, a) = (output_node(&e), add(&mut e, "stdlib::Constant"));
    let frames: Vec<_> = e.doc().unwrap().asset.comments.iter().map(|f| f.id.clone()).collect();
    e.apply(GraphEdit::Select([out.clone(), a.clone()].into()));
    e.apply(GraphEdit::AddComment(None));
    let f = e.doc().unwrap().asset.comments.iter().find(|f| !frames.contains(&f.id)).unwrap().id.clone();
    let before = e.doc().unwrap().asset.clone();
    e.apply(GraphEdit::Select([out.clone(), a.clone()].into()));
    e.apply(GraphEdit::SelectFrames([f.clone()].into()));
    assert_eq!(e.doc().unwrap().enclosed_frames, BTreeSet::from([f.clone()]));
    e.apply(Action::Tab(TabAction::DeleteSelection));
    let d = e.doc().unwrap();
    assert!(!d.asset.nodes.contains_key(&a), "the other node is deleted");
    assert!(d.asset.nodes.contains_key(&out), "the output node is never deleted");
    assert!(has_frame(&e, &f), "its frame stays");
    e.apply(Action::Tab(TabAction::Undo));
    assert_eq!(e.doc().unwrap().asset, before, "one undo step");
}

#[test]
fn select_clears_enclosed_frames() {
    let (_dir, mut e) = project();
    let (ids, f) = framed_nodes(&mut e, &[0, 1]);
    e.apply(GraphEdit::Select([ids[0].clone(), ids[1].clone()].into()));
    e.apply(GraphEdit::SelectFrames([f.clone()].into()));
    e.apply(GraphEdit::Select([ids[0].clone(), ids[1].clone()].into()));
    assert!(e.doc().unwrap().enclosed_frames.is_empty(), "a new node selection drops the mark");
    e.apply(Action::Tab(TabAction::DeleteSelection));
    assert!(has_frame(&e, &f));
    // Without nodes a frame is never marked: clicking a frame selects only the frame (design §4.5).
    e.apply(GraphEdit::Select(BTreeSet::new()));
    e.apply(GraphEdit::SelectFrames([f.clone()].into()));
    assert!(e.doc().unwrap().enclosed_frames.is_empty());
}

#[test]
fn frame_hue_edit_is_undoable() {
    let (_dir, mut e) = project();
    let (_, f) = framed_nodes(&mut e, &[0]);
    let hue = |e: &Editor| e.doc().unwrap().asset.comments.iter().find(|c| c.id == f).unwrap().hue;
    assert_eq!(hue(&e), None);
    e.apply(set_value(GraphProp::FrameHue(f.clone()), Value::Float(200.0), false));
    assert_eq!(hue(&e), Some(200.0));
    e.apply(Action::Tab(TabAction::Undo));
    assert_eq!(hue(&e), None);

    // A Hue drag is one undo step; "Default colour" clears the hue.
    for (v, is_final) in [(10.0, false), (20.0, false), (30.0, true)] {
        e.apply(set_component(GraphProp::FrameHue(f.clone()), 0, v, is_final));
    }
    assert_eq!(hue(&e), Some(30.0));
    e.apply(set_value(GraphProp::FrameHue(f.clone()), Value::Str(String::new()), false));
    assert_eq!(hue(&e), None);
    e.apply(Action::Tab(TabAction::Undo));
    assert_eq!(hue(&e), Some(30.0));
    e.apply(Action::Tab(TabAction::Undo));
    assert_eq!(hue(&e), None, "the drag was one step");
}

/// Design §4.10: `min` / `max` set on a Float FunctionInput (unsaved, in the open tab) reach the call node's pin.
#[test]
fn function_input_range_reaches_call_pin() {
    let (_dir, mut e) = project();
    e.apply(Action::OpenAsset(AssetId::new(AssetType::Function, "common/wet_mask").to_ref()));
    let input = e
        .doc()
        .unwrap()
        .asset
        .nodes
        .iter()
        .find(|(_, n)| n.kind == "output::FunctionInput" && n.params.get("name") == Some(&Value::Str("wetness".into())))
        .unwrap()
        .0
        .clone();
    for (key, v) in [("min", 0.2), ("max", 5.0)] {
        e.apply(set_value(GraphProp::Param { node: input.clone(), key: key.into() }, Value::Float(v), false));
    }
    e.apply(Action::OpenAsset(AssetId::new(AssetType::Material, "env/rock_wet").to_ref()));
    // Live validation refreshes the declared function signatures from the open tabs.
    e.validate();
    let call = node_of(&e, "function::common/wet_mask");
    let d = e.doc().unwrap();
    let sig =
        node_signature(&d.asset, &call, &e.domains.material().registry, &e.domains.material().functions, &d.id.path)
            .unwrap();
    assert_eq!(sig.input("wetness").unwrap().range, Some((0.2, 5.0)));
    assert_eq!(sig.input("base_color").unwrap().range, None, "Float3 input");
}

/// Review Focus 5: every sample function file (FunctionInputs without `min` / `max`) loads and saves byte-identical.
#[test]
fn function_files_unchanged() {
    let sample = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/sample_game");
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("Cargo.toml"), "[package]\nname = \"g\"\n").unwrap();
    std::fs::create_dir_all(dir.path().join("assets")).unwrap();
    let p = Project::open(dir.path()).unwrap();
    let mut files = Vec::new();
    let mut stack = vec![sample.join("quillshade/material")];
    while let Some(d) = stack.pop() {
        for entry in std::fs::read_dir(&d).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                stack.push(path);
            } else if path.to_string_lossy().ends_with(".function.ron") {
                files.push(path);
            }
        }
    }
    assert!(files.len() >= 4, "{files:?}");
    for file in files {
        let rel = file.strip_prefix(sample.join("quillshade/material")).unwrap().to_string_lossy().replace('\\', "/");
        let id = AssetId::new(AssetType::Function, rel.trim_end_matches(".function.ron"));
        let original = std::fs::read(&file).unwrap();
        std::fs::create_dir_all(id.file(&p).parent().unwrap()).unwrap();
        std::fs::write(id.file(&p), &original).unwrap();
        let asset = assets::load(&p, &id).unwrap();
        ops::save(&p, &id, &asset).unwrap();
        assert_eq!(std::fs::read(id.file(&p)).unwrap(), original, "{rel}");
    }
}

/// The comment frame `AddComment` added (the one not in `before`).
pub(super) fn new_frame(e: &Editor, before: &[String]) -> shader_graph_core::model::Frame {
    e.doc().unwrap().asset.comments.iter().find(|f| !before.contains(&f.id)).cloned().expect("a new comment frame")
}

pub(super) fn frame_ids(e: &Editor) -> Vec<String> {
    e.doc().unwrap().asset.comments.iter().map(|f| f.id.clone()).collect()
}

/// Design §6.1: with nothing selected, a comment is a 400x300 frame at the cursor snapped to the grid, around no
/// node (not around the whole graph), in one undo step.
#[test]
fn add_comment_without_selection_is_default_size_at_cursor() {
    let (_dir, mut e) = project();
    open_rock_wet(&mut e);
    assert!(e.doc().unwrap().asset.nodes.len() >= 5);
    e.apply(GraphEdit::Select(BTreeSet::new()));
    let before = frame_ids(&e);
    e.apply(GraphEdit::AddComment(Some((12.3, -7.6))));
    let f = new_frame(&e, &before);
    assert_eq!(f.rect, (10.0, -10.0, 400.0, 300.0));
    assert!(f.nodes.is_empty(), "{:?}", f.nodes);
    e.apply(Action::Tab(TabAction::Undo));
    assert_eq!(frame_ids(&e), before, "one undo step");
    assert!(!e.is_dirty());
    // Without a cursor: at the origin.
    e.apply(GraphEdit::AddComment(None));
    assert_eq!(new_frame(&e, &before).rect, (0.0, 0.0, 400.0, 300.0));
}

/// A comment with a selection wraps the selected nodes; the cursor does not matter.
#[test]
fn add_comment_with_selection_wraps_it() {
    let (_dir, mut e) = project();
    open_rock_wet(&mut e);
    let a = add(&mut e, "stdlib::Constant");
    e.apply(MaterialAction::AddNode {
        kind: "stdlib::Time".into(),
        pos: (-500.0, 200.0),
        link: None,
        params: Vec::new(),
    });
    let b = e.doc().unwrap().asset.nodes.iter().find(|(_, n)| n.kind == "stdlib::Time").unwrap().0.clone();
    let (pa, pb) = {
        let d = e.doc().unwrap();
        (d.asset.nodes[&a].pos, d.asset.nodes[&b].pos)
    };
    e.apply(GraphEdit::Select([a.clone(), b.clone()].into()));
    let before = frame_ids(&e);
    e.apply(GraphEdit::AddComment(Some((5000.0, 5000.0))));
    let f = new_frame(&e, &before);
    let (x0, y0, x1, y1) = (pa.0.min(pb.0), pa.1.min(pb.1), pa.0.max(pb.0), pa.1.max(pb.1));
    assert_eq!(f.rect, (x0 - 30.0, y0 - 50.0, x1 - x0 + 260.0, y1 - y0 + 220.0));
    let mut members = vec![a, b];
    members.sort();
    assert_eq!(f.nodes, members);
}

/// Design §5: the statusbar counts wires and comments too (the sidebar's "Info" subpane is gone).
#[test]
fn graph_status_counts_wires_and_comments() {
    let (_dir, mut e) = project();
    open_rock_wet(&mut e);
    let counts = e.status_info().unwrap().counts;
    let a = &e.doc().unwrap().asset;
    let (nodes, wires, comments) = (a.nodes.len(), a.edges.len(), a.comments.len());
    for want in [
        crate::util::text::plural(nodes, "node", "nodes"),
        crate::util::text::plural(wires, "wire", "wires"),
        crate::util::text::plural(comments, "comment", "comments"),
    ] {
        assert!(counts.contains(&want), "{want} in {counts:?}");
    }
    e.apply(GraphEdit::Select(BTreeSet::new()));
    e.apply(GraphEdit::AddComment(None));
    let want = crate::util::text::plural(comments + 1, "comment", "comments");
    assert!(e.status_info().unwrap().counts.contains(&want));
}
