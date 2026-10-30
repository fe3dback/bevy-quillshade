//! Material's look of the bevy_noodle canvas (design §7.3): pin colours by type, header colours by category or name.
//! The canvas itself (attach, rebuild, gestures, outlines, view requests) is `node_graph_base`'s, driven through
//! [`crate::graph_domain::MaterialGraph`]; its pieces are re-exported here.

use bevy::prelude::*;
use shader_graph_core::PinType;
use shader_graph_core::model::{Asset, NodeId};
use shader_graph_core::registry::{NodeRole, ParamKind, Registry};

use crate::doc::{has_label, usage_route};

pub use node_graph_base::PinKey;
pub use node_graph_base::canvas::{
    CanvasSystems, EditorCanvasMarker, FrameKey, FrameTitleStrip, NAV_SETTLE_SECS, NodeHeader, NodeKey,
    PIN_HOVER_LIGHTEN, PIN_HOVER_SCALE, PIN_PHYSICAL, PIN_VISUAL, PinLabel, PreviewPulse, SELECTED, WIRE_TARGET_RADIUS,
    node_outline, pulse_color, view_center, window_to_graph,
};

/// Material's canvas ([`node_graph_base::EditorCanvas`] of [`crate::graph_domain::MaterialGraph`]).
pub type EditorCanvas = node_graph_base::EditorCanvas<crate::graph_domain::MaterialGraph>;

pub fn pin_color(t: PinType) -> Color {
    let [r, g, b] = t.color();
    Color::srgb(r, g, b)
}

pub(crate) fn category_color(cat: &str) -> Color {
    match cat {
        "Constants" => Color::srgb_u8(70, 98, 58),
        "Parameters" => Color::srgb_u8(48, 104, 82),
        "Math" => Color::srgb_u8(66, 84, 60),
        "Vector" => Color::srgb_u8(82, 78, 52),
        "Coordinates" => Color::srgb_u8(110, 58, 58),
        "Texture" => Color::srgb_u8(50, 74, 120),
        "Scene" => Color::srgb_u8(40, 92, 104),
        "Utility" => Color::srgb_u8(74, 74, 80),
        "Color" => Color::srgb_u8(104, 62, 96),
        "Output" => Color::srgb_u8(90, 64, 34),
        "Function" | "Functions" => Color::srgb_u8(86, 60, 110),
        _ => Color::srgb_u8(60, 63, 71),
    }
}

/// Hue in degrees from a stable hash of a name (FNV-1a 32-bit of its UTF-8 bytes), design §4.2.
pub(crate) fn name_hue(name: &str) -> f32 {
    let hash = name.bytes().fold(2166136261u32, |h, b| (h ^ b as u32).wrapping_mul(16777619));
    (hash % 360) as f32
}

/// Header colour of a node named by the user: fixed saturation / lightness, hue from the name (design §4.2).
pub(crate) fn name_header_color(name: &str) -> Color {
    Color::hsl(name_hue(name), 0.45, 0.32)
}

/// Name a node's header colour comes from (design §4.2): a named route's name (usages: their declaration's, from
/// `routes`), a constant's user title, a `Global*` node's constant. `None`: the category colour.
fn header_name(registry: &Registry, asset: &Asset, id: &str, routes: &[(NodeId, String)]) -> Option<String> {
    let n = asset.nodes.get(id)?;
    let def = registry.get(&n.kind)?;
    let params = registry.resolved_params(&n.kind, &n.params);
    let name = match def.role() {
        NodeRole::RouteDeclaration => params.get("name").map(|v| v.as_str().to_string()),
        NodeRole::RouteUsage => usage_route(routes, &params).map(str::to_string),
        _ if has_label(&n.kind) => Some(n.label.clone()),
        _ => def
            .params()
            .into_iter()
            .find(|p| matches!(p.kind, ParamKind::GlobalConst(_)))
            .and_then(|p| params.get(&p.id).map(|v| v.as_str().to_string())),
    };
    name.filter(|s| !s.is_empty())
}

