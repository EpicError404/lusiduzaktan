use crate::protocol::{MouseBtn, RemoteInput};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, INPUT_MOUSE, KEYBDINPUT,
    KEYBD_EVENT_FLAGS, KEYEVENTF_KEYUP, MOUSEEVENTF_ABSOLUTE, MOUSEEVENTF_LEFTDOWN,
    MOUSEEVENTF_LEFTUP, MOUSEEVENTF_MIDDLEDOWN, MOUSEEVENTF_MIDDLEUP, MOUSEEVENTF_MOVE,
    MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP, MOUSEEVENTF_VIRTUALDESK, MOUSEEVENTF_WHEEL,
    MOUSEINPUT, MOUSE_EVENT_FLAGS, VIRTUAL_KEY,
};

pub struct InputInjector;

impl InputInjector {
    pub fn execute(event: RemoteInput) {
        match event {
            RemoteInput::MouseMove { x, y } => {
                let dx = (x.clamp(0.0, 1.0) * 65535.0) as i32;
                let dy = (y.clamp(0.0, 1.0) * 65535.0) as i32;
                Self::send_mouse(dx, dy, 0, MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK);
            }
            RemoteInput::MouseDown { button } => {
                let flag = match button {
                    MouseBtn::Left => MOUSEEVENTF_LEFTDOWN,
                    MouseBtn::Right => MOUSEEVENTF_RIGHTDOWN,
                    MouseBtn::Middle => MOUSEEVENTF_MIDDLEDOWN,
                };
                Self::send_mouse(0, 0, 0, flag);
            }
            RemoteInput::MouseUp { button } => {
                let flag = match button {
                    MouseBtn::Left => MOUSEEVENTF_LEFTUP,
                    MouseBtn::Right => MOUSEEVENTF_RIGHTUP,
                    MouseBtn::Middle => MOUSEEVENTF_MIDDLEUP,
                };
                Self::send_mouse(0, 0, 0, flag);
            }
            RemoteInput::MouseWheel { delta } => {
                Self::send_mouse(0, 0, (delta * 120) as u32, MOUSEEVENTF_WHEEL);
            }
            RemoteInput::KeyDown { vk } => {
                Self::send_keyboard(vk, false);
            }
            RemoteInput::KeyUp { vk } => {
                Self::send_keyboard(vk, true);
            }
        }
    }

    fn send_mouse(dx: i32, dy: i32, data: u32, flags: MOUSE_EVENT_FLAGS) {
        unsafe {
            let input = INPUT {
                r#type: INPUT_MOUSE,
                Anonymous: INPUT_0 {
                    mi: MOUSEINPUT {
                        dx,
                        dy,
                        mouseData: data,
                        dwFlags: flags,
                        time: 0,
                        dwExtraInfo: 0,
                    },
                },
            };
            SendInput(&[input], std::mem::size_of::<INPUT>() as i32);
        }
    }

    fn send_keyboard(vk: u16, is_up: bool) {
        unsafe {
            let flags = if is_up {
                KEYEVENTF_KEYUP
            } else {
                KEYBD_EVENT_FLAGS(0)
            };

            let input = INPUT {
                r#type: INPUT_KEYBOARD,
                Anonymous: INPUT_0 {
                    ki: KEYBDINPUT {
                        wVk: VIRTUAL_KEY(vk),
                        wScan: 0,
                        dwFlags: flags,
                        time: 0,
                        dwExtraInfo: 0,
                    },
                },
            };
            SendInput(&[input], std::mem::size_of::<INPUT>() as i32);
        }
    }
}
