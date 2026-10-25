//! Standard node library (spec §8): UE5-like node palette and the WESL modules it relies on.

mod def;
pub mod nodes;
pub mod user;

use shader_graph_core::Registry;

/// Version of the stdlib WESL modules, written into their headers.
pub const STDLIB_VERSION: u32 = 1;

/// Stdlib WESL modules (`autogen/stdlib/{name}.wesl`), regenerated on every project regen.
pub const MODULES: &[(&str, &str)] = &[
    ("color", include_str!("../wesl/color.wesl")),
    ("math", include_str!("../wesl/math.wesl")),
    ("noise", include_str!("../wesl/noise.wesl")),
    ("shading", include_str!("../wesl/shading.wesl")),
    ("texture", include_str!("../wesl/texture.wesl")),
    ("transform", include_str!("../wesl/transform.wesl")),
    ("uv", include_str!("../wesl/uv.wesl")),
];

/// Pseudo-module for Bevy sprite functions (`bevy_sprite_render::sprite_mesh::functions`).
pub const SPRITE_MODULE: &str = "@sprite";

/// Registry with every stdlib node.
pub fn registry() -> Registry {
    let mut r = Registry::new();
    nodes::register_all(&mut r);
    r
}
