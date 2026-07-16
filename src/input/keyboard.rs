//! Чтение клавиатуры через raw evdev (/dev/input/event*).
//!
//! libinput слишком много требует (seat, udev) — для замены agetty на TTY
//! проще читать /dev/input/event* напрямую или /dev/tty через KDSKBMODE.
//!
//! Стратегия:
//!   1. Открываем /dev/tty (это наш управляющий терминал от systemd).
//!   2. Переключаем клавиатурный режим в K_RAW (или K_MEDIUMRAW) — читаем сканкоды.
//!   3. Преобразуем scancodes в keycodes через встроенную таблицу.
//!   4. Хоткеи SuperHot: Mod4 (Super) — префикс, как в i3.
//!
//! Альтернатива: открыть /dev/input/by-path/*-event-kbd напрямую. Это даёт
//! доступ к сканкодам без K_RAW. Используем это как fallback.

use anyhow::Result;
use std::os::unix::io::RawFd;
use std::collections::HashSet;

#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct InputEvent {
    pub tv_sec: i64,
    pub tv_usec: i64,
    pub typ: u16,
    pub code: u16,
    pub value: i32,
}

const EV_KEY: u16 = 1;

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub enum Key {
    Char(char),
    Backspace,
    Tab,
    Enter,
    Escape,
    Left, Right, Up, Down,
    Home, End, PageUp, PageDown,
    Insert, Delete,
    F1, F2, F3, F4, F5, F6, F7, F8, F9, F10, F11, F12,
    LeftShift, RightShift,
    LeftCtrl, RightCtrl,
    LeftAlt, RightAlt,
    LeftSuper, RightSuper,
    Space,
    Other(u16),
}

#[derive(Debug, Copy, Clone)]
#[allow(dead_code)] // Release(Key) produced but Key not read (release events ignored)
pub enum KeyEvent {
    Press(Key),
    Release(Key),
    Repeat(Key),
}

pub struct Keyboard {
    fd: RawFd,
    pressed: HashSet<u16>,
    /// Shift state.
    pub shift: bool,
    pub ctrl: bool,
    pub alt: bool,
    pub super_: bool,
    pub altgr: bool,
}

impl Keyboard {
    /// Открывает /dev/input/event* для клавиатуры.
    pub fn open() -> Result<Self> {
        let candidates = [
            "/dev/input/by-path/platform-i8042-serio-0-event-kbd",
            "/dev/input/event0",
            "/dev/input/event1",
            "/dev/input/event2",
            "/dev/input/event3",
        ];
        for path in &candidates {
            if let Ok(file) = std::fs::OpenOptions::new().read(true).write(true).open(path) {
                use std::os::unix::io::IntoRawFd;
                let fd = file.into_raw_fd();
                log::info!("keyboard opened: {}", path);
                return Keyboard::from_raw_fd(fd);
            }
        }
        anyhow::bail!("no keyboard device found in /dev/input/")
    }

    /// Wrap an already-opened raw fd. Used after fork in privilege separation:
    /// parent (root) opens the device, child (superhot-tty user) wraps the
    /// inherited fd. Applies EVIOCGRAB to prevent events leaking to the
    /// underlying TTY.
    pub fn from_raw_fd(fd: RawFd) -> Result<Self> {
        const EVIOCGRAB: libc::c_ulong = 0x40044590;
        let ret = unsafe { libc::ioctl(fd, EVIOCGRAB, 1) };
        if ret < 0 {
            log::warn!("EVIOCGRAB failed: {}", std::io::Error::last_os_error());
        }
        Ok(Keyboard {
            fd,
            pressed: HashSet::new(),
            shift: false, ctrl: false, alt: false, super_: false, altgr: false,
        })
    }

