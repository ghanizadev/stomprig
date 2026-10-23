use super::{LivePalette, LiveWindowAction};
use crate::app::plugin_editor::PluginEffectState;

pub(super) fn show_effects(
    ui: &mut egui::Ui,
    palette: &LivePalette,
    effects: &[PluginEffectState],
    action: &mut Option<LiveWindowAction>,
) {
    if effects.is_empty() {
        ui.label(
            egui::RichText::new("NO EFFECTS LOADED")
                .size(28.0)
                .strong()
                .color(palette.text_secondary),
        );
        return;
    }

    egui::ScrollArea::vertical()
        .id_salt("live-stage-effects")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.columns(3, |columns| {
                for (index, effect) in effects.iter().enumerate() {
                    let column = index % 3;
                    if show_effect(&mut columns[column], palette, index, effect) {
                        *action = Some(LiveWindowAction::ToggleEffect(index));
                    }
                    columns[column].add_space(10.0);
                }
            });
        });
}

fn show_effect(
    ui: &mut egui::Ui,
    palette: &LivePalette,
    index: usize,
    effect: &PluginEffectState,
) -> bool {
    let (status, color, fill) = if effect.bypassed {
        ("BYPASSED", palette.amber, palette.panel_highlight)
    } else {
        ("ACTIVE", palette.green, egui::Color32::from_rgb(12, 34, 29))
    };
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(ui.available_width().max(1.0), 82.0),
        egui::Sense::click(),
    );
    if response.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    let painter = ui.painter();
    painter.rect_filled(
        rect,
        8.0,
        if response.hovered() {
            palette.panel_highlight
        } else {
            fill
        },
    );
    painter.rect_filled(
        egui::Rect::from_min_max(
            rect.left_top(),
            egui::pos2(rect.left() + 10.0, rect.bottom()),
        ),
        8.0,
        color,
    );
    painter.rect_stroke(
        rect,
        8.0,
        egui::Stroke::new(2.0_f32, color.gamma_multiply(0.8)),
        egui::StrokeKind::Inside,
    );
    painter.text(
        rect.left_center() + egui::vec2(30.0, 0.0),
        egui::Align2::LEFT_CENTER,
        format!("{:02}", index + 1),
        egui::FontId::proportional(24.0),
        palette.text_secondary,
    );
    painter.text(
        rect.left_center() + egui::vec2(82.0, 0.0),
        egui::Align2::LEFT_CENTER,
        &effect.name,
        egui::FontId::proportional(28.0),
        if effect.bypassed {
            palette.text_secondary
        } else {
            palette.text_primary
        },
    );
    painter.text(
        rect.right_center() - egui::vec2(26.0, 0.0),
        egui::Align2::RIGHT_CENTER,
        status,
        egui::FontId::proportional(20.0),
        color,
    );
    response.clicked()
}
