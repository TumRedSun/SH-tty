//! Hardware DRM cursor plane.
//!
//! DRM предоставляет dedicated cursor planes (отдельные от primary и overlay).
//! Это позволяет:
//!   - Двигать курсор без перерисовки framebuffer (0% CPU)
//!   - Аппаратное ускорение курсора (без blending в canvas)
//!   - Поддержку ARGB курсоров (alpha-blend over framebuffer)
//!
//! API:
//!   - DRM_IOCTL_MODE_CURSOR (legacy, простой)
//!   - или atomic commit с cursor plane (современный)
//!
//! Размер курсора: типично 64x64 (DRM_CAP_CURSOR_WIDTH/HEIGHT).
//! Формат: DRM_FORMAT_ARGB8888.

use anyhow::Result;
use std::os::unix::io::RawFd;
use crate::drm::kms::*;

// DRM_IOCTL_MODE_CURSOR2 (с hot spot, modern). NR = 0xA0 + 0x1D = 0xBD
const DRM_IOCTL_MODE_CURSOR2: u32 = iowr(0x64, 0xBD, std::mem::size_of::<DrmModeCursor2>() as u32);

#[repr(C)]
#[derive(Default, Debug)]
struct DrmModeCursor2 {
    flags: u32,
    crtc_id: u32,
    x: i32,
    y: i32,
    width: u32,
    height: u32,
    handle: u32,
    hot_x: i32,
    hot_y: i32,
}

// DRM_CAP_CURSOR_WIDTH = 0x12, DRM_CAP_CURSOR_HEIGHT = 0x13
const DRM_IOCTL_GET_CAP: u32 = iowr(0x64, 0x0C, std::mem::size_of::<DrmGetCap>() as u32);

#[repr(C)]
#[derive(Default, Debug)]
struct DrmGetCap {
    capability: u64,
    value: u64,
}

const DRM_CAP_CURSOR_WIDTH: u64 = 0x12;
const DRM_CAP_CURSOR_HEIGHT: u64 = 0x13;

const DRM_MODE_CURSOR_BO: u32 = 0x01; // set cursor buffer
const DRM_MODE_CURSOR_MOVE: u32 = 0x02; // move cursor
const DRM_BO_CURSOR: u32 = DRM_MODE_CURSOR_BO;

pub struct HardwareCursor {
    pub drm_fd: RawFd,
    pub crtc_id: u32,
    pub width: u32,
    pub height: u32,
    pub handle: u32,         // DRM handle to cursor dumb buffer
    pub mmap_addr: *mut u8,  // mmap'ed cursor buffer
    pub mmap_size: u64,
    pub x: i32,
    pub y: i32,
    pub visible: bool,
}

unsafe impl Send for HardwareCursor {}
unsafe impl Sync for HardwareCursor {}

impl HardwareCursor {
    /// Создаёт hardware cursor для указанного CRTC.
    pub fn new(drm_fd: RawFd, crtc_id: u32) -> Result<Self> {
        // Узнаём размер курсора через DRM_CAP_CURSOR_WIDTH/HEIGHT.
        let width = get_cap(drm_fd, DRM_CAP_CURSOR_WIDTH).unwrap_or(64) as u32;
        let height = get_cap(drm_fd, DRM_CAP_CURSOR_HEIGHT).unwrap_or(64) as u32;
        log::info!("hardware cursor: {}x{}", width, height);

        // Создаём dumb buffer для курсора.
        let dumb = create_dumb_buffer(drm_fd, width, height)?;
        // Инициализируем курсор ARGB пикселями (MCD-styled crosshair).
        let cursor = Self::default_mcd_cursor(width, height);
        unsafe {
            std::ptr::copy_nonoverlapping(
                cursor.as_ptr() as *const u8,
                dumb.mmap_addr,
                cursor.len() * 4,
            );
        }

        let mut hw_cursor = HardwareCursor {
            drm_fd,
            crtc_id,
            width,
            height,
            handle: dumb.handle,
            mmap_addr: dumb.mmap_addr,
            mmap_size: dumb.size,
            x: 0,
            y: 0,
            visible: false,
        };
        // Устанавливаем buffer курсора.
        hw_cursor.set_bo()?;
        hw_cursor.show()?;
        Ok(hw_cursor)
    }

    /// Устанавливает buffer курсора (BO = buffer object).
    fn set_bo(&mut self) -> Result<()> {
        let mut c = DrmModeCursor2::default();
        c.flags = DRM_BO_CURSOR;
        c.crtc_id = self.crtc_id;
        c.width = self.width;
        c.height = self.height;
        c.handle = self.handle;
        c.x = 0;
        c.y = 0;
        c.hot_x = (self.width / 2) as i32;
        c.hot_y = (self.height / 2) as i32;
        let ret = unsafe { libc::ioctl(self.drm_fd, DRM_IOCTL_MODE_CURSOR2 as _, &c as *const _ as *const _) };
        if ret < 0 {
            anyhow::bail!("DRM_IOCTL_MODE_CURSOR2 (BO) failed: {}", std::io::Error::last_os_error());
        }
        Ok(())
    }

