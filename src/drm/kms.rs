//! DRM/KMS инициализация через прямые libc ioctls.
//!
//! Это надёжнее, чем завязываться на API drm-rs, который меняется между версиями.
//! Мы вручную делаем:
//!   1. open(/dev/dri/card0)
//!   2. DRM_IOCTL_SET_MASTER
//!   3. DRM_IOCTL_MODE_GETRESOURCES + GETCONNECTOR + GETENCODER
//!   4. DRM_IOCTL_MODE_CREATE_DUMB (×2 для double buffering)
//!   5. DRM_IOCTL_MODE_MAP_DUMB + mmap
//!   6. DRM_IOCTL_MODE_ADDFB2
//!   7. DRM_IOCTL_MODE_SETCRTC
//!   8. DRM_IOCTL_MODE_PAGE_FLIP — каждый кадр

use anyhow::{Context, Result};
use std::os::unix::io::RawFd;
use std::path::Path;

// ===== DRM ioctl definitions =====
// Эти константы одинаковые на всех Linux-архитектурах (x86, x86_64, aarch64).

const DRM_IOCTL_BASE: u32 = b'd' as u32;

// ioctls (MAGIC, NR, SIZE, DIR): encoded as 0x<DIR><SIZE><NR><TYPE>
// TYPE = 'd' = 0x64
// NR for each command:
const DRM_IOCTL_VERSION: u32         = ior(0, 0, 24); // version
const DRM_IOCTL_GET_MAGIC: u32       = ior(0, 2, 6);
const DRM_IOCTL_SET_MASTER: u32      = io(0x64, 0x1e); // no args
const DRM_IOCTL_DROP_MASTER: u32     = io(0x64, 0x1f);

// DRM_MODE_ ioctls (NR starts at 0xA0).
const DRM_IOCTL_MODE_GETRESOURCES: u32      = ior(0xA0, 0, std::mem::size_of::<drm_mode_card_res>() as u32);
const DRM_IOCTL_MODE_GETCONNECTOR: u32      = iowr(0xA0, 7, std::mem::size_of::<drm_mode_get_connector>() as u32);
const DRM_IOCTL_MODE_GETENCODER: u32        = ior(0xA0, 8, std::mem::size_of::<drm_mode_get_encoder>() as u32);
const DRM_IOCTL_MODE_GETCRTC: u32           = iowr(0xA0, 9, std::mem::size_of::<drm_mode_crtc>() as u32);
const DRM_IOCTL_MODE_SETCRTC: u32           = iow(0xA0, 10, std::mem::size_of::<drm_mode_crtc>() as u32);
// (CURSOR не используется в MVP)
const DRM_IOCTL_MODE_CREATE_DUMB: u32       = iowr(0xA0, 0xB2, std::mem::size_of::<drm_mode_create_dumb>() as u32);
const DRM_IOCTL_MODE_MAP_DUMB: u32          = iowr(0xA0, 0xB3, std::mem::size_of::<drm_mode_map_dumb>() as u32);
const DRM_IOCTL_MODE_DESTROY_DUMB: u32      = iowr(0xA0, 0xB4, std::mem::size_of::<drm_mode_destroy_dumb>() as u32);
const DRM_IOCTL_MODE_ADDFB2: u32            = iowr(0xA0, 0xB8, std::mem::size_of::<drm_mode_fb_cmd2>() as u32);
const DRM_IOCTL_MODE_PAGE_FLIP: u32         = iow(0xA0, 0x0B, std::mem::size_of::<drm_mode_crtc_page_flip>() as u32);

// Encoding helpers (linux asm-generic ioctl encoding: dir<<30 | size<<16 | type<<8 | nr)
const fn _io(typ: u32, nr: u32) -> u32 { (0 << 30) | (0 << 16) | (typ << 8) | nr }
const fn io(typ: u32, nr: u32) -> u32 { _io(typ, nr) }
const fn ior(typ: u32, nr: u32, size: u32) -> u32 { (2 << 30) | (size << 16) | (typ << 8) | nr }
const fn iow(typ: u32, nr: u32, size: u32) -> u32 { (1 << 30) | (size << 16) | (typ << 8) | nr }
const fn iowr(typ: u32, nr: u32, size: u32) -> u32 { (3 << 30) | (size << 16) | (typ << 8) | nr }

// ===== C structs (must match kernel layout exactly) =====

