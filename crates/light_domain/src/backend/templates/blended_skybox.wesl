import bevy_render::view::View;

struct BlendedSkyboxUniforms {
    transform_a: mat4x4<f32>,
    transform_b: mat4x4<f32>,
    color_a: vec4<f32>,
    color_b: vec4<f32>,
    brightness_a: f32,
    brightness_b: f32,
    blend: f32,
    alpha: f32,
    has_a: u32,
    has_b: u32,
    _pad0: u32,
    _pad1: u32,
}

@group(0) @binding(0) var skybox_a: texture_cube<f32>;
@group(0) @binding(1) var skybox_a_sampler: sampler;
@group(0) @binding(2) var skybox_b: texture_cube<f32>;
@group(0) @binding(3) var skybox_b_sampler: sampler;
@group(0) @binding(4) var<uniform> view: View;
@group(0) @binding(5) var<uniform> uniforms: BlendedSkyboxUniforms;

fn coords_to_viewport_uv(position: vec2<f32>, viewport: vec4<f32>) -> vec2<f32> {
    return (position - viewport.xy) / viewport.zw;
}

fn coords_to_view_ray_direction(position: vec2<f32>, viewport: vec4<f32>) -> vec3<f32> {
    let view_position_homogeneous = view.view_from_clip * vec4(
        coords_to_viewport_uv(position, viewport) * vec2(2.0, -2.0) + vec2(-1.0, 1.0),
        1.0,
        1.0,
    );
    let view_ray_direction = view_position_homogeneous.xyz / view_position_homogeneous.w;
    return (view.world_from_view * vec4(view_ray_direction, 0.0)).xyz;
}

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
}

@vertex
fn skybox_vertex(@builtin(vertex_index) vertex_index: u32) -> VertexOutput {
    let clip_position = vec2(
        f32(vertex_index & 1u),
        f32((vertex_index >> 1u) & 1u),
    ) * 4.0 - vec2(1.0);
    return VertexOutput(vec4(clip_position, 0.0, 1.0));
}

@fragment
fn skybox_fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let world_ray = coords_to_view_ray_direction(in.position.xy, view.main_pass_viewport);
    let ray_a = normalize((uniforms.transform_a * vec4(world_ray, 0.0)).xyz);
    let ray_b = normalize((uniforms.transform_b * vec4(world_ray, 0.0)).xyz);

    // Cube maps are left-handed so we negate the z coordinate.
    let sample_a = textureSample(skybox_a, skybox_a_sampler, ray_a * vec3(1.0, 1.0, -1.0));
    let sample_b = textureSample(skybox_b, skybox_b_sampler, ray_b * vec3(1.0, 1.0, -1.0));
    var sky_a = uniforms.color_a.rgb;
    if uniforms.has_a != 0u {
        sky_a = sample_a.rgb * uniforms.brightness_a;
    }
    var sky_b = uniforms.color_b.rgb;
    if uniforms.has_b != 0u {
        sky_b = sample_b.rgb * uniforms.brightness_b;
    }
    let rgb = mix(sky_a, sky_b, uniforms.blend);
    return vec4(rgb, uniforms.alpha);
}
