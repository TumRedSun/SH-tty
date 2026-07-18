//! Чтение клавиатуры через raw evdev (/dev/input/event*).
//!
//! Device detection: перебираем /dev/input/event* и проверяем через
//! EVIOCGBIT(EV_KEY, ...) что устройство поддерживает key events,
//! плюс наличие типичных keyboard keys (KEY_ENTER, KEY_LEFTSHIFT, KEY_SPACE,
//! KEY_ESC, KEY_A) в bitmap — это отличает клавиатуру от power button,
//! ACPI кнопок, joystick'ов.

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

// evdev event types (linux/input-event-codes.h)
const EV_KEY: u16 = 1;
#[allow(dead_code)] // EV_SYN referenced in eviocgbit call as 0
const EV_SYN: u16 = 0;

// ioctl numbers for evdev. x86_64 encoding, matches libc on all Linux
// architectures we target. libc crate doesn't expose EVIOC* constants.
// EVIOCGBIT(ev, len) = _IOR('E', 0x20 + ev, u8[len])
// _IOR(dir=2, type='E'=0x45, nr, size) = (2<<30) | (size<<16) | (0x45<<8) | nr
const fn eviocgbit(ev: u32, len: u32) -> libc::c_ulong {
    ((2u32 << 30) | (len << 16) | (0x45 << 8) | (0x20 + ev)) as libc::c_ulong
}
// EVIOCGRAB = _IOW('E', 0x90, int) = (1<<30) | (4<<16) | (0x45<<8) | 0x90
const EVIOCGRAB: libc::c_ulong = ((1u32 << 30) | (4 << 16) | (0x45 << 8) | 0x90) as libc::c_ulong;

// Key codes we check to identify a keyboard (must have most of these).
const KEY_ENTER: u16 = 28;
const KEY_LEFTSHIFT: u16 = 42;
const KEY_SPACE: u16 = 57;
const KEY_ESC: u16 = 1;
const KEY_A: u16 = 30;

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
#[allow(dead_code)] // Release(Key) produced but Key value not read
pub enum KeyEvent {
    Press(Key),
    Release(Key),
    Repeat(Key),
}

pub struct Keyboard {
    fd: RawFd,
    pressed: HashSet<u16>,
    pub shift: bool,
    pub ctrl: bool,
    pub alt: bool,
    pub super_: bool,
    pub altgr: bool,
}

impl Keyboard {
    /// Открывает клавиатуру через /dev/input/event*.
    ///
    /// Использует EVIOCGBIT для проверки что устройство реально поддерживает
    /// key events и имеет типичные клавиатурные keys. Перебираем все
    /// event* устройства — не полагаемся на by-path symlinks (они зависят
    /// от platform driver: i8042 для PS/2, usb для USB клавиатур, etc).
    pub fn open() -> Result<Self> {
        // Сначала пробуем by-path symlinks (быстрее, точнее).
        let by_path_patterns = [
            "/dev/input/by-path/platform-i8042-serio-0-event-kbd",
            "/dev/input/by-path/*-event-kbd",
        ];
        for pattern in &by_path_patterns {
            if let Ok(entries) = glob::glob(pattern) {
                for entry in entries.flatten() {
                    if let Some(path_str) = entry.to_str() {
                        if let Ok(fd) = open_device_rw(path_str) {
                            if is_keyboard(fd) {
                                log::info!("keyboard opened (by-path): {}", entry.display());
                                return Keyboard::from_raw_fd(fd);
                            }
                            unsafe { libc::close(fd); }
                        }
                    }
                }
            }
        }

        // Fallback: перебираем все /dev/input/event* с EVIOCGBIT проверкой.
        for n in 0..=63u32 {
            let path = format!("/dev/input/event{}", n);
            if let Ok(fd) = open_device_rw(&path) {
                if is_keyboard(fd) {
                    log::info!("keyboard opened (event{}): {}", n, path);
                    return Keyboard::from_raw_fd(fd);
                }
                unsafe { libc::close(fd); }
            }
        }
        anyhow::bail!("no keyboard device found in /dev/input/ (checked event0..event63)")
    }

    /// Wrap an already-opened raw fd. Used after fork in privilege separation:
    /// parent (root) opens the device, child (superhot-tty user) wraps the
    /// inherited fd. Applies EVIOCGRAB to prevent events leaking to TTY.
    pub fn from_raw_fd(fd: RawFd) -> Result<Self> {
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
            libc::ioctl(self.fd, EVIOCGRAB, 0);
            libc::close(self.fd);
        }
    }
}