#[repr(C)]
#[derive(Default, Debug)]
pub struct drm_mode_card_res {
    pub fb_id_ptr: u64,
    pub crtc_id_ptr: u64,
    pub connector_id_ptr: u64,
    pub encoder_id_ptr: u64,
    pub count_fbs: u32,
    pub count_crtcs: u32,
    pub count_connectors: u32,
    pub count_encoders: u32,
    pub min_width: u32,
    pub max_width: u32,
    pub min_height: u32,
    pub max_height: u32,
}

#[repr(C)]
#[derive(Default, Debug)]
pub struct drm_mode_get_connector {
    pub encoders_ptr: u64,
    pub modes_ptr: u64,
    pub props_ptr: u64,
    pub prop_values_ptr: u64,
    pub count_modes: u32,
    pub count_props: u32,
    pub count_encoders: u32,
    pub encoder_id: u32,
    pub connector_id: u32,
    pub connector_type: u32,
    pub connector_type_id: u32,
    pub connection: u32,
    pub mm_width: u32,
    pub mm_height: u32,
    pub subpixel: u32,
    pub pad: u32,
}

#[repr(C)]
#[derive(Default, Debug, Copy, Clone)]
pub struct drm_mode_modeinfo {
    pub clock: u32,
    pub hdisplay: u16,
    pub hsync_start: u16,
    pub hsync_end: u16,
    pub htotal: u16,
    pub hskew: u16,
    pub vdisplay: u16,
    pub vsync_start: u16,
    pub vsync_end: u16,
    pub vtotal: u16,
    pub vscan: u16,
    pub vrefresh: u32,
    pub flags: u32,
    pub type_: u32,
    pub name: [u8; 32],
}

#[repr(C)]
#[derive(Default, Debug)]
pub struct drm_mode_get_encoder {
    pub encoder_id: u32,
    pub encoder_type: u32,
    pub crtc_id: u32,
    pub possible_crtcs: u32,
    pub possible_clones: u32,
}

#[repr(C)]
#[derive(Default, Debug)]
pub struct drm_mode_crtc {
    pub set_connectors_ptr: u64,
    pub count_connectors: u32,
    pub crtc_id: u32,
    pub fb_id: u32,
    pub x: u32,
    pub y: u32,
    pub gamma_size: u32,
    pub mode_valid: u32,
    pub mode: drm_mode_modeinfo,
    pub pad: u32,
}

#[repr(C)]
#[derive(Default, Debug)]
pub struct drm_mode_create_dumb {
    pub height: u32,
    pub width: u32,
    pub bpp: u32,
    pub flags: u32,
    pub handle: u32,
    pub pitch: u32,
    pub size: u64,
}

#[repr(C)]
#[derive(Default, Debug)]
pub struct drm_mode_map_dumb {
    pub handle: u32,
    pub pad: u32,
    pub offset: u64,
}

#[repr(C)]
#[derive(Default, Debug)]
pub struct drm_mode_destroy_dumb {
    pub handle: u32,
}

#[repr(C)]
#[derive(Default, Debug)]
pub struct drm_mode_fb_cmd2 {
    pub fb_id: u32,
    pub width: u32,
    pub height: u32,
    pub pixel_format: u32,
    pub flags: u32,
    pub handles: [u32; 4],
    pub pitches: [u32; 4],
    pub offsets: [u32; 4],
    pub modifier: [u64; 4],
}

#[repr(C)]
#[derive(Default, Debug)]
pub struct drm_mode_crtc_page_flip {
    pub crtc_id: u32,
    pub fb_id: u32,
    pub flags: u32,
    pub sequence: u32,
    pub user_data: u64,
}

// DRM_MODE_PAGE_FLIP_FLAGS
const DRM_MODE_PAGE_FLIP_EVENT: u32 = 0x01;
const DRM_MODE_PAGE_FLIP_ASYNC: u32 = 0x02;
#[allow(dead_code)]
const DRM_MODE_PAGE_FLIP_TARGET: u32 = 0x04;

// DRM_MODE_FLAG_INTERLACE = 0x02 — мы пропускаем interlaced modes.
const DRM_MODE_FLAG_INTERLACE: u32 = 0x02;

// Connector status.
#[allow(dead_code)]
const DRM_MODE_CONNECTED_LEGACY: u32 = 1;

const DRM_FOURCC_XRGB8888: u32 = 0x34325258; // 'XR24' little-endian

// ===== Backend struct =====

