//! Ready-made scenes for the default look: plain functions returning
//! [`Scene`]s, composed with Bevy's [`bsn!`] macro.
//!
//! ```no_run
//! use bevy::prelude::*;
//! use bevy_noodle::{prelude::*, style::kit};
//! const NUMBER: PortType = PortType::named("number");
//! const BLUE: Color = Color::srgb(0.3, 0.6, 1.0);
//! # let mut world = World::new();
//! # let mut commands = world.commands();
//! let canvas = commands.spawn_scene(kit::canvas()).id();
//! commands.spawn_scene(bsn! {
//!     @kit::node(Vec2::new(40.0, 40.0))
//!     // Patch single fields; the others keep the kit's values.
//!     Node { min_width: px(220) }
//!     ChildOf(canvas)
//!     Children [
//!         @kit::title("Add")
//!         --
//!         @kit::row("a", Port::input(NUMBER), BLUE)
//!         --
//!         @kit::row("sum", Port::output(NUMBER), BLUE)
//!     ]
//! });
//! ```
//!
//! A node is a padded column: its title, then rows. Any other content, such
//! as feathers controls, can go between them and lines up with the rows.

use bevy::picking::Pickable;
use bevy::picking::cursor::EntityCursor;
use bevy::prelude::*;
use bevy::window::SystemCursorIcon;

use super::{
    CanvasGrid, EdgeStyle, FocusOutline, PortColor, PortDot, PortHighlight, SelectedBorderColor, SelectionBoxStyle,
};
use crate::components::{GraphNode, NodeCanvas, NodePosition, Port, PortDirection};
use crate::interaction::CanvasInteraction;

const BORDER: f32 = 1.5;
/// Horizontal padding inside a node.
const PADDING: f32 = 10.0;
/// The diameter of a [`port`] dot, border included, in logical pixels.
pub const PORT_SIZE: f32 = 12.0;
/// The border width of a [`port`] dot, in logical pixels.
pub const PORT_BORDER: f32 = 1.5;

/// An interactive canvas filling its parent, with the whole default look.
///
/// It clips its content, which also lets Bevy skip drawing nodes and edges
/// that are out of view.
pub fn canvas() -> impl Scene {
    bsn! {
        NodeCanvas
        CanvasInteraction
        Node { width: percent(100), height: percent(100), overflow: Overflow::clip() }
        EdgeStyle
        CanvasGrid
        SelectionBoxStyle
        FocusOutline
    }
}

/// A node frame at `position` whose border follows selection: a padded
/// column of its title and rows, given as its `Children`.
pub fn node(position: Vec2) -> impl Scene {
    bsn! {
        GraphNode
        NodePosition(position)
        Node {
            flex_direction: FlexDirection::Column,
            row_gap: px(6),
            min_width: px(160),
            padding: UiRect::new(px(PADDING), px(PADDING), px(0), px(8)),
            border: UiRect::all(px(BORDER)),
            border_radius: BorderRadius::all(px(8)),
        }
        BackgroundColor(Color::srgb_u8(44, 46, 52))
        SelectedBorderColor
        BoxShadow::new(Color::srgba(0.0, 0.0, 0.0, 0.45), px(0), px(4), px(0), px(14))
        // Controls inside keep their own cursors.
        EntityCursor::System(SystemCursorIcon::Grab)
    }
}

/// A title bar spanning the node's width.
pub fn title(text: impl Into<String>) -> impl Scene {
    let text = text.into();
    bsn! {
        Text(text)
        TextFont { font_size: FontSize::Px(14.0) }
        Node {
            padding: UiRect::axes(px(PADDING), px(5)),
            // Out over the node's padding, to its border.
            margin: UiRect::new(px(-PADDING), px(-PADDING), px(0), px(2)),
            border_radius: BorderRadius::top(px(6.5)),
        }
        BackgroundColor(Color::srgb_u8(60, 63, 71))
    }
}

/// A port dot centered on the node's border, on the side its direction faces.
pub fn port(port: Port, color: Color) -> impl Scene {
    port_sized(port, color, PortSize::default())
}

/// The size of a [`port_sized`] port, in logical pixels. `physical` is the
/// port's own node: where picking hovers and presses it (drags, drops), where
/// its wires are anchored and the room it takes in its row. `visual` is only
/// the drawn dot, centred in it, its border scaled with it (wires are drawn
/// up to its rim, and the hover look grows it, not the box). The default is
/// the [`port`] dot: both [`PORT_SIZE`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PortSize {
    /// The port's box: hit area, wire anchor, layout.
    pub physical: f32,
    /// The drawn dot's diameter.
    pub visual: f32,
}

impl Default for PortSize {
    fn default() -> Self {
        Self { physical: PORT_SIZE, visual: PORT_SIZE }
    }
}

