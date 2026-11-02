use crate::capture::ScreenCapturer;
use crate::input::InputInjector;
use crate::protocol::{Packet, RemoteInput};
use futures_util::{SinkExt, StreamExt};
use std::io::{BufRead, BufReader};
use std::net::{Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message;

use rand::Rng;

#[cfg(windows)]
use std::os::windows::process::CommandExt;

const CREATE_NO_WINDOW: u32 = 0x08000000;

pub const DEFAULT_PORT: u16 = 8990;

// 9 haneli kullanıcı dostu Cihaz ID üretir (AnyDesk / TeamViewer formatında: "742 819 305")
pub fn generate_device_id() -> (String, String) {
    let mut rng = rand::thread_rng();
    let num: u32 = rng.gen_range(100_000_000..999_999_999);
    let digits = num.to_string();
    let formatted = format!("{} {} {}", &digits[0..3], &digits[3..6], &digits[6..9]);
    (digits, formatted)
}

// Cihaz ID eşlemesini güvenli brokerlara iletir (ntfy.sh & ntfy.envs.net)
pub fn post_pin_to_brokers(pin: &str, subdomain: &str) {
    let urls = [
        format!("https://ntfy.sh/lusid_pin_{}", pin),
        format!("https://ntfy.envs.net/lusid_pin_{}", pin),
    ];
    for url in &urls {
        #[cfg(windows)]
        let _ = Command::new("curl.exe")
            .args(["-s", "--max-time", "5", "-d", subdomain, url])
            .creation_flags(CREATE_NO_WINDOW)
            .status();

        #[cfg(not(windows))]
        let _ = Command::new("curl")
            .args(["-s", "--max-time", "5", "-d", subdomain, url])
            .status();
    }
}

// Cihaz ID tünel kaydını başlatır ve oturum boyunca arka planda taze tutar (Heartbeat)
pub fn register_pin_tunnel(pin_digits: &str, subdomain: &str) {
    let pin = pin_digits.to_string();
    let sub = subdomain.to_string();
    tokio::spawn(async move {
        // İlk kayıt
        let p1 = pin.clone();
        let s1 = sub.clone();
        tokio::task::spawn_blocking(move || post_pin_to_brokers(&p1, &s1)).await.ok();

        // 2 dakikada bir heartbeat (oturum açık kaldığı sürece taze tutulur)
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(120));
        loop {
            interval.tick().await;
            let p2 = pin.clone();
            let s2 = sub.clone();
            tokio::task::spawn_blocking(move || post_pin_to_brokers(&p2, &s2)).await.ok();
        }
    });
}

// 9 haneli Cihaz ID'sinden Cloudflare subdomain'ini çözer
pub fn resolve_pin_to_subdomain(pin: &str) -> Result<String, String> {
    let urls = [
        format!("https://ntfy.sh/lusid_pin_{}/json?poll=1&since=all", pin),
        format!("https://ntfy.envs.net/lusid_pin_{}/json?poll=1&since=all", pin),
    ];

    for url in &urls {
        #[cfg(windows)]
        let output = Command::new("curl.exe")
            .args(["-s", "--max-time", "4", url])
            .creation_flags(CREATE_NO_WINDOW)
            .output();

        #[cfg(not(windows))]
        let output = Command::new("curl")
            .args(["-s", "--max-time", "4", url])
            .output();

        if let Ok(out) = output {
            if out.status.success() {
                let text = String::from_utf8_lossy(&out.stdout);
                let mut last_msg = None;
                for line in text.lines() {
                    if let Some(pos) = line.find("\"message\":\"") {
                        let rest = &line[pos + 11..];
                        if let Some(end) = rest.find('"') {
                            let msg = rest[..end].trim();
                            if !msg.is_empty() {
                                last_msg = Some(msg.to_string());
                            }
                        }
                    }
                }
                if let Some(subdomain) = last_msg {
                    return Ok(subdomain);
                }
            }
        }
    }

    Err(format!("'{}' numaralı Cihaz ID bulunamadı veya uzak bilgisayar çevrimdışı. Lütfen numarayı kontrol edin.", pin))
}

pub struct NetworkUtils;

