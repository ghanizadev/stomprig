use super::cpu::CpuUsage;
use std::time::Duration;

pub(crate) fn show(ui: &mut egui::Ui, cpu_usage: &mut CpuUsage) {
    ui.horizontal(|ui| {
        ui.weak("Ready");
        ui.separator();
        match cpu_usage.sample() {
            Some(usage) => ui.label(format!("CPU {:.1}%", usage)),
            None => ui.weak("CPU --"),
        };
    });
    ui.ctx().request_repaint_after(Duration::from_millis(500));
}