/// A [`port`] of another [`PortSize`]: a `physical` box, still centered on
/// the node's border and on its row, drawing a `visual` dot. When the sizes
/// differ the box is undrawn and a [`PortDot`] child, ignored by picking,
/// draws the dot ([`PortHighlight`] colours and scales it, wires end at its
/// rim); at the same size the port draws itself, as [`port`] does.
pub fn port_sized(port: Port, color: Color, size: PortSize) -> impl Scene {
    let PortSize { physical, visual } = size;
    let radius = physical / 2.0;
    // Centered on the frame's border; rows reach its inner edge.
    let inset = px(-(BORDER + radius));
    let (left, right) = match port.direction {
        PortDirection::Input => (inset, Val::Auto),
        PortDirection::Output => (Val::Auto, inset),
    };
    let border = |size: f32| PORT_BORDER * size / PORT_SIZE;
    let split = visual != physical;
    let (fill, own_border) = if split { (Color::NONE, 0.0) } else { (color, border(physical)) };
    let dot = split.then(|| -> Box<dyn Scene> {
        let at = px((physical - visual) / 2.0);
        Box::new(bsn! {
            PortDot
            Node {
                position_type: PositionType::Absolute,
                width: px(visual),
                height: px(visual),
                left: at,
                top: at,
                border: UiRect::all(px(border(visual))),
                border_radius: BorderRadius::MAX,
            }
            BackgroundColor(color)
            BorderColor::all(color)
            Pickable { should_block_lower: false, is_hoverable: false }
        })
    });
    let dot: Vec<Box<dyn Scene>> = dot.into_iter().collect();
    bsn! {
        port
        Node {
            position_type: PositionType::Absolute,
            width: px(physical),
            height: px(physical),
            top: percent(50),
            left: left,
            right: right,
            margin: UiRect::top(px(-radius)),
            border: UiRect::all(px(own_border)),
            border_radius: BorderRadius::MAX,
        }
        PortColor(color)
        PortHighlight
        BackgroundColor(fill)
        BorderColor::all(fill)
        EntityCursor::System(SystemCursorIcon::Crosshair)
        Children [ {dot} ]
    }
}

/// A row with a label and `port`, both on the side its direction faces:
/// `row("a", Port::input(NUMBER), BLUE)`.
pub fn row(label: impl Into<String>, kind: Port, color: Color) -> impl Scene {
    let label = label.into();
    let justify = match kind.direction {
        PortDirection::Input => JustifyContent::FlexStart,
        PortDirection::Output => JustifyContent::FlexEnd,
    };
    bsn! {
        Node {
            justify_content: justify,
            align_items: AlignItems::Center,
            min_height: px(22),
            // Out over the node's padding, so the port reaches its border.
            margin: UiRect::horizontal(px(-PADDING)),
            padding: UiRect::horizontal(px(PADDING)),
        }
        Children [
            Text(label)
            TextFont { font_size: FontSize::Px(13.0) }
            TextColor(Color::srgb_u8(214, 218, 224))
            Pickable { should_block_lower: false, is_hoverable: false }
            --
            @port(kind, color)
        ]
    }
}

/// A row holding only a port: no label, so no text to lay out. For dense or
/// very large graphs.
pub fn dot(kind: Port, color: Color) -> impl Scene {
    bsn! {
        Node { min_height: px(16), margin: UiRect::horizontal(px(-PADDING)) }
        Children [ @port(kind, color) ]
    }
}

