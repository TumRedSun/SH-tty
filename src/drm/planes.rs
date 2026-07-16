//! Hardware DRM overlay planes.
//!
//! Каждый CRTC имеет несколько planes:
//!   - primary plane (scanout)
//!   - overlay planes (дополнительные слои)
//!   - cursor plane (dedicated)
//!
//! Использование overlay planes для X11 окон:
//!   - Вместо blitting X11 backing в canvas (CPU)
//!   - Импортируем dma-buf из X11 через DRI3
//!   - Создаём DRM framebuffer из dma-buf
//!   - Присваиваем framebuffer к overlay plane через atomic commit
//!   - GPU composite это на лету (0% CPU)
//!
//! Результат: браузер, Steam, Discord — всё на hardware planes,
//! CPU используется только для терминального canvas.

use anyhow::{Context, Result};
use std::os::unix::io::RawFd;
use crate::drm::kms::*;
use crate::x11::dri3::DmaBuf;

// DRM_IOCTL_MODE_GETPLANERESOURCES
const DRM_IOCTL_MODE_GETPLANERESOURCES: u32 = iowr(0xA0, 0x36, std::mem::size_of::<DrmModeGetPlaneRes>() as u32);
// DRM_IOCTL_MODE_GETPLANE
const DRM_IOCTL_MODE_GETPLANE: u32 = iowr(0xA0, 0x37, std::mem::size_of::<DrmModeGetPlane>() as u32);
// DRM_IOCTL_MODE_SETPLANE (legacy, non-atomic)
const DRM_IOCTL_MODE_SETPLANE: u32 = iow(0xA0, 0x38, std::mem::size_of::<DrmModeSetPlane>() as u32);
// DRM_IOCTL_PRIME_FD_TO_HANDLE
const DRM_IOCTL_PRIME_FD_TO_HANDLE: u32 = iowr(0x64, 0x2E, std::mem::size_of::<DrmPrimeHandle>() as u32);
// DRM_IOCTL_MODE_ADDFB2
const DRM_IOCTL_MODE_ADDFB2: u32 = iowr(0xA0, 0xB8, std::mem::size_of::<DrmModeFbCmd2>() as u32);
// DRM_IOCTL_MODE_RMFB
const DRM_IOCTL_MODE_RMFB: u32 = ior(0xA0, 0x05, 4);

// DRM_MODE_PROP enums и property IDs нам нужны для atomic commit.
// CRTC_ID, FB_ID, SRC_X/Y/W/H, CRTC_X/Y/W/H, type
// Для упрощения используем legacy SETPLANE (без atomic).

#[repr(C)]
#[derive(Default, Debug)]
struct DrmModeGetPlaneRes {
    plane_id_ptr: u64,
    count_planes: u32,
    pad: u32,
}

#[repr(C)]
#[derive(Default, Debug)]
struct DrmModeGetPlane {
    plane_id: u32,
    crtc_id: u32,
    fb_id: u32,
    possible_crtcs: u32,
    gamma_size: u32,
    count_format_types: u32,
    format_type_ptr: u64,
    pad: u32,
}

#[repr(C)]
#[derive(Default, Debug)]
struct DrmModeSetPlane {
    plane_id: u32,
    crtc_id: u32,
    fb_id: u32,
    flags: u32,
    crtc_x: i32,
    crtc_y: i32,
    crtc_w: u32,
    crtc_h: u32,
    src_x: u32,
    src_y: u32,
    src_w: u32,
    src_h: u32,
}

#[repr(C)]
#[derive(Default, Debug)]
struct DrmPrimeHandle {
    handle: u32,
    pad: u32,
    fd: i32,
    flags: u32,
}

#[repr(C)]
#[derive(Default, Debug)]
struct DrmModeFbCmd2 {
    fb_id: u32,
    width: u32,
    height: u32,
    pixel_format: u32,
    flags: u32,
    handles: [u32; 4],
    pitches: [u32; 4],
    offsets: [u32; 4],
    modifier: [u64; 4],
}

// DrmModeAtomic — структура для DRM_IOCTL_MODE_ATOMIC (atomic commit).
// Сейчас не используется (используем legacy SETPLANE), но оставлена для
// будущей реализации atomic modesetting. Помечена allow(dead_code) чтобы
// не засорять warnings.
#[repr(C)]
#[derive(Default, Debug)]
#[allow(dead_code)]
struct DrmModeAtomic {
    flags: u32,
    count_objs: u32,
    objs_ptr: u64,
    count_props_ptr: u64,
    props_ptr: u64,
    prop_values_ptr: u64,
    reserved: u64,
    user_data: u64,
}

// Plane types.
const DRM_PLANE_TYPE_OVERLAY: u32 = 0;
const DRM_PLANE_TYPE_PRIMARY: u32 = 1;
const DRM_PLANE_TYPE_CURSOR: u32 = 2;

