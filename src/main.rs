#![windows_subsystem = "windows"]

use eframe::egui;
use uzakbaglanti::app::RemoteApp;

#[tokio::main]
async fn main() -> Result<(), eframe::Error> {
    let icon_bytes = include_bytes!("../assets/icon.png");
    let icon = eframe::icon_data::from_png_bytes(icon_bytes).ok();

    let mut viewport = egui::ViewportBuilder::default()
        .with_inner_size([980.0, 590.0])
        .with_min_inner_size([820.0, 500.0])
        .with_title("Lusid - Uzak Bağlantı (Admin)");

    if let Some(ic) = icon {
        viewport = viewport.with_icon(ic);
    }

    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };

    eframe::run_native(
        "Lusid",
        options,
        Box::new(|cc| Ok(Box::new(RemoteApp::new(cc)))),
    )
}
