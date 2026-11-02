use crate::network::{
    connect_to_host, generate_device_id, register_pin_tunnel, start_cloudflare_tunnel,
    start_host_server, ClientSessionHandles, HostServerEvents, NetworkUtils, TunnelInfo,
    DEFAULT_PORT,
};
use crate::protocol::{MouseBtn, RemoteInput};
use eframe::egui::{
    self, Color32, ColorImage, ImageData, Key, Pos2, Rect, RichText, Stroke, TextureHandle, Vec2,
};
use rand::Rng;
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;

// Rastgele 6 haneli şifre üretici
pub fn generate_password() -> String {
    let chars: &[u8] = b"abcdefghjkmnpqrstuvwxyz23456789";
    let mut rng = rand::thread_rng();
    (0..6)
        .map(|_| chars[rng.gen_range(0..chars.len())] as char)
        .collect()
}

pub struct RemoteApp {
    // Host (Bu Bilgisayar) Bilgileri
    my_id: String,
    my_pin_digits: String,
    my_pin_formatted: String,
    my_ip_port: String,
    my_password: Arc<Mutex<String>>,
    is_host_active: bool,
    host_session_active: bool,
    incoming_request: Option<(String, mpsc::Sender<bool>)>,

    // Zero-Config WAN İnternet Tüneli (Cloudflare TLS 1.3)
    tunnel_subdomain: Option<String>,
    tunnel_status: String,
    rx_tunnel_info: mpsc::Receiver<TunnelInfo>,
    tunnel_child: Arc<Mutex<Option<std::process::Child>>>,

    // Client (Uzak Bağlantı) Bilgileri
    target_input: String,
    target_password: String,
    client_status: String,
    is_connecting: bool,
    client_handles: Option<ClientSessionHandles>,

    // Uzak Masaüstü Ekranı (Viewer)
    remote_texture: Option<TextureHandle>,
    remote_resolution: (u32, u32),

    // Siber Temalar & Doku Kaynakları
    icon_texture: Option<TextureHandle>,
    bg_texture: Option<TextureHandle>,

    // Kanallar
    rx_incoming_req: mpsc::Receiver<(String, mpsc::Sender<bool>)>,
    rx_session_status: mpsc::Receiver<bool>,
    tx_client_result: mpsc::Sender<Result<ClientSessionHandles, String>>,
    rx_client_result: mpsc::Receiver<Result<ClientSessionHandles, String>>,
}

