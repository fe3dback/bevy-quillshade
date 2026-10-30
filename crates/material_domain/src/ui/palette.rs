//! Node palette, Material's half (spec §9.3): its entries ([`entries_for`]) and what picking one does. The popup and
//! the ranking are `node_graph_base`'s ([`PalettePlugin`], `node_graph_base::palette::results`); the palette rows are
//! `crate::graph_domain::MaterialGraph::palette`.

use crate::material_state::MaterialStates;
use crate::ops::MaterialAction;
use bevy::prelude::*;

use crate::graph_domain::MaterialGraph;
use crate::ops::prelude::*;
use crate::ops::{Action, Editor, Link};
use crate::search::{self, Entry, WireEnd};

pub use node_graph_base::palette::{PaletteInput, PaletteList, PaletteRoot, PaletteRow};

/// The open palette of Material's graph.
pub type Palette = node_graph_base::palette::Palette<MaterialGraph>;

/// Material's palette (`node_graph_base`'s for [`MaterialGraph`]; the window app adds it through `register_graph`,
/// UI tests add this).
pub struct PalettePlugin;

impl Plugin for PalettePlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(node_graph_base::palette::PalettePlugin::<MaterialGraph>::default());
    }
}

/// Entries of the palette on the active graph, by category and name: every node it may add, filtered by the wire's
/// type when opened from one.
pub fn entries_for(editor: &Editor, link: Option<&Link>) -> Vec<Entry> {
    let Some(d) = editor.doc() else { return Vec::new() };
    let mut entries = search::entries(
        &editor.domains.material().registry,
        &editor.domains.material().functions,
        d.asset.kind,
        &d.id.path,
    );
    entries.extend(search::route_entries(&editor.domains.material().registry, &d.asset));
    entries.sort_by(|a, b| (&a.category, &a.name).cmp(&(&b.category, &b.name)));
    if let Some(link) = link
        && let Some(t) = editor.pin_type(&link.node, &link.pin, link.from_output)
    {
        let end = if link.from_output { WireEnd::FromOutput(t) } else { WireEnd::FromInput(t) };
        entries.retain(|e| {
            search::compatible_pin(
                &editor.domains.material().registry,
                &editor.domains.material().functions,
                d.asset.kind,
                &e.kind,
                end,
            )
            .is_some()
        });
    }
    entries
}

/// What picking `entry` does: add its node at `pos` (graph units), connected to `link`. A Named Route declaration
/// (`route`) then opens its rename dialog (design §6.4: a route always needs a name). After a wire from an output the
/// declaration also wins ties on the score (`MaterialGraph::palette` sets the entry's `lead`).
pub(crate) fn pick_for(route: bool, entry: &Entry, pos: (f32, f32), link: Option<Link>) -> Action {
    let (kind, params) = (entry.kind.clone(), entry.params.clone());
    if route {
        MaterialAction::AddNodeAndRename { kind, pos, link, params }.into()
    } else {
        MaterialAction::AddNode { kind, pos, link, params }.into()
    }
}
