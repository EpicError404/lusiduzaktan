use std::net::{Ipv4Addr, SocketAddr};
use uzakbaglanti::capture::ScreenCapturer;
use uzakbaglanti::input::InputInjector;
use uzakbaglanti::network::{NetworkUtils, DEFAULT_PORT};
use uzakbaglanti::protocol::{MouseBtn, Packet, RemoteInput};

#[test]
fn test_ip_and_id_codec() {
    let test_ips = vec![
        Ipv4Addr::new(127, 0, 0, 1),
        Ipv4Addr::new(192, 168, 1, 105),
        Ipv4Addr::new(176, 240, 12, 85),
        Ipv4Addr::new(10, 0, 0, 1),
    ];

    for ip in test_ips {
        let id_str = NetworkUtils::ip_to_id(ip);
        assert!(!id_str.is_empty());
        
        let parsed_addr = NetworkUtils::parse_target_address(&id_str).expect("ID çözülemedi");
        assert_eq!(parsed_addr.ip(), std::net::IpAddr::V4(ip));
        assert_eq!(parsed_addr.port(), DEFAULT_PORT);
    }
}

#[test]
fn test_direct_ip_port_parsing() {
    let direct_addr_str = "127.0.0.1:9090";
    let parsed = NetworkUtils::parse_target_address(direct_addr_str).expect("IP:Port çözülemedi");
    assert_eq!(parsed, "127.0.0.1:9090".parse::<SocketAddr>().unwrap());
}

#[test]
fn test_protocol_packet_serialization() {
    let packets = vec![
        Packet::HandshakeRequest {
            client_name: "Teknisyen".into(),
            password: "test_password_123".into(),
        },
        Packet::HandshakeResponse {
            accepted: true,
            reason: None,
            screen_width: 1920,
            screen_height: 1080,
        },
        Packet::Input(RemoteInput::MouseMove { x: 0.5, y: 0.75 }),
        Packet::Input(RemoteInput::MouseDown {
            button: MouseBtn::Left,
        }),
        Packet::Input(RemoteInput::MouseWheel { delta: 1 }),
        Packet::Input(RemoteInput::KeyDown { vk: 0x41 }), // 'A' key
        Packet::Disconnect,
    ];

    for pkt in packets {
        let serialized = bincode::serialize(&pkt).expect("Serileştirme hatası");
        let deserialized: Packet = bincode::deserialize(&serialized).expect("Geri çözme hatası");
        
        // Yeniden serialize edince aynı baytları vermeli
        let reserialized = bincode::serialize(&deserialized).unwrap();
        assert_eq!(serialized, reserialized);
    }
}

#[test]
fn test_screen_capturer_and_compression() {
    let capturer_res = ScreenCapturer::new();
    assert!(capturer_res.is_ok(), "ScreenCapturer ilklendirilemedi: {:?}", capturer_res.err());

    let mut capturer = capturer_res.unwrap();
    let frame_res = capturer.capture_frame();
    assert!(frame_res.is_ok(), "Ekran karesi alınamadı: {:?}", frame_res.err());

    let frame = frame_res.unwrap();
    assert!(frame.width > 0, "Genişlik 0'dan büyük olmalı");
    assert!(frame.height > 0, "Yükseklik 0'dan büyük olmalı");
    assert_eq!(frame.rgba.len(), (frame.width * frame.height * 4) as usize);

    // LZ4 sıkıştırma testi
    let compressed = lz4_flex::compress_prepend_size(&frame.rgba);
    assert!(!compressed.is_empty());
    
    // LZ4 açma testi
    let decompressed = lz4_flex::decompress_size_prepended(&compressed).expect("LZ4 decompress hatası");
    assert_eq!(decompressed.len(), frame.rgba.len());
    assert_eq!(decompressed, frame.rgba);
}

#[test]
fn test_input_injection_does_not_panic() {
    // Normal sınırlar içinde güvenli giriş testi
    InputInjector::execute(RemoteInput::MouseMove { x: 0.1, y: 0.1 });
    InputInjector::execute(RemoteInput::MouseWheel { delta: 0 });
}

