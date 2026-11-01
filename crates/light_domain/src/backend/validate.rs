//! Validation of the Light domain (design §9.2): L-prefixed codes (decision 0043), the texture kit's rules under
//! [`LIGHT_TEXTURES`], curve validation, PascalCase enum variant collision checks, and the game code conflict
//! scanner (`LW102`).

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use project_core::{Diagnostic, Project};
use texture_kit::{ProblemKind, TextureAsset};

use super::LIGHT;
use super::model::{LIGHT_TEXTURES, LightAssets, curve_ref, environment_ref, texture_ref, to_pascal_case};

/// Bevy lighting and camera-environment types managed by `quillshade::light`.
const CONFLICT_TYPES: &[&str] = &[
    "DirectionalLight",
    "GlobalAmbientLight",
    "AmbientLight",
    "EnvironmentMapLight",
    "GeneratedEnvironmentMapLight",
    "Skybox",
    "DistanceFog",
    "FogFalloff",
    "Exposure",
    "Tonemapping",
    "DebandDither",
    "ColorGrading",
    "ColorGradingGlobal",
    "ColorGradingSection",
    "ClearColor",
];

/// Every diagnostic of the loaded Light assets, each `in_domain(LIGHT)` at the asset's source path: the load errors
/// (`LE009`), then the Texture assets' problems (`LE013` sampler, `LE012` path, `LE011` one game file shared by
/// several assets, `LW109` file missing, `LE014` not a cube map, `LW104` faces Bevy cannot filter for image-based
/// lighting), then the environments' (`LW101` no cube map,
/// `LE001` a cube map that names no Light Texture asset file, `LE002` PascalCase enum variant collision), then the
/// curves' (`LE002` PascalCase collision, `LE003` invalid range, `LW103` non-unit endpoints), and finally `LW102`
/// manual lighting/camera conflicts in game `.rs` files.
pub fn validate(project: &Project, assets: &LightAssets) -> Vec<Diagnostic> {
    let texture_diags = validate_textures(project, &assets.textures);
    let game_conflicts = scan_game_conflicts(project);
    validate_in_memory(assets, &texture_diags, &game_conflicts)
}

/// Validates a single Light Texture asset `tex` at `path` against `others` (the `source_rel` paths of other Light
/// Texture assets referencing the same `tex.path`) and the game's `assets/` directory.
pub fn validate_texture(project: &Project, path: &str, tex: &TextureAsset, others: &[String]) -> Vec<Diagnostic> {
    let at = texture_ref(path).source_rel();
    let assets_dir = project.assets_dir();
    texture_kit::problems(tex, others, &assets_dir, &LIGHT_TEXTURES)
        .into_iter()
        .map(|problem| {
            let d = match problem.kind {
                ProblemKind::Sampler => Diagnostic::error("LE013", problem.message),
                ProblemKind::Path => Diagnostic::error("LE012", problem.message),
                ProblemKind::SharedFile => Diagnostic::error("LE011", problem.message),
                ProblemKind::Missing => Diagnostic::warning("LW109", problem.message),
                ProblemKind::Shape => Diagnostic::error("LE014", problem.message),
            };
            d.in_domain(LIGHT).in_asset(at.clone())
        })
        .chain(
            texture_kit::read_ktx2_header(&assets_dir.join(&tex.path))
                .and_then(|h| ibl_problem(&h))
                .map(|msg| Diagnostic::warning("LW104", msg).in_domain(LIGHT).in_asset(at.clone())),
        )
        .collect()
}

/// `LW104`: a 6-face KTX2 cube map whose faces are not square power-of-two (at most 8192) shows as a skybox, but
/// Bevy's `GeneratedEnvironmentMapLight` cannot filter it, so the runtime lights the scene with the environment's
/// procedural sky colors instead. `None` for other headers (`LE014` reports a wrong shape).
fn ibl_problem(header: &[u8]) -> Option<String> {
    if header.len() < texture_kit::KTX2_HEADER_LEN {
        return None;
    }
    let field = |at: usize| u32::from_le_bytes([header[at], header[at + 1], header[at + 2], header[at + 3]]);
    let (width, height, faces) = (field(20), field(24), field(36));
    if faces != 6 || (width == height && width.is_power_of_two() && width <= 8192) {
        return None;
    }
    Some(format!(
        "cube map faces are {width}x{height}: image-based lighting needs square power-of-two faces (at most 8192), \
         so environments using it are lit by their procedural sky colors"
    ))
}

/// Validates all `textures` against `project`, returning per-texture diagnostics keyed by texture path.
pub fn validate_textures(
    project: &Project,
    textures: &BTreeMap<String, TextureAsset>,
) -> BTreeMap<String, Vec<Diagnostic>> {
    let mut by_file: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for (path, t) in textures {
        by_file.entry(t.path.as_str()).or_default().push(path.as_str());
    }
    textures
        .iter()
        .map(|(path, t)| {
            let others: Vec<String> =
                by_file[t.path.as_str()].iter().filter(|o| **o != path).map(|o| texture_ref(o).source_rel()).collect();
            (path.clone(), validate_texture(project, path, t, &others))
        })
        .collect()
}