impl RemoteApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let local_ip = NetworkUtils::get_local_ip();
        let (pin_digits, pin_formatted) = generate_device_id();
        let my_id = pin_formatted.clone();
        let my_ip_port = format!("{}:{}", local_ip, DEFAULT_PORT);
        let initial_pw = generate_password();
        let my_password = Arc::new(Mutex::new(initial_pw));

        let (incoming_tx, incoming_rx) = mpsc::channel(1);
        let (session_tx, session_rx) = mpsc::channel(5);
        let (client_res_tx, client_res_rx) = mpsc::channel(1);
        let (tunnel_info_tx, tunnel_info_rx) = mpsc::channel(2);
        let tunnel_child = Arc::new(Mutex::new(None));

        // Host WebSocket Sunucusunu Başlat
        let pw_clone = my_password.clone();
        tokio::spawn(async move {
            let events = HostServerEvents {
                incoming_request_tx: incoming_tx,
                session_active_tx: session_tx,
            };
            if let Err(e) = start_host_server(DEFAULT_PORT, pw_clone, events).await {
                eprintln!("Sunucu başlatma hatası: {:?}", e);
            }
        });

        // Güvenli WAN İnternet Tünelini Başlat (Cloudflare TLS 1.3 - Sansür & Port Açma Yok)
        let child_clone = tunnel_child.clone();
        tokio::spawn(async move {
            start_cloudflare_tunnel(DEFAULT_PORT, tunnel_info_tx, child_clone).await;
        });

        // 1. Lusid İkonunu Yükle
        let icon_texture = image::load_from_memory(include_bytes!("../assets/icon.png"))
            .ok()
            .map(|img| {
                let rgba = img.to_rgba8();
                let size = [rgba.width() as usize, rgba.height() as usize];
                let color_image = ColorImage::from_rgba_unmultiplied(size, &rgba);
                cc.egui_ctx.load_texture(
                    "lusid_icon",
                    ImageData::Color(Arc::new(color_image)),
                    egui::TextureOptions::LINEAR,
                )
            });

        // 2. Siber Arka Plan Dokusunu Yükle
        let bg_texture = image::load_from_memory(include_bytes!("../assets/theme_preview.jpg"))
            .ok()
            .map(|img| {
                let rgba = img.to_rgba8();
                let size = [rgba.width() as usize, rgba.height() as usize];
                let color_image = ColorImage::from_rgba_unmultiplied(size, &rgba);
                cc.egui_ctx.load_texture(
                    "cyber_bg",
                    ImageData::Color(Arc::new(color_image)),
                    egui::TextureOptions::LINEAR,
                )
            });

        // 3. Synthwave / Cyberpunk Genel Stil Ayarları
        let mut visuals = egui::Visuals::dark();
        visuals.panel_fill = Color32::from_rgb(6, 2, 14);
        visuals.window_fill = Color32::from_rgb(10, 4, 20);
        visuals.extreme_bg_color = Color32::from_rgb(14, 6, 26);
        visuals.widgets.inactive.bg_fill = Color32::from_rgba_premultiplied(16, 8, 30, 220);
        visuals.widgets.inactive.bg_stroke = Stroke::new(1.0_f32, Color32::from_rgb(140, 0, 180));
        visuals.widgets.hovered.bg_fill = Color32::from_rgba_premultiplied(28, 12, 50, 240);
        visuals.widgets.hovered.bg_stroke = Stroke::new(1.5_f32, Color32::from_rgb(0, 220, 255));
        visuals.widgets.active.bg_fill = Color32::from_rgba_premultiplied(35, 18, 65, 255);
        visuals.widgets.active.bg_stroke = Stroke::new(2.0_f32, Color32::from_rgb(0, 240, 255));
        cc.egui_ctx.set_visuals(visuals);

        Self {
            my_id,
            my_pin_digits: pin_digits,
            my_pin_formatted: pin_formatted,
            my_ip_port,
            my_password,
            is_host_active: true,
            host_session_active: false,
            incoming_request: None,

            tunnel_subdomain: None,
            tunnel_status: "🌐 Cloudflare güvenli tüneli başlatılıyor...".to_string(),
            rx_tunnel_info: tunnel_info_rx,
            tunnel_child,

            target_input: "".to_string(),
            target_password: "".to_string(),
            client_status: "Bağlantıya hazır.".to_string(),
            is_connecting: false,
            client_handles: None,

            remote_texture: None,
            remote_resolution: (0, 0),

            icon_texture,
            bg_texture,

            rx_incoming_req: incoming_rx,
            rx_session_status: session_rx,
            tx_client_result: client_res_tx,
            rx_client_result: client_res_rx,
        }
    }
}

impl Drop for RemoteApp {
    fn drop(&mut self) {
        if let Ok(mut lock) = self.tunnel_child.lock() {
            if let Some(mut child) = lock.take() {
                let _ = child.kill();
            }
        }
    }
}

// ==========================================
// SİBER KART VE NEON ÇİZİM YARDIMCILARI
// ==========================================

