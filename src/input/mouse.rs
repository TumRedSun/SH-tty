//! Мышь через evdev (/dev/input/event*).
//!
//! Device detection: перебираем /dev/input/event* и проверяем через
//! EVIOCGBIT что устройство поддерживает EV_REL (relative motion) и
//! имеет BTN_LEFT. Это отсекает клавиатуры, joystick'и, touchscreens.

use anyhow::Result;
use std::os::unix::io::RawFd;

#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct InputEvent {
    pub tv_sec: i64,
    pub tv_usec: i64,
    pub typ: u16,
    pub code: u16,
    pub value: i32,
}

const EV_REL: u16 = 2;
const EV_KEY: u16 = 1;
#[allow(dead_code)]
const EV_ABS: u16 = 3;

const REL_X: u16 = 0;
const REL_Y: u16 = 1;
const REL_WHEEL: u16 = 8;
const REL_HWHEEL: u16 = 6;

const BTN_LEFT: u16 = 0x110;
const BTN_RIGHT: u16 = 0x111;
const BTN_MIDDLE: u16 = 0x112;

// ioctl: EVIOCGBIT(ev, len) = _IOR('E', 0x20 + ev, u8[len])
const fn eviocgbit(ev: u32, len: u32) -> libc::c_ulong {
    ((2u32 << 30) | (len << 16) | (0x45 << 8) | (0x20 + ev)) as libc::c_ulong
}

pub struct Mouse {
    pub fd: RawFd,
    pub x: i32,
    pub y: i32,
    pub left: bool,
    pub right: bool,
    pub middle: bool,
    pub screen_w: u32,
    pub screen_h: u32,
}

impl Mouse {
    pub fn open(screen_w: u32, screen_h: u32) -> Result<Self> {
        // Сначала пробуем by-path symlinks.
        let by_path_patterns = [
            "/dev/input/by-path/platform-i8042-serio-1-event-mouse",
            "/dev/input/by-path/*-event-mouse",
        ];
        for pattern in &by_path_patterns {
            if let Ok(entries) = glob::glob(pattern) {
                for entry in entries.flatten() {
                    if let Some(path_str) = entry.to_str() {
                        if let Ok(fd) = open_device_rw(path_str) {
                            if is_mouse(fd) {
                                log::info!("mouse opened (by-path): {}", entry.display());
                                return Ok(Mouse::new(fd, screen_w, screen_h));
                            }
                            unsafe { libc::close(fd); }
                        }
                    }
                }
            }
        }

        // Fallback: перебираем /dev/input/event* с EVIOCGBIT проверкой.
        for n in 0..=63u32 {
            let path = format!("/dev/input/event{}", n);
            if let Ok(fd) = open_device_rw(&path) {
                if is_mouse(fd) {
                    log::info!("mouse opened (event{}): {}", n, path);
                    return Ok(Mouse::new(fd, screen_w, screen_h));
                }
                unsafe { libc::close(fd); }
            }
        }

        // Last resort: legacy /dev/input/mice (shared PS/2 mouse device).
        if let Ok(fd) = open_device_rw("/dev/input/mice") {
            log::info!("mouse opened (legacy /dev/input/mice)");
            return Ok(Mouse::new(fd, screen_w, screen_h));
        }

        anyhow::bail!("no mouse device found in /dev/input/")
    }

    fn new(fd: RawFd, screen_w: u32, screen_h: u32) -> Self {
        Mouse {
            fd,
            x: (screen_w / 2) as i32,
            y: (screen_h / 2) as i32,
            left: false, right: false, middle: false,
            screen_w, screen_h,
        }
    }

