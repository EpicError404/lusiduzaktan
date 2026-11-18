#![windows_subsystem = "windows"]

use eframe::egui;
use uzakbaglanti::app::RemoteApp;

fn main() -> Result<(), eframe::Error> {
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|a| a == "--service") {
        uzakbaglanti::service::run_windows_service();
        return Ok(());
    }
    if args.iter().any(|a| a == "--desktop-agent") {
        uzakbaglanti::service::run_desktop_agent();
        return Ok(());
    }

    // Yönetici yetkisi varsa arka planda UAC / Secure Desktop destek hizmetini başlat
    std::thread::spawn(|| {
        let _ = uzakbaglanti::service::ensure_system_service_running();
    });

    let rt = tokio::runtime::Runtime::new().expect("Tokio runtime başlatılamadı");
    let _guard = rt.enter();

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
