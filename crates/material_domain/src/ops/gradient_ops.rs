use super::*;

/// The gradients tab's edits (gradient design §8, design §6.2): the body of [`TabDoc::apply`] /
/// [`TabDoc::current_value`] of [`GradientsDoc`].
impl GradientsDoc {
    pub(crate) fn apply_edit(&mut self, ctx: &mut DocCtx, action: &TabAction) -> Handled {
        let redraw = match action {
            TabAction::Undo => self.undo(),
            TabAction::Redo => self.redo(),
            TabAction::Copy => self.gradients_action(ctx, &GradientsAction::CopySlot),
            // The selected slot is copied and the selected stop deleted, as Copy then Delete.
            TabAction::Cut => {
                self.gradients_action(ctx, &GradientsAction::CopySlot);
                self.gradients_action(ctx, &GradientsAction::DeleteSelected)
            }
            TabAction::Paste(_) => self.gradients_action(ctx, &GradientsAction::PasteSlot),
            TabAction::Duplicate => self.gradients_action(ctx, &GradientsAction::DuplicateSlot),
            TabAction::DeleteSelection => self.gradients_action(ctx, &GradientsAction::DeleteSelected),
            TabAction::SelectAll => return Handled::No,
            TabAction::CommitInline => {
                self.end_merge();
                true
            }
            TabAction::SetValue { key, value, merge } => {
                let Some(target) = key.downcast::<GradientProp>() else { return Handled::No };
                self.set_prop(ctx, key, target, value, *merge)
            }
            TabAction::SetComponent { key, comp, value, is_final } => {
                let Some(target) = key.downcast::<GradientProp>() else { return Handled::No };
                // `None`: a field sent its shown (rounded) text back on focus loss, which is not an edit.
                let Some(new) = self.prop_of(target).and_then(|cur| cur.with_component(*comp, *value)) else {
                    return Handled::Yes;
                };
                let redraw = self.set_prop(ctx, key, target, &new, true);
                if *is_final {
                    self.end_merge();
                }
                redraw
            }
            TabAction::Doc(p) => {
                let Some(a) = p.downcast::<GradientsAction>() else { return Handled::No };
                self.gradients_action(ctx, a)
            }
        };
        ctx.redraw_props |= redraw;
        Handled::Yes
    }

    /// A [`GradientsAction`]; returns whether the right sidebar must redraw.
    fn gradients_action(&mut self, ctx: &mut DocCtx, action: &GradientsAction) -> bool {
        let Some(m) = ctx.domain::<MaterialState>() else { return false };
        let warn = |ctx: &mut DocCtx, text: String| ctx.messages.push(Message::new(Level::Warning, text));
        match action {
            GradientsAction::CopySlot => {
                m.gradient_clipboard = Some(self.copied_slot());
                return false;
            }
            GradientsAction::PasteSlot => match m.gradient_clipboard.clone() {
                Some(slot) => self.paste_slot(&slot),
                None => return false,
            },
            GradientsAction::SelectSlot(i) => self.select_slot(*i),
            GradientsAction::Select(sel) => self.select(*sel),
            // Merged edits (a drag in progress) leave the right sidebar alone until `CommitInline`.
            GradientsAction::SetStops(stops, sel, key) => {
                self.set_stops(stops.clone(), *sel, key.clone());
                return key.is_none();
            }
            GradientsAction::AddStop(t) => {
                if !self.add_stop(*t) {
                    warn(ctx, format!("A gradient has at most {MAX_STOPS} stops"));
                    return false;
                }
            }
            GradientsAction::DeleteSelected => self.delete_selected(),
            GradientsAction::SetMid(seg, mid, key) => {
                self.set_mid(*seg, *mid, key.clone());
                return key.is_none();
            }
            GradientsAction::SetStopT(i, t, key) => {
                let was_selected = self.selection == Some(GradientSel::Stop(*i));
                self.set_stop_t(*i, *t, key.clone());
                // A newly selected stop, or one the re-sort gave another index, rebinds the fields.
                return key.is_none() || !was_selected || self.selection != Some(GradientSel::Stop(*i));
            }
            GradientsAction::SetStopColor(i, color, key) => {
                self.set_stop_color(*i, *color, key.clone());
                return key.is_none();
            }
            GradientsAction::SetName(name, key) => {
                // Not while typing: a redrawn right sidebar would take the field's focus.
                self.set_name(name.clone(), key.clone());
                return key.is_none();
            }
            GradientsAction::MoveSlot(from, to) => self.move_slot(*from, *to),
            GradientsAction::ClearSlot => self.clear_slot(),
            GradientsAction::DuplicateSlot => {
                if !self.duplicate_slot() {
                    warn(ctx, "No empty gradient slot to duplicate into".into());
                    return false;
                }
            }
        }
        true
    }

    /// Value a property widget shows (a field of the selected slot).
    pub(crate) fn prop_of(&self, target: &GradientProp) -> Option<PropValue> {
        let slot = self.current()?;
        match target {
            GradientProp::Stop(i, 0) => Some(PropValue::Float(slot.stops.get(*i)?.t)),
            GradientProp::Stop(..) => None,
            GradientProp::Mid(i) => Some(PropValue::Float(slot.stops.get(*i)?.mid)),
            GradientProp::Name => Some(PropValue::Str(slot.name.clone())),
            GradientProp::Color(i) => Some(PropValue::Color(slot.stops.get(*i)?.color)),
        }
    }

    /// Sets a property (merged into the last undo step of the same `key` while `merge`); returns whether the right
    /// sidebar must redraw.
    fn set_prop(
        &mut self,
        ctx: &mut DocCtx,
        key: &PropKey,
        target: &GradientProp,
        value: &PropValue,
        merge: bool,
    ) -> bool {
        if self.prop_of(target).as_ref() == Some(value) {
            return false;
        }
        let mkey = merge.then(|| MergeKey::Prop(key.clone()));
        let action = match target {
            GradientProp::Stop(i, 0) => GradientsAction::SetStopT(*i, value.as_f32(), mkey),
            GradientProp::Stop(..) => return false,
            GradientProp::Mid(i) => GradientsAction::SetMid(*i, value.as_f32(), mkey),
            GradientProp::Name => GradientsAction::SetName(value.as_str().to_string(), mkey),
            GradientProp::Color(i) => {
                let c = model_value(value).coerce(PinType::Color).floats();
                let color = [0, 1, 2, 3].map(|k| c.get(k).copied().unwrap_or(1.0));
                GradientsAction::SetStopColor(*i, color, mkey)
            }
        };
        self.gradients_action(ctx, &action)
    }
}
