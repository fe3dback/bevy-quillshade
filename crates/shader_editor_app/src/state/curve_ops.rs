use super::*;

impl Editor {
    pub(super) fn curve_action(&mut self, action: CurveAction) {
        let Some(c) = self.curve_doc_mut() else { return };
        let mut refresh = true;
        match action {
            CurveAction::Select(sel) => c.select(sel),
            CurveAction::Add(p) => c.add_point(p),
            CurveAction::SetPoints { points, selection, merge } => {
                c.set_points(points, selection, merge.then_some("drag"));
                // Properties catch up on CommitInline.
                refresh = !merge;
            }
            CurveAction::Nudge(delta) => {
                let (points, selection) = CurveDoc::moved(&c.curve.points, &c.selection, delta);
                c.set_points(points, selection, Some("nudge"));
            }
        }
        if refresh {
            self.ui.props += 1;
        }
    }
}