    /// Читает события (неблокирующе). Если данных нет — возвращает пустой вектор.
    pub fn poll(&mut self) -> Vec<KeyEvent> {
        let mut events = Vec::new();
        let mut buf = [0u8; std::mem::size_of::<InputEvent>() * 64];
        loop {
            let n = unsafe {
                libc::read(self.fd, buf.as_mut_ptr() as *mut _, buf.len())
            };
            if n <= 0 { break; }
            let cnt = (n as usize) / std::mem::size_of::<InputEvent>();
            let ptr = buf.as_ptr() as *const InputEvent;
            for i in 0..cnt {
                let ev = unsafe { ptr.add(i).read() };
                if ev.typ != EV_KEY { continue; }
                let key = keycode_to_key(ev.code);
                // Update modifiers.
                match key {
                    Key::LeftShift | Key::RightShift => self.shift = ev.value != 0,
                    Key::LeftCtrl | Key::RightCtrl   => self.ctrl = ev.value != 0,
                    Key::LeftAlt                      => self.alt = ev.value != 0,
                    Key::RightAlt                     => self.altgr = ev.value != 0,
                    Key::LeftSuper | Key::RightSuper  => self.super_ = ev.value != 0,
                    _ => {}
                }
                let ke = match ev.value {
                    0 => KeyEvent::Release(key),
                    1 => KeyEvent::Press(key),
                    2 => KeyEvent::Repeat(key),
                    _ => continue,
                };
                if ev.value != 0 { self.pressed.insert(ev.code); }
                else { self.pressed.remove(&ev.code); }
                events.push(ke);
            }
            if (n as usize) < buf.len() { break; }
        }
        events
    }

    #[allow(dead_code)] // utility for future key-combo detection
    pub fn is_pressed(&self, code: u16) -> bool {
        self.pressed.contains(&code)
    }
}

impl Drop for Keyboard {
    fn drop(&mut self) {
        unsafe {
            const EVIOCGRAB: libc::c_ulong = 0x40044590;
            libc::ioctl(self.fd, EVIOCGRAB, 0); // release grab
            libc::close(self.fd);
        }
    }
}

/// Преобразует Linux keycode (input-event-codes) в Key.
/// Reference: /usr/include/linux/input-event-codes.h
fn keycode_to_key(code: u16) -> Key {
    match code {
        1  => Key::Escape,
        14 => Key::Backspace,
        15 => Key::Tab,
        28 => Key::Enter,
        57 => Key::Space,
        103 => Key::Up,
        105 => Key::Left,
        106 => Key::Right,
        108 => Key::Down,
        102 => Key::Home,
        107 => Key::End,
        104 => Key::PageUp,
        109 => Key::PageDown,
        110 => Key::Insert,
        111 => Key::Delete,
        59 => Key::F1, 60 => Key::F2, 61 => Key::F3, 62 => Key::F4,
        63 => Key::F5, 64 => Key::F6, 65 => Key::F7, 66 => Key::F8,
        67 => Key::F9, 68 => Key::F10, 87 => Key::F11, 88 => Key::F12,
        29 => Key::LeftCtrl,
        97 => Key::RightCtrl,
        42 => Key::LeftShift,
        54 => Key::RightShift,
        56 => Key::LeftAlt,
        100 => Key::RightAlt,
        125 => Key::LeftSuper,
        126 => Key::RightSuper,
        // Letters.
        2..=11 => Key::Char(((b'1' + (code - 2) as u8)) as char), // 1-9, 0
        16..=25 => Key::Char(((b'q' + (code - 16) as u8)) as char), // q-p
        30..=38 => Key::Char(((b'a' + (code - 30) as u8)) as char), // a-l
        44..=50 => Key::Char(((b'z' + (code - 44) as u8)) as char), // z-m
        _ => Key::Other(code),
    }
}

impl Key {
    pub fn as_char(&self, shift: bool) -> Option<char> {
        match self {
            Key::Char(c) => {
                let c = *c;
                if shift {
                    // Shift map for letters & digits & symbols.
                    Some(match c {
                        'a'..='z' => ((c as u8) - 32) as char,
                        '1' => '!', '2' => '@', '3' => '#', '4' => '$', '5' => '%',
                        '6' => '^', '7' => '&', '8' => '*', '9' => '(', '0' => ')',
                        '-' => '_', '=' => '+', '[' => '{', ']' => '}',
                        '\\' => '|', ';' => ':', '\'' => '"',
                        ',' => '<', '.' => '>', '/' => '?',
                        '`' => '~', _ => c,
                    })
                } else {
                    Some(c)
                }
            }
            Key::Space => Some(' '),
            Key::Enter => Some('\n'),
            Key::Tab => Some('\t'),
            Key::Backspace => Some('\x08'),
            _ => None,
        }
    }
}
