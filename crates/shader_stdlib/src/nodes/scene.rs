//! Scene category: reads of the camera's view textures (stdlib `scene.wesl`). They need the camera's prepass
//! components; without one the read falls back to "nothing in front of the far plane".

use shader_graph_core::PinType;
use shader_graph_core::registry::{Builtin, Registry, SceneTexture};

use crate::def::{def, pin_in, pin_out, sig};

const CAT: &str = "Scene";

pub fn register(r: &mut Registry) {
    r.register(
        def("stdlib::SceneDepth", "SceneDepth", CAT)
            .kw(&["depth", "scene depth", "depth buffer", "prepass", "z buffer"])
            .desc(
                "Depth of the opaque scene at a viewport UV (the current pixel when unconnected), read from the depth \
                 prepass: linear distance along the view direction, like PixelDepth, and the raw reverse-Z value. \
                 Needs `DepthPrepass` on the camera, otherwise reads as infinitely far (raw 0).",
            )
            .build(
                |_| {
                    sig(
                        vec![pin_in("uv", "UV", PinType::Float2, 0.0).implicit("pixel")],
                        vec![pin_out("depth", "Depth", PinType::Float), pin_out("raw", "Raw", PinType::Float)],
                    )
                },
                |c| {
                    c.scene_texture(SceneTexture::Depth);
                    let pixel = if c.is_connected("uv") {
                        let f = c.stdlib("scene", "sg_scene_pixel");
                        format!("{f}({})", c.input("uv"))
                    } else {
                        format!("{}.xy", c.builtin(Builtin::FragCoord))
                    };
                    let (raw_fn, linear_fn) =
                        (c.stdlib("scene", "sg_scene_depth_raw"), c.stdlib("scene", "sg_linear_depth"));
                    c.define("raw", format!("{raw_fn}({pixel})"));
                    let raw = c.var("raw");
                    c.define("depth", format!("{linear_fn}({raw})"));
                },
            ),
    );

    r.register(
        def("stdlib::DepthFade", "DepthFade", CAT)
            .kw(&["depth fade", "soft particles", "shoreline", "intersection", "soft edge"])
            .desc(
                "Opacity times 0 where the surface touches the scene behind it, rising to 1 at Fade Distance in front \
                 of it (soft particles, shorelines). Use it in a translucent material; needs `DepthPrepass` on the \
                 camera, otherwise outputs Opacity.",
            )
            .build(
                |_| {
                    sig(
                        vec![
                            pin_in("opacity", "Opacity", PinType::Float, 1.0),
                            pin_in("distance", "Fade Distance", PinType::Float, 1.0).range(0.0, 10.0),
                        ],
                        vec![pin_out("out", "", PinType::Float)],
                    )
                },
                |c| {
                    c.scene_texture(SceneTexture::Depth);
                    let f = c.stdlib("scene", "sg_depth_fade");
                    let fc = c.builtin(Builtin::FragCoord);
                    c.define("out", format!("({} * {f}({fc}, {}))", c.input("opacity"), c.input("distance")));
                },
            ),
    );
}
