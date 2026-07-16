//! libvterm FFI bindings (runtime loading через libloading).
//!
//! libvterm — это C-библиотека, реализующая полноценный xterm-совместимый
//! terminal state machine. Мы используем её для:
//!   - Полной поддержки ANSI escape sequences (включая редко используемые)
//!   - SGR extended colors (256 + truecolor)
//!   - Cursor styles, blink, save/restore
//!   - DEC special modes (insert, origin, autowrap, etc.)
//!   - OSC sequences (palette, clipboard, hyperlink)
//!   - Mouse tracking (SGR mouse, urxvt mouse)
//!
//! Если libvterm.so.0 недоступна — fallback на встроенный minimal VTerm
//! (src/term/vterm.rs). Это позволяет WM работать на системах без libvterm-dev.
//!
//! Архитектура: VTerm (в vterm.rs) при feed() если libvterm доступна —
//! проксирует туда байты и затем копирует сетку через read_grid. Если
//! недоступна — использует встроенный обработчик CSI/OSC.

use crate::term::vterm::Cell;
use crate::ui::theme::ANSI_PALETTE;
use libloading::Library;
use std::os::raw::{c_char, c_int, c_void};
use std::sync::OnceLock;

/// Maximum chars per cell (UTF-8 multi-byte sequences).
const VTERM_MAX_CHARS_PER_CELL: usize = 16;

#[repr(C)]
#[derive(Default, Clone, Copy)]
pub struct VTermColor {
    pub red: u8,
    pub green: u8,
    pub blue: u8,
}

#[repr(C)]
#[derive(Default, Clone, Copy)]
pub struct VTermScreenCellAttrs {
    pub bold: u8,
    pub underline: u8,
    pub italic: u8,
    pub blink: u8,
    pub reverse: u8,
    pub conceal: u8,
    pub strike: u8,
    pub font: u8,
    pub dwl: u8,
    pub dhl: u8,
    pub small: u8,
    pub baseline: u8,
    pub strikethrough: u8,
    pub _padding: [u8; 2],
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct VTermScreenCell {
    pub chars: [u32; VTERM_MAX_CHARS_PER_CELL],
    pub width: u8,
    pub attrs: VTermScreenCellAttrs,
    pub fg: VTermColor,
    pub bg: VTermColor,
}

impl Default for VTermScreenCell {
    fn default() -> Self {
        VTermScreenCell {
            chars: [0; VTERM_MAX_CHARS_PER_CELL],
            width: 1,
            attrs: VTermScreenCellAttrs::default(),
            fg: VTermColor::default(),
            bg: VTermColor::default(),
        }
    }
}

/// Глобальный singleton: загруженная libvterm и указатели на функции.
struct LibVTerm {
    _lib: Library,
    vterm_new: unsafe extern "C" fn(rows: c_int, cols: c_int) -> *mut c_void,
    vterm_free: unsafe extern "C" fn(vt: *mut c_void),
    vterm_input_write: unsafe extern "C" fn(vt: *mut c_void, buf: *const c_char, len: usize) -> usize,
    vterm_obtain_screen: unsafe extern "C" fn(vt: *mut c_void) -> *mut c_void,
    vterm_screen_reset: unsafe extern "C" fn(screen: *mut c_void, hard: c_int),
    vterm_screen_get_cell: unsafe extern "C" fn(screen: *mut c_void, row: c_int, col: c_int, cell: *mut VTermScreenCell) -> c_int,
    vterm_state_get_cursorpos: unsafe extern "C" fn(vt: *mut c_void, pos: *mut VTermPos),
}

#[repr(C)]
struct VTermPos {
    row: c_int,
    col: c_int,
}

static LIBVTERM: OnceLock<Option<LibVTerm>> = OnceLock::new();

fn load_libvterm() -> Option<&'static LibVTerm> {
    LIBVTERM.get_or_init(|| {
        let lib = unsafe {
            Library::new("libvterm.so.0").ok()
                .or_else(|| Library::new("libvterm.so").ok())
        };
        let lib = lib?;
        unsafe {
            let vterm_new: unsafe extern "C" fn(c_int, c_int) -> *mut c_void =
                *lib.get(b"vterm_new\0").ok()?;
            let vterm_free: unsafe extern "C" fn(*mut c_void) =
                *lib.get(b"vterm_free\0").ok()?;
            let vterm_input_write: unsafe extern "C" fn(*mut c_void, *const c_char, usize) -> usize =
                *lib.get(b"vterm_input_write\0").ok()?;
            let vterm_obtain_screen: unsafe extern "C" fn(*mut c_void) -> *mut c_void =
                *lib.get(b"vterm_obtain_screen\0").ok()?;
            let vterm_screen_reset: unsafe extern "C" fn(*mut c_void, c_int) =
                *lib.get(b"vterm_screen_reset\0").ok()?;
            let vterm_screen_get_cell: unsafe extern "C" fn(*mut c_void, c_int, c_int, *mut VTermScreenCell) -> c_int =
                *lib.get(b"vterm_screen_get_cell\0").ok()?;
            let vterm_state_get_cursorpos: unsafe extern "C" fn(*mut c_void, *mut VTermPos) =
                *lib.get(b"vterm_state_get_cursorpos\0").ok()?;
            Some(LibVTerm {
                _lib: lib,
                vterm_new,
                vterm_free,
                vterm_input_write,
                vterm_obtain_screen,
                vterm_screen_reset,
                vterm_screen_get_cell,
                vterm_state_get_cursorpos,
            })
        }
    }).as_ref()
}

