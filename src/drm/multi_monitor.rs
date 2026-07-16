//! Multi-monitor support: enumeration + per-monitor workspace bindings.
//!
//! При инициализации мы находим все подключённые коннекторы (HDMI-A-1, DP-1, eDP-1, и т.д.).
//! Из конфига берём привязки workspaces к коннекторам.
//!
//! Каждый монитор имеет:
//!   - свой DRM CRTC
//!   - свой dumb buffer + framebuffer
//!   - свой layout (через Workspaces, привязанные к этому монитору)
//!
//! При переключении workspace:
//!   - если workspace привязан к другому монитору — фокус переходит на этот монитор
//!   - layout целевого workspace рендерится на целевом мониторе

use anyhow::Result;
use std::os::unix::io::RawFd;

use crate::config::MonitorCfg;
use super::kms::*;

#[derive(Debug, Clone)]
pub struct Monitor {
    pub crtc_id: u32,
    pub width: u32,
    pub height: u32,
    pub stride: u32,
    pub front_handle: u32,
    pub back_handle: u32,
    pub front_fb: u32,
    pub back_fb: u32,
    pub front_mmap: *mut u8,
    pub back_mmap: *mut u8,
    pub mmap_size: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)] // future use for multi-monitor layout
pub enum MonitorPosition {
    Primary,
    LeftOf,
    RightOf,
    Above,
    Below,
}

unsafe impl Send for Monitor {}
unsafe impl Sync for Monitor {}

pub struct MultiMonitorBackend {
    pub fd: RawFd,
    pub monitors: Vec<Monitor>,
    /// Index of primary/active monitor (для keyboard focus).
    pub active_monitor: usize,
}

impl MultiMonitorBackend {
    /// Инициализация с конфигом мониторов.
    pub fn new(monitor_cfgs: &[MonitorCfg]) -> Result<Self> {
        let fd = open_drm_card()?;
        set_drm_master(fd)?;

        // Получаем все connectors.
        let connectors = enumerate_connectors(fd)?;
        log::info!("found {} connectors, {} connected",
            connectors.len(),
            connectors.iter().filter(|c| c.connection == DRM_MODE_CONNECTED).count());

        let mut monitors = Vec::new();

        // Для каждого подключённого коннектора — создаём dumb buffer + fb + modeset.
        for conn in &connectors {
            if conn.connection != DRM_MODE_CONNECTED { continue; }
            if conn.modes.is_empty() { continue; }

            // Проверяем конфиг — включён ли этот монитор.
            let cfg = monitor_cfgs.iter().find(|m| m.connector == conn.connector_name);
            if let Some(c) = cfg {
                if !c.enabled { continue; }
            }

            // Имя коннектора из конфига.
            let connector_name = conn.connector_name.clone();

            // Выбираем mode: из конфига или preferred.
            let mode = if let Some(c) = cfg {
                if let Some((w, h)) = c.resolution {
                    conn.modes.iter().find(|m| m.hdisplay as u32 == w && m.vdisplay as u32 == h)
                        .copied()
                        .unwrap_or(conn.modes[0])
                } else {
                    conn.modes[0]
                }
            } else {
                conn.modes[0]
            };

            let width = mode.hdisplay as u32;
            let height = mode.vdisplay as u32;

            // Создаём dumb buffers (double buffering).
            let front = create_dumb_buffer(fd, width, height)?;
            let back = create_dumb_buffer(fd, width, height)?;
            let front_fb = create_fb2(fd, width, height, front.handle, front.stride)?;
            let back_fb = create_fb2(fd, width, height, back.handle, back.stride)?;

            // Находим CRTC.
            let crtc_id = conn.encoder_id
                .and_then(|eid| get_encoder_crtc(fd, eid))
                .or_else(|| get_first_crtc(fd))
                .unwrap_or(0);

            // Modeset.
            set_crtc(fd, crtc_id, back_fb, conn.connector_id, &mode)?;

            log::info!("monitor [{}]: {}x{} crtc={}",
                connector_name, width, height, crtc_id);

            monitors.push(Monitor {
                crtc_id,
                width, height,
                stride: back.stride,
                front_handle: front.handle,
                back_handle: back.handle,
                front_fb,
                back_fb,
                front_mmap: front.mmap_addr,
                back_mmap: back.mmap_addr,
                mmap_size: back.size,
            });
        }

        if monitors.is_empty() {
            anyhow::bail!("no monitors initialized");
        }

        log::info!("multi-monitor backend: {} monitor(s) active", monitors.len());
        Ok(MultiMonitorBackend {
            fd,
            monitors,
            active_monitor: 0,
        })
    }

    pub fn primary_monitor(&self) -> &Monitor {
        &self.monitors[0]
    }

    /// Возвращает back buffer для активного монитора.
    pub fn active_back_buffer(&self) -> (*mut u8, u64, u32, u32, u32) {
        let m = &self.monitors[self.active_monitor];
        (m.back_mmap, m.mmap_size, m.stride, m.width, m.height)
    }

    /// Page-flip на всех мониторах.
    pub fn flip_all(&mut self) -> Result<()> {
        for i in 0..self.monitors.len() {
            let (crtc_id, back_fb) = {
                let m = &self.monitors[i];
                (m.crtc_id, m.back_fb)
            };
            let _ = page_flip(self.fd, crtc_id, back_fb);
            let m = &mut self.monitors[i];
            std::mem::swap(&mut m.front_handle, &mut m.back_handle);
            std::mem::swap(&mut m.front_fb, &mut m.back_fb);
            std::mem::swap(&mut m.front_mmap, &mut m.back_mmap);
        }
        Ok(())
    }
}

impl Drop for MultiMonitorBackend {
    fn drop(&mut self) {
        unsafe {
            drop_drm_master(self.fd);
            libc::close(self.fd);
        }
    }
}
