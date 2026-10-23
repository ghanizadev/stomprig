use super::{LivePalette, LiveWindowAction};

pub(super) fn show_preset(
    ui: &mut egui::Ui,
    palette: &LivePalette,
    name: Option<&str>,
    boards: Option<&[String]>,
    current_board_name: Option<&str>,
    action: &mut Option<LiveWindowAction>,
) {
    let Some(name) = name else {
        show_empty(ui, palette, "NO LIVE PRESET SELECTED");
        return;
    };
    ui.label(
        egui::RichText::new(name)
            .size(34.0)
            .strong()
            .color(palette.text_primary),
    );
    ui.add_space(14.0);
    let Some(boards) = boards else {
        show_empty(ui, palette, "NO BOARD LIST");
        return;
    };
    if boards.is_empty() {
        show_empty(ui, palette, "NO BOARD PRESETS");
        return;
    }

    let list_height = ui.available_height().max(0.0);
    egui::ScrollArea::vertical()
        .id_salt("live-stage-preset-boards")
        .max_height(list_height)
        .auto_shrink([false, false])
        .show(ui, |ui| {
            for (index, board) in boards.iter().enumerate() {
                let loaded = current_board_name == Some(board.as_str());
                if show_board_row(ui, palette, index, board, loaded) {
                    *action = Some(LiveWindowAction::SelectBoard(board.clone()));
                }
                ui.add_space(7.0);
            }
        });
}

fn show_board_row(
    ui: &mut egui::Ui,
    palette: &LivePalette,
    index: usize,
    board: &str,
    loaded: bool,
) -> bool {
    let color = if loaded { palette.green } else { palette.cyan };
    let fill = if loaded {
        egui::Color32::from_rgb(24, 52, 37)
    } else {
        palette.panel_highlight
    };
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(ui.available_width().max(1.0), 58.0),
        egui::Sense::click(),
    );
    if response.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    let painter = ui.painter();
    painter.rect_filled(
        rect,
        7.0,
        if response.hovered() {
            palette.panel_highlight
        } else {
            fill
        },
    );
    painter.rect_stroke(
        rect,
        7.0,
        egui::Stroke::new(2.0_f32, color.gamma_multiply(0.7)),
        egui::StrokeKind::Inside,
    );
    painter.text(
        rect.left_center() + egui::vec2(20.0, 0.0),
        egui::Align2::LEFT_CENTER,
        format!("{:02}", index + 1),
        egui::FontId::proportional(20.0),
        palette.text_secondary,
    );
    painter.text(
        rect.left_center() + egui::vec2(70.0, 0.0),
        egui::Align2::LEFT_CENTER,
        board,
        egui::FontId::proportional(23.0),
        palette.text_primary,
    );
    if loaded {
        painter.text(
            rect.right_center() - egui::vec2(20.0, 0.0),
            egui::Align2::RIGHT_CENTER,
            "LOADED",
            egui::FontId::proportional(17.0),
            palette.green,
        );
    }
    response.clicked()
}

fn show_empty(ui: &mut egui::Ui, palette: &LivePalette, message: &str) {
    ui.add_space(30.0);
    ui.vertical_centered(|ui| {
        ui.label(
            egui::RichText::new(message)
                .size(28.0)
                .strong()
                .color(palette.text_secondary),
        );
    });
}
