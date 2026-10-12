use super::super::ui::{ButtonSize, Theme, paint as paint_label};
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
    painter.rect_filled(rect, egui::CornerRadius::ZERO, visuals.window_fill());
    paint_label(
        &painter,
        rect.left_top() + egui::vec2(14.0, 19.0),
        egui::Align2::LEFT_CENTER,
        plugin_name,
        egui::FontId::proportional(14.0),
        visuals.text_color(),
    );
    paint_label(
        &painter,
        rect.left_top() + egui::vec2(14.0, 43.0),
        egui::Align2::LEFT_CENTER,
        format!("Plugin editor · {parameter_count} parameters"),
        egui::FontId::proportional(11.0),
        visuals.text_color(),
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
    _theme: &Theme,
    width: f32,
    icon: &dyn Fn(&egui::Painter, egui::Pos2, f32, egui::Color32),
) -> egui::Response {
    let height = ButtonSize::Default.height() * 2.0;
    let desired_width = width.min(ui.available_width()).max(0.0);
    let response = ui.add(egui::Button::new("").min_size(egui::vec2(desired_width, height)));
    icon(
        ui.painter(),
        response.rect.center(),
        ButtonSize::IconLg.icon_size(),
        ui.style().interact(&response).text_color(),
    );
    response
}
