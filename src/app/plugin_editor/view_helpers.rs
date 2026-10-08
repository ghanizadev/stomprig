use super::super::ui::{ButtonSize, ButtonStyle, ButtonVariant, Theme, paint as paint_label};
use lucide_icons::Icon;

pub(crate) fn show_plugin_drag_preview(
    ctx: &egui::Context,
    plugin_name: &str,
    parameter_count: usize,
) {
    let Some(pointer) = ctx.pointer_interact_pos() else {
        return;
    };

    let visuals = &ctx.style().visuals;
    let rect = egui::Rect::from_min_size(pointer + egui::vec2(16.0, 16.0), egui::vec2(280.0, 64.0));
    let painter = ctx.layer_painter(egui::LayerId::new(
        egui::Order::Tooltip,
        egui::Id::new("plugin-drag-preview"),
    ));
    painter.rect_filled(
        rect,
        7.0,
        visuals.widgets.active.bg_fill.gamma_multiply(0.75),
    );
    painter.rect_stroke(
        rect,
        7.0,
        egui::Stroke::new(1.0_f32, visuals.selection.stroke.color.gamma_multiply(0.8)),
        egui::StrokeKind::Outside,
    );
    paint_label(
        &painter,
        rect.left_top() + egui::vec2(14.0, 19.0),
        egui::Align2::LEFT_CENTER,
        plugin_name,
        egui::FontId::proportional(14.0),
        visuals.text_color().gamma_multiply(0.85),
    );
    paint_label(
        &painter,
        rect.left_top() + egui::vec2(14.0, 43.0),
        egui::Align2::LEFT_CENTER,
        format!("Plugin editor · {parameter_count} parameters"),
        egui::FontId::proportional(11.0),
        visuals.text_color().gamma_multiply(0.65),
    );
}

pub(crate) fn paint_lucide_icon(
    painter: &egui::Painter,
    center: egui::Pos2,
    size: f32,
    color: egui::Color32,
    icon: Icon,
) {
    painter.text(
        center,
        egui::Align2::CENTER_CENTER,
        icon.unicode(),
        egui::FontId::new(size, egui::FontFamily::Name("lucide".to_owned().into())),
        color,
    );
}

pub(crate) fn show_tall_outline_icon_button(
    ui: &mut egui::Ui,
    theme: &Theme,
    width: f32,
    icon: &dyn Fn(&egui::Painter, egui::Pos2, f32, egui::Color32),
) -> egui::Response {
    let height = ButtonSize::Default.height() * 2.0;
    let (rect, response) = ui.allocate_exact_size(egui::vec2(width, height), egui::Sense::click());
    let style = ButtonStyle::from_variant(theme, ButtonVariant::Outline);
    let background = if response.is_pointer_button_down_on() {
        style.bg_active
    } else if response.hovered() {
        style.bg_hover
    } else {
        style.bg
    };
    let border = if response.hovered() {
        style.border_hover
    } else {
        style.border
    };
    let painter = ui.painter();
    painter.rect_filled(rect, style.rounding, background);
    painter.rect_stroke(
        rect,
        style.rounding,
        egui::Stroke::new(1.0_f32, border),
        egui::StrokeKind::Inside,
    );
    if response.has_focus() {
        painter.rect_stroke(
            rect.expand(2.0),
            style.rounding,
            egui::Stroke::new(3.0_f32, style.focus_ring),
            egui::StrokeKind::Outside,
        );
    }
    icon(
        painter,
        rect.center(),
        ButtonSize::IconLg.icon_size(),
        style.text,
    );
    response
}