#[derive(Debug, Clone)]
pub struct Plane {
    pub id: u32,
    pub kind: u32, // DRM_PLANE_TYPE_*
}

/// Менеджер overlay planes: отслеживает занятые/свободные planes,
/// импортирует dma-buf'ы из X11, создаёт DRM framebuffers.
pub struct OverlayManager {
    pub drm_fd: RawFd,
    pub planes: Vec<Plane>,
    /// Map: x11_window_id → OverlayAssignment.
    pub assignments: std::collections::HashMap<u32, OverlayAssignment>,
}

#[derive(Debug)]
pub struct OverlayAssignment {
    pub plane_id: u32,
    pub crtc_id: u32,
    pub fb_id: u32,
    pub gem_handle: u32,
    pub dmabuf_fd: RawFd,
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
}

impl OverlayManager {
    pub fn new(drm_fd: RawFd) -> Result<Self> {
        let planes = enumerate_planes(drm_fd)?;
        log::info!("DRM planes: {} total, {} overlay, {} primary, {} cursor",
            planes.len(),
            planes.iter().filter(|p| p.kind == DRM_PLANE_TYPE_OVERLAY).count(),
            planes.iter().filter(|p| p.kind == DRM_PLANE_TYPE_PRIMARY).count(),
            planes.iter().filter(|p| p.kind == DRM_PLANE_TYPE_CURSOR).count(),
        );
        Ok(OverlayManager {
            drm_fd,
            planes,
            assignments: std::collections::HashMap::new(),
        })
    }

    /// Присваивает dma-buf X11 окна к overlay plane.
    /// `crtc_id` — CRTC монитора, на котором окно должно отображаться.
    /// `dmabuf` — dma-buf из DRI3.
    /// `(x, y, w, h)` — позиция и размер в экранных координатах.
    pub fn assign_window(
        &mut self,
        x11_window_id: u32,
        crtc_id: u32,
        dmabuf: &DmaBuf,
        x: i32, y: i32, w: u32, h: u32,
    ) -> Result<()> {
        // Импортируем dma-buf fd в DRM как GEM handle.
        let gem_handle = prime_fd_to_handle(self.drm_fd, dmabuf.fd)?;
        // Создаём DRM framebuffer из GEM handle.
        let fb_id = add_fb2_with_modifiers(
            self.drm_fd,
            dmabuf.width,
            dmabuf.height,
            dmabuf.fourcc,
            gem_handle,
            dmabuf.stride,
            dmabuf.offset,
            dmabuf.modifier,
        )?;
        // Находим свободный overlay plane для этого CRTC.
        let plane_id = self.find_free_overlay_plane(crtc_id)
            .context("no free overlay plane available")?;
        // Устанавливаем plane.
        set_plane(self.drm_fd, plane_id, crtc_id, fb_id, x, y, w, h,
            dmabuf.width, dmabuf.height)?;
        log::info!("overlay assigned: win=0x{:x} plane={} fb={} {}x{}@{},{}",
            x11_window_id, plane_id, fb_id, w, h, x, y);
        self.assignments.insert(x11_window_id, OverlayAssignment {
            plane_id,
            crtc_id,
            fb_id,
            gem_handle,
            dmabuf_fd: dmabuf.fd,
            x, y, w, h,
        });
        Ok(())
    }

    /// Обновляет позицию overlay для окна (без пересоздания fb).
    #[allow(dead_code)] // overlay positioning not yet wired into WM main loop
    pub fn update_position(&mut self, x11_window_id: u32, x: i32, y: i32, w: u32, h: u32) -> Result<()> {
        let assignment = self.assignments.get(&x11_window_id)
            .context("window not assigned to overlay")?;
        set_plane(self.drm_fd, assignment.plane_id, assignment.crtc_id,
            assignment.fb_id, x, y, w, h, assignment.w, assignment.h)?;
        let a = self.assignments.get_mut(&x11_window_id).unwrap();
        a.x = x; a.y = y; a.w = w; a.h = h;
        Ok(())
    }

    /// Освобождает overlay plane для окна.
    #[allow(dead_code)] // overlay teardown not yet wired into WM main loop
    pub fn unassign_window(&mut self, x11_window_id: u32) -> Result<()> {
        if let Some(assignment) = self.assignments.remove(&x11_window_id) {
            // Disable plane (fb_id = 0).
            let _ = set_plane(self.drm_fd, assignment.plane_id, assignment.crtc_id,
                0, 0, 0, 0, 0, 0, 0);
            // Destroy framebuffer.
            let fb_id = assignment.fb_id;
            unsafe {
                libc::ioctl(self.drm_fd, DRM_IOCTL_MODE_RMFB as _, &fb_id as *const _ as *const _);
            }
            // Close GEM handle.
            let mut close = DrmPrimeHandle { handle: assignment.gem_handle, pad: 0, fd: 0, flags: 0 };
            unsafe {
                libc::ioctl(self.drm_fd, 0x40046441 /* DRM_IOCTL_GEM_CLOSE */, &mut close as *mut _ as *mut _);
            }
            // Close dma-buf fd.
            unsafe { libc::close(assignment.dmabuf_fd); }
            log::info!("overlay unassigned: win=0x{:x}", x11_window_id);
        }
        Ok(())
    }