pub struct DrmBackend {
    pub fd: RawFd,
    pub width: u32,
    pub height: u32,
    pub stride: u32,
    pub connector_id: u32,
    pub crtc_id: u32,
    pub mode: drm_mode_modeinfo,
    pub front: DumbBuffer,
    pub back: DumbBuffer,
    pub front_fb: u32,
    pub back_fb: u32,
}

pub struct DumbBuffer {
    pub handle: u32,
    pub width: u32,
    pub height: u32,
    pub stride: u32,
    pub size: u64,
    pub mmap_addr: *mut u8,
}

unsafe impl Send for DumbBuffer {}
unsafe impl Sync for DumbBuffer {}

impl DumbBuffer {
    pub fn as_slice(&self) -> &mut [u8] {
        unsafe { std::slice::from_raw_parts_mut(self.mmap_addr, self.size as usize) }
    }
}

impl Drop for DumbBuffer {
    fn drop(&mut self) {
        if !self.mmap_addr.is_null() {
            unsafe { libc::munmap(self.mmap_addr as *mut _, self.size as usize); }
        }
    }
}

impl DrmBackend {
    pub fn new(path: &str, _pref_w: Option<u32>, _pref_h: Option<u32>) -> Result<Self> {
        // 1. Open /dev/dri/card0.
        let fd = unsafe {
            libc::open(
                std::ffi::CString::new(path).unwrap().as_ptr(),
                libc::O_RDWR | libc::O_CLOEXEC,
            )
        };
        if fd < 0 {
            anyhow::bail!("open {}: {}", path, std::io::Error::last_os_error());
        }

        // 2. Set master.
        let ret = unsafe { libc::ioctl(fd, DRM_IOCTL_SET_MASTER as _, 0 as *mut libc::c_void) };
        if ret < 0 {
            log::warn!("DRM_IOCTL_SET_MASTER failed ({}): {} — already master?",
                fd, std::io::Error::last_os_error());
        }

        // 3. Get resources.
        let mut res = drm_mode_card_res::default();
        // First call to get counts.
        let _ = ioctl(fd, DRM_IOCTL_MODE_GETRESOURCES, &mut res)?;
        let connectors_count = res.count_connectors as usize;
        let crtcs_count = res.count_crtcs as usize;
        let mut connector_ids = vec![0u32; connectors_count];
        let mut crtc_ids = vec![0u32; crtcs_count];
        res.connector_id_ptr = connector_ids.as_mut_ptr() as u64;
        res.crtc_id_ptr = crtc_ids.as_mut_ptr() as u64;
        let _ = ioctl(fd, DRM_IOCTL_MODE_GETRESOURCES, &mut res)?;
        log::debug!("DRM resources: {} connectors, {} crtcs", connectors_count, crtcs_count);

        // 4. Find first connected connector.
        let mut chosen_connector: Option<u32> = None;
        let mut chosen_mode: Option<drm_mode_modeinfo> = None;
        let mut chosen_encoder: Option<u32> = None;
        for cid in &connector_ids {
            let mut conn = drm_mode_get_connector::default();
            conn.connector_id = *cid;
            // First call: get counts.
            let _ = ioctl(fd, DRM_IOCTL_MODE_GETCONNECTOR, &mut conn)?;
            let n_modes = conn.count_modes as usize;
            let n_encs = conn.count_encoders as usize;
            let mut modes = vec![drm_mode_modeinfo::default(); n_modes];
            let mut encs = vec![0u32; n_encs];
            conn.modes_ptr = modes.as_mut_ptr() as u64;
            conn.encoders_ptr = encs.as_mut_ptr() as u64;
            let _ = ioctl(fd, DRM_IOCTL_MODE_GETCONNECTOR, &mut conn)?;
            if conn.connection != DRM_MODE_CONNECTED_LEGACY || n_modes == 0 {
                continue;
            }
            // Choose first non-interlaced mode (preferably preferred).
            let mode = modes.iter()
                .find(|m| m.flags & DRM_MODE_FLAG_INTERLACE == 0 && m.type_ & 0x04 != 0) // PREFERRED bit
                .or_else(|| modes.iter().find(|m| m.flags & DRM_MODE_FLAG_INTERLACE == 0))
                .copied()
                .ok_or_else(|| anyhow::anyhow!("no usable mode on connector {}", cid))?;
            chosen_connector = Some(*cid);
            chosen_mode = Some(mode);
            chosen_encoder = if conn.encoder_id != 0 { Some(conn.encoder_id) } else {
                encs.first().copied()
            };
            log::info!("connector {}: {}x{}@{} enc={:?}",
                cid, mode.hdisplay, mode.vdisplay, mode.vrefresh, chosen_encoder);
            break;
        }
        let connector_id = chosen_connector.context("no connected display")?;
        let mode = chosen_mode.context("no mode chosen")?;
        let encoder_id = chosen_encoder.unwrap_or(0);

        // 5. Resolve CRTC from encoder, or pick first.
        let crtc_id = if encoder_id != 0 {
            let mut enc = drm_mode_get_encoder::default();
            enc.encoder_id = encoder_id;
            let _ = ioctl(fd, DRM_IOCTL_MODE_GETENCODER, &mut enc)?;
            if enc.crtc_id != 0 { enc.crtc_id }
            else { crtc_ids.first().copied().context("no crtc")? }
        } else {
            crtc_ids.first().copied().context("no crtc")?
        };

        let width = mode.hdisplay as u32;
        let height = mode.vdisplay as u32;

        // 6. Create dumb buffers (double buffered).
        let front = create_dumb_buffer(fd, width, height)?;
        let back = create_dumb_buffer(fd, width, height)?;

        // 7. Create KMS framebuffers.
        let front_fb = create_fb2(fd, width, height, front.handle, front.stride)?;
        let back_fb = create_fb2(fd, width, height, back.handle, back.stride)?;

        // 8. Modeset: set CRTC with back buffer + mode + connector.
        let mut crtc = drm_mode_crtc::default();
        crtc.crtc_id = crtc_id;
        crtc.set_connectors_ptr = &connector_id as *const u32 as u64;
        crtc.count_connectors = 1;
        crtc.fb_id = back_fb;
        crtc.x = 0;
        crtc.y = 0;
        crtc.mode_valid = 1;
        crtc.mode = mode;
        let ret = unsafe { libc::ioctl(fd, DRM_IOCTL_MODE_SETCRTC as _, &crtc as *const _ as *const _) };
        if ret < 0 {
            anyhow::bail!("DRM_IOCTL_MODE_SETCRTC failed: {}", std::io::Error::last_os_error());
        }

        log::info!("DRM/KMS initialized: {}x{} crtc={} connector={} fb={}",
            width, height, crtc_id, connector_id, back_fb);

        Ok(DrmBackend {
            fd,
            width, height,
            stride: back.stride,
            connector_id,
            crtc_id,
            mode,
            front,
            back,
            front_fb,
            back_fb,
        })
    }

