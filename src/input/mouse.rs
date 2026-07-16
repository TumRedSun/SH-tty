//! Мышь через evdev (/dev/input/event*).
//!
//! Стратегия:
//!   1. Находим mouse device в /dev/input/by-path/ или /dev/input/event*.
//!   2. Читаем REL_X/REL_Y/BTN_LEFT/BTN_RIGHT/BTN_MIDDLE.
//!   3. Поддерживаем виртуальный курсор:
//!      - Если активный tile — X11: отправляем события через XTest extension
//!        в наш Xephyr display.
//!      - Иначе: обновляем позицию курсора, клики по tile-ам переключают фокус.
//!   4. Hardware cursor через DRM cursor plane (если поддерживается) —
//!      иначе рисуем программно в canvas.

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
#[allow(dead_code)] // EV_ABS not used for mouse (only relative motion)
const EV_ABS: u16 = 3;

const REL_X: u16 = 0;
const REL_Y: u16 = 1;
const REL_WHEEL: u16 = 8;
const REL_HWHEEL: u16 = 6;

const BTN_LEFT: u16 = 0x110;
const BTN_RIGHT: u16 = 0x111;
const BTN_MIDDLE: u16 = 0x112;

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
        let candidates = [
            "/dev/input/by-path/platform-i8042-serio-1-event-mouse",
            "/dev/input/by-path/pci0000:00-event-mouse",
            "/dev/input/mice",
            "/dev/input/event4",
            "/dev/input/event5",
            "/dev/input/event6",
            "/dev/input/event7",
            "/dev/input/event8",
            "/dev/input/event9",
            "/dev/input/event10",
        ];
        for path in &candidates {
            if let Ok(file) = std::fs::OpenOptions::new().read(true).write(true).open(path) {
                use std::os::unix::io::IntoRawFd;
                let fd = file.into_raw_fd();
                log::info!("mouse opened: {}", path);
                return Ok(Mouse {
                    fd,
                    x: (screen_w / 2) as i32,
                    y: (screen_h / 2) as i32,
                    left: false, right: false, middle: false,
                    screen_w, screen_h,
                });
            }
        }
        anyhow::bail!("no mouse device found in /dev/input/")
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
        // Простой неоновый курсор-крестик в MCD-стиле.
        use crate::ui::theme::Color;
        let x = self.x;
        let y = self.y;
        // Горизонтальная линия.
        canvas.fill_rect(x - 8, y - 1, 17, 2, theme.accent_magenta);
        // Вертикальная линия.
        canvas.fill_rect(x - 1, y - 8, 2, 17, theme.accent_magenta);
        // Точка в центре.
        canvas.fill_rect(x - 1, y - 1, 2, 2, Color(0xFF, 0xFF, 0xFF));
        // Glow.
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