#[tokio::test]
async fn test_end_to_end_handshake_and_acceptance() {
    let test_port = 18995;
    let expected_password = std::sync::Arc::new(std::sync::Mutex::new("secret123".to_string()));

    let (incoming_tx, mut incoming_rx) = tokio::sync::mpsc::channel(1);
    let (session_tx, mut session_rx) = tokio::sync::mpsc::channel(5);

    let pw_clone = expected_password.clone();
    tokio::spawn(async move {
        let events = uzakbaglanti::network::HostServerEvents {
            incoming_request_tx: incoming_tx,
            session_active_tx: session_tx,
        };
        let _ = uzakbaglanti::network::start_host_server(test_port, pw_clone, events).await;
    });

    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    let target_addr: std::net::SocketAddr = format!("127.0.0.1:{}", test_port).parse().unwrap();
    let client_task = tokio::spawn(async move {
        uzakbaglanti::network::connect_to_host_addr(target_addr, "TestTeknisyen".into(), "secret123".into()).await
    });

    let (client_name, approval_tx) = incoming_rx.recv().await.expect("Gelen istek alınamadı");
    assert_eq!(client_name, "TestTeknisyen");

    // "Bağlantıyı Kabul Et" butonuna basılır
    approval_tx.send(true).await.expect("Onay gönderilemedi");

    let mut handles = client_task.await.unwrap().expect("Client oturumu kurulamadı");

    let is_active = session_rx.recv().await.unwrap();
    assert!(is_active);

    let frame_res = tokio::time::timeout(std::time::Duration::from_secs(3), handles.rx_frame.recv()).await;
    assert!(frame_res.is_ok(), "Frame alma zaman aşımına uğradı");
    let (w, h, rgba) = frame_res.unwrap().expect("Frame verisi boş geldi");
    assert!(w > 0 && h > 0);
    assert_eq!(rgba.len(), (w * h * 4) as usize);

    let send_res = handles.tx_input.send(RemoteInput::MouseMove { x: 0.5, y: 0.5 }).await;
    assert!(send_res.is_ok());
}

#[test]
fn test_parse_target_to_url() {
    assert_eq!(
        NetworkUtils::parse_target_to_url("mazda-inf-collectors-fewer").unwrap(),
        "wss://mazda-inf-collectors-fewer.trycloudflare.com/"
    );
    assert_eq!(
        NetworkUtils::parse_target_to_url("https://my-tunnel.trycloudflare.com").unwrap(),
        "wss://my-tunnel.trycloudflare.com/"
    );
    assert_eq!(
        NetworkUtils::parse_target_to_url("wss://custom.domain.com/").unwrap(),
        "wss://custom.domain.com/"
    );
    assert_eq!(
        NetworkUtils::parse_target_to_url("127.0.0.1:8990").unwrap(),
        "ws://127.0.0.1:8990/"
    );
    assert_eq!(
        NetworkUtils::parse_target_to_url("192.168.1.50").unwrap(),
        "ws://192.168.1.50:8990/"
    );
}

#[test]
fn test_cloudflared_binary_available() {
    let cf_path = uzakbaglanti::network::locate_or_download_cloudflared();
    assert!(cf_path.is_some(), "cloudflared.exe bulunamadı");
    assert!(cf_path.unwrap().exists());
}

#[test]
fn test_generate_device_id() {
    let (digits, formatted) = uzakbaglanti::network::generate_device_id();
    assert_eq!(digits.len(), 9);
    assert_eq!(formatted.len(), 11); // 9 digits + 2 spaces
    assert!(digits.chars().all(|c| c.is_ascii_digit()));
    assert_eq!(
        format!("{} {} {}", &digits[0..3], &digits[3..6], &digits[6..9]),
        formatted
    );
}

#[tokio::test]
async fn test_pin_registration_and_async_resolve() {
    let test_pin = "889900112";
    let test_subdomain = "rebel-unless-variation-propecia";
    
    // Register pin
    uzakbaglanti::network::post_pin_to_brokers(test_pin, test_subdomain);
    
    // Resolve with formatted pin (with spaces: "889 900 112")
    let resolved_url = NetworkUtils::resolve_target_to_url("889 900 112").await;
    assert!(resolved_url.is_ok(), "PIN çözülemedi: {:?}", resolved_url.err());
    assert_eq!(
        resolved_url.unwrap(),
        "wss://rebel-unless-variation-propecia.trycloudflare.com/"
    );
}

#[tokio::test]
async fn test_legacy_32bit_ip_fallback() {
    // 3232235630 = 192.168.0.110
    let resolved_url = NetworkUtils::resolve_target_to_url("323 223 5630").await;
    assert!(resolved_url.is_ok());
    assert_eq!(resolved_url.unwrap(), "ws://192.168.0.110:8990/");

    let resolved_url2 = NetworkUtils::resolve_target_to_url("3232235630").await;
    assert!(resolved_url2.is_ok());
    assert_eq!(resolved_url2.unwrap(), "ws://192.168.0.110:8990/");
}


