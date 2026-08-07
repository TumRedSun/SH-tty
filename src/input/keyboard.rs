//! Чтение клавиатуры через raw evdev (/dev/input/event\*).
//!
//! Device detection: перебираем /dev/input/event\* и проверяем через
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
// architectures we target. libc crate doesn't expose EVIOC\* constants.
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

/// Full key event with raw evdev keycode, for callers that need the
/// underlying Linux keycode (e.g. to forward to X11 via XTest, where
/// the X keycode = evdev keycode + 8).
#[derive(Debug, Copy, Clone)]
pub struct RawKeyEvent {
    pub event: KeyEvent,
    /// Raw evdev keycode (linux/input-event-codes.h). Available for
    /// forwarding to other input systems (XTest, uinput, etc.).
    pub keycode: u16,
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
    pub fn open() -> Result<Self> {
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
                            log::debug!("by-path {}: not a keyboard, skipping", entry.display());
                            unsafe { libc::close(fd); }
                        }
                    }
                }
            }
        }

        let by_id_patterns = [
            "/dev/input/by-id/*-kbd",
            "/dev/input/by-id/*-event-kbd",
        ];
        for pattern in &by_id_patterns {
            if let Ok(entries) = glob::glob(pattern) {
                for entry in entries.flatten() {
                    if let Some(path_str) = entry.to_str() {
                        if let Ok(fd) = open_device_rw(path_str) {
                            if is_keyboard(fd) {
                                log::info!("keyboard opened (by-id): {}", entry.display());
                                return Keyboard::from_raw_fd(fd);
                            }
                            log::debug!("by-id {}: not a keyboard, skipping", entry.display());
                            unsafe { libc::close(fd); }
                        }
                    }
                }
            }
        }

        for n in 0..=127u32 {
            let path = format!("/dev/input/event{}", n);
            if let Ok(fd) = open_device_rw(&path) {
                if is_keyboard(fd) {
                    log::info!("keyboard opened (event{}): {}", n, path);
                    return Keyboard::from_raw_fd(fd);
                }
                log::debug!("event{}: not a keyboard, skipping", n);
                unsafe { libc::close(fd); }
            }
        }
        anyhow::bail!("no keyboard device found in /dev/input/ (checked event0..event127)")
    }

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

    pub fn poll(&mut self) -> Vec<KeyEvent> {
        self.poll_with_keycodes().into_iter().map(|r| r.event).collect()
    }

    /// То же что poll(), но также возвращает raw evdev keycode для каждого
    /// события. Нужен для X11 keyboard forwarding через XTest (X keycode =
    /// evdev keycode + 8). WM использует этот метод если активный tile —
    /// X11; для terminal tiles достаточно обычного poll().
    pub fn poll_with_keycodes(&mut self) -> Vec<RawKeyEvent> {
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
                    Key::LeftCtrl | Key::RightCtrl => self.ctrl = ev.value != 0,
                    Key::LeftAlt => self.alt = ev.value != 0,
                    Key::RightAlt => self.altgr = ev.value != 0,
                    Key::LeftSuper | Key::RightSuper => self.super_ = ev.value != 0,
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
                events.push(RawKeyEvent { event: ke, keycode: ev.code });
            }
            if (n as usize) < buf.len() { break; }
        }
        events
    }

    #[allow(dead_code)]
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

fn open_device_rw(path: &str) -> std::io::Result<RawFd> {
    let c_path = std::ffi::CString::new(path)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e))?;
    // O_NONBLOCK критичен — без него read() на evdev fd блокирует до
    // появления события. Раньше мы открывали без O_NONBLOCK, и main loop
    // висел в keyboard.poll() до нажатия любой клавиши — из-за этого экран
    // обновлялся только когда пользователь что-то нажимал.
    // (O_RDWR нужен чтобы EVIOCGRAB и LED events работали.)
    let fd = unsafe { libc::open(c_path.as_ptr(), libc::O_RDWR | libc::O_CLOEXEC | libc::O_NONBLOCK) };
    if fd >= 0 { return Ok(fd); }
    // Fallback на read-only если нет прав на запись (некоторые системы
    // дают только read). С NONBLOCK это всё ещё безопасно — read() вернёт
    // EAGAIN вместо блокировки.
    let fd = unsafe { libc::open(c_path.as_ptr(), libc::O_RDONLY | libc::O_CLOEXEC | libc::O_NONBLOCK) };
    if fd < 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(fd)
}