/// Проверяет, доступна ли libvterm в системе.
pub fn available() -> bool {
    load_libvterm().is_some()
}

/// Handle на libvterm instance.
pub struct LibVTermHandle {
    vt: *mut c_void,
    screen: *mut c_void,
}

unsafe impl Send for LibVTermHandle {}

impl LibVTermHandle {
    /// Создаёт новый libvterm instance. Возвращает None если libvterm недоступна.
    pub fn new(cols: u16, rows: u16) -> Option<Self> {
        let lib = load_libvterm()?;
        unsafe {
            let vt = (lib.vterm_new)(rows as c_int, cols as c_int);
            if vt.is_null() { return None; }
            let screen = (lib.vterm_obtain_screen)(vt);
            if screen.is_null() {
                (lib.vterm_free)(vt);
                return None;
            }
            (lib.vterm_screen_reset)(screen, 1);
            Some(LibVTermHandle { vt, screen })
        }
    }

    /// Записывает данные в libvterm. Возвращает количество обработанных байт.
    pub fn feed(&mut self, data: &[u8]) -> usize {
        let lib = match load_libvterm() { Some(l) => l, None => return 0 };
        unsafe {
            (lib.vterm_input_write)(self.vt, data.as_ptr() as *const c_char, data.len())
        }
    }

    /// Возвращает позицию курсора (col, row).
    pub fn cursor_pos(&self) -> (u16, u16) {
        let lib = match load_libvterm() { Some(l) => l, None => return (0, 0) };
        let mut pos = VTermPos { row: 0, col: 0 };
        unsafe {
            (lib.vterm_state_get_cursorpos)(self.vt, &mut pos);
        }
        (pos.col.max(0) as u16, pos.row.max(0) as u16)
    }

    /// Читает всю сетку ячеек и маппит в наш Cell формат.
    pub fn read_grid(&self, out: &mut [Cell], cols: u16, rows: u16) {
        let lib = match load_libvterm() { Some(l) => l, None => return };
        let mut cell = VTermScreenCell::default();
        for r in 0..rows as c_int {
            for c in 0..cols as c_int {
                unsafe {
                    if (lib.vterm_screen_get_cell)(self.screen, r, c, &mut cell) == 0 {
                        continue;
                    }
                }
                let idx = (r as usize) * (cols as usize) + (c as usize);
                if idx >= out.len() { break; }
                let ch = if cell.chars[0] != 0 {
                    char::from_u32(cell.chars[0]).unwrap_or(' ')
                } else { ' ' };
                out[idx] = Cell {
                    ch,
                    fg: color_to_index(&cell.fg, true, cell.attrs.reverse != 0),
                    bg: color_to_index(&cell.bg, false, cell.attrs.reverse != 0),
                    bold: cell.attrs.bold != 0,
                    underline: cell.attrs.underline != 0,
                };
            }
        }
    }
}

impl Drop for LibVTermHandle {
    fn drop(&mut self) {
        let lib = match load_libvterm() { Some(l) => l, None => return };
        unsafe { (lib.vterm_free)(self.vt); }
    }
}

/// Маппит VTermColor → индекс в ANSI_PALETTE (0-15) или 255 (default).
fn color_to_index(c: &VTermColor, _is_fg: bool, _reversed: bool) -> u8 {
    if c.red == 0 && c.green == 0 && c.blue == 0 {
        return 0;
    }
    let mut best_idx = 255u8;
    let mut best_dist = u32::MAX;
    for (i, p) in ANSI_PALETTE.iter().enumerate() {
        let dr = c.red as i32 - p.0 as i32;
        let dg = c.green as i32 - p.1 as i32;
        let db = c.blue as i32 - p.2 as i32;
        let d = (dr * dr + dg * dg + db * db) as u32;
        if d < best_dist {
            best_dist = d;
            best_idx = i as u8;
        }
    }
    best_idx
}
