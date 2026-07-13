//! Прямой framebuffer canvas — рисуем пиксели в mmap-нутый dumb buffer.

use crate::ui::theme::{Color, PixelFmt};
use std::sync::Arc;

#[derive(Clone)]
pub struct Canvas {
    pub data: Arc<parking_lot::Mutex<Vec<u8>>>,
    pub width: u32,
    pub height: u32,
    pub stride: u32, // bytes per row
    pub fmt: PixelFmt,
}

impl Canvas {
    pub fn new(width: u32, height: u32, fmt: PixelFmt) -> Self {
        let bpp = fmt.bytes_per_pixel() as u32;
        let stride = width * bpp;
        let len = (stride * height) as usize;
        Canvas {
            data: Arc::new(parking_lot::Mutex::new(vec![0u8; len])),
            width,
            height,
            stride,
            fmt,
        }
    }

    /// Заменяет backing-store (используется после реконфигурации KMS).
    pub fn reallocate(&mut self, width: u32, height: u32, fmt: PixelFmt) {
        let bpp = fmt.bytes_per_pixel() as u32;
        let stride = width * bpp;
        let len = (stride * height) as usize;
        self.width = width;
        self.height = height;
        self.stride = stride;
        self.fmt = fmt;
        *self.data.lock() = vec![0u8; len];
    }

    pub fn fill(&self, c: Color) {
        let mut buf = self.data.lock();
        let pixel = c.as_u32(self.fmt);
        let bpp = self.fmt.bytes_per_pixel();
        match bpp {
            4 => {
                let px_bytes = pixel.to_le_bytes();
                for chunk in buf.chunks_exact_mut(4) {
                    chunk.copy_from_slice(&px_bytes);
                }
            }
            2 => {
                let px_bytes = (pixel as u16).to_le_bytes();
                for chunk in buf.chunks_exact_mut(2) {
                    chunk.copy_from_slice(&px_bytes);
                }
            }
            _ => {}
        }
    }

    pub fn fill_rect(&self, x: i32, y: i32, w: u32, h: u32, c: Color) {
        let mut buf = self.data.lock();
        let pixel = c.as_u32(self.fmt);
        let bpp = self.fmt.bytes_per_pixel() as u32;
        let x0 = x.max(0) as u32;
        let y0 = y.max(0) as u32;
        let x1 = ((x + w as i32).min(self.width as i32)).max(0) as u32;
        let y1 = ((y + h as i32).min(self.height as i32)).max(0) as u32;
        if x0 >= x1 || y0 >= y1 { return; }
        for row in y0..y1 {
            let row_start = (row * self.stride + x0 * bpp) as usize;
            let row_end = (row * self.stride + x1 * bpp) as usize;
            match bpp {
                4 => {
                    let px_bytes = pixel.to_le_bytes();
                    for off in (row_start..row_end).step_by(4) {
                        buf[off..off+4].copy_from_slice(&px_bytes);
                    }
                }
                2 => {
                    let px_bytes = (pixel as u16).to_le_bytes();
                    for off in (row_start..row_end).step_by(2) {
                        buf[off..off+2].copy_from_slice(&px_bytes);
                    }
                }
                _ => {}
            }
        }
        drop(buf);
    }

    /// Рисует пиксель с alpha-blend (только для 32bpp, alpha игнорируется в 565).
    pub fn blend_pixel(&self, x: i32, y: i32, c: Color, alpha: u8) {
        if x < 0 || y < 0 || x as u32 >= self.width || y as u32 >= self.height { return; }
        let mut buf = self.data.lock();
        let bpp = self.fmt.bytes_per_pixel() as u32;
        let off = ((y as u32) * self.stride + (x as u32) * bpp) as usize;
        if bpp == 4 {
            let cur = u32::from_le_bytes(buf[off..off+4].try_into().unwrap());
            // Распаковываем текущий пиксель (по формату)
            let (cr, cg, cb) = match self.fmt {
                PixelFmt::Xrgb8888 | PixelFmt::Argb8888 =>
                    ((cur >> 16) & 0xff, (cur >> 8) & 0xff, cur & 0xff),
                PixelFmt::Bgrx8888 =>
                    (cur & 0xff, (cur >> 8) & 0xff, (cur >> 16) & 0xff),
                _ => unreachable!(),
            };
            let a = alpha as u32;
            let inv = 255 - a;
            let r = (c.0 as u32 * a + cr * inv) / 255;
            let g = (c.1 as u32 * a + cg * inv) / 255;
            let b = (c.2 as u32 * a + cb * inv) / 255;
            let out = Color(r as u8, g as u8, b as u8).as_u32(self.fmt);
            buf[off..off+4].copy_from_slice(&out.to_le_bytes());
        }
        drop(buf);
    }