fn is_keyboard(fd: RawFd) -> bool {
    let mut ev_bits = [0u8; 4];
    let ret = unsafe {
        libc::ioctl(fd, eviocgbit(0 /* EV_SYN */, ev_bits.len() as u32), ev_bits.as_mut_ptr())
    };
    if ret < 0 {
        log::debug!("EVIOCGBIT(EV) ioctl failed: {}", std::io::Error::last_os_error());
        return false;
    }
    if (ev_bits[0] & (1 << EV_KEY)) == 0 {
        log::debug!("device does not support EV_KEY, skipping");
        return false;
    }

    let mut key_bits = [0u8; 96];
    let ret = unsafe {
        libc::ioctl(fd, eviocgbit(EV_KEY as u32, key_bits.len() as u32), key_bits.as_mut_ptr())
    };
    if ret < 0 {
        log::debug!("EVIOCGBIT(KEY) ioctl failed: {}", std::io::Error::last_os_error());
        return false;
    }

    let has = |code: u16| -> bool {
        let byte_idx = (code / 8) as usize;
        let bit_idx = (code % 8) as u8;
        if byte_idx >= key_bits.len() { return false; }
        (key_bits[byte_idx] >> bit_idx) & 1 == 1
    };

    let required = [KEY_ENTER, KEY_LEFTSHIFT, KEY_SPACE, KEY_ESC, KEY_A];
    let count = required.iter().filter(|&&k| has(k)).count();
    if count < 3 {
        log::debug!("device has EV_KEY but only {}/5 keyboard keys, skipping", count);
    }
    count >= 3
}