    /// Обрабатывает события evdev. Возвращает список MouseEvents для WM.
    pub fn poll(&mut self) -> Vec<MouseEvent> {
        let mut events = Vec::new();
        let mut buf = [0u8; std::mem::size_of::<InputEvent>() * 64];
        loop {
            let n = unsafe { libc::read(self.fd, buf.as_mut_ptr() as *mut _, buf.len()) };
            if n <= 0 { break; }
            let cnt = (n as usize) / std::mem::size_of::<InputEvent>();
            let ptr = buf.as_ptr() as *const InputEvent;
            for i in 0..cnt {
                let ev = unsafe { ptr.add(i).read() };
                match ev.typ {
                    EV_REL => match ev.code {
                        REL_X => {
                            self.x += ev.value;
                            self.x = self.x.clamp(0, self.screen_w as i32 - 1);
                            events.push(MouseEvent::Move(self.x, self.y));
                        }
                        REL_Y => {
                            self.y += ev.value;
                            self.y = self.y.clamp(0, self.screen_h as i32 - 1);
                            events.push(MouseEvent::Move(self.x, self.y));
                        }
                        REL_WHEEL => events.push(MouseEvent::Scroll(ev.value)),
                        REL_HWHEEL => events.push(MouseEvent::HScroll(ev.value)),
                        _ => {}
                    },
                    EV_KEY => match ev.code {
                        BTN_LEFT => {
                            self.left = ev.value != 0;
                            events.push(if ev.value != 0 { MouseEvent::LeftPress } else { MouseEvent::LeftRelease });
                        }
                        BTN_RIGHT => {
                            self.right = ev.value != 0;
                            events.push(if ev.value != 0 { MouseEvent::RightPress } else { MouseEvent::RightRelease });
                        }
                        BTN_MIDDLE => {
                            self.middle = ev.value != 0;
                            events.push(if ev.value != 0 { MouseEvent::MiddlePress } else { MouseEvent::MiddleRelease });
                        }
                        _ => {}
                    },
                    _ => {}
                }
            }
            if (n as usize) < buf.len() { break; }
        }
        events
    }

    /// Рисует курсор на canvas (софтверный курсор, fallback если нет DRM cursor plane).
    pub fn render_cursor(&self, canvas: &crate::render::canvas::Canvas, theme: &crate::ui::theme::Theme) {
        use crate::ui::theme::Color;
        let x = self.x;
        let y = self.y;
        canvas.fill_rect(x - 8, y - 1, 17, 2, theme.accent_magenta);
        canvas.fill_rect(x - 1, y - 8, 2, 17, theme.accent_magenta);
        canvas.fill_rect(x - 1, y - 1, 2, 2, Color(0xFF, 0xFF, 0xFF));
        for i in 1..=2 {
            let alpha = (30 / i) as u8;
            let _ = alpha;
            canvas.fill_rect(x - 8 - i as i32, y - 1, 17, 2, Color(0x80, 0x17, 0x4B));
            canvas.fill_rect(x - 1, y - 8 - i as i32, 2, 17, Color(0x80, 0x17, 0x4B));
        }
    }
}

impl Drop for Mouse {
    fn drop(&mut self) {
        unsafe { libc::close(self.fd); }
    }
}

#[derive(Debug, Clone, Copy)]
#[allow(dead_code)] // Scroll/HScroll produced but not yet handled in WM loop
pub enum MouseEvent {
    Move(i32, i32),
    LeftPress, LeftRelease,
    RightPress, RightRelease,
    MiddlePress, MiddleRelease,
    Scroll(i32),
    HScroll(i32),
}

/// Открывает устройство на чтение/запись.
fn open_device_rw(path: &str) -> std::io::Result<RawFd> {
    let c_path = std::ffi::CString::new(path)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e))?;
    let fd = unsafe { libc::open(c_path.as_ptr(), libc::O_RDWR | libc::O_CLOEXEC) };
    if fd < 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(fd)
}

/// Проверяет, является ли устройство мышью через EVIOCGBIT.
///
/// Устройство считается мышью если:
/// 1. Поддерживает EV_REL (relative motion)
/// 2. Поддерживает EV_KEY с BTN_LEFT
/// Это отсекает клавиатуры, touchscreens (EV_ABS), joystick'и.
fn is_mouse(fd: RawFd) -> bool {
    let mut ev_bits = [0u8; 4];
    let ret = unsafe {
        libc::ioctl(fd, eviocgbit(0 /* EV_SYN */, ev_bits.len() as u32), ev_bits.as_mut_ptr())
    };
    if ret < 0 { return false; }
    let has_rel = (ev_bits[0] & (1 << EV_REL)) != 0;
    let has_key = (ev_bits[0] & (1 << EV_KEY)) != 0;
    if !has_rel || !has_key { return false; }

    let mut key_bits = [0u8; 96];
    let ret = unsafe {
        libc::ioctl(fd, eviocgbit(EV_KEY as u32, key_bits.len() as u32), key_bits.as_mut_ptr())
    };
    if ret < 0 { return false; }

    // BTN_LEFT = 0x110 = 272. byte 34, bit 0.
    let byte_idx = (BTN_LEFT / 8) as usize;
    let bit_idx = (BTN_LEFT % 8) as u8;
    if byte_idx >= key_bits.len() { return false; }
    (key_bits[byte_idx] >> bit_idx) & 1 == 1
}