fn draw_cyber_neon_panel(painter: &egui::Painter, rect: Rect, theme_color: Color32) {
    // A. Dış Yumuşak Neon Bloom (Glow)
    painter.rect_stroke(
        rect.expand(5.0),
        egui::Rounding::same(12.0),
        Stroke::new(
            4.0_f32,
            Color32::from_rgba_premultiplied(
                theme_color.r(),
                theme_color.g(),
                theme_color.b(),
                22,
            ),
        ),
    );
    painter.rect_stroke(
        rect.expand(2.5),
        egui::Rounding::same(10.0),
        Stroke::new(
            2.0_f32,
            Color32::from_rgba_premultiplied(
                theme_color.r(),
                theme_color.g(),
                theme_color.b(),
                55,
            ),
        ),
    );

    // B. Ana Kart Arka Planı (Karanlık Yarı-Saydam Mor)
    painter.rect(
        rect,
        egui::Rounding::same(8.0),
        Color32::from_rgba_premultiplied(8, 4, 18, 235),
        Stroke::new(1.8_f32, theme_color),
    );

    // C. İç İnce Teknoloji Çizgisi
    let inner_rect = rect.shrink(3.5);
    painter.rect_stroke(
        inner_rect,
        egui::Rounding::same(6.0),
        Stroke::new(
            0.7_f32,
            Color32::from_rgba_premultiplied(
                theme_color.r(),
                theme_color.g(),
                theme_color.b(),
                80,
            ),
        ),
    );

    // D. Fütüristik Siber Köşe Çentikleri (Cyber Brackets & White Accents)
    let b_len = 16.0;
    let bracket_stroke = Stroke::new(2.2_f32, Color32::from_rgb(220, 240, 255));

    // Sol-Üst
    painter.line_segment([rect.left_top(), rect.left_top() + Vec2::new(b_len, 0.0)], bracket_stroke);
    painter.line_segment([rect.left_top(), rect.left_top() + Vec2::new(0.0, b_len)], bracket_stroke);

    // Sağ-Üst
    painter.line_segment([rect.right_top(), rect.right_top() + Vec2::new(-b_len, 0.0)], bracket_stroke);
    painter.line_segment([rect.right_top(), rect.right_top() + Vec2::new(0.0, b_len)], bracket_stroke);

    // Sol-Alt
    painter.line_segment([rect.left_bottom(), rect.left_bottom() + Vec2::new(b_len, 0.0)], bracket_stroke);
    painter.line_segment([rect.left_bottom(), rect.left_bottom() + Vec2::new(0.0, -b_len)], bracket_stroke);

    // Sağ-Alt
    painter.line_segment([rect.right_bottom(), rect.right_bottom() + Vec2::new(-b_len, 0.0)], bracket_stroke);
    painter.line_segment([rect.right_bottom(), rect.right_bottom() + Vec2::new(0.0, -b_len)], bracket_stroke);
}