    pub fn put_pixel(&self, x: i32, y: i32, c: Color) {
        if x < 0 || y < 0 || x as u32 >= self.width || y as u32 >= self.height { return; }
        let mut buf = self.data.lock();
        let bpp = self.fmt.bytes_per_pixel() as u32;
        let off = ((y as u32) * self.stride + (x as u32) * bpp) as usize;
        let pixel = c.as_u32(self.fmt);
        match bpp {
            4 => buf[off..off+4].copy_from_slice(&pixel.to_le_bytes()),
            2 => buf[off..off+2].copy_from_slice(&(pixel as u16).to_le_bytes()),
            _ => {}
        }
    }

    /// Прямая заливка прямоугольника 32bpp XRGB из сырого буфера (для blit X11-окон).
    pub fn blit_argb(&self, dst_x: i32, dst_y: i32, src: &[u32], src_w: u32, src_h: u32) {
        if self.fmt.bytes_per_pixel() != 4 { return; }
        let mut buf = self.data.lock();
        for row in 0..src_h {
            let dy = dst_y + row as i32;
            if dy < 0 || dy >= self.height as i32 { continue; }
            for col in 0..src_w {
                let dx = dst_x + col as i32;
                if dx < 0 || dx >= self.width as i32 { continue; }
                let src_px = src[(row * src_w + col) as usize];
                let off = ((dy as u32) * self.stride + (dx as u32) * 4) as usize;
                // Конвертируем ARGB → формат canvas.
                let out = match self.fmt {
                    PixelFmt::Xrgb8888 | PixelFmt::Argb8888 => src_px,
                    PixelFmt::Bgrx8888 => {
                        let a = (src_px >> 24) & 0xff;
                        let r = (src_px >> 16) & 0xff;
                        let g = (src_px >> 8) & 0xff;
                        let b = src_px & 0xff;
                        if a == 0xff {
                            (b << 16) | (g << 8) | r
                        } else if a == 0 {
                            // transparent — пропускаем
                            continue;
                        } else {
                            // alpha-blend
                            let cur = u32::from_le_bytes(buf[off..off+4].try_into().unwrap());
                            let cr = cur & 0xff;
                            let cg = (cur >> 8) & 0xff;
                            let cb = (cur >> 16) & 0xff;
                            let inv = 255 - a;
                            let nr = (r * a + cr * inv) / 255;
                            let ng = (g * a + cg * inv) / 255;
                            let nb = (b * a + cb * inv) / 255;
                            (nb << 16) | (ng << 8) | nr
                        }
                    }
                    _ => src_px,
                };
                buf[off..off+4].copy_from_slice(&out.to_le_bytes());
            }
        }
    }

    pub fn rect_outline(&self, x: i32, y: i32, w: u32, h: u32, thickness: u32, c: Color) {
        if thickness == 0 || w == 0 || h == 0 { return; }
        // top
        self.fill_rect(x, y, w, thickness, c);
        // bottom
        self.fill_rect(x, y + h as i32 - thickness as i32, w, thickness, c);
        // left
        self.fill_rect(x, y, thickness, h, c);
        // right
        self.fill_rect(x + w as i32 - thickness as i32, y, thickness, h, c);
    }

    /// Неоновая рамка: сначала тёмный ореол, потом яркая линия.
    pub fn neon_border(&self, x: i32, y: i32, w: u32, h: u32, c: Color) {
        // Glow halo (3px, alpha ~30%)
        for t in (1..=3).rev() {
            self.glow_rect(x - t as i32, y - t as i32, w + 2 * t as u32, h + 2 * t as u32, c, 30);
        }
        // Яркая линия
        self.rect_outline(x, y, w, h, 1, c);
    }

    fn glow_rect(&self, x: i32, y: i32, w: u32, h: u32, c: Color, alpha: u8) {
        let _ = (x, y, w, h, c, alpha); // упрощённо — рисуем прямоугольник с blend
        // Тонкое внешнее свечение через blend_pixel.
        if w == 0 || h == 0 { return; }
        for i in 0..w as i32 {
            self.blend_pixel(x + i, y, c, alpha);
            self.blend_pixel(x + i, y + h as i32 - 1, c, alpha);
        }
        for j in 0..h as i32 {
            self.blend_pixel(x, y + j, c, alpha);
            self.blend_pixel(x + w as i32 - 1, y + j, c, alpha);
        }
    }
}