    pub fn back_buffer_slice(&self) -> &mut [u8] {
        self.back.as_slice()
    }

    /// Page-flip: показываем back buffer, после чего front и back меняются местами.
    pub fn flip(&mut self) -> Result<()> {
        let mut pf = drm_mode_crtc_page_flip::default();
        pf.crtc_id = self.crtc_id;
        pf.fb_id = self.back_fb;
        pf.flags = 0; // синхронный page-flip (без DRM_MODE_PAGE_FLIP_EVENT)
        pf.user_data = 0;
        let ret = unsafe { libc::ioctl(self.fd, DRM_IOCTL_MODE_PAGE_FLIP as _, &pf as *const _ as *const _) };
        if ret < 0 {
            // EBUSY = page-flip уже в процессе; EINVAL = CRTC doesn't support page-flip — fallback.
            let err = std::io::Error::last_os_error();
            if err.raw_os_error() == Some(libc::EBUSY) {
                // ok, drop frame
                return Ok(());
            }
            log::warn!("page_flip failed: {}", err);
            return Ok(()); // не критично — данные уже в back buffer
        }
        // Swap.
        std::mem::swap(&mut self.front, &mut self.back);
        std::mem::swap(&mut self.front_fb, &mut self.back_fb);
        Ok(())
    }
}

impl Drop for DrmBackend {
    fn drop(&mut self) {
        unsafe {
            libc::ioctl(self.fd, DRM_IOCTL_DROP_MASTER as _, 0 as *mut libc::c_void);
            libc::close(self.fd);
        }
    }
}

fn ioctl<T>(fd: RawFd, req: u32, arg: &mut T) -> Result<()> {
    let ret = unsafe { libc::ioctl(fd, req as libc::c_ulong, arg as *mut T as *mut libc::c_void) };
    if ret < 0 {
        anyhow::bail!("ioctl(0x{:x}) failed: {}", req, std::io::Error::last_os_error());
    }
    Ok(())
}

