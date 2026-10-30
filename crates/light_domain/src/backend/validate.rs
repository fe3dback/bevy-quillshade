//! Validation of the Light domain (design §9.2): L-prefixed codes (decision 0043), the texture kit's rules under
//! [`LIGHT_TEXTURES`].

use std::collections::BTreeMap;

use project_core::{Diagnostic, Project};
use texture_kit::ProblemKind;

use super::LIGHT;
use super::model::{LIGHT_TEXTURES, LightAssets, environment_ref, texture_ref};

/// Every diagnostic of the loaded Light assets, each `in_domain(LIGHT)` at the asset's source path: the load errors
/// (LE009), then the Texture assets' problems (LE013 sampler, LE012 path, LE011 one game file shared by several
/// assets, LW109 file missing, LE014 not a cube map), then the environments' (LW101 no cube map, LE001 a cube map that
/// names no Light Texture asset file), in path order.
pub fn validate(project: &Project, assets: &LightAssets) -> Vec<Diagnostic> {
    let mut out = assets.load_errors.clone();
    let mut by_file: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for (path, t) in &assets.textures {
        by_file.entry(t.path.as_str()).or_default().push(path.as_str());
    }
    let assets_dir = project.assets_dir();
    for (path, t) in &assets.textures {
        let others: Vec<String> =
            by_file[t.path.as_str()].iter().filter(|o| **o != path).map(|o| texture_ref(o).source_rel()).collect();
        let at = texture_ref(path).source_rel();
        for problem in texture_kit::problems(t, &others, &assets_dir, &LIGHT_TEXTURES) {
            let d = match problem.kind {
                ProblemKind::Sampler => Diagnostic::error("LE013", problem.message),
                ProblemKind::Path => Diagnostic::error("LE012", problem.message),
                ProblemKind::SharedFile => Diagnostic::error("LE011", problem.message),
                ProblemKind::Missing => Diagnostic::warning("LW109", problem.message),
                ProblemKind::Shape => Diagnostic::error("LE014", problem.message),
            };
            out.push(d.in_domain(LIGHT).in_asset(at.clone()));
        }
    }
    for (path, env) in &assets.environments {
        let at = environment_ref(path).source_rel();
        let d = match &env.cubemap {
            None => Diagnostic::warning("LW101", "no cube map set: the environment shows nothing"),
            // A texture file that does not load still exists: it has its own LE009.
            Some(cubemap)
                if !assets.textures.contains_key(cubemap)
                    && !assets.load_errors.iter().any(|d| d.asset == Some(texture_ref(cubemap).source_rel())) =>
            {
                let file = texture_ref(cubemap).source_rel();
                Diagnostic::error("LE001", format!("cube map `{cubemap}` is not a Light Texture asset (`{file}`)"))
                    .at("cubemap")
            }
            Some(_) => continue,
        };
        out.push(d.in_domain(LIGHT).in_asset(at));
    }
    out
}
