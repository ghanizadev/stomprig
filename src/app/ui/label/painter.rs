use egui::{Align2, Color32, FontId, Painter, Pos2};

/// Paint text that belongs to a custom canvas overlay while keeping text rendering behind the
/// application label boundary.
pub(crate) fn paint(
    painter: &Painter,
    position: Pos2,
    alignment: Align2,
    text: impl ToString,
    font_id: FontId,
    color: Color32,
) {
    painter.text(position, alignment, text, font_id, color);
}
