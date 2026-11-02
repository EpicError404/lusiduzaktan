use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseBtn {
    Left,
    Right,
    Middle,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum RemoteInput {
    MouseMove { x: f32, y: f32 }, // 0.0 to 1.0 normalized screen coordinate
    MouseDown { button: MouseBtn },
    MouseUp { button: MouseBtn },
    MouseWheel { delta: i32 },
    KeyDown { vk: u16 },
    KeyUp { vk: u16 },
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum Packet {
    // 1. Handshake
    HandshakeRequest {
        client_name: String,
        password: String,
    },
    HandshakeResponse {
        accepted: bool,
        reason: Option<String>,
        screen_width: u32,
        screen_height: u32,
    },

    // 2. Screen Streaming
    ScreenFrame {
        width: u32,
        height: u32,
        compressed_rgba: Vec<u8>,
    },

    // 3. Remote Control
    Input(RemoteInput),

    // 4. Session Control
    Disconnect,
    Ping,
    Pong,
}
