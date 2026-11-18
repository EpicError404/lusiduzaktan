use windows::Win32::Foundation::HWND;
use windows::Win32::Graphics::Gdi::{
    BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, CreateDCA, DeleteDC, DeleteObject,
    GetDC, GetDeviceCaps, GetDIBits, ReleaseDC, SelectObject, BITMAPINFO, BITMAPINFOHEADER,
    BI_RGB, DIB_RGB_COLORS, HORZRES, RGBQUAD, SRCCOPY, VERTRES,
};
use windows::Win32::System::StationsAndDesktops::{
    CloseDesktop, OpenInputDesktop, SetThreadDesktop, DESKTOP_ACCESS_FLAGS,
    DESKTOP_CONTROL_FLAGS,
};
use windows::Win32::UI::WindowsAndMessaging::{GetSystemMetrics, SM_CXSCREEN, SM_CYSCREEN};

pub struct ScreenFrame {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

#[derive(Default)]
pub struct ScreenCapturer {
    cached_width: u32,
    cached_height: u32,
}

/// İş parçacığını o an aktif olan masaüstüne (Default, Winlogon/UAC veya Ekran Koruyucu) geçirir.
/// Not: Çağıran thread'in açık bir GUI penceresi olmamalıdır.
pub fn switch_to_input_desktop() -> bool {
    unsafe {
        // DESKTOP_READOBJECTS (0x0001) | DESKTOP_WRITEOBJECTS (0x0080) | DESKTOP_SWITCHDESKTOP (0x0100)
        let access = DESKTOP_ACCESS_FLAGS(0x0001 | 0x0080 | 0x0100);
        if let Ok(hdesk) = OpenInputDesktop(DESKTOP_CONTROL_FLAGS(0), false, access) {
            let switched = SetThreadDesktop(hdesk).is_ok();
            let _ = CloseDesktop(hdesk);
            return switched;
        }
        false
    }
}

impl ScreenCapturer {
    pub fn new() -> Result<Self, String> {
        let (w, h) = Self::get_screen_metrics();
        if w == 0 || h == 0 {
            return Err("Ekran çözünürlüğü tespit edilemedi.".into());
        }
        Ok(Self {
            cached_width: w,
            cached_height: h,
        })
    }

    pub fn screen_size(&self) -> (u32, u32) {
        Self::get_screen_metrics()
    }

    fn get_screen_metrics() -> (u32, u32) {
        unsafe {
            let width = GetSystemMetrics(SM_CXSCREEN);
            let height = GetSystemMetrics(SM_CYSCREEN);
            (width.max(1) as u32, height.max(1) as u32)
        }
    }

    /// Gerçek ekranı yakalamayı dener. UAC / Secure Desktop sırasında yetki yetersizse
    /// sahte gradyan üretmek yerine `Err` döner, böylece bağlantı ve son ekran karesi korunur.
    pub fn try_capture_live_frame(&mut self) -> Result<ScreenFrame, String> {
        unsafe {
            let _ = switch_to_input_desktop();

            let mut used_get_dc = true;
            let mut hdc_screen = GetDC(HWND::default());
            if hdc_screen.0.is_null() {
                used_get_dc = false;
                hdc_screen = CreateDCA(windows::core::s!("DISPLAY"), None, None, None);
            }

            if hdc_screen.0.is_null() {
                return Err("Ekran DC açılamadı.".into());
            }

            let dc_w = GetDeviceCaps(hdc_screen, HORZRES);
            let dc_h = GetDeviceCaps(hdc_screen, VERTRES);
            let (width, height) = if dc_w > 0 && dc_h > 0 {
                (dc_w as u32, dc_h as u32)
            } else {
                Self::get_screen_metrics()
            };
            self.cached_width = width;
            self.cached_height = height;

            let mut result: Result<ScreenFrame, String> =
                Err("Güvenli masaüstü (UAC) veya kilit ekranı nedeniyle kare alınamadı.".into());

            let hdc_mem = CreateCompatibleDC(hdc_screen);
            if !hdc_mem.0.is_null() {
                let hbitmap = CreateCompatibleBitmap(hdc_screen, width as i32, height as i32);
                if !hbitmap.0.is_null() {
                    let old_obj = SelectObject(hdc_mem, hbitmap);

                    let blt_res = BitBlt(
                        hdc_mem,
                        0,
                        0,
                        width as i32,
                        height as i32,
                        hdc_screen,
                        0,
                        0,
                        SRCCOPY,
                    );

                    if blt_res.is_ok() {
                        let mut bmi = BITMAPINFO {
                            bmiHeader: BITMAPINFOHEADER {
                                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                                biWidth: width as i32,
                                biHeight: -(height as i32),
                                biPlanes: 1,
                                biBitCount: 32,
                                biCompression: BI_RGB.0,
                                biSizeImage: 0,
                                biXPelsPerMeter: 0,
                                biYPelsPerMeter: 0,
                                biClrUsed: 0,
                                biClrImportant: 0,
                            },
                            bmiColors: [RGBQUAD {
                                rgbBlue: 0,
                                rgbGreen: 0,
                                rgbRed: 0,
                                rgbReserved: 0,
                            }; 1],
                        };

                        let pixel_count = (width * height) as usize;
                        let mut bgra_buffer: Vec<u8> = vec![0u8; pixel_count * 4];

                        let lines_copied = GetDIBits(
                            hdc_mem,
                            hbitmap,
                            0,
                            height,
                            Some(bgra_buffer.as_mut_ptr() as *mut _),
                            &mut bmi,
                            DIB_RGB_COLORS,
                        );

                        if lines_copied > 0 {
                            for chunk in bgra_buffer.chunks_exact_mut(4) {
                                let b = chunk[0];
                                let r = chunk[2];
                                chunk[0] = r;
                                chunk[2] = b;
                                chunk[3] = 255;
                            }

                            result = Ok(ScreenFrame {
                                width,
                                height,
                                rgba: bgra_buffer,
                            });
                        }
                    }

                    SelectObject(hdc_mem, old_obj);
                    let _ = DeleteObject(hbitmap);
                }
                let _ = DeleteDC(hdc_mem);
            }

            if used_get_dc {
                let _ = ReleaseDC(HWND::default(), hdc_screen);
            } else {
                let _ = DeleteDC(hdc_screen);
            }

            result
        }
    }

    pub fn capture_frame(&mut self) -> Result<ScreenFrame, String> {
        if let Ok(frame) = self.try_capture_live_frame() {
            return Ok(frame);
        }

        let (width, height) = Self::get_screen_metrics();
        self.cached_width = width;
        self.cached_height = height;

        // Headless / kısıtlı masaüstü oturumları için güvenli fallback karesi
        let pixel_count = (width * height) as usize;
        let mut fallback_buffer = vec![30u8; pixel_count * 4];
        for (i, chunk) in fallback_buffer.chunks_exact_mut(4).enumerate() {
            chunk[0] = ((i % width as usize) * 255 / width as usize) as u8;
            chunk[1] = (((i / width as usize) * 255) / height as usize) as u8;
            chunk[2] = 120;
            chunk[3] = 255;
        }

        Ok(ScreenFrame {
            width,
            height,
            rgba: fallback_buffer,
        })
    }
}