fn keycode_to_key(code: u16) -> Key {
    match code {
        1 => Key::Escape,
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
        2..=11 => Key::Char(((b'1' + (code - 2) as u8)) as char),
        12 => Key::Char('-'), 13 => Key::Char('='),
        16 => Key::Char('q'), 17 => Key::Char('w'), 18 => Key::Char('e'),
        19 => Key::Char('r'), 20 => Key::Char('t'), 21 => Key::Char('y'),
        22 => Key::Char('u'), 23 => Key::Char('i'), 24 => Key::Char('o'),
        25 => Key::Char('p'),
        26 => Key::Char('['), 27 => Key::Char(']'),
        30 => Key::Char('a'), 31 => Key::Char('s'), 32 => Key::Char('d'),
        33 => Key::Char('f'), 34 => Key::Char('g'), 35 => Key::Char('h'),
        36 => Key::Char('j'), 37 => Key::Char('k'), 38 => Key::Char('l'),
        39 => Key::Char(';'), 40 => Key::Char('\''),
        41 => Key::Char('`'),
        43 => Key::Char('\\'),
        44 => Key::Char('z'), 45 => Key::Char('x'), 46 => Key::Char('c'),
        47 => Key::Char('v'), 48 => Key::Char('b'), 49 => Key::Char('n'),
        50 => Key::Char('m'),
        51 => Key::Char(','), 52 => Key::Char('.'), 53 => Key::Char('/'),
        // Numpad keys with NumLock ON (produce digits/operators).
        // Without NumLock they map to navigation keys (Home/End/Arrows/PgUp/PgDn/Insert/Delete).
        // We send them as chars by default; if application keypad mode is on, the
        // application should handle them appropriately. This at least makes the
        // numpad usable for typing numbers (was previously dead — Key::Other).
        71 => Key::Char('7'),  // KP7
        72 => Key::Char('8'),  // KP8
        73 => Key::Char('9'),  // KP9
        74 => Key::Char('-'),  // KP_MINUS
        75 => Key::Char('4'),  // KP4
        76 => Key::Char('5'),  // KP5
        77 => Key::Char('6'),  // KP6
        78 => Key::Char('+'),  // KP_PLUS
        79 => Key::Char('1'),  // KP1
        80 => Key::Char('2'),  // KP2
        81 => Key::Char('3'),  // KP3
        82 => Key::Char('0'),  // KP0
        83 => Key::Char('.'),  // KP_DOT
        96 => Key::Enter,      // KP_ENTER — behaves like Enter
        // CapsLock / NumLock / ScrollLock — modifier toggles, no char output.
        58 | 69 | 70 => Key::Other(code),
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
            Key::Enter => Some('\r'),
            Key::Tab => Some('\t'),
            Key::Backspace => Some('\x7f'),
            _ => None,
        }
    }

    /// Возвращает ANSI escape-последовательность для специальных клавиш
    /// (стрелки, F1-F12, Home/End, PageUp/Down, Insert/Delete, Escape, Backspace).
    ///
    /// Учитывает режим cursor keys (DECCKM): если `app_cursor_keys = true`
    /// (zsh/vim включил CSI ?1h), стрелки отправляются как `ESC O A` вместо
    /// `ESC [ A`. Без этого zsh line editor (и многие TUI) не понимают стрелки.
    ///
    /// Учитывает модификаторы (Shift/Ctrl/Alt) — добавляет modifier parameter
    /// (например Shift+Up = `ESC [ 1 ; 2 A`). Это нужно для правильной работы
    /// в современных терминалах, где Shift+Arrow прокручивает scrollback и т.д.
    ///
    /// Возвращает None для обычных печатных символов — caller должен использовать
    /// as_char() для них.
    pub fn to_pty_bytes(&self, app_cursor_keys: bool, shift: bool, ctrl: bool, alt: bool) -> Option<Vec<u8>> {
        // Modifier parameter for xterm-style modifier-aware sequences.
        // 1 = none, 2 = shift, 3 = alt, 4 = shift+alt, 5 = ctrl,
        // 6 = shift+ctrl, 7 = alt+ctrl, 8 = shift+alt+ctrl.
        let mod_param: u8 = {
            let mut m = 1u8;
            if shift { m += 1; }
            if alt   { m += 2; }
            if ctrl  { m += 4; }
            m
        };
        let has_modifier = mod_param != 1;

        // Helper: builds a CSI sequence respecting modifier + cursor mode.
        // normal_seq = e.g. "A" (Up) — produces "\x1B[A" or "\x1B[1;2A" (with mods)
        // app_seq    = e.g. "A" (Up) — produces "\x1BOA" (DECCKM mode, no mods only)
        let build_csi = |normal_final: &str, app_final: &str| -> Vec<u8> {
            if has_modifier {
                // xterm modifier-aware: ESC [ 1 ; <mod> <final>
                format!("\x1B[1;{}{}", mod_param, normal_final).into_bytes()
            } else if app_cursor_keys {
                format!("\x1BO{}", app_final).into_bytes()
            } else {
                format!("\x1B[{}", normal_final).into_bytes()
            }
        };

        match self {
            // Arrow keys — respect DECCKM.
            Key::Up    => Some(build_csi("A", "A")),
            Key::Down  => Some(build_csi("B", "B")),
            Key::Right => Some(build_csi("C", "C")),
            Key::Left  => Some(build_csi("D", "D")),
            // Home/End — respect DECCKM too (DECCKM covers Home/End in some terminals).
            Key::Home  => Some(build_csi("H", "H")),
            Key::End   => Some(build_csi("F", "F")),
            // F1-F4 — xterm uses ESC O P/Q/R/S in normal mode, ESC [ 1 ; <mod> P.. in mod mode.
            Key::F1 => {
                Some(if has_modifier { format!("\x1B[1;{}P", mod_param).into_bytes() }
                     else { b"\x1BOP".to_vec() })
            }
            Key::F2 => {
                Some(if has_modifier { format!("\x1B[1;{}Q", mod_param).into_bytes() }
                     else { b"\x1BOQ".to_vec() })
            }
            Key::F3 => {
                Some(if has_modifier { format!("\x1B[1;{}R", mod_param).into_bytes() }
                     else { b"\x1BOR".to_vec() })
            }
            Key::F4 => {
                Some(if has_modifier { format!("\x1B[1;{}S", mod_param).into_bytes() }
                     else { b"\x1BOS".to_vec() })
            }
            // F5-F12 — always CSI, with optional modifier.
            Key::F5  => Some(if has_modifier { format!("\x1B[15;{}~", mod_param).into_bytes() }
                             else { b"\x1B[15~".to_vec() }),
            Key::F6  => Some(if has_modifier { format!("\x1B[17;{}~", mod_param).into_bytes() }
                             else { b"\x1B[17~".to_vec() }),
            Key::F7  => Some(if has_modifier { format!("\x1B[18;{}~", mod_param).into_bytes() }
                             else { b"\x1B[18~".to_vec() }),
            Key::F8  => Some(if has_modifier { format!("\x1B[19;{}~", mod_param).into_bytes() }
                             else { b"\x1B[19~".to_vec() }),
            Key::F9  => Some(if has_modifier { format!("\x1B[20;{}~", mod_param).into_bytes() }
                             else { b"\x1B[20~".to_vec() }),
            Key::F10 => Some(if has_modifier { format!("\x1B[21;{}~", mod_param).into_bytes() }
                             else { b"\x1B[21~".to_vec() }),
            Key::F11 => Some(if has_modifier { format!("\x1B[23;{}~", mod_param).into_bytes() }
                             else { b"\x1B[23~".to_vec() }),
            Key::F12 => Some(if has_modifier { format!("\x1B[24;{}~", mod_param).into_bytes() }
                             else { b"\x1B[24~".to_vec() }),
            // PageUp/PageDown/Insert/Delete — CSI with tilde, mod-aware.
            Key::PageUp    => Some(if has_modifier { format!("\x1B[5;{}~", mod_param).into_bytes() }
                                   else { b"\x1B[5~".to_vec() }),
            Key::PageDown  => Some(if has_modifier { format!("\x1B[6;{}~", mod_param).into_bytes() }
                                   else { b"\x1B[6~".to_vec() }),
            Key::Insert    => Some(if has_modifier { format!("\x1B[2;{}~", mod_param).into_bytes() }
                                   else { b"\x1B[2~".to_vec() }),
            Key::Delete    => Some(if has_modifier { format!("\x1B[3;{}~", mod_param).into_bytes() }
                                   else { b"\x1B[3~".to_vec() }),
            // Escape — single byte. With Alt modifier, prefix other chars with ESC,
            // but for the Escape key itself we just send ESC.
            Key::Escape    => Some(b"\x1B".to_vec()),
            // Backspace — most modern shells expect DEL (0x7F), not BS (0x08).
            // xterm sends DEL by default; VT220 with DECBKM sends BS.
            // We use DEL which is what bash/zsh/readline expect.
            Key::Backspace => Some(b"\x7f".to_vec()),
            // Tab — Ctrl+I is the same byte, but explicit Tab sends HT.
            Key::Tab       => Some(b"\t".to_vec()),
            // Enter — CR (\r). Most shells accept this; LF is also OK but CR is standard.
            Key::Enter     => Some(b"\r".to_vec()),
            // Space — explicit Space key sends SP.
            Key::Space     => Some(b" ".to_vec()),
            // Printable chars — caller should use as_char().
            // But we handle Alt+<printable> by prefixing with ESC (meta key, xterm-style).
            Key::Char(c) => {
                if alt {
                    let mut buf = [0u8; 4];
                    let s = c.encode_utf8(&mut buf);
                    let mut v = Vec::with_capacity(s.len() + 1);
                    v.push(0x1B); // ESC prefix for meta
                    v.extend_from_slice(s.as_bytes());
                    Some(v)
                } else {
                    None
                }
            }
            // Unknown key — no sequence.
            Key::Other(_) => None,
            // Modifier keys alone — no sequence.
            _ => None,
        }
    }
}