impl NetworkUtils {
    // IPv4'ü kullanıcı dostu ID'ye dönüştürür (32-bit u32)
    pub fn ip_to_id(ip: Ipv4Addr) -> String {
        let octets = ip.octets();
        let num = ((octets[0] as u32) << 24)
            | ((octets[1] as u32) << 16)
            | ((octets[2] as u32) << 8)
            | (octets[3] as u32);
        format!("{:03} {:03} {:04}", num / 10_000_000, (num / 10_000) % 1_000, num % 10_000)
    }

    // ID'yi veya doğrudan girilen IP'yi çözer
    pub fn parse_target_address(input: &str) -> Result<SocketAddr, String> {
        let trimmed = input.trim();

        if let Ok(addr) = trimmed.parse::<SocketAddr>() {
            return Ok(addr);
        }
        if let Ok(ip) = trimmed.parse::<Ipv4Addr>() {
            return Ok(SocketAddr::new(std::net::IpAddr::V4(ip), DEFAULT_PORT));
        }

        let digits: String = trimmed.chars().filter(|c| c.is_ascii_digit()).collect();
        if !digits.is_empty() {
            if let Ok(num) = digits.parse::<u32>() {
                let ip = Ipv4Addr::new(
                    (num >> 24) as u8,
                    (num >> 16) as u8,
                    (num >> 8) as u8,
                    num as u8,
                );
                return Ok(SocketAddr::new(std::net::IpAddr::V4(ip), DEFAULT_PORT));
            }
        }

        Err("Geçersiz ID veya IP adresi. Örnek: '213 070 6433' veya '127.0.0.1:8990'".into())
    }

    // Senkron URL dönüştürücü (Doğrudan girilen formatlar için)
    pub fn parse_target_to_url(input: &str) -> Result<String, String> {
        let trimmed = input.trim();
        if trimmed.is_empty() {
            return Err("Hedef adres veya Cihaz ID boş bırakılamaz.".into());
        }

        if trimmed.starts_with("ws://") || trimmed.starts_with("wss://") {
            return Ok(trimmed.to_string());
        }

        if let Some(rest) = trimmed.strip_prefix("https://") {
            return Ok(format!("wss://{}/", rest.trim_end_matches('/')));
        }
        if let Some(rest) = trimmed.strip_prefix("http://") {
            return Ok(format!("ws://{}/", rest.trim_end_matches('/')));
        }

        if trimmed.ends_with(".trycloudflare.com") {
            return Ok(format!("wss://{}/", trimmed));
        }

        if trimmed.contains(':') {
            return Ok(format!("ws://{}/", trimmed));
        }

        if let Ok(ip) = trimmed.parse::<Ipv4Addr>() {
            return Ok(format!("ws://{}:{}/", ip, DEFAULT_PORT));
        }

        if trimmed.contains('-') && !trimmed.contains('/') {
            return Ok(format!("wss://{}.trycloudflare.com/", trimmed));
        }

        if trimmed.contains('.') {
            return Ok(format!("ws://{}:{}/", trimmed, DEFAULT_PORT));
        }

        Ok(format!("wss://{}.trycloudflare.com/", trimmed))
    }

