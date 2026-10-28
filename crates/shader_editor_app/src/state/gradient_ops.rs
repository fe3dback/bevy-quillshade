use super::*;

impl Editor {
    /// Edits of the active gradients tab (gradient design §8).
    pub(super) fn gradients_action(&mut self, action: GradientsAction) {
        if let GradientsAction::CopySlot = action {
            self.gradient_clipboard = self.gradients_doc().map(GradientsDoc::copied_slot);
            return;
        }
        let clipboard = self.gradient_clipboard.clone();
        let Some(g) = self.gradients_doc_mut() else { return };
        // Merged edits (a drag in progress) leave the right sidebar alone until `CommitInline`.
        let mut refresh = true;
        match action {
            GradientsAction::SelectSlot(i) => g.select_slot(i),
            GradientsAction::Select(sel) => g.select(sel),
            GradientsAction::SetStops(stops, sel, key) => {
                refresh = key.is_none();
                g.set_stops(stops, sel, key.as_deref());
            }
            GradientsAction::AddStop(t) => {
                if !g.add_stop(t) {
                    self.push_msg(Level::Warning, format!("A gradient has at most {MAX_STOPS} stops"));
                    return;
                }
            }
            GradientsAction::DeleteSelected => g.delete_selected(),
            GradientsAction::SetMid(seg, mid, key) => {
                refresh = key.is_none();
                g.set_mid(seg, mid, key.as_deref());
            }
            GradientsAction::SetStopT(i, t, key) => {
                let was_selected = g.selection == Some(GradientSel::Stop(i));
                g.set_stop_t(i, t, key.as_deref());
                // A newly selected stop, or one the re-sort gave another index, rebinds the fields.
                refresh = key.is_none() || !was_selected || g.selection != Some(GradientSel::Stop(i));
            }
            GradientsAction::SetStopColor(i, color, key) => {
                refresh = key.is_none();
                g.set_stop_color(i, color, key.as_deref());
            }
            GradientsAction::SetName(name, key) => {
                // Not while typing: a redrawn right sidebar would take the field's focus.
                refresh = key.is_none();
                g.set_name(name, key.as_deref());
            }
            GradientsAction::MoveSlot(from, to) => g.move_slot(from, to),
            GradientsAction::ClearSlot => g.clear_slot(),
            GradientsAction::DuplicateSlot => {
                if !g.duplicate_slot() {
                    self.push_msg(Level::Warning, "No empty gradient slot to duplicate into".into());
                    return;
                }
            }
            GradientsAction::PasteSlot => match clipboard {
                Some(slot) => g.paste_slot(&slot),
                None => return,
            },
            GradientsAction::CopySlot => {}
        }
        if refresh {
            self.ui.props += 1;
        }
    }
}