#[cfg(test)]
#[allow(clippy::disallowed_methods, clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::{GraphEdit, GraphQuery, GraphWorldExt, NoodleCorePlugin, PortType};
    use bevy::ecs::system::RunSystemOnce;
    use bevy::scene::ScenePlugin;

    const NUM: PortType = PortType::named("num");

    fn app() -> App {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, AssetPlugin::default(), ScenePlugin, NoodleCorePlugin));
        app
    }

    #[test]
    fn ports_are_port_size_with_a_port_border() {
        let mut app = app();
        let w = app.world_mut();
        let port = w.spawn_scene(port(Port::input(NUM), Color::WHITE)).unwrap().id();
        let node = w.get::<Node>(port).unwrap();
        assert_eq!((node.width, node.height), (px(PORT_SIZE), px(PORT_SIZE)));
        assert_eq!(node.border, UiRect::all(px(PORT_BORDER)));
    }

    /// The children of `port` drawing its dot.
    fn dots(w: &mut World, port: Entity) -> Vec<Entity> {
        let children = w.get::<Children>(port).map(|c| c.to_vec()).unwrap_or_default();
        children.into_iter().filter(|c| w.get::<PortDot>(*c).is_some()).collect()
    }

    #[test]
    fn the_default_port_size_is_the_upstream_port() {
        let mut app = app();
        let w = app.world_mut();
        assert_eq!(PortSize::default(), PortSize { physical: PORT_SIZE, visual: PORT_SIZE });
        let plain = w.spawn_scene(port(Port::input(NUM), Color::WHITE)).unwrap().id();
        let same = w.spawn_scene(port_sized(Port::input(NUM), Color::WHITE, PortSize::default())).unwrap().id();
        assert_eq!(w.get::<Node>(same).unwrap(), w.get::<Node>(plain).unwrap());
        assert_eq!(*w.get::<BackgroundColor>(same).unwrap(), BackgroundColor(Color::WHITE), "it draws itself");
        assert!(dots(w, same).is_empty());
    }

    #[test]
    fn a_port_is_its_physical_box_and_draws_a_visual_dot_inside_it() {
        use bevy::ui::{ComputedNode, ResolvedBorderRadius, UiGlobalTransform};
        let mut app = app();
        let w = app.world_mut();
        let size = PortSize { physical: 24.0, visual: 16.0 };
        let big = w.spawn_scene(port_sized(Port::input(NUM), Color::WHITE, size)).unwrap().id();
        let node = w.get::<Node>(big).unwrap();
        assert_eq!((node.width, node.height), (px(24.0), px(24.0)), "the physical box");
        assert_eq!(node.margin, UiRect::top(px(-12.0)), "centred on its row");
        assert_eq!(node.left, px(-(BORDER + 12.0)), "centred on the node's border");
        assert_eq!((node.border, node.border_radius), (UiRect::all(px(0.0)), BorderRadius::MAX));
        assert_eq!(w.get::<BackgroundColor>(big).unwrap().0.alpha(), 0.0, "the box itself is not drawn");
        let [dot] = dots(w, big)[..] else { panic!("one dot") };
        let node = w.get::<Node>(dot).unwrap();
        assert_eq!((node.width, node.height), (px(16.0), px(16.0)), "the visual dot");
        assert_eq!((node.left, node.top), (px(4.0), px(4.0)), "centred in the box");
        assert_eq!(node.border, UiRect::all(px(PORT_BORDER * 16.0 / PORT_SIZE)), "the border scales with the dot");
        assert_eq!(*w.get::<BackgroundColor>(dot).unwrap(), BackgroundColor(Color::WHITE));
        let pickable = w.get::<Pickable>(dot).unwrap();
        assert!(!pickable.is_hoverable && !pickable.should_block_lower, "picking sees the port, not its dot");
        // A 1 px dot changes nothing else.
        let tiny = PortSize { visual: 1.0, ..size };
        let tiny = w.spawn_scene(port_sized(Port::input(NUM), Color::WHITE, tiny)).unwrap().id();
        assert_eq!(w.get::<Node>(tiny).unwrap(), w.get::<Node>(big).unwrap());

        // bevy_picking hovers and presses a UI node where its laid-out, rounded rect contains the pointer: the port's
        // `BorderRadius::MAX` box is a circle of the physical size.
        let laid_out = |size: f32| {
            let r = Vec2::splat(size / 2.0);
            let border_radius = ResolvedBorderRadius { top_left: r, top_right: r, bottom_right: r, bottom_left: r };
            ComputedNode { size: Vec2::splat(size), border_radius, ..default() }
        };
        let at = UiGlobalTransform::from_translation(Vec2::new(100.0, 100.0));
        let ten_px = Vec2::new(100.0, 100.0) + Vec2::new(6.0, 8.0);
        assert!(laid_out(24.0).contains_point(at, ten_px), "10 px from the centre of a 24 px port");
        assert!(!laid_out(PORT_SIZE).contains_point(at, ten_px), "outside the upstream port");
        assert!(!laid_out(24.0).contains_point(at, Vec2::new(120.0, 100.0)), "20 px away: not on the port");
    }

    #[test]
    fn a_field_override_keeps_the_other_defaults() {
        let mut app = app();
        let w = app.world_mut();
        let plain = w.spawn_scene(node(Vec2::ZERO)).unwrap().id();
        let scene = bsn! { @node(Vec2::ZERO) Node { min_width: px(240) } };
        let wide = w.spawn_scene(scene).unwrap().id();
        let (plain, wide) = (w.get::<Node>(plain).unwrap(), w.get::<Node>(wide).unwrap());
        let expected = Node { min_width: px(240), ..plain.clone() };
        assert_eq!(*wide, expected);
    }

    #[test]
    fn scene_nodes_join_the_graph() {
        let mut app = app();
        let w = app.world_mut();
        let canvas = w.spawn_scene(canvas()).unwrap().id();
        let mut spawn = |kind: Port| {
            let scene = bsn! { @node(Vec2::ZERO) ChildOf(canvas) Children [ @row("p", kind, Color::WHITE) ] };
            w.spawn_scene(scene).unwrap().id()
        };
        let (a, b) = (spawn(Port::output(NUM)), spawn(Port::input(NUM)));
        app.update();
        let w = app.world_mut();
        let ports = w.run_system_once(move |graph: GraphQuery| {
            assert_eq!(graph.nodes_in(canvas).count(), 2);
            (graph.outputs_of(a).next(), graph.inputs_of(b).next())
        });
        let Ok((Some(from), Some(to))) = ports else {
            panic!("ports not found: {ports:?}");
        };
        assert!(w.graph_edit(canvas, GraphEdit::Connect { from, to }).is_ok());
    }
}