    // Asenkron hedef çözücü: 9 haneli Cihaz ID (PIN), Cloudflare subdomain, Domain veya IP çözer
    pub async fn resolve_target_to_url(input: &str) -> Result<String, String> {
        let trimmed = input.trim();
        if trimmed.is_empty() {
            return Err("Hedef Cihaz ID veya IP adresi boş bırakılamaz.".into());
        }

        // Doğrudan ws:// veya wss:// girilmişse
        if trimmed.starts_with("ws://") || trimmed.starts_with("wss://") {
            return Ok(trimmed.to_string());
        }

        // https:// veya http:// linki yapıştırılmışsa
        if let Some(rest) = trimmed.strip_prefix("https://") {
            return Ok(format!("wss://{}/", rest.trim_end_matches('/')));
        }
        if let Some(rest) = trimmed.strip_prefix("http://") {
            return Ok(format!("ws://{}/", rest.trim_end_matches('/')));
        }

        // xxx.trycloudflare.com formatında girilmişse
        if trimmed.ends_with(".trycloudflare.com") {
            return Ok(format!("wss://{}/", trimmed));
        }

        // IP:PORT veya HOST:PORT (örn: 192.168.1.5:8990 veya 127.0.0.1:8990)
        if trimmed.contains(':') {
            return Ok(format!("ws://{}/", trimmed));
        }

        // Doğrudan IPv4 girilmişse (örn: 192.168.1.50)
        if let Ok(ip) = trimmed.parse::<Ipv4Addr>() {
            return Ok(format!("ws://{}:{}/", ip, DEFAULT_PORT));
        }

        // Rakamlardan oluşan Cihaz ID (örn: "742 819 305" veya "742819305")
        let digits: String = trimmed.chars().filter(|c| c.is_ascii_digit()).collect();
        if !digits.is_empty() && digits.len() >= 6 && digits.len() <= 12 && !trimmed.contains('.') && !trimmed.chars().any(|c| c.is_alphabetic()) {
            let pin = digits.clone();
            let resolve_res = tokio::task::spawn_blocking(move || {
                resolve_pin_to_subdomain(&pin)
            }).await.map_err(|e| format!("Sistem hatası: {}", e))?;

            match resolve_res {
                Ok(subdomain) => {
                    let clean_sub = subdomain.trim_end_matches(".trycloudflare.com").trim();
                    return Ok(format!("wss://{}.trycloudflare.com/", clean_sub));
                }
                Err(e) => {
                    // Eğer brokerda bulunamadıysa, 32-bit IP kodlaması olabilir (örn: 3232235630 -> 192.168.0.110)
                    if let Ok(num) = digits.parse::<u32>() {
                        let ip = Ipv4Addr::new(
                            (num >> 24) as u8,
                            (num >> 16) as u8,
                            (num >> 8) as u8,
                            num as u8,
                        );
                        if !ip.is_unspecified() && !ip.is_broadcast() {
                            return Ok(format!("ws://{}:{}/", ip, DEFAULT_PORT));
                        }
                    }
                    return Err(e);
                }
            }
        }

        // Sadece Cloudflare subdomain ID'si girilmişse (örn. rebel-unless-variation-propecia)
        if trimmed.contains('-') && !trimmed.contains('/') {
            return Ok(format!("wss://{}.trycloudflare.com/", trimmed));
        }

        // Genel bir alan adı girilmişse
        if trimmed.contains('.') {
            return Ok(format!("ws://{}:{}/", trimmed, DEFAULT_PORT));
        }

        // Varsayılan olarak Cloudflare subdomain kabul et
        Ok(format!("wss://{}.trycloudflare.com/", trimmed))
    }

    // Bilgisayarın yerel IP adresini bulur
    pub fn get_local_ip() -> Ipv4Addr {
        match local_ip_address::local_ip() {
            Ok(std::net::IpAddr::V4(ip)) => ip,
            _ => Ipv4Addr::new(127, 0, 0, 1),
        }
    }
}

// ==========================================
// GÜVENLİ İNTERNET TÜNELİ (CLOUDFLARE TLS 1.3)
// ==========================================

#[derive(Clone, Debug)]
pub struct TunnelInfo {
    pub id: String,          // e.g. "mazda-inf-collectors-fewer"
    pub public_url: String,  // e.g. "https://mazda-inf-collectors-fewer.trycloudflare.com"
    pub wss_url: String,     // e.g. "wss://mazda-inf-collectors-fewer.trycloudflare.com/"
}

pub fn locate_or_download_cloudflared() -> Option<PathBuf> {
    // 1. Mevcut executable dizini
    if let Ok(mut exe_dir) = std::env::current_exe() {
        exe_dir.pop();
        let p = exe_dir.join("cloudflared.exe");
        if p.exists() {
            return Some(p);
        }
    }

    // 2. Mevcut çalışma dizini
    let cwd_path = PathBuf::from("cloudflared.exe");
    if cwd_path.exists() {
        return Some(cwd_path);
    }

    // 3. APPDATA\Lusid dizini
    if let Ok(appdata) = std::env::var("APPDATA") {
        let lusid_dir = PathBuf::from(appdata).join("Lusid");
        let cf_path = lusid_dir.join("cloudflared.exe");
        if cf_path.exists() {
            return Some(cf_path);
        }

        // Eğer hiçbir yerde yoksa curl.exe ile arka planda indir
        let _ = std::fs::create_dir_all(&lusid_dir);
        let url = "https://github.com/cloudflare/cloudflared/releases/latest/download/cloudflared-windows-amd64.exe";
        println!("[Lusid Tunnel] cloudflared.exe otomatik indiriliyor...");
        #[cfg(windows)]
        let status = Command::new("curl.exe")
            .args(["-L", "-o", cf_path.to_str().unwrap(), url])
            .creation_flags(CREATE_NO_WINDOW)
            .status();

        #[cfg(not(windows))]
        let status = Command::new("curl")
            .args(["-L", "-o", cf_path.to_str().unwrap(), url])
            .status();

        if let Ok(st) = status {
            if st.success() && cf_path.exists() {
                return Some(cf_path);
            }
        }
    }

    None
}

