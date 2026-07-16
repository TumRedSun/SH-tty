pub mod kms;
pub mod fbdev;
pub mod multi_monitor;
pub mod cursor;
pub mod planes;

pub use kms::DrmBackend;
pub use fbdev::FbdevBackend;
pub use multi_monitor::MultiMonitorBackend;
pub use cursor::HardwareCursor;
pub use planes::OverlayManager;

/// Унифицированный backend.
pub enum Backend {
    Drm(DrmBackend),
    Fbdev(FbdevBackend),
}

impl Backend {
    /// Пробуем DRM/KMS first, fallback на fbdev.
    pub fn open(preferred_w: Option<u32>, preferred_h: Option<u32>) -> anyhow::Result<Self> {
        let candidates = &["/dev/dri/card0", "/dev/dri/card1"];
        for path in candidates {
            if std::path::Path::new(path).exists() {
                match DrmBackend::new(path, preferred_w, preferred_h) {
                    Ok(b) => return Ok(Backend::Drm(b)),
                    Err(e) => log::warn!("DRM/KMS init failed on {}: {}", path, e),
                }
            }
        }
        if std::path::Path::new("/dev/fb0").exists() {
            let fb = FbdevBackend::new("/dev/fb0")?;
            return Ok(Backend::Fbdev(fb));
        }
        anyhow::bail!("no DRM device nor fbdev available — superhot-tty requires a graphical console")
    }

    pub fn dimensions(&self) -> (u32, u32) {
        match self {
            Backend::Drm(d) => (d.width, d.height),
            Backend::Fbdev(f) => (f.width, f.height),
        }
    }

    pub fn back_buffer(&mut self) -> &mut [u8] {
        match self {
            Backend::Drm(d) => d.back_buffer_slice(),
            Backend::Fbdev(f) => f.buffer_slice(),
        }
    }

    pub fn flip(&mut self) -> anyhow::Result<()> {
        match self {
            Backend::Drm(d) => d.flip(),
            Backend::Fbdev(f) => f.flip(),
        }
    }
}
