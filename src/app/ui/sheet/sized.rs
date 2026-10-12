use super::super::theme::Theme;
use super::{SheetContext, SheetSide};

pub(super) fn content<R>(
    ui: &mut egui::Ui,
    _theme: &Theme,
    context: &mut SheetContext<'_>,
    height: f32,
    add_contents: impl FnOnce(&mut egui::Ui) -> R,
) -> Option<R> {
    debug_assert_eq!(context.side, SheetSide::Bottom);

    let ctx = ui.ctx();
    let state_id = context.id.with("last-open");
    let last_open = ctx
        .data(|data| data.get_temp::<bool>(state_id))
        .unwrap_or(false);
    let was_open = *context.open || last_open;
    let opened_now = *context.open && !last_open;
    let animation = ctx.animate_bool(context.id.with("open-anim"), *context.open);
    if !*context.open && animation <= 0.0 {
        ctx.data_mut(|data| data.insert_temp(state_id, false));
        return None;
    }

    let screen = ctx.available_rect();
    let panel_height = height.max(0.0).min(screen.height());
    let panel_size = egui::vec2(screen.width(), panel_height);
    let base_position = egui::pos2(screen.left(), screen.bottom() - panel_height);
    let animated_position = base_position + egui::vec2(0.0, panel_height) * (1.0 - animation);
    let scrim_layer = egui::LayerId::new(egui::Order::Foreground, context.id.with("overlay"));
    ctx.layer_painter(scrim_layer).rect_filled(
        screen,
        egui::CornerRadius::ZERO,
        egui::Color32::from_black_alpha((160.0 * animation).round().clamp(0.0, 255.0) as u8),
    );

    egui::Area::new(context.id.with("scrim"))
        .order(egui::Order::Foreground)
        .interactable(true)
        .movable(false)
        .fixed_pos(screen.min)
        .show(ctx, |scrim_ui| {
            scrim_ui.allocate_exact_size(screen.size(), egui::Sense::click());
        });

    let mut result = None;
    let content_id = context.id.with("content");
    egui::Area::new(content_id)
        .order(egui::Order::Tooltip)
        .interactable(true)
        .movable(false)
        .fixed_pos(animated_position)
        .show(ctx, |area_ui| {
            let frame = egui::Frame::popup(area_ui.style());
            let content_size = (panel_size - frame.total_margin().sum()).max(egui::Vec2::ZERO);
            let frame_response = frame.show(area_ui, |content_ui| {
                content_ui.set_min_width(content_size.x);
                content_ui.set_max_width(content_size.x);
                content_ui.set_min_height(content_size.y);
                content_ui.set_max_height(content_size.y);
                content_ui.set_min_size(content_size);
                result = Some(add_contents(content_ui));
            });

            let close_rect = egui::Rect::from_min_size(
                egui::pos2(
                    frame_response.response.rect.right() - 40.0,
                    frame_response.response.rect.top() + 8.0,
                ),
                egui::vec2(32.0, 32.0),
            );
            let close_response = area_ui.put(
                close_rect,
                egui::Button::new(
                    egui::RichText::new(lucide_icons::Icon::X.unicode().to_string())
                        .family(egui::FontFamily::Name("lucide".into()))
                        .size(16.0),
                )
                .min_size(close_rect.size()),
            );
            if close_response.clicked() {
                *context.open = false;
            }

            let escape = area_ui.input(|input| input.key_pressed(egui::Key::Escape));
            let any_click = area_ui.input(|input| input.pointer.any_click());
            let interact_position = area_ui.input(|input| input.pointer.interact_pos());
            let contains_click = interact_position
                .map(|position| frame_response.response.rect.contains(position))
                .unwrap_or(false);
            if escape || (!opened_now && was_open && any_click && !contains_click) {
                *context.open = false;
            }
        });

    ctx.data_mut(|data| data.insert_temp(state_id, *context.open));
    result
}