pub async fn start_cloudflare_tunnel(
    local_port: u16,
    info_tx: mpsc::Sender<TunnelInfo>,
    tunnel_child: Arc<Mutex<Option<Child>>>,
) {
    loop {
        let cf_binary = match locate_or_download_cloudflared() {
            Some(path) => path,
            None => {
                eprintln!("[Lusid Tunnel] cloudflared.exe bulunamadı. 5sn sonra yeniden denenecek...");
                tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
                continue;
            }
        };

        println!("[Lusid Tunnel] Cloudflare güvenli tüneli başlatılıyor: {:?}", cf_binary);

        let mut cmd = Command::new(&cf_binary);
        cmd.args(["tunnel", "--url", &format!("http://127.0.0.1:{}", local_port)])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        #[cfg(windows)]
        cmd.creation_flags(CREATE_NO_WINDOW);

        match cmd.spawn() {
            Ok(mut child) => {
                let stderr = match child.stderr.take() {
                    Some(s) => s,
                    None => {
                        let _ = child.kill();
                        tokio::time::sleep(tokio::time::Duration::from_secs(3)).await;
                        continue;
                    }
                };

                // Child process handle'ını sakla
                if let Ok(mut lock) = tunnel_child.lock() {
                    *lock = Some(child);
                }

                let reader = BufReader::new(stderr);
                let mut tunnel_reported = false;

                for line in reader.lines() {
                    if let Ok(l) = line {
                        if !tunnel_reported {
                            if let Some(pos) = l.find("https://") {
                                let rest = &l[pos + 8..];
                                if let Some(end) = rest.find(".trycloudflare.com") {
                                    let subdomain = rest[..end].trim().to_string();
                                    let public_url = format!("https://{}.trycloudflare.com", subdomain);
                                    let wss_url = format!("wss://{}.trycloudflare.com/", subdomain);

                                    println!("[Lusid Tunnel] Cloudflare tüneli aktif: {}", subdomain);
                                    let _ = info_tx.send(TunnelInfo {
                                        id: subdomain,
                                        public_url,
                                        wss_url,
                                    }).await;
                                    tunnel_reported = true;
                                }
                            }
                        }
                    } else {
                        break;
                    }
                }

                println!("[Lusid Tunnel] Cloudflare tüneli kapandı. 3 saniye sonra yeniden bağlanılıyor...");
            }
            Err(e) => {
                eprintln!("[Lusid Tunnel] cloudflared çalıştırılamadı: {:?}. 5sn sonra tekrar denenecek.", e);
            }
        }

        tokio::time::sleep(tokio::time::Duration::from_secs(3)).await;
    }
}

// ==========================================
// HOST (MÜŞTERİ) SUNUCU DİNLEYİCİSİ
// ==========================================

pub struct HostServerEvents {
    pub incoming_request_tx: mpsc::Sender<(String, mpsc::Sender<bool>)>,
    pub session_active_tx: mpsc::Sender<bool>,
}

pub async fn start_host_server(
    port: u16,
    expected_password: Arc<Mutex<String>>,
    events: HostServerEvents,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let listener = TcpListener::bind(format!("0.0.0.0:{}", port)).await?;

    loop {
        let (stream, _peer_addr) = listener.accept().await?;
        let expected_pw = expected_password.lock().unwrap().clone();
        let req_tx = events.incoming_request_tx.clone();
        let session_tx = events.session_active_tx.clone();

        tokio::spawn(async move {
            if let Err(e) = handle_host_connection(stream, expected_pw, req_tx, session_tx).await {
                eprintln!("Host bağlantı hatası: {:?}", e);
            }
        });
    }
}

