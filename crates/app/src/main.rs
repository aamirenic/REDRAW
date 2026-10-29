#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title("REDRAW")
            .with_inner_size([1440.0, 900.0])
            .with_min_inner_size([1024.0, 640.0]),
        ..Default::default()
    };
    eframe::run_native(
        "REDRAW",
        options,
        Box::new(|cc| Ok(Box::new(redraw_app::RedrawApp::new(cc)))),
    )
}