// Private helpers renamed to _inner to avoid conflicts with public versions below.

// ===== Public helpers for multi-monitor =====

/// Information about a connected DRM connector.
pub struct ConnectorInfo {
    pub connector_id: u32,
    pub connector_name: String,
    pub connection: u32,
    pub encoder_id: Option<u32>,
    pub modes: Vec<drm_mode_modeinfo>,
}

pub const DRM_MODE_CONNECTED: u32 = 1;

/// Открывает /dev/dri/card0 (или card1 как fallback).
pub fn open_drm_card() -> Result<RawFd> {
    for path in &["/dev/dri/card0", "/dev/dri/card1"] {
        let fd = unsafe {
            libc::open(
                std::ffi::CString::new(*path).unwrap().as_ptr(),
                libc::O_RDWR | libc::O_CLOEXEC,
            )
        };
        if fd >= 0 { return Ok(fd); }
    }
    anyhow::bail!("no DRM device available");
}

pub fn set_drm_master(fd: RawFd) -> Result<()> {
    let ret = unsafe { libc::ioctl(fd, DRM_IOCTL_SET_MASTER as _, 0 as *mut libc::c_void) };
    if ret < 0 {
        log::warn!("DRM_IOCTL_SET_MASTER failed: {}", std::io::Error::last_os_error());
    }
    Ok(())
}

pub fn drop_drm_master(fd: RawFd) {
    unsafe { libc::ioctl(fd, DRM_IOCTL_DROP_MASTER as _, 0 as *mut libc::c_void); }
}

/// Перечисляет все коннекторы на DRM card.
pub fn enumerate_connectors(fd: RawFd) -> Result<Vec<ConnectorInfo>> {
    let mut res = drm_mode_card_res::default();
    ioctl(fd, DRM_IOCTL_MODE_GETRESOURCES, &mut res)?;
    let n = res.count_connectors as usize;
    let mut connector_ids = vec![0u32; n];
    res.connector_id_ptr = connector_ids.as_mut_ptr() as u64;
    ioctl(fd, DRM_IOCTL_MODE_GETRESOURCES, &mut res)?;

    let mut out = Vec::new();
    for cid in &connector_ids {
        let mut conn = drm_mode_get_connector::default();
        conn.connector_id = *cid;
        let _ = ioctl(fd, DRM_IOCTL_MODE_GETCONNECTOR, &mut conn);
        let nmodes = conn.count_modes as usize;
        let mut modes = vec![drm_mode_modeinfo::default(); nmodes];
        conn.modes_ptr = modes.as_mut_ptr() as u64;
        let _ = ioctl(fd, DRM_IOCTL_MODE_GETCONNECTOR, &mut conn);

        let connector_name = format!("{}-{}", connector_type_name(conn.connector_type), conn.connector_type_id);
        out.push(ConnectorInfo {
            connector_id: *cid,
            connector_name,
            connection: conn.connection,
            encoder_id: if conn.encoder_id != 0 { Some(conn.encoder_id) } else { None },
            modes,
        });
    }
    Ok(out)
}

fn connector_type_name(t: u32) -> &'static str {
    match t {
        0 => "Unknown",
        1 => "VGA",
        2 => "DVII",
        3 => "DVID",
        4 => "DVIA",
        5 => "Composite",
        6 => "SVIDEO",
        7 => "LVDS",
        8 => "Component",
        9 => "9PinDIN",
        10 => "DisplayPort",
        11 => "HDMIA",
        12 => "HDMIB",
        13 => "TV",
        14 => "eDP",
        15 => "Virtual",
        16 => "DSI",
        17 => "DPI",
        _ => "Unknown",
    }
}

pub fn get_encoder_crtc(fd: RawFd, encoder_id: u32) -> Option<u32> {
    let mut enc = drm_mode_get_encoder::default();
    enc.encoder_id = encoder_id;
    if ioctl(fd, DRM_IOCTL_MODE_GETENCODER, &mut enc).is_err() { return None; }
    if enc.crtc_id != 0 { Some(enc.crtc_id) } else { None }
}