/// Validates `assets` purely in memory using precomputed per-texture diagnostics (`texture_diags`) and cached game
/// Rust conflict warnings (`game_conflicts`), performing zero filesystem I/O.
pub fn validate_in_memory(
    assets: &LightAssets,
    texture_diags: &BTreeMap<String, Vec<Diagnostic>>,
    game_conflicts: &[Diagnostic],
) -> Vec<Diagnostic> {
    let mut out = assets.load_errors.clone();
    for path in assets.textures.keys() {
        if let Some(diags) = texture_diags.get(path) {
            out.extend(diags.iter().cloned());
        }
    }

    let mut env_by_pascal: BTreeMap<String, Vec<&str>> = BTreeMap::new();
    for path in assets.environments.keys() {
        env_by_pascal.entry(to_pascal_case(path)).or_default().push(path.as_str());
    }

    for (path, env) in &assets.environments {
        let at = environment_ref(path).source_rel();
        let pascal = to_pascal_case(path);
        if matches!(pascal.as_str(), "None" | "Transitioning") {
            out.push(
                Diagnostic::error(
                    "LE002",
                    format!("environment `{path}` maps to reserved variant name `{pascal}` in `light::State`"),
                )
                .in_domain(LIGHT)
                .in_asset(at.clone()),
            );
        }
        if let Some(group) = env_by_pascal.get(&pascal)
            && group.len() > 1
        {
            let others: Vec<&str> = group.iter().copied().filter(|&o| o != path.as_str()).collect();
            out.push(
                Diagnostic::error(
                    "LE002",
                    format!("environment `{path}` collides in PascalCase (`{pascal}`) with {}", others.join(", ")),
                )
                .in_domain(LIGHT)
                .in_asset(at.clone()),
            );
        }

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

    let mut curve_by_pascal: BTreeMap<String, Vec<&str>> = BTreeMap::new();
    for path in assets.curves.keys() {
        curve_by_pascal.entry(to_pascal_case(path)).or_default().push(path.as_str());
    }

    for (path, curve) in &assets.curves {
        let at = curve_ref(path).source_rel();
        let pascal = to_pascal_case(path);
        if pascal == "Linear" {
            out.push(
                Diagnostic::error(
                    "LE002",
                    format!("curve `{path}` maps to reserved variant name `Linear` in `light::Curve`"),
                )
                .in_domain(LIGHT)
                .in_asset(at.clone()),
            );
        }
        if let Some(group) = curve_by_pascal.get(&pascal)
            && group.len() > 1
        {
            let others: Vec<&str> = group.iter().copied().filter(|&o| o != path.as_str()).collect();
            out.push(
                Diagnostic::error(
                    "LE002",
                    format!("curve `{path}` collides in PascalCase (`{pascal}`) with {}", others.join(", ")),
                )
                .in_domain(LIGHT)
                .in_asset(at.clone()),
            );
        }

        if let Some(msg) = curve_kit::range_error(curve.min, curve.max) {
            out.push(Diagnostic::error("LE003", msg).in_domain(LIGHT).in_asset(at));
        } else {
            let range_ok = (curve.min - 0.0).abs() <= 1e-4 && (curve.max - 1.0).abs() <= 1e-4;
            let start_ok =
                curve.points.first().is_some_and(|&(t, v)| (t - 0.0).abs() <= 1e-3 && (v - 0.0).abs() <= 1e-3);
            let end_ok = curve.points.last().is_some_and(|&(t, v)| (t - 1.0).abs() <= 1e-3 && (v - 1.0).abs() <= 1e-3);
            if !range_ok || !start_ok || !end_ok {
                out.push(
                    Diagnostic::warning(
                        "LW103",
                        "transition curve does not start at (0, 0) and end at (1, 1) with range 0..1; transitions may jump at endpoints",
                    )
                    .in_domain(LIGHT)
                    .in_asset(at),
                );
            }
        }
    }

    out.extend(game_conflicts.iter().cloned());
    out
}

/// Scans game `.rs` files under `project.root` (skipping `crates/quillshade`, `target/`, and hidden directories) for
/// manual usages of Bevy lighting/camera-environment types managed by `quillshade::light`, returning `LW102` warnings
/// sorted by `(rel_path, line_no, ident)`.
pub fn scan_game_conflicts(project: &Project) -> Vec<Diagnostic> {
    let crate_dir = project.crate_dir();
    let mut hits: BTreeSet<(String, usize, &'static str)> = BTreeSet::new();
    for file in project_core::files::walk_files(&project.root) {
        if !should_scan_rs(&project.root, &crate_dir, &file) {
            continue;
        }
        let Ok(src) = std::fs::read_to_string(&file) else { continue };
        let rel = project.rel(&file);
        for (line_no, ident) in scan_rust_source(&src) {
            hits.insert((rel.clone(), line_no, ident));
        }
    }
    hits.into_iter()
        .map(|(rel_path, line_no, ident)| {
            Diagnostic::warning(
                "LW102",
                format!(
                    "game code uses `{ident}` at line {line_no}; remove manual lighting/environment setup so `quillshade::light` can manage it without conflicts"
                ),
            )
            .in_domain(LIGHT)
            .in_asset(rel_path)
            .at(format!("line {line_no}"))
        })
        .collect()
}

fn should_scan_rs(root: &Path, crate_dir: &Path, file: &Path) -> bool {
    if file.extension().is_none_or(|e| e != "rs") || file.starts_with(crate_dir) {
        return false;
    }
    let Ok(rel) = file.strip_prefix(root) else { return false };
    !rel.components().any(|c| {
        let s = c.as_os_str().to_string_lossy();
        s == "target" || s.starts_with('.')
    })
}

/// Scans a Rust source string for identifiers in [`CONFLICT_TYPES`], ignoring line comments, nested block comments,
/// char literals, string literals, and raw string literals while preserving exact 1-based line numbers.
fn scan_rust_source(src: &str) -> Vec<(usize, &'static str)> {
    let bytes = src.as_bytes();
    let len = bytes.len();
    let mut i = 0;
    let mut line_no = 1usize;
    let mut out = Vec::new();

    while i < len {
        let b = bytes[i];
        if b == b'\n' {
            line_no += 1;
            i += 1;
            continue;
        }
        // Line comment `//...`
        if b == b'/' && i + 1 < len && bytes[i + 1] == b'/' {
            i += 2;
            while i < len && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        // Nested block comment `/* ... */`
        if b == b'/' && i + 1 < len && bytes[i + 1] == b'*' {
            i += 2;
            let mut depth = 1usize;
            while i < len && depth > 0 {
                if bytes[i] == b'\n' {
                    line_no += 1;
                    i += 1;
                } else if bytes[i] == b'/' && i + 1 < len && bytes[i + 1] == b'*' {
                    depth += 1;
                    i += 2;
                } else if bytes[i] == b'*' && i + 1 < len && bytes[i + 1] == b'/' {
                    depth -= 1;
                    i += 2;
                } else {
                    i += 1;
                }
            }
            continue;
        }
        // Char literal (`'x'` or `'\x'`) vs lifetime `'a`
        if b == b'\'' {
            if i + 3 < len && bytes[i + 1] == b'\\' && bytes[i + 2] != b'\n' && bytes[i + 3] == b'\'' {
                i += 4;
            } else if i + 2 < len
                && bytes[i + 1] != b'\\'
                && bytes[i + 1] != b'\''
                && bytes[i + 1] != b'\n'
                && bytes[i + 2] == b'\''
            {
                i += 3;
            } else {
                i += 1;
            }
            continue;
        }
        // Normal string literal `"..."`
        if b == b'"' {
            i += 1;
            while i < len {
                if bytes[i] == b'\\' {
                    if i + 1 < len {
                        if bytes[i + 1] == b'\n' {
                            line_no += 1;
                        }
                        i += 2;
                    } else {
                        i += 1;
                    }
                } else if bytes[i] == b'"' {
                    i += 1;
                    break;
                } else {
                    if bytes[i] == b'\n' {
                        line_no += 1;
                    }
                    i += 1;
                }
            }
            continue;
        }
        // Identifier or raw string literal (`r"..."`, `r#"..."#`, `br#"..."#`, `cr#"..."#`)
        if b.is_ascii_alphabetic() || b == b'_' {
            let start = i;
            i += 1;
            while i < len && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                i += 1;
            }
            let ident = &src[start..i];
            if matches!(ident, "r" | "br" | "cr") {
                let mut hashes = 0usize;
                while i + hashes < len && bytes[i + hashes] == b'#' {
                    hashes += 1;
                }
                if i + hashes < len && bytes[i + hashes] == b'"' {
                    i += hashes + 1;
                    while i < len {
                        if bytes[i] == b'\n' {
                            line_no += 1;
                            i += 1;
                        } else if bytes[i] == b'"'
                            && i + 1 + hashes <= len
                            && bytes[i + 1..i + 1 + hashes].iter().all(|&h| h == b'#')
                        {
                            i += 1 + hashes;
                            break;
                        } else {
                            i += 1;
                        }
                    }
                    continue;
                }
            }
            if let Some(&matched) = CONFLICT_TYPES.iter().find(|&&t| t == ident) {
                out.push((line_no, matched));
            }
            continue;
        }
        i += 1;
    }
    out
}
