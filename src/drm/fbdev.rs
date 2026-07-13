//! Fallback путь через legacy /dev/fb0 (если DRM/KMS недоступен).
//!
//! Это для случаев: серверные системы без DRM, очень старые GPU,
//! или виртуальные FBDEV-устройства. Без page-flip, без ускорения,
//! но рендеринг текста и тайлинг работают.

use anyhow::{Context, Result};
use std::fs::OpenOptions;
use std::os::unix::io::AsRawFd;

pub struct FbdevBackend {
    pub file: std::fs::File,
    pub mmap_addr: *mut u8,
    pub size: usize,
    pub width: u32,
    pub height: u32,
    pub stride: u32,
    pub bpp: u32,
}

unsafe impl Send for FbdevBackend {}
unsafe impl Sync for FbdevBackend {}

impl FbdevBackend {
    pub fn new(path: &str) -> Result<Self> {
        let file = OpenOptions::new()
            .read(true).write(true)
            .open(path)
            .with_context(|| format!("opening {}", path))?;
        // Читаем фиксированный screen_info из /sys/class/graphics/fb0/virtual_size и stride.
        // fb_var_screeninfo и fb_fix_screeninfo — через ioctl.
        let (width, height, stride, bpp, size) = read_fb_info(&file)?;
        let addr = unsafe {
            let result = libc::mmap(
                std::ptr::null_mut(),
                size,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_SHARED,
                file.as_raw_fd(),
                0,
            );
            if result == libc::MAP_FAILED {
                anyhow::bail!("mmap fbdev failed: {}", std::io::Error::last_os_error());
            }
            result as *mut u8
        };
        log::info!("fbdev initialized: {}x{} ({}bpp, stride {})", width, height, bpp, stride);
        Ok(FbdevBackend { file, mmap_addr: addr, size, width, height, stride, bpp })
    }

    pub fn buffer_slice(&self) -> &mut [u8] {
        unsafe { std::slice::from_raw_parts_mut(self.mmap_addr, self.size) }
    }

    pub fn flip(&mut self) -> Result<()> {
        // fbdev не имеет page-flip — данные уже на экране.
        Ok(())
    }
}

impl Drop for FbdevBackend {
    fn drop(&mut self) {
        if !self.mmap_addr.is_null() {
            unsafe { libc::munmap(self.mmap_addr as *mut _, self.size); }
        }
    }
}

/// Читает fb_fix_screeninfo + fb_var_screeninfo через ioctl.
fn read_fb_info(file: &std::fs::File) -> Result<(u32, u32, u32, u32, usize)> {
    // fb_fix_screeninfo: ioctl 0x4602
    // fb_var_screeninfo: ioctl 0x4601
    //
    // Сложно делать через raw ioctl в Rust без структур. Поэтому читаем из
    // /sys/class/graphics/fb0/{virtual_size,stride,bits_per_pixel}.
    let p = std::path::Path::new("/sys/class/graphics/fb0");
    let virt_size = std::fs::read_to_string(p.join("virtual_size"))
        .unwrap_or_else(|_| "1920,1080".to_string());
    let stride = std::fs::read_to_string(p.join("stride"))
        .unwrap_or_else(|_| "7680".to_string());
    let bpp = std::fs::read_to_string(p.join("bits_per_pixel"))
        .unwrap_or_else(|_| "32".to_string());

    let parse_pair = |s: &str| -> (u32, u32) {
        let mut it = s.trim().split(',');
        let a = it.next().and_then(|x| x.parse().ok()).unwrap_or(1920);
        let b = it.next().and_then(|x| x.parse().ok()).unwrap_or(1080);
        (a, b)
    };
    let (w, h) = parse_pair(&virt_size);
    let stride: u32 = stride.trim().parse().unwrap_or(w * 4);
    let bpp: u32 = bpp.trim().parse().unwrap_or(32);
    let size = (stride as usize) * (h as usize);
    Ok((w, h, stride, bpp, size))
}
