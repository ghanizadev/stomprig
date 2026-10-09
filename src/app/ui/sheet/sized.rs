pub(super) fn content<R>(
    ui: &mut egui::Ui,
    theme: &egui_shadcn::Theme,
    context: &mut egui_shadcn::SheetContext<'_>,
    height: f32,
    add_contents: impl FnOnce(&mut egui::Ui) -> R,
) -> Option<R> {
    debug_assert_eq!(context.side, egui_shadcn::SheetSide::Bottom);

    let ctx = ui.ctx();
    let state_id = context.id_source.with("last-open");
    let last_open = ctx
        .data(|data| data.get_temp::<bool>(state_id))
        .unwrap_or(false);
    let was_open = *context.open || last_open;
    let opened_now = *context.open && !last_open;

    let animation = ctx.animate_bool(context.id_source.with("open-anim"), *context.open);
    if !*context.open && animation <= 0.0 {
        ctx.data_mut(|data| data.insert_temp(state_id, *context.open));
        return None;
    }

    let screen = ctx.available_rect();
    let panel_height = height.max(0.0).min(screen.height());
    let panel_size = egui::vec2(screen.width(), panel_height);
    let base_position = egui::pos2(screen.left(), screen.bottom() - panel_height);
    let animated_position = base_position + egui::vec2(0.0, panel_height) * (1.0 - animation);

    let overlay_id = egui::LayerId::new(egui::Order::Foreground, context.id_source.with("overlay"));
    ctx.layer_painter(overlay_id).rect_filled(
        screen,
        egui::CornerRadius::same(0),
        egui::Color32::from_rgba_unmultiplied(
            0,
            0,
            0,
            (160.0 * animation).round().clamp(0.0, 255.0) as u8,
        ),
    );

    egui::Area::new(context.id_source.with("scrim"))
        .order(egui::Order::Foreground)
        .interactable(true)
        .movable(false)
        .fixed_pos(screen.min)
        .show(ctx, |scrim_ui| {
            scrim_ui.allocate_exact_size(screen.size(), egui::Sense::click());
        });

    let mut result = None;
    let content_id = context.id_source.with("content");
    egui::Area::new(content_id)
        .order(egui::Order::Tooltip)
        .interactable(true)
        .movable(false)
        .fixed_pos(animated_position)
        .show(ctx, |area_ui| {
            let border = egui::Stroke::new(1.0_f32, theme.palette.border);
            let shadow = egui::epaint::Shadow {
                offset: [0, 4],
                blur: 12,
                spread: 0,
                color: egui::Color32::from_rgba_unmultiplied(
                    theme.palette.foreground.r(),
                    theme.palette.foreground.g(),
                    theme.palette.foreground.b(),
                    70,
                ),
            };
            let frame = egui::Frame::popup(area_ui.style())
                .fill(theme.palette.background)
                .stroke(border)
                .shadow(shadow)
                .corner_radius(egui::CornerRadius::same(0));

            let frame_response = frame.show(area_ui, |content_ui| {
                content_ui.set_min_width(panel_size.x);
                content_ui.set_max_width(panel_size.x);
                content_ui.set_min_height(panel_size.y);
                content_ui.set_max_height(panel_size.y);
                content_ui.set_min_size(panel_size);
                result = Some(add_contents(content_ui));
            });

            let close_size = egui::vec2(24.0, 24.0);
            let close_rect = egui::Rect::from_min_size(
                egui::pos2(
                    frame_response.response.rect.right() - 16.0 - close_size.x,
                    frame_response.response.rect.top() + 16.0,
                ),
                close_size,
            );
            let close_response = area_ui.interact(
                close_rect,
                content_id.with("close-button"),
                egui::Sense::click(),
            );
            let icon_color = if close_response.hovered() {
                theme.palette.foreground
            } else {
                theme.palette.muted_foreground
            };
            area_ui.painter().text(
                close_rect.center(),
                egui::Align2::CENTER_CENTER,
                lucide_icons::Icon::X.unicode().to_string(),
                egui::FontId::proportional(16.0),
                icon_color,
            );

            if close_response.has_focus() {
                area_ui.painter().rect_stroke(
                    close_rect,
                    egui::CornerRadius::same(2),
                    egui::Stroke::new(2.0_f32, theme.palette.ring),
                    egui::StrokeKind::Outside,
                );
            }
            if close_response.clicked() {
                close_response.request_focus();
                *context.open = false;
            }

            let escape = area_ui.input(|input| input.key_pressed(egui::Key::Escape));
            let any_click = area_ui.input(|input| input.pointer.any_click());
            let interact_position = area_ui.input(|input| input.pointer.interact_pos());
            let contains_click = interact_position
                .map(|position| frame_response.response.rect.contains(position))
                .unwrap_or(false);
            let outside_click = !opened_now && was_open && any_click && !contains_click;
            if escape || outside_click {
                *context.open = false;
            }
        });

    ctx.data_mut(|data| data.insert_temp(state_id, *context.open));
    result
}