async fn handle_host_connection(
    stream: TcpStream,
    expected_pw: String,
    incoming_req_tx: mpsc::Sender<(String, mpsc::Sender<bool>)>,
    session_tx: mpsc::Sender<bool>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let mut ws = tokio_tungstenite::accept_async(stream).await?;

    // 1. HandshakeRequest bekle
    let handshake_packet = match ws.next().await {
        Some(Ok(Message::Binary(bin))) => bincode::deserialize::<Packet>(&bin)?,
        _ => return Err("İlk paket binary bekleniyordu".into()),
    };

    let (client_name, client_pw) = match handshake_packet {
        Packet::HandshakeRequest {
            client_name,
            password,
        } => (client_name, password),
        _ => return Err("Beklenmeyen el sıkışma paketi".into()),
    };

    // 2. Şifre Kontrolü
    if client_pw != expected_pw {
        let resp = Packet::HandshakeResponse {
            accepted: false,
            reason: Some("Hatalı şifre girildi!".into()),
            screen_width: 0,
            screen_height: 0,
        };
        ws.send(Message::Binary(bincode::serialize(&resp)?)).await?;
        ws.close(None).await?;
        return Ok(());
    }

    // 3. UI'a "Kabul Et" Onay Pop-up'ı Gönder
    let (approval_tx, mut approval_rx) = mpsc::channel::<bool>(1);
    if incoming_req_tx.send((client_name, approval_tx)).await.is_err() {
        return Err("UI onay kanalı kapalı".into());
    }

    // Kullanıcının "Bağlantıyı Kabul Et" butonuna basması beklenir
    let is_accepted = approval_rx.recv().await.unwrap_or(false);

    if !is_accepted {
        let resp = Packet::HandshakeResponse {
            accepted: false,
            reason: Some("Kullanıcı bağlantıyı reddetti.".into()),
            screen_width: 0,
            screen_height: 0,
        };
        ws.send(Message::Binary(bincode::serialize(&resp)?)).await?;
        ws.close(None).await?;
        return Ok(());
    }

    // 4. Bağlantı Kabul Edildi -> Oturum Başlat
    let mut capturer = match ScreenCapturer::new() {
        Ok(c) => c,
        Err(e) => {
            let resp = Packet::HandshakeResponse {
                accepted: false,
                reason: Some(format!("Ekran yakalayıcı başlatılamadı: {}", e)),
                screen_width: 0,
                screen_height: 0,
            };
            ws.send(Message::Binary(bincode::serialize(&resp)?)).await?;
            return Ok(());
        }
    };

    let (screen_w, screen_h) = capturer.screen_size();
    let resp = Packet::HandshakeResponse {
        accepted: true,
        reason: None,
        screen_width: screen_w,
        screen_height: screen_h,
    };
    ws.send(Message::Binary(bincode::serialize(&resp)?)).await?;

    let _ = session_tx.send(true).await;

    // Split WebSocket
    let (mut ws_sink, mut ws_stream) = ws.split();

    // Görev A: Ekran Yakalama ve Teknisyene Gönderme
    let screen_task = tokio::spawn(async move {
        let mut last_frame_rgba = Vec::new();

        loop {
            if let Ok(frame) = capturer.capture_frame() {
                // Sadece kare değiştiyse veya ilk kare ise gönder (Bant genişliği tasarrufu)
                let should_send = last_frame_rgba.len() != frame.rgba.len() 
                    || last_frame_rgba.is_empty() 
                    || !last_frame_rgba.iter().zip(&frame.rgba).all(|(a, b)| a == b);

                if should_send {
                    let compressed = lz4_flex::compress_prepend_size(&frame.rgba);
                    let packet = Packet::ScreenFrame {
                        width: frame.width,
                        height: frame.height,
                        compressed_rgba: compressed,
                    };
                    if let Ok(bin) = bincode::serialize(&packet) {
                        if ws_sink.send(Message::Binary(bin)).await.is_err() {
                            break;
                        }
                    }
                    last_frame_rgba = frame.rgba;
                }
            }
            // ~30 FPS döngü
            tokio::time::sleep(tokio::time::Duration::from_millis(33)).await;
        }
    });

    // Görev B: Gelen Fare / Klavye Girişlerini Windows'a Bas (Admin)
    let input_task = tokio::spawn(async move {
        while let Some(msg_res) = ws_stream.next().await {
            match msg_res {
                Ok(Message::Binary(bin)) => {
                    if let Ok(packet) = bincode::deserialize::<Packet>(&bin) {
                        match packet {
                            Packet::Input(input_event) => {
                                InputInjector::execute(input_event);
                            }
                            Packet::Disconnect => break,
                            _ => {}
                        }
                    }
                }
                _ => break,
            }
        }
    });

    tokio::select! {
        _ = screen_task => {},
        _ = input_task => {},
    }

    let _ = session_tx.send(false).await;
    Ok(())
}