impl eframe::App for RemoteApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // 1. Arka Plandan Gelen Olayları Kontrol Et
        if let Ok(info) = self.rx_tunnel_info.try_recv() {
            self.tunnel_subdomain = Some(info.id.clone());
            self.my_id = self.my_pin_formatted.clone();
            self.tunnel_status = format!("🌐 Cloudflare Tüneli: {} ({}.trycloudflare.com)", self.my_pin_formatted, info.id);
            register_pin_tunnel(&self.my_pin_digits, &info.id);
        }
        if let Ok(req) = self.rx_incoming_req.try_recv() {
            self.incoming_request = Some(req);
        }
        if let Ok(active) = self.rx_session_status.try_recv() {
            self.host_session_active = active;
        }
        if let Ok(res) = self.rx_client_result.try_recv() {
            self.is_connecting = false;
            match res {
                Ok(handles) => {
                    self.client_status = "✅ Bağlantı onaylandı! Uzak masaüstü açılıyor...".into();
                    self.client_handles = Some(handles);
                }
                Err(e) => {
                    self.client_status = format!("❌ Bağlantı başarısız: {}", e);
                }
            }
        }

        // 2. Uzak Masaüstü Karesi Geldiyse Texture Güncelle
        if let Some(handles) = &mut self.client_handles {
            while let Ok((width, height, rgba)) = handles.rx_frame.try_recv() {
                self.remote_resolution = (width, height);
                let color_image = ColorImage::from_rgba_unmultiplied(
                    [width as usize, height as usize],
                    &rgba,
                );
                self.remote_texture = Some(ctx.load_texture(
                    "remote_screen",
                    ImageData::Color(Arc::new(color_image)),
                    egui::TextureOptions::LINEAR,
                ));
            }
        }

        // 3. ÜST BİLGİ VE BAŞLIK ÇUBUĞU (Cyber Header)
        egui::TopBottomPanel::top("cyber_header_panel")
            .frame(
                egui::Frame::default()
                    .fill(Color32::from_rgb(8, 3, 16))
                    .inner_margin(egui::Margin::symmetric(14.0, 8.0)),
            )
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    // Lusid Logo Simgesi
                    if let Some(icon) = &self.icon_texture {
                        ui.image((icon.id(), Vec2::new(26.0, 26.0)));
                    } else {
                        ui.label(RichText::new("◖L◗").strong().color(Color32::from_rgb(0, 220, 255)));
                    }

                    ui.add_space(4.0);
                    ui.label(
                        RichText::new("Lusid")
                            .strong()
                            .size(17.0)
                            .color(Color32::from_rgb(255, 0, 180)),
                    );
                    ui.label(
                        RichText::new("Uzak Bağlantı (Admin - TeamViewer Tarzı)")
                            .size(14.0)
                            .color(Color32::from_rgb(210, 180, 255)),
                    );

                    ui.separator();

                    ui.label(
                        RichText::new("🛡️ RUST REMOTE DESKTOP (Yönetici & Tam Yetki)")
                            .size(13.0)
                            .color(Color32::from_rgb(100, 200, 255)),
                    );

                    ui.label(RichText::new("|").color(Color32::DARK_GRAY));

                    // Sunucu Durum Rozeti
                    if self.host_session_active {
                        ui.colored_label(
                            Color32::from_rgb(255, 230, 0),
                            "● Oturum Aktif (Ekran Paylaşılıyor)",
                        );
                    } else if self.is_host_active {
                        ui.colored_label(
                            Color32::from_rgb(32, 232, 68),
                            "□ Sunucu Hazır (Bağlantı Bekleniyor)",
                        );
                    } else {
                        ui.colored_label(Color32::RED, "● Sunucu Kapalı");
                    }
                });
            });

        // 4. MÜŞTERİYE GELEN BAĞLANTI ONAY POP-UP'I (Glow Alert Modal)
        if let Some((tech_name, approval_tx)) = self.incoming_request.take() {
            let mut accepted_or_rejected = None;

            egui::Window::new("⚠️ Gelen Uzak Bağlantı Talebi")
                .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
                .resizable(false)
                .collapsible(false)
                .frame(
                    egui::Frame::default()
                        .fill(Color32::from_rgb(14, 4, 22))
                        .stroke(Stroke::new(2.0_f32, Color32::from_rgb(255, 0, 100)))
                        .inner_margin(egui::Margin::same(20.0))
                        .rounding(egui::Rounding::same(10.0)),
                )
                .show(ctx, |ui| {
                    ui.add_space(4.0);
                    ui.label(
                        RichText::new(format!("Teknisyen [{}] bilgisayarınıza bağlanmak istiyor.", tech_name))
                            .strong()
                            .size(16.0)
                            .color(Color32::from_rgb(0, 230, 255)),
                    );
                    ui.label(
                        RichText::new("✅ Şifre başarıyla doğrulandı.")
                            .size(14.0)
                            .color(Color32::from_rgb(32, 232, 68)),
                    );
                    ui.add_space(8.0);
                    ui.colored_label(
                        Color32::from_rgb(255, 200, 40),
                        "DİKKAT: Onaylarsanız karşı taraf bilgisayarınızı TAM YÖNETİCİ yetkisiyle kontrol edecektir!",
                    );
                    ui.add_space(16.0);

                    ui.horizontal(|ui| {
                        // KULLANICININ ONAY BUTONU (Yeşil Neon)
                        let accept_btn = ui.add(
                            egui::Button::new(
                                RichText::new("✅ Bağlantıyı Kabul Et (Tam Yetki)")
                                    .size(15.0)
                                    .color(Color32::BLACK),
                            )
                            .fill(Color32::from_rgb(32, 232, 68))
                            .rounding(egui::Rounding::same(6.0)),
                        );
                        if accept_btn.clicked() {
                            accepted_or_rejected = Some(true);
                        }

                        ui.add_space(10.0);
                        // REDDET BUTONU
                        let reject_btn = ui.add(
                            egui::Button::new(
                                RichText::new("❌ Reddet")
                                    .size(15.0)
                                    .color(Color32::WHITE),
                            )
                            .fill(Color32::from_rgb(180, 20, 50))
                            .rounding(egui::Rounding::same(6.0)),
                        );
                        if reject_btn.clicked() {
                            accepted_or_rejected = Some(false);
                        }
                    });
                });

            if let Some(accepted) = accepted_or_rejected {
                let _ = approval_tx.try_send(accepted);
            } else {
                self.incoming_request = Some((tech_name, approval_tx));
            }
        }

        // 5. Eğer Teknisyen olarak bir makineye bağlıysak: UZAK MASAÜSTÜ İZLEYİCİ GÖRÜNÜMÜ
        if self.client_handles.is_some() {
            self.render_remote_viewer(ctx);
            return;
        }

        // 6. ANA SİBER KULLANICI ARAYÜZÜ (Cyberpunk Dual Dashboard)
        egui::CentralPanel::default()
            .frame(egui::Frame::default().fill(Color32::from_rgb(6, 2, 12)))
            .show(ctx, |ui| {
                let full_rect = ui.available_rect_before_wrap();

                // Arka Plan Siber Doku Çizimi
                if let Some(bg_tex) = &self.bg_texture {
                    ui.painter().image(
                        bg_tex.id(),
                        full_rect,
                        Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(1.0, 1.0)),
                        Color32::from_rgba_premultiplied(160, 160, 190, 110), // Hafif saydam siber doku
                    );
                }

                ui.add_space(12.0);

                // İki Ana Siber Kart Düzeni
                let available_size = ui.available_size();
                let card_width = (available_size.x - 24.0) / 2.0;
                let card_height = available_size.y - 16.0;

                ui.horizontal(|ui| {
                    // ==========================================
                    // SOL KART: BU CİHAZA İZİN VER (Neon Magenta)
                    // ==========================================
                    let (left_rect, _) = ui.allocate_exact_size(Vec2::new(card_width, card_height), egui::Sense::hover());
                    draw_cyber_neon_panel(ui.painter(), left_rect, Color32::from_rgb(220, 0, 180));

                    let mut left_ui = ui.new_child(
                        egui::UiBuilder::new()
                            .max_rect(left_rect.shrink(18.0))
                            .layout(egui::Layout::top_down(egui::Align::LEFT)),
                    );

                    left_ui.horizontal(|ui| {
                        ui.label(RichText::new("📥").size(20.0).color(Color32::from_rgb(0, 229, 255)));
                        ui.heading(
                            RichText::new("Bu Cihaza İzin Ver")
                                .strong()
                                .size(19.0)
                                .color(Color32::from_rgb(0, 229, 255)),
                        );
                    });

                    left_ui.add_space(2.0);
                    left_ui.label(
                        RichText::new("Bilgisayarınıza uzaktan müdahale edilmesi için bu bilgileri yetkiyene iletin:")
                            .size(12.5)
                            .color(Color32::from_rgb(210, 190, 240)),
                    );

                    left_ui.add_space(16.0);
                    left_ui.horizontal(|ui| {
                        ui.label(RichText::new("Cihaz ID:").size(14.0).color(Color32::from_rgb(128, 216, 255)));
                        if self.tunnel_subdomain.is_some() {
                            ui.label(RichText::new("● İNTERNET AKTİF (Cloudflare TLS 1.3)").size(11.0).strong().color(Color32::from_rgb(32, 232, 68)));
                        } else {
                            ui.label(RichText::new("○ Tünel Bekleniyor...").size(11.0).color(Color32::from_rgb(255, 200, 0)));
                        }
                    });
                    left_ui.horizontal(|ui| {
                        ui.monospace(
                            RichText::new(&self.my_id)
                                .size(26.0)
                                .strong()
                                .color(Color32::from_rgb(32, 232, 68)), // Parlak Neon Yeşil
                        );

                        ui.add_space(8.0);
                        let copy_btn = ui.add(
                            egui::Button::new(
                                RichText::new("📋 Kopyala")
                                    .size(13.0)
                                    .color(Color32::from_rgb(0, 229, 255)),
                            )
                            .stroke(Stroke::new(1.2_f32, Color32::from_rgb(0, 180, 255)))
                            .fill(Color32::from_rgba_premultiplied(0, 32, 64, 180))
                            .rounding(egui::Rounding::same(6.0)),
                        );
                        if copy_btn.clicked() {
                            ui.output_mut(|o| o.copied_text = self.my_id.clone());
                        }

                        ui.add_space(4.0);
                        let refresh_id_btn = ui.add(
                            egui::Button::new(
                                RichText::new("🔄")
                                    .size(13.0)
                                    .color(Color32::from_rgb(0, 229, 255)),
                            )
                            .stroke(Stroke::new(1.2_f32, Color32::from_rgb(0, 180, 255)))
                            .fill(Color32::from_rgba_premultiplied(0, 32, 64, 180))
                            .rounding(egui::Rounding::same(6.0)),
                        );
                        if refresh_id_btn.clicked() {
                            let (digits, formatted) = generate_device_id();
                            self.my_pin_digits = digits.clone();
                            self.my_pin_formatted = formatted.clone();
                            self.my_id = formatted;
                            if let Some(sub) = &self.tunnel_subdomain {
                                register_pin_tunnel(&self.my_pin_digits, sub);
                            }
                        }
                    });

                    left_ui.add_space(14.0);
                    let mut pw_guard = self.my_password.lock().unwrap();
                    left_ui.label(RichText::new("Tek Kullanımlık Şifre:").size(14.0).color(Color32::from_rgb(128, 216, 255)));
                    left_ui.horizontal(|ui| {
                        ui.monospace(
                            RichText::new(&*pw_guard)
                                .size(23.0)
                                .strong()
                                .color(Color32::from_rgb(255, 230, 0)), // Parlak Neon Sarı
                        );

                        ui.add_space(8.0);
                        let refresh_btn = ui.add(
                            egui::Button::new(
                                RichText::new("🔄 Yenile")
                                    .size(13.0)
                                    .color(Color32::from_rgb(0, 229, 255)),
                            )
                            .stroke(Stroke::new(1.2_f32, Color32::from_rgb(0, 180, 255)))
                            .fill(Color32::from_rgba_premultiplied(0, 32, 64, 180))
                            .rounding(egui::Rounding::same(6.0)),
                        );
                        if refresh_btn.clicked() {
                            *pw_guard = generate_password();
                        }
                    });

                    left_ui.add_space(16.0);
                    left_ui.label(RichText::new("Bağlantı Kanalları:").size(13.0).color(Color32::from_rgb(128, 216, 255)));
                    left_ui.monospace(
                        RichText::new(&self.tunnel_status)
                            .size(12.5)
                            .color(Color32::from_rgb(0, 229, 255)),
                    );
                    left_ui.monospace(
                        RichText::new(format!("🏠 Yerel Ağ: ws://{}/", self.my_ip_port))
                            .size(12.0)
                            .color(Color32::from_rgb(160, 200, 240)),
                    );

                    left_ui.add_space(14.0);
                    left_ui.label(
                        RichText::new("🛡️ Güvenlik & Hız: Cloudflare TLS 1.3 tüneli ile şifreli, sansürsüz ve doğrudan bağlantı. Port yönlendirme gerekmez. Karşı taraf 'Bağlantıyı Kabul Et' onayı verilmeden kontrol edemez.")
                            .size(11.0)
                            .color(Color32::from_rgb(160, 140, 210)),
                    );

                    ui.add_space(12.0);

                    // ==========================================
                    // SAĞ KART: UZAK CİHAZI KONTROL ET (Neon Cyan)
                    // ==========================================
                    let (right_rect, _) = ui.allocate_exact_size(Vec2::new(card_width, card_height), egui::Sense::hover());
                    draw_cyber_neon_panel(ui.painter(), right_rect, Color32::from_rgb(0, 212, 255));

                    let mut right_ui = ui.new_child(
                        egui::UiBuilder::new()
                            .max_rect(right_rect.shrink(18.0))
                            .layout(egui::Layout::top_down(egui::Align::LEFT)),
                    );

                    right_ui.horizontal(|ui| {
                        ui.label(RichText::new("🚀").size(20.0).color(Color32::from_rgb(0, 229, 255)));
                        ui.heading(
                            RichText::new("Uzak Cihazı Kontrol Et")
                                .strong()
                                .size(19.0)
                                .color(Color32::from_rgb(0, 229, 255)),
                        );
                    });

                    right_ui.add_space(2.0);
                    right_ui.label(
                        RichText::new("Müşteriden aldığınız 9 haneli Cihaz ID ve şifreyi girin:")
                            .size(12.5)
                            .color(Color32::from_rgb(210, 190, 240)),
                    );

                    right_ui.add_space(16.0);
                    right_ui.label(RichText::new("Hedef Cihaz ID (9 haneli) veya IP:Port:").size(14.0).color(Color32::from_rgb(128, 216, 255)));

                    let id_edit = egui::TextEdit::singleline(&mut self.target_input)
                        .margin(egui::Margin::symmetric(10.0, 7.0))
                        .font(egui::TextStyle::Monospace)
                        .hint_text("Örn: 742 819 305 veya 192.168.1.50:8990")
                        .text_color(Color32::BLACK);
                    right_ui.add(id_edit);

                    right_ui.add_space(12.0);
                    right_ui.label(RichText::new("Hedef Şifre:").size(14.0).color(Color32::from_rgb(128, 216, 255)));

                    let pw_edit = egui::TextEdit::singleline(&mut self.target_password)
                        .password(true)
                        .margin(egui::Margin::symmetric(10.0, 7.0))
                        .text_color(Color32::BLACK);
                    right_ui.add(pw_edit);

                    right_ui.add_space(20.0);

                    // BÜYÜK SİBER BAĞLAN BUTONU
                    let connect_btn = right_ui.add_enabled(
                        !self.is_connecting,
                        egui::Button::new(
                            RichText::new("⚡  BAĞLAN (Admin Kontrolü)")
                                .size(16.5)
                                .strong()
                                .color(Color32::WHITE),
                        )
                        .min_size(Vec2::new(right_ui.available_width(), 46.0))
                        .stroke(Stroke::new(2.0_f32, Color32::from_rgb(0, 220, 255)))
                        .fill(Color32::from_rgba_premultiplied(0, 36, 75, 230))
                        .rounding(egui::Rounding::same(8.0)),
                    );

                    if connect_btn.clicked() {
                        self.is_connecting = true;
                        self.client_status = "Cihaz ID doğrulanıyor ve bağlanılıyor...".into();

                        let target = self.target_input.clone();
                        let pw = self.target_password.clone();
                        let tx = self.tx_client_result.clone();
                        let ctx_clone = ctx.clone();

                        tokio::spawn(async move {
                            match NetworkUtils::resolve_target_to_url(&target).await {
                                Ok(target_url) => {
                                    let result = connect_to_host(&target_url, "Teknisyen (Admin)".into(), pw).await;
                                    let _ = tx.send(result).await;
                                }
                                Err(err_msg) => {
                                    let _ = tx.send(Err(err_msg)).await;
                                }
                            }
                            ctx_clone.request_repaint();
                        });
                    }

                    right_ui.add_space(14.0);
                    right_ui.label(
                        RichText::new(&self.client_status)
                            .size(13.5)
                            .strong()
                            .color(Color32::from_rgb(32, 232, 68)), // Neon yeşil durum
                    );
                });
            });

        ctx.request_repaint_after(std::time::Duration::from_millis(33));
    }
}