/// Открывает устройство на чтение/запись, возвращает fd или io::Error.
fn open_device_rw(path: &str) -> std::io::Result<RawFd> {
    let c_path = std::ffi::CString::new(path)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e))?;
    let fd = unsafe { libc::open(c_path.as_ptr(), libc::O_RDWR | libc::O_CLOEXEC) };
    if fd < 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(fd)
}

/// Проверяет, является ли устройство клавиатурой через EVIOCGBIT.
///
/// Устройство считается клавиатурой если:
/// 1. Поддерживает EV_KEY (даёт key events)
/// 2. Имеет в bitmap'е ключевые клавиши: KEY_ENTER, KEY_LEFTSHIFT, KEY_SPACE,
///    KEY_ESC, KEY_A. Это отсекает power button, ACPI кнопки, joystick'и.
fn is_keyboard(fd: RawFd) -> bool {
    // Получаем bitmap поддерживаемых event types (EV_SYN..EV_MAX).
    let mut ev_bits = [0u8; 4];
    let ret = unsafe {
        libc::ioctl(fd, eviocgbit(0 /* EV_SYN */, ev_bits.len() as u32), ev_bits.as_mut_ptr())
    };
    if ret < 0 { return false; }
    // Проверяем что EV_KEY (bit 1) установлен.
    if (ev_bits[0] & (1 << EV_KEY)) == 0 { return false; }

    // Получаем bitmap поддерживаемых key codes. KEY_MAX = 0x2ff,
    // нужен bitmap на 768 bits = 96 байт.
    let mut key_bits = [0u8; 96];
    let ret = unsafe {
        libc::ioctl(fd, eviocgbit(EV_KEY as u32, key_bits.len() as u32), key_bits.as_mut_ptr())
    };
    if ret < 0 { return false; }

    let has = |code: u16| -> bool {
        let byte_idx = (code / 8) as usize;
        let bit_idx = (code % 8) as u8;
        if byte_idx >= key_bits.len() { return false; }
        (key_bits[byte_idx] >> bit_idx) & 1 == 1
    };

    // Устройство — клавиатура если есть хотя бы 4 из 5 ключевых клавиш.
    let required = [KEY_ENTER, KEY_LEFTSHIFT, KEY_SPACE, KEY_ESC, KEY_A];
    let count = required.iter().filter(|&&k| has(k)).count();
    count >= 4
}

/// Преобразует Linux keycode (input-event-codes) в Key.
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
        2..=11 => Key::Char(((b'1' + (code - 2) as u8)) as char), // 1-9, 0
        // Number row symbols (Shift)
        12 => Key::Char('-'), 13 => Key::Char('='),
        // QWERTY row 1: Q W E R T Y U I O P (keycodes 16-25)
        16 => Key::Char('q'), 17 => Key::Char('w'), 18 => Key::Char('e'),
        19 => Key::Char('r'), 20 => Key::Char('t'), 21 => Key::Char('y'),
        22 => Key::Char('u'), 23 => Key::Char('i'), 24 => Key::Char('o'),
        25 => Key::Char('p'),
        26 => Key::Char('['), 27 => Key::Char(']'),
        // QWERTY row 2: A S D F G H J K L (keycodes 30-38)
        30 => Key::Char('a'), 31 => Key::Char('s'), 32 => Key::Char('d'),
        33 => Key::Char('f'), 34 => Key::Char('g'), 35 => Key::Char('h'),
        36 => Key::Char('j'), 37 => Key::Char('k'), 38 => Key::Char('l'),
        39 => Key::Char(';'), 40 => Key::Char('\''),
        41 => Key::Char('`'),
        // Left Shift is 42 (already mapped above)
        43 => Key::Char('\\'),
        // QWERTY row 3: Z X C V B N M (keycodes 44-50)
        44 => Key::Char('z'), 45 => Key::Char('x'), 46 => Key::Char('c'),
        47 => Key::Char('v'), 48 => Key::Char('b'), 49 => Key::Char('n'),
        50 => Key::Char('m'),
        51 => Key::Char(','), 52 => Key::Char('.'), 53 => Key::Char('/'),
        _ => Key::Other(code),
    }
}

impl Key {
    pub fn as_char(&self, shift: bool) -> Option<char> {
        match self {
            Key::Char(c) => {
                let c = *c;
                if shift {
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