// ==========================================
// CLIENT (TEKNİSYEN) BAĞLANTI İŞLEYİCİSİ
// ==========================================

pub struct ClientSessionHandles {
    pub tx_input: mpsc::Sender<RemoteInput>,
    pub rx_frame: mpsc::Receiver<(u32, u32, Vec<u8>)>,
}

pub async fn connect_to_host(
    target: &str,
    client_name: String,
    password: String,
) -> Result<ClientSessionHandles, String> {
    let url = if target.starts_with("ws://") || target.starts_with("wss://") {
        target.to_string()
    } else {
        format!("ws://{}/", target)
    };

    // Cloudflare TLS ve DNS el sıkışması için yeniden deneme mekanizması
    let mut ws_stream_opt = None;
    let mut last_err = String::new();

    for attempt in 1..=6 {
        match tokio_tungstenite::connect_async(&url).await {
            Ok((stream, _)) => {
                ws_stream_opt = Some(stream);
                break;
            }
            Err(e) => {
                last_err = format!("{}", e);
                if attempt < 6 {
                    tokio::time::sleep(tokio::time::Duration::from_millis(1500)).await;
                }
            }
        }
    }

    let ws_stream = ws_stream_opt.ok_or_else(|| format!("Uzak bilgisayara bağlanılamadı: {}", last_err))?;

    let (mut sink, mut stream) = ws_stream.split();

    // 1. Handshake Gönder
    let handshake = Packet::HandshakeRequest {
        client_name,
        password,
    };
    let bin = bincode::serialize(&handshake).map_err(|e| e.to_string())?;
    sink.send(Message::Binary(bin)).await.map_err(|e| e.to_string())?;

    // 2. Handshake Yanıtı Bekle
    let resp_packet = match stream.next().await {
        Some(Ok(Message::Binary(bin))) => {
            bincode::deserialize::<Packet>(&bin).map_err(|e| e.to_string())?
        }
        _ => return Err("Sunucudan el sıkışma yanıtı alınamadı.".into()),
    };

    match resp_packet {
        Packet::HandshakeResponse {
            accepted: true, ..
        } => {}
        Packet::HandshakeResponse {
            accepted: false,
            reason,
            ..
        } => {
            return Err(reason.unwrap_or_else(|| "Bağlantı kabul edilmedi.".into()));
        }
        _ => return Err("Geçersiz el sıkışma yanıtı.".into()),
    }

    // 3. Kanalları Oluştur
    let (tx_input, mut rx_input) = mpsc::channel::<RemoteInput>(100);
    let (tx_frame, rx_frame) = mpsc::channel::<(u32, u32, Vec<u8>)>(2);

    // Giriş Gönderme Görevi
    tokio::spawn(async move {
        while let Some(input_event) = rx_input.recv().await {
            let packet = Packet::Input(input_event);
            if let Ok(bin) = bincode::serialize(&packet) {
                if sink.send(Message::Binary(bin)).await.is_err() {
                    break;
                }
            }
        }
        let _ = sink.send(Message::Binary(bincode::serialize(&Packet::Disconnect).unwrap())).await;
    });

    // Ekran Karesi Alma Görevi
    tokio::spawn(async move {
        while let Some(msg_res) = stream.next().await {
            match msg_res {
                Ok(Message::Binary(bin)) => {
                    if let Ok(Packet::ScreenFrame {
                        width,
                        height,
                        compressed_rgba,
                    }) = bincode::deserialize::<Packet>(&bin)
                    {
                        if let Ok(decompressed) = lz4_flex::decompress_size_prepended(&compressed_rgba) {
                            let _ = tx_frame.send((width, height, decompressed)).await;
                        }
                    }
                }
                _ => break,
            }
        }
    });

    Ok(ClientSessionHandles { tx_input, rx_frame })
}

pub async fn connect_to_host_addr(
    target_addr: SocketAddr,
    client_name: String,
    password: String,
) -> Result<ClientSessionHandles, String> {
    connect_to_host(&format!("ws://{}/", target_addr), client_name, password).await
}