impl RemoteApp {
    // Uzak Masaüstü Görüntüleme Penceresi (Teknisyen Gözü)
    fn render_remote_viewer(&mut self, ctx: &egui::Context) {
        // Üst Kontrol Çubuğu
        egui::TopBottomPanel::top("viewer_bar")
            .frame(
                egui::Frame::default()
                    .fill(Color32::from_rgb(10, 4, 20))
                    .inner_margin(egui::Margin::symmetric(12.0, 6.0)),
            )
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    let disconnect_btn = ui.add(
                        egui::Button::new(RichText::new("🔴 Bağlantıyı Kes").strong().color(Color32::WHITE))
                            .fill(Color32::from_rgb(180, 20, 50))
                            .rounding(egui::Rounding::same(6.0)),
                    );

                    if disconnect_btn.clicked() {
                        self.client_handles = None;
                        self.remote_texture = None;
                        self.client_status = "Bağlantı sonlandırıldı.".into();
                    }

                    ui.separator();
                    ui.label(
                        RichText::new(format!("Çözünürlük: {}x{}", self.remote_resolution.0, self.remote_resolution.1))
                            .color(Color32::from_rgb(0, 220, 255)),
                    );
                    ui.separator();

                    // Hızlı Admin Kısayolları
                    if ui.button(RichText::new("⊞ Windows Tuşu").color(Color32::WHITE)).clicked() {
                        if let Some(handles) = &self.client_handles {
                            let _ = handles.tx_input.try_send(RemoteInput::KeyDown { vk: 0x5B });
                            let _ = handles.tx_input.try_send(RemoteInput::KeyUp { vk: 0x5B });
                        }
                    }
                });
            });

        // Ekran Tuvali
        egui::CentralPanel::default().show(ctx, |ui| {
            if let Some(texture) = &self.remote_texture {
                let available_size = ui.available_size();
                let image_widget = egui::Image::new(texture)
                    .fit_to_exact_size(available_size)
                    .sense(egui::Sense::click_and_drag());

                let response = ui.add(image_widget);

                if let Some(handles) = &self.client_handles {
                    let rect = response.rect;

                    // FARE HAREKETİ VE TIKLAMA
                    if response.hovered() {
                        if let Some(pointer_pos) = ctx.input(|i| i.pointer.hover_pos()) {
                            let norm_x = (pointer_pos.x - rect.left()) / rect.width();
                            let norm_y = (pointer_pos.y - rect.top()) / rect.height();

                            let _ = handles.tx_input.try_send(RemoteInput::MouseMove {
                                x: norm_x.clamp(0.0, 1.0),
                                y: norm_y.clamp(0.0, 1.0),
                            });
                        }

                        // Sol Tık
                        if ctx.input(|i| i.pointer.primary_pressed()) {
                            let _ = handles.tx_input.try_send(RemoteInput::MouseDown {
                                button: MouseBtn::Left,
                            });
                        }
                        if ctx.input(|i| i.pointer.primary_released()) {
                            let _ = handles.tx_input.try_send(RemoteInput::MouseUp {
                                button: MouseBtn::Left,
                            });
                        }

                        // Sağ Tık
                        if ctx.input(|i| i.pointer.secondary_pressed()) {
                            let _ = handles.tx_input.try_send(RemoteInput::MouseDown {
                                button: MouseBtn::Right,
                            });
                        }
                        if ctx.input(|i| i.pointer.secondary_released()) {
                            let _ = handles.tx_input.try_send(RemoteInput::MouseUp {
                                button: MouseBtn::Right,
                            });
                        }

                        // Tekerlek
                        let scroll_delta = ctx.input(|i| i.raw_scroll_delta.y);
                        if scroll_delta.abs() > 0.1 {
                            let _ = handles.tx_input.try_send(RemoteInput::MouseWheel {
                                delta: if scroll_delta > 0.0 { 1 } else { -1 },
                            });
                        }
                    }

                    // KLAVYE GİRDİLERİ
                    ctx.input(|i| {
                        for event in &i.events {
                            if let egui::Event::Key { key, pressed, .. } = event {
                                let vk = match key {
                                    Key::Enter => 0x0D,
                                    Key::Backspace => 0x08,
                                    Key::Tab => 0x09,
                                    Key::Escape => 0x1B,
                                    Key::Space => 0x20,
                                    Key::ArrowLeft => 0x25,
                                    Key::ArrowUp => 0x26,
                                    Key::ArrowRight => 0x27,
                                    Key::ArrowDown => 0x28,
                                    _ => 0,
                                };
                                if vk != 0 {
                                    if *pressed {
                                        let _ = handles.tx_input.try_send(RemoteInput::KeyDown { vk });
                                    } else {
                                        let _ = handles.tx_input.try_send(RemoteInput::KeyUp { vk });
                                    }
                                }
                            }
                        }
                    });
                }
            } else {
                ui.centered_and_justified(|ui| {
                    ui.spinner();
                    ui.heading("Uzak masaüstü ekranı yükleniyor...");
                });
            }
        });
    }
}
