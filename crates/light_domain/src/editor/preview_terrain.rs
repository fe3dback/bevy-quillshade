//! Procedural 1 km landscape for the Light domain's 3D preview studio ([`super::preview`]).
//!
//! A deterministic heightfield (one height point per 4 m, 251 × 251 vertices) centered on the studio: the middle
//! band (`|z| <` [`FLAT_HALF`]) stays flat at the studio floor so the calibrated spheres and depth boxes are unchanged,
//! ridged mountains up to [`MOUNTAIN_HEIGHT`] rise behind the depth boxes (`-Z`, the default view direction), and a
//! basin down to [`LOWLAND_DEPTH`] below the floor sinks behind the camera (`+Z`). It gives `DistanceFog` real
//! distances and silhouettes to fade while authoring an Environment asset.

use bevy::asset::RenderAssetUsages;
use bevy::mesh::{Indices, Mesh, PrimitiveTopology};

/// Edge length of the square landscape in meters.
pub const TERRAIN_SIZE: f32 = 1000.0;
/// Distance between neighbouring height points in meters.
pub const TERRAIN_STEP: f32 = 4.0;
/// Highest mountain point above the studio floor in meters.
pub const MOUNTAIN_HEIGHT: f32 = 96.0;
/// Deepest lowland point below the studio floor in meters.
pub const LOWLAND_DEPTH: f32 = 32.0;
/// Half-width of the flat studio band around `z = 0` in meters (covers the depth boxes down to `z = -36`).
pub const FLAT_HALF: f32 = 48.0;

/// Number of cells along one edge.
const CELLS: u32 = (TERRAIN_SIZE / TERRAIN_STEP) as u32;

/// Height of the landscape above (positive) or below (negative) the studio floor at `(x, z)`.
pub fn terrain_height(x: f32, z: f32) -> f32 {
    // Wobble the band edges so the transition is not a straight line.
    let edge = FLAT_HALF + 24.0 * value_noise(x * 0.008 + 17.0, 3.0);
    if -z > edge {
        let mask = smoothstep(edge, edge + 200.0, -z);
        let shape = 0.25 + 0.75 * ridged(x * 0.006, z * 0.006);
        (MOUNTAIN_HEIGHT * mask * shape).min(MOUNTAIN_HEIGHT)
    } else if z > edge {
        let mask = smoothstep(edge, edge + 140.0, z);
        let shape = 0.55 + 0.45 * fbm(x * 0.01 + 41.0, z * 0.01 - 13.0);
        (-LOWLAND_DEPTH * mask * shape).max(-LOWLAND_DEPTH)
    } else {
        0.0
    }
}

/// Builds the landscape mesh centered at the origin, heights relative to `y = 0` (spawn it at the studio floor).
pub fn terrain_mesh() -> Mesh {
    let n = CELLS + 1;
    let half = TERRAIN_SIZE * 0.5;
    let coord = |i: u32| -half + i as f32 * TERRAIN_STEP;
    let heights: Vec<f32> =
        (0..n).flat_map(|j| (0..n).map(move |i| (i, j))).map(|(i, j)| terrain_height(coord(i), coord(j))).collect();
    let h = |i: u32, j: u32| heights[(j.min(n - 1) * n + i.min(n - 1)) as usize];

    let mut positions = Vec::with_capacity((n * n) as usize);
    let mut normals = Vec::with_capacity((n * n) as usize);
    let mut uvs = Vec::with_capacity((n * n) as usize);
    for j in 0..n {
        for i in 0..n {
            positions.push([coord(i), h(i, j), coord(j)]);
            // Central differences, one-sided at the borders.
            let (il, ir) = (i.saturating_sub(1), (i + 1).min(n - 1));
            let (jd, ju) = (j.saturating_sub(1), (j + 1).min(n - 1));
            let dx = (h(ir, j) - h(il, j)) / ((ir - il) as f32 * TERRAIN_STEP);
            let dz = (h(i, ju) - h(i, jd)) / ((ju - jd) as f32 * TERRAIN_STEP);
            let len = (dx * dx + 1.0 + dz * dz).sqrt();
            normals.push([-dx / len, 1.0 / len, -dz / len]);
            uvs.push([i as f32 / CELLS as f32, j as f32 / CELLS as f32]);
        }
    }

    let mut indices = Vec::with_capacity((CELLS * CELLS * 6) as usize);
    for j in 0..CELLS {
        for i in 0..CELLS {
            let v00 = j * n + i;
            let (v10, v01, v11) = (v00 + 1, v00 + n, v00 + n + 1);
            // Counter-clockwise seen from +Y.
            indices.extend([v00, v01, v10, v10, v01, v11]);
        }
    }

    Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default())
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
        .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
        .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uvs)
        .with_inserted_indices(Indices::U32(indices))
}

fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Integer lattice hash in `0..=1`.
fn hash(ix: i32, iz: i32) -> f32 {
    let mut h = (ix as u32).wrapping_mul(0x8da6_b343) ^ (iz as u32).wrapping_mul(0xd816_3841);
    h ^= h >> 15;
    h = h.wrapping_mul(0x2c1b_3c6d);
    h ^= h >> 12;
    h = h.wrapping_mul(0x297a_2d39);
    h ^= h >> 15;
    h as f32 / u32::MAX as f32
}

/// Smooth value noise in `0..=1`.
fn value_noise(x: f32, z: f32) -> f32 {
    let (fx, fz) = (x.floor(), z.floor());
    let (ix, iz) = (fx as i32, fz as i32);
    let (tx, tz) = (x - fx, z - fz);
    let (sx, sz) = (tx * tx * (3.0 - 2.0 * tx), tz * tz * (3.0 - 2.0 * tz));
    let a = hash(ix, iz) + (hash(ix + 1, iz) - hash(ix, iz)) * sx;
    let b = hash(ix, iz + 1) + (hash(ix + 1, iz + 1) - hash(ix, iz + 1)) * sx;
    a + (b - a) * sz
}

/// Four-octave fractal value noise in `0..=1`.
fn fbm(x: f32, z: f32) -> f32 {
    let (mut sum, mut amp, mut freq, mut norm) = (0.0, 1.0, 1.0, 0.0);
    for octave in 0..4 {
        let o = octave as f32 * 31.7;
        sum += amp * value_noise(x * freq + o, z * freq - o);
        norm += amp;
        amp *= 0.5;
        freq *= 2.0;
    }
    sum / norm
}

/// Five-octave ridged noise in `0..=1` (sharp crests where the base noise crosses its midpoint).
fn ridged(x: f32, z: f32) -> f32 {
    let (mut sum, mut amp, mut freq, mut norm) = (0.0, 1.0, 1.0, 0.0);
    for octave in 0..5 {
        let o = octave as f32 * 19.3;
        let r = 1.0 - (2.0 * value_noise(x * freq + o, z * freq + o) - 1.0).abs();
        sum += amp * r * r;
        norm += amp;
        amp *= 0.5;
        freq *= 2.0;
    }
    sum / norm
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::mesh::VertexAttributeValues;

    fn grid() -> impl Iterator<Item = (f32, f32)> {
        let n = CELLS + 1;
        let c = |i: u32| -TERRAIN_SIZE * 0.5 + i as f32 * TERRAIN_STEP;
        (0..n).flat_map(move |j| (0..n).map(move |i| (c(i), c(j))))
    }

    #[test]
    fn studio_band_is_flat() {
        for (x, z) in grid().filter(|&(_, z)| z.abs() <= FLAT_HALF) {
            assert_eq!(terrain_height(x, z), 0.0, "({x}, {z})");
        }
    }

    #[test]
    fn mountains_behind_lowlands_in_front() {
        let (mut max_m, mut min_l) = (0.0f32, 0.0f32);
        for (x, z) in grid() {
            let h = terrain_height(x, z);
            if z < 0.0 {
                assert!((0.0..=MOUNTAIN_HEIGHT).contains(&h), "mountain side ({x}, {z}) = {h}");
                max_m = max_m.max(h);
            } else {
                assert!((-LOWLAND_DEPTH..=0.0).contains(&h), "lowland side ({x}, {z}) = {h}");
                min_l = min_l.min(h);
            }
        }
        assert!(max_m > 0.7 * MOUNTAIN_HEIGHT, "highest peak {max_m}");
        assert!(min_l < -0.7 * LOWLAND_DEPTH, "deepest point {min_l}");
    }

    #[test]
    fn mesh_is_a_4m_grid_facing_up() {
        let mesh = terrain_mesh();
        let n = (CELLS + 1) as usize;
        assert_eq!(mesh.count_vertices(), n * n);
        assert_eq!(mesh.indices().map(Indices::len), Some((CELLS * CELLS * 6) as usize));
        let Some(VertexAttributeValues::Float32x3(normals)) = mesh.attribute(Mesh::ATTRIBUTE_NORMAL) else {
            panic!("normals missing");
        };
        assert!(normals.iter().all(|n| n[1] > 0.0));
    }
}