    /// Показывает курсор (без изменения позиции).
    pub fn show(&mut self) -> Result<()> {
        self.visible = true;
        self.move_to(self.x, self.y)
    }

    /// Скрывает курсор.
    #[allow(dead_code)] // currently unused, kept for future hide-on-keyboard-input
    pub fn hide(&mut self) -> Result<()> {
        self.visible = false;
        let mut c = DrmModeCursor2::default();
        c.flags = 0; // hide
        c.crtc_id = self.crtc_id;
        c.handle = 0;
        let ret = unsafe { libc::ioctl(self.drm_fd, DRM_IOCTL_MODE_CURSOR2 as _, &c as *const _ as *const _) };
        if ret < 0 {
            log::warn!("cursor hide failed: {}", std::io::Error::last_os_error());
        }
        Ok(())
    }

    /// Перемещает курсор. Не требует перерисовки framebuffer.
    pub fn move_to(&mut self, x: i32, y: i32) -> Result<()> {
        self.x = x;
        self.y = y;
        if !self.visible { return Ok(()); }
        let mut c = DrmModeCursor2::default();
        c.flags = DRM_MODE_CURSOR_MOVE;
        c.crtc_id = self.crtc_id;
        c.x = x;
        c.y = y;
        c.width = self.width;
        c.height = self.height;
        c.handle = self.handle;
        c.hot_x = (self.width / 2) as i32;
        c.hot_y = (self.height / 2) as i32;
        let ret = unsafe { libc::ioctl(self.drm_fd, DRM_IOCTL_MODE_CURSOR2 as _, &c as *const _ as *const _) };
        if ret < 0 {
            log::warn!("cursor move failed: {}", std::io::Error::last_os_error());
        }
        Ok(())
    }

    /// Обновляет изображение курсора (например, для анимации).
    #[allow(dead_code)] // currently unused, kept for future cursor themes
    pub fn update_image(&mut self, pixels: &[u32]) -> Result<()> {
        if pixels.len() != (self.width * self.height) as usize {
            anyhow::bail!("cursor image size mismatch");
        }
        unsafe {
            std::ptr::copy_nonoverlapping(
                pixels.as_ptr() as *const u8,
                self.mmap_addr,
                (self.width * self.height * 4) as usize,
            );
        }
        // Нужно переустановить BO чтобы DRM перечитал буфер.
        self.set_bo()
    }

    /// Дефолтный MCD-styled курсор: неоновый крестик с магента-центром.
    fn default_mcd_cursor(w: u32, h: u32) -> Vec<u32> {
        let mut buf = vec![0u32; (w * h) as usize];
        let cx = (w / 2) as i32;
        let cy = (h / 2) as i32;
        // Цвета ARGB.
        let magenta: u32 = 0xFFFF2E97;
        let white: u32 = 0xFFFFFFFF;
        let dim: u32 = 0x80FF2E97;
        // Рисуем крестик.
        for i in -8..=8 {
            let x = cx + i;
            let y = cy + i;
            if x >= 0 && (x as u32) < w {
                buf[(cy * w as i32 + x) as usize] = if i.abs() <= 2 { white } else { magenta };
            }
            if y >= 0 && (y as u32) < h {
                buf[(y * w as i32 + cx) as usize] = if i.abs() <= 2 { white } else { magenta };
            }
        }
        // Glow halo.
        for _r in 1..=3 {
            for i in -8..=8 {
                for &(dx, dy) in &[(i, 0), (0, i)] {
                    let x = cx + dx;
                    let y = cy + dy;
                    if x >= 0 && (x as u32) < w && y >= 0 && (y as u32) < h {
                        let idx = (y * w as i32 + x) as usize;
                        if buf[idx] == 0 {
                            buf[idx] = dim;
                        }
                    }
                }
            }
        }
        buf
    }
}

impl Drop for HardwareCursor {
    fn drop(&mut self) {
        if !self.mmap_addr.is_null() {
            unsafe { libc::munmap(self.mmap_addr as *mut _, self.mmap_size as usize); }
        }
        // Уничтожаем dumb buffer.
        let mut destroy = drm_mode_destroy_dumb { handle: self.handle };
        unsafe {
            libc::ioctl(self.drm_fd, DRM_IOCTL_MODE_DESTROY_DUMB_PUB as _, &mut destroy as *mut _ as *mut _);
        }
    }
}

fn get_cap(fd: RawFd, cap: u64) -> Result<u64> {
    let mut gc = DrmGetCap { capability: cap, value: 0 };
    let ret = unsafe { libc::ioctl(fd, DRM_IOCTL_GET_CAP as _, &mut gc as *mut _ as *mut _) };
    if ret < 0 {
        anyhow::bail!("DRM_IOCTL_GET_CAP failed: {}", std::io::Error::last_os_error());
    }
    Ok(gc.value)
}
