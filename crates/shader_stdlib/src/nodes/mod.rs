//! Node definitions grouped by palette category.

pub mod attributes;
pub mod constants;
pub mod coords;
pub mod math;
pub mod output;
pub mod texture;
pub mod utility;
pub mod vector;

use shader_graph_core::Registry;

pub fn register_all(r: &mut Registry) {
    constants::register(r);
    math::register(r);
    vector::register(r);
    coords::register(r);
    texture::register(r);
    utility::register(r);
    output::register(r);
    attributes::register(r);
}
