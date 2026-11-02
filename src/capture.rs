use windows::Win32::Graphics::Gdi::{
    BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, DeleteDC, DeleteObject, GetDIBits,
    SelectObject, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS, SRCCOPY,
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

    pub fn capture_frame(&mut self) -> Result<ScreenFrame, String> {
        unsafe {
            let (width, height) = Self::get_screen_metrics();
            self.cached_width = width;
            self.cached_height = height;

            let hdc_screen = windows::Win32::Graphics::Gdi::CreateDCA(
                windows::core::s!("DISPLAY"),
                None,
                None,
                None,
            );

            if !hdc_screen.0.is_null() {
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
                                bmiColors: [windows::Win32::Graphics::Gdi::RGBQUAD {
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

                            SelectObject(hdc_mem, old_obj);
                            let _ = DeleteObject(hbitmap);
                            let _ = DeleteDC(hdc_mem);
                            let _ = DeleteDC(hdc_screen);

                            if lines_copied > 0 {
                                // BGRA -> RGBA
                                for chunk in bgra_buffer.chunks_exact_mut(4) {
                                    let b = chunk[0];
                                    let r = chunk[2];
                                    chunk[0] = r;
                                    chunk[2] = b;
                                    chunk[3] = 255;
                                }

                                return Ok(ScreenFrame {
                                    width,
                                    height,
                                    rgba: bgra_buffer,
                                });
                            }
                        } else {
                            SelectObject(hdc_mem, old_obj);
                            let _ = DeleteObject(hbitmap);
                        }
                    }
                    let _ = DeleteDC(hdc_mem);
                }
                let _ = DeleteDC(hdc_screen);
            }

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
}