pub fn get_first_crtc(fd: RawFd) -> Option<u32> {
    let mut res = drm_mode_card_res::default();
    if ioctl(fd, DRM_IOCTL_MODE_GETRESOURCES, &mut res).is_err() { return None; }
    let n = res.count_crtcs as usize;
    let mut crtc_ids = vec![0u32; n];
    res.crtc_id_ptr = crtc_ids.as_mut_ptr() as u64;
    if ioctl(fd, DRM_IOCTL_MODE_GETRESOURCES, &mut res).is_err() { return None; }
    crtc_ids.first().copied()
}

pub fn create_dumb_buffer(fd: RawFd, w: u32, h: u32) -> Result<DumbBuffer> {
    let mut create = drm_mode_create_dumb::default();
    create.width = w;
    create.height = h;
    create.bpp = 32;
    create.flags = 0;
    let ret = unsafe { libc::ioctl(fd, DRM_IOCTL_MODE_CREATE_DUMB as _, &mut create as *mut _ as *mut _) };
    if ret < 0 {
        anyhow::bail!("CREATE_DUMB failed: {}", std::io::Error::last_os_error());
    }
    let mut map = drm_mode_map_dumb::default();
    map.handle = create.handle;
    let ret = unsafe { libc::ioctl(fd, DRM_IOCTL_MODE_MAP_DUMB as _, &mut map as *mut _ as *mut _) };
    if ret < 0 {
        anyhow::bail!("MAP_DUMB failed: {}", std::io::Error::last_os_error());
    }
    let addr = unsafe {
        libc::mmap(
            std::ptr::null_mut(),
            create.size as usize,
            libc::PROT_READ | libc::PROT_WRITE,
            libc::MAP_SHARED,
            fd,
            map.offset as i64,
        )
    };
    if addr == libc::MAP_FAILED {
        anyhow::bail!("mmap dumb buffer failed: {}", std::io::Error::last_os_error());
    }
    unsafe { libc::memset(addr, 0, create.size as usize); }
    Ok(DumbBuffer {
        handle: create.handle,
        width: w,
        height: h,
        stride: create.pitch,
        size: create.size,
        mmap_addr: addr as *mut u8,
    })
}

pub fn create_fb2(fd: RawFd, w: u32, h: u32, handle: u32, stride: u32) -> Result<u32> {
    create_fb2_inner(fd, w, h, handle, stride)
}

pub fn set_crtc(fd: RawFd, crtc_id: u32, fb_id: u32, connector_id: u32, mode: &drm_mode_modeinfo) -> Result<()> {
    let mut crtc = drm_mode_crtc::default();
    crtc.crtc_id = crtc_id;
    crtc.set_connectors_ptr = &connector_id as *const u32 as u64;
    crtc.count_connectors = 1;
    crtc.fb_id = fb_id;
    crtc.x = 0;
    crtc.y = 0;
    crtc.mode_valid = 1;
    crtc.mode = *mode;
    let ret = unsafe { libc::ioctl(fd, DRM_IOCTL_MODE_SETCRTC as _, &crtc as *const _ as *const _) };
    if ret < 0 {
        anyhow::bail!("SETCRTC failed: {}", std::io::Error::last_os_error());
    }
    Ok(())
}

pub fn page_flip(fd: RawFd, crtc_id: u32, fb_id: u32) -> Result<()> {
    let mut pf = drm_mode_crtc_page_flip::default();
    pf.crtc_id = crtc_id;
    pf.fb_id = fb_id;
    pf.flags = 0;
    pf.user_data = 0;
    let ret = unsafe { libc::ioctl(fd, DRM_IOCTL_MODE_PAGE_FLIP as _, &pf as *const _ as *const _) };
    if ret < 0 {
        let err = std::io::Error::last_os_error();
        if err.raw_os_error() == Some(libc::EBUSY) { return Ok(()); }
        log::warn!("page_flip on crtc {}: {}", crtc_id, err);
    }
    Ok(())
}

// rename internal function to avoid conflict
fn create_fb2_inner(fd: RawFd, w: u32, h: u32, handle: u32, stride: u32) -> Result<u32> {
    let mut fb = drm_mode_fb_cmd2::default();
    fb.width = w;
    fb.height = h;
    fb.pixel_format = DRM_FOURCC_XRGB8888;
    fb.handles[0] = handle;
    fb.pitches[0] = stride;
    let ret = unsafe { libc::ioctl(fd, DRM_IOCTL_MODE_ADDFB2 as _, &mut fb as *mut _ as *mut _) };
    if ret < 0 {
        anyhow::bail!("ADDFB2 failed: {}", std::io::Error::last_os_error());
    }
    Ok(fb.fb_id)
}
