// Schlick-style fresnel term: base + (1 - base) * (1 - N.V)^exponent.
fn sg_fresnel(n: vec3<f32>, v: vec3<f32>, exponent: f32, base: f32) -> f32 {
    let ndv = saturate(dot(normalize(n), normalize(v)));
    return base + (1.0 - base) * pow(1.0 - ndv, exponent);
}
