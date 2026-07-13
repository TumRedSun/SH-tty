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

use anyhow::{Context, Result};
use std::collections::HashMap;
use std::os::unix::io::RawFd;

use crate::config::MonitorCfg;
use super::kms::*;

#[derive(Debug, Clone)]
pub struct Monitor {
    pub connector_name: String,
    pub connector_id: u32,
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
    /// Workspaces привязанные к этому монитору (из конфига).
    pub workspaces: Vec<u8>,
    /// Активный workspace на этом мониторе.
    pub active_workspace: u8,
    /// Позиция монитора в логической раскладке (для future).
    pub position: MonitorPosition,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
    /// Map: workspace_id → index in monitors[].
    pub workspace_to_monitor: HashMap<u8, usize>,
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
        let mut workspace_to_monitor = HashMap::new();

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

            // Workspaces из конфига (или все, если конфига нет).
            let workspaces = cfg.map(|c| c.workspaces.clone()).unwrap_or_default();
            let position = cfg.and_then(|c| c.position.as_ref()).map(|p| parse_position(p))
                .unwrap_or(MonitorPosition::Primary);

            let active_workspace = workspaces.first().copied().unwrap_or(1);

            // Регистрируем workspace → monitor mapping.
            for &ws in &workspaces {
                workspace_to_monitor.insert(ws, monitors.len());
            }

            log::info!("monitor [{}]: {}x{} crtc={} ws={:?} active_ws={}",
                connector_name, width, height, crtc_id, workspaces, active_workspace);

            monitors.push(Monitor {
                connector_name,
                connector_id: conn.connector_id,
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
                workspaces,
                active_workspace,
                position,
            });
        }

        if monitors.is_empty() {
            anyhow::bail!("no monitors initialized");
        }

        log::info!("multi-monitor backend: {} monitor(s) active", monitors.len());
        Ok(MultiMonitorBackend {
            fd,
            monitors,
            workspace_to_monitor,
            active_monitor: 0,
        })
    }

    pub fn primary_monitor(&self) -> &Monitor {
        &self.monitors[0]
    }

    pub fn primary_monitor_mut(&mut self) -> &mut Monitor {
        &mut self.monitors[0]
    }

    /// Возвращает индекс монитора для данного workspace.
    pub fn monitor_for_workspace(&self, ws: u8) -> Option<usize> {
        self.workspace_to_monitor.get(&ws).copied()
    }

    /// Переключает активный workspace на указанном мониторе.
    pub fn set_monitor_workspace(&mut self, monitor_idx: usize, ws: u8) {
        if monitor_idx < self.monitors.len() {
            self.monitors[monitor_idx].active_workspace = ws;
            self.active_monitor = monitor_idx;
        }
    }

    /// Переключает на workspace N. Если workspace привязан к другому монитору —
    /// активируем этот монитор.
    pub fn switch_workspace(&mut self, ws: u8) -> Option<usize> {
        if let Some(&mon_idx) = self.workspace_to_monitor.get(&ws) {
            self.monitors[mon_idx].active_workspace = ws;
            self.active_monitor = mon_idx;
            Some(mon_idx)
        } else {
            // Если ws не привязан ни к одному монитору — переключаем на текущем.
            if let Some(m) = self.monitors.get_mut(self.active_monitor) {
                m.active_workspace = ws;
            }
            Some(self.active_monitor)
        }
    }

    /// Возвращает back buffer для активного монитора.
    pub fn active_back_buffer(&self) -> (*mut u8, u64, u32, u32, u32) {
        let m = &self.monitors[self.active_monitor];
        (m.back_mmap, m.mmap_size, m.stride, m.width, m.height)
    }

    /// Page-flip на активном мониторе.
    pub fn flip_active(&mut self) -> Result<()> {
        let m = &self.monitors[self.active_monitor];
        page_flip(self.fd, m.crtc_id, m.back_fb)?;
        // Swap front/back.
        let m = &mut self.monitors[self.active_monitor];
        std::mem::swap(&mut m.front_handle, &mut m.back_handle);
        std::mem::swap(&mut m.front_fb, &mut m.back_fb);
        std::mem::swap(&mut m.front_mmap, &mut m.back_mmap);
        Ok(())
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

fn parse_position(s: &str) -> MonitorPosition {
    if s == "primary" { MonitorPosition::Primary }
    else if s.starts_with("left-of") { MonitorPosition::LeftOf }
    else if s.starts_with("right-of") { MonitorPosition::RightOf }
    else if s.starts_with("above") { MonitorPosition::Above }
    else if s.starts_with("below") { MonitorPosition::Below }
    else { MonitorPosition::Primary }
}