/// Header colour of a node: by name (see [`header_name`]) or by category.
pub(crate) fn header_color(registry: &Registry, asset: &Asset, id: &str, routes: &[(NodeId, String)]) -> Color {
    if let Some(name) = header_name(registry, asset, id, routes) {
        return name_header_color(&name);
    }
    let cat = asset.nodes.get(id).and_then(|n| registry.get(&n.kind)).map(|d| d.info().category.clone());
    category_color(cat.as_deref().unwrap_or("Unknown"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::doc::route_declarations;
    use bevy::ui::Selected;
    use shader_graph_core::Value;
    use shader_graph_core::model::AssetKind;
    use shader_graph_core::registry::ROUTE_PARAM;

    #[test]
    fn name_hue_is_stable() {
        let h = name_hue("albedo");
        assert_eq!(h, name_hue("albedo"));
        assert!((0.0..360.0).contains(&h), "{h}");
        assert_ne!(h, name_hue("normal"));
        // FNV-1a of no bytes is the offset basis: pins the algorithm.
        assert_eq!(name_hue(""), (2166136261u32 % 360) as f32);
    }

    fn material() -> Asset {
        use shader_graph_core::model::{MaterialKind, PbrSettings};
        let mut a = Asset::new("m", AssetKind::Material(MaterialKind::Pbr3d(PbrSettings::default())));
        a.nodes.clear();
        a
    }

    /// [`header_color`] with the asset's routes.
    fn color(reg: &Registry, a: &Asset, id: &str) -> Color {
        header_color(reg, a, id, &route_declarations(a, reg))
    }

    #[test]
    fn route_usage_shares_declaration_header() {
        use shader_graph_core::model::Node;
        let reg = shader_stdlib::registry();
        let mut a = material();
        let decl = a.add_node(Node::new("stdlib::NamedRoute", (0.0, 0.0)).with_param("name", Value::Str("wet".into())));
        let usage = a.add_node(
            Node::new("stdlib::NamedRouteUsage", (0.0, 0.0)).with_param(ROUTE_PARAM, Value::Str(decl.clone())),
        );
        let c = color(&reg, &a, &decl);
        assert_eq!(c, name_header_color("wet"));
        assert_eq!(color(&reg, &a, &usage), c);
        // A usage of no declaration keeps the category colour.
        let lost = a.add_node(Node::new("stdlib::NamedRouteUsage", (0.0, 0.0)));
        assert_eq!(color(&reg, &a, &lost), category_color("Utility"));
    }

    #[test]
    fn unlabelled_constant_keeps_category_colour() {
        use shader_graph_core::model::Node;
        let reg = shader_stdlib::registry();
        let mut a = material();
        let plain = a.add_node(Node::new("stdlib::Constant", (0.0, 0.0)));
        assert_eq!(color(&reg, &a, &plain), category_color("Constants"));
        let mut labelled = Node::new("stdlib::Constant3Vector", (0.0, 0.0));
        labelled.label = "tint".into();
        let labelled = a.add_node(labelled);
        assert_eq!(color(&reg, &a, &labelled), name_header_color("tint"));
        // Global*: by the constant's name, set or not.
        let global = a.add_node(
            Node::new("stdlib::GlobalConstF32", (0.0, 0.0)).with_param("const", Value::Str("roughness_base".into())),
        );
        assert_eq!(color(&reg, &a, &global), name_header_color("roughness_base"));
        let unset = a.add_node(Node::new("stdlib::GlobalConstF32", (0.0, 0.0)));
        assert_eq!(color(&reg, &a, &unset), category_color("Constants"));
    }

    /// The system itself, not only [`node_outline`]: a selected node gets the 3 px selection outline, the previewed
    /// one the 6 px pulse, a plain node none (a stale outline is removed).
    #[test]
    fn node_outlines_selected_and_previewed() {
        use bevy::ecs::system::RunSystemOnce;
        let mut world = World::new();
        world.insert_resource(crate::test_editor());
        world.insert_resource(Time::<()>::default());
        // Material's canvas entities.
        let ours = node_graph_base::GraphOf(crate::editors::graph::GRAPH);
        let selected = world.spawn((NodeKey("a".into()), ours, Selected)).id();
        let previewed = world.spawn((NodeKey("b".into()), ours, PreviewPulse)).id();
        let plain = world.spawn((NodeKey("c".into()), ours)).id();
        let stale = world.spawn((NodeKey("d".into()), ours, Outline::new(px(3), px(0), SELECTED))).id();
        world.run_system_once(node_graph_base::canvas::node_outlines::<crate::graph_domain::MaterialGraph>).unwrap();
        let outline = |e: Entity| world.get::<Outline>(e).copied();
        let o = outline(selected).expect("a selected node is outlined");
        assert_eq!((o.width, o.offset, o.color), (px(3), px(0), SELECTED));
        let o = outline(previewed).expect("the previewed node is outlined");
        assert_eq!((o.width, o.offset, o.color), (px(6), px(0), pulse_color(0.0)));
        assert!(outline(plain).is_none());
        assert!(outline(stale).is_none(), "an outline nothing asks for is removed");
    }
}