    /// Находит свободный overlay plane для указанного CRTC.
    fn find_free_overlay_plane(&self, _crtc_id: u32) -> Option<u32> {
        // TODO: properly map crtc_id to possible_crtcs bitmask.
        // Для упрощения возвращаем первый overlay plane не занятый в assignments.
        let used_planes: std::collections::HashSet<u32> = self.assignments.values()
            .map(|a| a.plane_id).collect();
        for plane in &self.planes {
            if plane.kind == DRM_PLANE_TYPE_OVERLAY && !used_planes.contains(&plane.id) {
                return Some(plane.id);
            }
        }
        None
    }
}

fn enumerate_planes(fd: RawFd) -> Result<Vec<Plane>> {
    let mut res = DrmModeGetPlaneRes::default();
    ioctl(fd, DRM_IOCTL_MODE_GETPLANERESOURCES, &mut res)?;
    let n = res.count_planes as usize;
    let mut plane_ids = vec![0u32; n];
    res.plane_id_ptr = plane_ids.as_mut_ptr() as u64;
    ioctl(fd, DRM_IOCTL_MODE_GETPLANERESOURCES, &mut res)?;

    let mut planes = Vec::new();
    for pid in &plane_ids {
        let mut p = DrmModeGetPlane::default();
        p.plane_id = *pid;
        if ioctl(fd, DRM_IOCTL_MODE_GETPLANE, &mut p).is_err() { continue; }
        // TODO: get plane type через DRM_IOCTL_MODE_GETPROPERTIES.
        // Для упрощения считаем все non-primary planes как overlay.
        let kind = if p.fb_id == 0 { DRM_PLANE_TYPE_OVERLAY } else { DRM_PLANE_TYPE_PRIMARY };
        planes.push(Plane {
            id: *pid,
            kind,
        });
    }
    Ok(planes)
}

fn prime_fd_to_handle(fd: RawFd, dma_buf_fd: RawFd) -> Result<u32> {
    let mut ph = DrmPrimeHandle {
        handle: 0,
        pad: 0,
        fd: dma_buf_fd,
        flags: 0,
    };
    let ret = unsafe { libc::ioctl(fd, DRM_IOCTL_PRIME_FD_TO_HANDLE as _, &mut ph as *mut _ as *mut _) };
    if ret < 0 {
        anyhow::bail!("PRIME_FD_TO_HANDLE failed: {}", std::io::Error::last_os_error());
    }
    Ok(ph.handle)
}

fn add_fb2_with_modifiers(
    fd: RawFd, w: u32, h: u32, fourcc: u32, handle: u32,
    stride: u32, offset: u32, modifier: u64,
) -> Result<u32> {
    let mut fb = DrmModeFbCmd2::default();
    fb.width = w;
    fb.height = h;
    fb.pixel_format = fourcc;
    fb.flags = 0x10000000; // DRM_MODE_FB_MODIFIERS (1 << 28)
    fb.handles[0] = handle;
    fb.pitches[0] = stride;
    fb.offsets[0] = offset;
    fb.modifier[0] = modifier;
    let ret = unsafe { libc::ioctl(fd, DRM_IOCTL_MODE_ADDFB2 as _, &mut fb as *mut _ as *mut _) };
    if ret < 0 {
        // Fallback без modifiers (если modifier == 0).
        if modifier == 0 {
            return create_fb2(fd, w, h, handle, stride);
        }
        anyhow::bail!("ADDFB2 with modifiers failed: {}", std::io::Error::last_os_error());
    }
    Ok(fb.fb_id)
}

fn set_plane(
    fd: RawFd, plane_id: u32, crtc_id: u32, fb_id: u32,
    crtc_x: i32, crtc_y: i32, crtc_w: u32, crtc_h: u32,
    src_w: u32, src_h: u32,
) -> Result<()> {
    let mut p = DrmModeSetPlane::default();
    p.plane_id = plane_id;
    p.crtc_id = crtc_id;
    p.fb_id = fb_id;
    p.flags = 0;
    p.crtc_x = crtc_x;
    p.crtc_y = crtc_y;
    p.crtc_w = crtc_w;
    p.crtc_h = crtc_h;
    // src координаты в 16.16 fixed point.
    p.src_x = 0;
    p.src_y = 0;
    p.src_w = src_w << 16;
    p.src_h = src_h << 16;
    let ret = unsafe { libc::ioctl(fd, DRM_IOCTL_MODE_SETPLANE as _, &p as *const _ as *const _) };
    if ret < 0 {
        anyhow::bail!("SETPLANE failed: {}", std::io::Error::last_os_error());
    }
    Ok(())
}
