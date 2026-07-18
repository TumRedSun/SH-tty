//! Минимальная terminal state machine с опциональной libvterm backend.
//!
//! Если libvterm.so.0 доступна в системе — VTerm использует её для полноценной
//! xterm-совместимости (полный набор CSI/OSC/DCS sequences, 256/truecolor,
//! cursor styles, mouse tracking, DEC modes). Иначе — fallback на встроенный
//! обработчик базовых ANSI sequences.
//!
//! Поддерживаемые escape sequences (fallback):
//!   - CSI n;m H    — cursor position
//!   - CSI n A/B/C/D — cursor up/down/fwd/back
//!   - CSI n E/F    — cursor next/prev line
//!   - CSI n G/d    — column / row set
//!   - CSI n J      — erase display
//!   - CSI n K      — erase line
//!   - CSI n m      — SGR (colors, bold, italic, underline, reverse, etc.)
//!   - CSI 6n       — device status report (cursor position)
//!   - CSI n L/M    — insert/delete lines
//!   - CSI n P/@    — delete chars / insert blanks
//!   - CSI n S/T    — scroll up/down
//!   - ESC [ ...    — handled
//!   - BEL, BS, HT, LF, VT, FF, CR
//!   - DECSTBM (scroll region)
//!   - DECSET/DECRST ?1049 (alt screen), ?25 (cursor visibility)
//!   - ?7 (autowrap), ?1 (cursor keys), ?12 (cursor blink)
//!   - OSC 0;title / 2;title — window title
//!   - OSC 4;N;COLOR — palette set
//!   - OSC 8;params;uri — hyperlink (упрощённо, игнорируем)
//!   - CSI > 4 n / CSI > 4 ; 2 n  — modify keyboard (упрощённо)
//!   - CSI s / CSI u — save/restore cursor
//!   - ESC 7 / ESC 8 — DECSC/DECRC (save/restore cursor)
//!   - ESC = / ESC > — application keypad
//!   - ESC D / ESC M — IND / RI
//!   - ESC E       — NEL
//!   - ESC c       — RIS (reset)

use crate::ui::theme::{Color, ANSI_PALETTE};

#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct Cell {
    pub ch: char,
    pub fg: u8,   // index into ANSI_PALETTE (0-15), or 255 = default
    pub bg: u8,
    pub bold: bool,
    pub underline: bool,
}

impl Cell {
    pub fn blank() -> Self {
        Cell { ch: ' ', fg: 255, bg: 0, bold: false, underline: false }
    }
    pub fn fg_color(&self) -> Color {
        if self.fg == 255 { Color(0xE6, 0xE1, 0xF0) }
        else if (self.fg as usize) < ANSI_PALETTE.len() { ANSI_PALETTE[self.fg as usize] }
        else { Color(0xE6, 0xE1, 0xF0) }
    }
    pub fn bg_color(&self) -> Color {
        if self.bg == 255 { Color(0x0F, 0x0A, 0x1E) }
        else if (self.bg as usize) < ANSI_PALETTE.len() { ANSI_PALETTE[self.bg as usize] }
        else { Color(0x0F, 0x0A, 0x1E) }
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum State {
    Ground,
    Esc,
    Csi,
    Osc,
    /// ESC [ ? ... — private mode (DEC).
    CsiPrivate,
    /// ESC [ > ... — modifier sequences.
    CsiGt,
    /// DCS (Device Control String) — игнорируем содержимое до ST.
    Dcs,
}

pub struct VTerm {
    pub cols: u16,
    pub rows: u16,
    pub grid: Vec<Cell>,
    pub alt_grid: Vec<Cell>,
    pub on_alt: bool,
    pub cursor_x: u16,
    pub cursor_y: u16,
    pub cursor_visible: bool,
    pub cur_fg: u8,
    pub cur_bg: u8,
    pub cur_bold: bool,
    pub cur_underline: bool,
    pub cur_italic: bool,
    pub cur_reverse: bool,
    pub autowrap: bool,
    pub scroll_top: u16,
    pub scroll_bottom: u16,
    /// Saved cursor for DECSC/DECRC and CSI s/u.
    saved_cursor: (u16, u16, u8, u8, bool, bool),
    state: State,
    csi_buf: String,
    osc_buf: String,
    utf8_buf: Vec<u8>,
    pub dirty_top: u16,
    pub dirty_bottom: u16,
    pub title: String,
    /// libvterm backend (если доступен).
    libvterm: Option<crate::term::libvterm::LibVTermHandle>,
}

impl VTerm {
    pub fn new(cols: u16, rows: u16) -> Self {
        let cells = (cols as usize) * (rows as usize);
        // libvterm отключён — вызывает SIGSEGV при обработке сложных escape
        // sequences от zsh/powerlevel10k. Используем built-in ANSI parser.
        let libvterm: Option<crate::term::libvterm::LibVTermHandle> = None;
        log::info!("using built-in ANSI parser (libvterm disabled for stability)");
        VTerm {
            cols, rows,
            grid: vec![Cell::blank(); cells],
            alt_grid: vec![Cell::blank(); cells],
            on_alt: false,
            cursor_x: 0, cursor_y: 0,
            cursor_visible: true,
            cur_fg: 255, cur_bg: 255,
            cur_bold: false, cur_underline: false, cur_italic: false, cur_reverse: false,
            autowrap: true,
            scroll_top: 0,
            scroll_bottom: rows.saturating_sub(1),
            saved_cursor: (0, 0, 255, 255, false, false),
            state: State::Ground,
            csi_buf: String::new(),
            osc_buf: String::new(),
            utf8_buf: Vec::new(),
            dirty_top: 0,
            dirty_bottom: rows.saturating_sub(1),
            title: String::new(),
            libvterm,
        }
    }

    /// Resize terminal grid. Copies existing cells into new grid (truncated
    /// if smaller, padded with blanks if larger). Updates scroll region.
    pub fn resize(&mut self, cols: u16, rows: u16) {
        let new_cells = (cols as usize) * (rows as usize);
        let mut new_grid = vec![Cell::blank(); new_cells];
        let mut new_alt = vec![Cell::blank(); new_cells];
        let copy_cols = cols.min(self.cols) as usize;
        let copy_rows = rows.min(self.rows) as usize;
        for r in 0..copy_rows {
            for c in 0..copy_cols {
                new_grid[r * cols as usize + c] = self.grid[r * self.cols as usize + c];
                new_alt[r * cols as usize + c] = self.alt_grid[r * self.cols as usize + c];
            }
        }
        self.cols = cols;
        self.rows = rows;
        self.grid = new_grid;
        self.alt_grid = new_alt;
        self.scroll_top = 0;
        self.scroll_bottom = rows.saturating_sub(1);
        self.cursor_x = self.cursor_x.min(cols.saturating_sub(1));
        self.cursor_y = self.cursor_y.min(rows.saturating_sub(1));
        self.mark_dirty(0, rows.saturating_sub(1));
    }

    #[allow(dead_code)]
    pub fn cell(&self, x: u16, y: u16) -> &Cell {
        let idx = (y as usize) * (self.cols as usize) + (x as usize);
        &self.grid[idx]
    }

    pub fn grid_slice(&self) -> &[Cell] {
        if self.on_alt { &self.alt_grid } else { &self.grid }
    }

    fn mark_dirty(&mut self, top: u16, bottom: u16) {
        if top < self.dirty_top { self.dirty_top = top; }
        if bottom > self.dirty_bottom { self.dirty_bottom = bottom; }
    }

    #[allow(dead_code)]
    pub fn clear_dirty(&mut self) {
        self.dirty_top = self.rows;
        self.dirty_bottom = 0;
    }

    #[allow(dead_code)]
    pub fn is_dirty(&self) -> bool { self.dirty_top <= self.dirty_bottom }

    /// Главный обработчик потока байтов от PTY.
    /// Если libvterm активна — проксируем туда и синхронизируем grid.
    pub fn feed(&mut self, data: &[u8]) -> Option<String> {
        if self.libvterm.is_some() {
            // Берём handle out. lv — это owned value (не заём self), поэтому
            // можем одновременно мутировать self.grid и lv. Если что-то здесь
            // запаникует, lv будет корректно освобождена через Drop (vterm_free),
            // а self.libvterm останется None — при следующем feed() переключимся
            // на fallback parser. Это graceful degradation вместо crash.
            let mut lv = match self.libvterm.take() {
                Some(lv) => lv,
                None => return None,
            };

            lv.feed(data);
            let cols = self.cols;
            let rows = self.rows;
            let len = (cols as usize) * (rows as usize);

            // Берём mutable slice из нужного grid. Используем безопасный
            // slice indexing вместо raw pointer + from_raw_parts_mut —
            // это автоматически проверяет длину и не создаёт dangling
            // slice для пустого Vec (что было UB в старом коде).
            let grid: &mut [Cell] = if self.on_alt {
                &mut self.alt_grid
            } else {
                &mut self.grid
            };
            let effective_len = len.min(grid.len());
            let slice: &mut [Cell] = &mut grid[..effective_len];
            lv.read_grid(slice, cols, rows);

            let (cx, cy) = lv.cursor_pos();
            self.cursor_x = cx.min(cols.saturating_sub(1));
            self.cursor_y = cy.min(rows.saturating_sub(1));
            self.mark_dirty(0, rows.saturating_sub(1));
            self.libvterm = Some(lv);
            return None;
        }
        // Fallback: встроенный обработчик.
        let mut response: Option<String> = None;
        for &b in data {
            match self.state {
                State::Ground => match b {
                    0x1B => { // ESC
                        self.flush_utf8();
                        self.state = State::Esc;
                        self.csi_buf.clear();
                    }
                    0x07 => { // BEL
                        if self.state == State::Osc {
                            self.handle_osc();
                            self.state = State::Ground;
                        } else {
                            self.flush_utf8();
                        }
                    }
                    0x08 => { self.flush_utf8(); if self.cursor_x > 0 { self.cursor_x -= 1; } }
                    0x09 => {
                        self.flush_utf8();
                        let next = (self.cursor_x + 8) & !7;
                        self.cursor_x = next.min(self.cols.saturating_sub(1));
                    }
                    0x0A | 0x0B | 0x0C => { self.flush_utf8(); self.line_feed(); }
                    0x0D => { self.flush_utf8(); self.cursor_x = 0; }
                    _ => self.utf8_buf.push(b),
                },
                State::Esc => {
                    match b {
                        b'[' => { self.state = State::Csi; self.csi_buf.clear(); }
                        b']' => { self.state = State::Osc; self.osc_buf.clear(); }
                        b'M' => {
                            // RI — reverse index.
                            self.flush_utf8();
                            if self.cursor_y > self.scroll_top { self.cursor_y -= 1; }
                            else { self.scroll_down_at_top(); }
                            self.state = State::Ground;
                        }
                        b'D' => {
                            // IND — index (down + scroll if needed).
                            self.flush_utf8();
                            self.line_feed();
                            self.state = State::Ground;
                        }
                        b'E' => {
                            // NEL — next line.
                            self.flush_utf8();
                            self.cursor_x = 0;
                            self.line_feed();
                            self.state = State::Ground;
                        }
                        b'c' => { self.reset(); self.state = State::Ground; }
                        b'7' => {
                            // DECSC — save cursor.
                            self.saved_cursor = (self.cursor_x, self.cursor_y,
                                self.cur_fg, self.cur_bg, self.cur_bold, self.cur_underline);
                            self.state = State::Ground;
                        }
                        b'8' => {
                            // DECRC — restore cursor.
                            let (cx, cy, fg, bg, bo, un) = self.saved_cursor;
                            self.cursor_x = cx;
                            self.cursor_y = cy;
                            self.cur_fg = fg;
                            self.cur_bg = bg;
                            self.cur_bold = bo;
                            self.cur_underline = un;
                            self.state = State::Ground;
                        }
                        b'=' => {
                            // Application keypad — игнорируем.
                            self.state = State::Ground;
                        }
                        b'>' => {
                            // Normal keypad — игнорируем.
                            self.state = State::Ground;
                        }
                        b'P' => {
                            // DCS — Device Control String. Игнорируем до ST.
                            self.state = State::Dcs;
                        }
                        b'\\' => {
                            // ST — String Terminator (если пришёл вне OSC).
                            self.state = State::Ground;
                        }
                        _ => { self.state = State::Ground; }
                    }
                }
                State::Csi => {
                    if b == b'?' {
                        self.state = State::CsiPrivate;
                    } else if b == b'>' {
                        self.state = State::CsiGt;
                    } else if (0x40..=0x7E).contains(&b) {
                        let params = self.csi_buf.clone();
                        self.handle_csi(b, &params, &mut response, false);
                        self.state = State::Ground;
                        self.csi_buf.clear();
                    } else {
                        self.csi_buf.push(b as char);
                    }
                }
                State::CsiPrivate => {
                    if (0x40..=0x7E).contains(&b) {
                        let params = self.csi_buf.clone();
                        self.handle_csi_private(b, &params);
                        self.state = State::Ground;
                        self.csi_buf.clear();
                    } else {
                        self.csi_buf.push(b as char);
                    }
                }
                State::CsiGt => {
                    if (0x40..=0x7E).contains(&b) {
                        // modifier sequences — игнорируем.
                        self.state = State::Ground;
                        self.csi_buf.clear();
                    } else {
                        self.csi_buf.push(b as char);
                    }
                }
                State::Osc => {
                    if b == 0x1B {
                        // ST = ESC \ — упрощённо.
                        self.handle_osc();
                        self.state = State::Esc;
                    } else if b == 0x07 {
                        self.handle_osc();
                        self.state = State::Ground;
                    } else {
                        self.osc_buf.push(b as char);
                    }
                }
                State::Dcs => {
                    if b == 0x1B || b == 0x07 || b == b'\\' {
                        // DCS terminator — игнорируем содержимое.
                        self.state = if b == 0x1B { State::Esc } else { State::Ground };
                    }
                    // Иначе копим в osc_buf (не используется).
                }
            }
        }
        self.flush_utf8();
        response
    }

    #[allow(dead_code)]
    fn grid_slice_mut(&mut self) -> &mut [Cell] {
        if self.on_alt { &mut self.alt_grid } else { &mut self.grid }
    }

    fn flush_utf8(&mut self) {
        if self.utf8_buf.is_empty() { return; }
        let bytes = std::mem::take(&mut self.utf8_buf);
        let s = String::from_utf8_lossy(&bytes);
        for ch in s.chars() {
            self.put_char(ch);
        }
    }

    fn handle_osc(&mut self) {
        // OSC sequences: 0;title (icon+title), 2;title (title), 4;N;COLOR (palette),
        // 8;params;uri (hyperlink), 52;clipboard (clipboard), 10/11/12 (fg/bg/cursor colors).
        let osc = std::mem::take(&mut self.osc_buf);
        if osc.starts_with("0;") || osc.starts_with("2;") {
            self.title = osc[2..].to_string();
        } else if osc.starts_with("4;") {
            // Palette set: "4;N;rgb:RR/GG/BB" — игнорируем, у нас 16-color палитра.
        } else if osc.starts_with("8;") {
            // Hyperlink — игнорируем.
        } else if osc.starts_with("52;") {
            // Clipboard — игнорируем.
        }
        // Остальные OSC игнорируем.
    }

    fn put_char(&mut self, ch: char) {
        if self.cursor_x >= self.cols {
            if self.autowrap {
                self.cursor_x = 0;
                self.line_feed();
            } else {
                self.cursor_x = self.cols.saturating_sub(1);
            }
        }
        let x = self.cursor_x as usize;
        let y = self.cursor_y as usize;
        let idx = y * self.cols as usize + x;
        let cell = Cell {
            ch,
            fg: if self.cur_reverse { self.cur_bg } else { self.cur_fg },
            bg: if self.cur_reverse { self.cur_fg } else { self.cur_bg },
            bold: self.cur_bold,
            underline: self.cur_underline,
        };
        let grid: &mut Vec<Cell> = if self.on_alt { &mut self.alt_grid } else { &mut self.grid };
        if idx < grid.len() {
            grid[idx] = cell;
        }
        self.cursor_x += 1;
        self.mark_dirty(self.cursor_y, self.cursor_y);
    }

    fn line_feed(&mut self) {
        if self.cursor_y == self.scroll_bottom {
            self.scroll_up(1);
        } else if self.cursor_y < self.rows.saturating_sub(1) {
            self.cursor_y += 1;
        }
        self.mark_dirty(self.cursor_y, self.cursor_y);
    }

    fn scroll_up(&mut self, n: u16) {
        let top = self.scroll_top as usize;
        let bot = self.scroll_bottom as usize;
        let cols = self.cols as usize;
        let n = n as usize;
        let blank = Cell::blank();
        let grid: &mut Vec<Cell> = if self.on_alt { &mut self.alt_grid } else { &mut self.grid };
        for r in top..=bot {
            let src = if r + n <= bot { r + n } else { bot + 1 };
            if src <= bot {
                let dst_row = r * cols;
                let src_row = src * cols;
                grid.copy_within(src_row..src_row + cols, dst_row);
            } else {
                for c in 0..cols {
                    grid[r * cols + c] = blank;
                }
            }
        }
        self.mark_dirty(self.scroll_top, self.scroll_bottom);
    }

    fn scroll_down_at_top(&mut self) {
        let top = self.scroll_top as usize;
        let bot = self.scroll_bottom as usize;
        let cols = self.cols as usize;
        let blank = Cell::blank();
        let grid: &mut Vec<Cell> = if self.on_alt { &mut self.alt_grid } else { &mut self.grid };
        for r in (top + 1..=bot).rev() {
            let dst_row = r * cols;
            let src_row = (r - 1) * cols;
            grid.copy_within(src_row..src_row + cols, dst_row);
        }
        for c in 0..cols { grid[top * cols + c] = blank; }
        self.mark_dirty(self.scroll_top, self.scroll_bottom);
    }

    fn reset(&mut self) {
        let blank = Cell::blank();
        for c in self.grid.iter_mut() { *c = blank; }
        for c in self.alt_grid.iter_mut() { *c = blank; }
        self.cursor_x = 0;
        self.cursor_y = 0;
        self.cur_fg = 255;
        self.cur_bg = 255;
        self.cur_bold = false;
        self.cur_underline = false;
        self.cur_italic = false;
        self.cur_reverse = false;
        self.autowrap = true;
        self.scroll_top = 0;
        self.scroll_bottom = self.rows.saturating_sub(1);
        self.on_alt = false;
        self.mark_dirty(0, self.rows.saturating_sub(1));
    }

    fn handle_csi(&mut self, final_byte: u8, params: &str, response: &mut Option<String>, _private: bool) {
        let nums: Vec<i32> = params
            .trim_start_matches('?')
            .trim_start_matches('>')
            .split(|c| c == ';' || c == ':')
            .filter(|s| !s.is_empty())
            .map(|s| s.parse().unwrap_or(0))
            .collect();
        // Clamp all numeric parameters to u16 range to prevent truncation panics.
        // Without this, a CSI like `CSI 999999 H` would truncate to a value
        // that could exceed self.rows / self.cols and cause out-of-bounds
        // access in subsequent operations.
        let clamp_u16 = |v: i32| -> u16 {
            if v < 1 { 1 } else { (v as u64).min(u16::MAX as u64) as u16 }
        };
        let n = clamp_u16(nums.first().copied().unwrap_or(0)).max(1);
        match final_byte {
            b'H' | b'f' => {
                let row = clamp_u16(nums.first().copied().unwrap_or(1)).saturating_sub(1);
                let col = clamp_u16(nums.get(1).copied().unwrap_or(1)).saturating_sub(1);
                self.cursor_y = row.min(self.rows.saturating_sub(1));
                self.cursor_x = col.min(self.cols.saturating_sub(1));
            }
            b'A' => self.cursor_y = self.cursor_y.saturating_sub(n).max(self.scroll_top),
            b'B' => self.cursor_y = (self.cursor_y + n).min(self.scroll_bottom),
            b'C' => self.cursor_x = (self.cursor_x + n).min(self.cols.saturating_sub(1)),
            b'D' => self.cursor_x = self.cursor_x.saturating_sub(n),
            b'E' => {
                // CNL — cursor next line.
                self.cursor_y = (self.cursor_y + n).min(self.scroll_bottom);
                self.cursor_x = 0;
            }
            b'F' => {
                // CPL — cursor prev line.
                self.cursor_y = self.cursor_y.saturating_sub(n).max(self.scroll_top);
                self.cursor_x = 0;
            }
            b'd' => {
                let row = clamp_u16(nums.first().copied().unwrap_or(1)).saturating_sub(1);
                self.cursor_y = row.min(self.rows.saturating_sub(1));
            }
            b'G' => {
                let col = clamp_u16(nums.first().copied().unwrap_or(1)).saturating_sub(1);
                self.cursor_x = col.min(self.cols.saturating_sub(1));
            }
            b'J' => self.erase_display(nums.first().copied().unwrap_or(0)),
            b'K' => self.erase_line(nums.first().copied().unwrap_or(0)),
            b'm' => self.handle_sgr(&nums),
            b'r' => {
                let top = clamp_u16(nums.first().copied().unwrap_or(1)).saturating_sub(1);
                let bot = clamp_u16(nums.get(1).copied().unwrap_or(self.rows as i32));
                if top < bot && bot <= self.rows {
                    self.scroll_top = top;
                    self.scroll_bottom = bot.saturating_sub(1);
                    self.cursor_x = 0;
                    self.cursor_y = self.scroll_top;
                }
            }
            b'n' => {
                if nums.first().copied() == Some(6) {
                    *response = Some(format!("\x1B[{};{}R",
                        self.cursor_y + 1, self.cursor_x + 1));
                }
            }
            b's' => {
                // Save cursor (ANSI.SYS).
                self.saved_cursor = (self.cursor_x, self.cursor_y,
                    self.cur_fg, self.cur_bg, self.cur_bold, self.cur_underline);
            }
            b'u' => {
                // Restore cursor.
                let (cx, cy, fg, bg, bo, un) = self.saved_cursor;
                self.cursor_x = cx;
                self.cursor_y = cy;
                self.cur_fg = fg;
                self.cur_bg = bg;
                self.cur_bold = bo;
                self.cur_underline = un;
            }
            b'L' => { for _ in 0..n { self.scroll_down_at_top(); } }
            b'M' => self.scroll_up(n),
            b'P' => self.delete_chars(n),
            b'@' => self.insert_blanks(n),
            b'S' => self.scroll_up(n),
            b'T' => { for _ in 0..n { self.scroll_down_at_top(); } }
            b'h' | b'l' => {
                // Non-private modes: ANSI modes (7 = autowrap, 4 = insert).
                let mode = nums.first().copied().unwrap_or(0);
                let set = final_byte == b'h';
                match mode {
                    7 => self.autowrap = set,
                    4 => { /* insert mode — упрощённо игнорируем */ }
                    _ => {}
                }
            }
            _ => {}
        }
    }

    fn handle_csi_private(&mut self, final_byte: u8, params: &str) {
        let nums: Vec<i32> = params
            .split(|c| c == ';' || c == ':')
            .filter(|s| !s.is_empty())
            .map(|s| s.parse().unwrap_or(0))
            .collect();
        let set = final_byte == b'h';
        let mode = nums.first().copied().unwrap_or(0);
        match mode {
            1049 => {
                self.on_alt = set;
                self.mark_dirty(0, self.rows.saturating_sub(1));
            }
            25 => self.cursor_visible = set,
            7 => self.autowrap = set,
            1 => { /* application cursor keys — игнорируем */ }
            12 => { /* cursor blink — игнорируем */ }
            47 | 1047 => {
                // Alternate screen (older variants).
                self.on_alt = set;
                self.mark_dirty(0, self.rows.saturating_sub(1));
            }
            _ => {}
        }
    }

    fn erase_display(&mut self, mode: i32) {
        let cols = self.cols as usize;
        let rows = self.rows as usize;
        let cx = self.cursor_x as usize;
        let cy = self.cursor_y as usize;
        let blank = Cell::blank();
        let grid: &mut Vec<Cell> = if self.on_alt { &mut self.alt_grid } else { &mut self.grid };
        match mode {
            0 => {
                for c in cx..cols { grid[cy * cols + c] = blank; }
                for r in (cy + 1)..rows {
                    for c in 0..cols { grid[r * cols + c] = blank; }
                }
            }
            1 => {
                for r in 0..cy {
                    for c in 0..cols { grid[r * cols + c] = blank; }
                }
                for c in 0..=cx { grid[cy * cols + c] = blank; }
            }
            2 | 3 => {
                for c in grid.iter_mut() { *c = blank; }
            }
            _ => {}
        }
        self.mark_dirty(0, self.rows.saturating_sub(1));
    }

    fn erase_line(&mut self, mode: i32) {
        let cy = self.cursor_y as usize;
        let cx = self.cursor_x as usize;
        let cols = self.cols as usize;
        let blank = Cell::blank();
        let grid: &mut Vec<Cell> = if self.on_alt { &mut self.alt_grid } else { &mut self.grid };
        match mode {
            0 => {
                for c in cx..cols { grid[cy * cols + c] = blank; }
            }
            1 => {
                for c in 0..=cx { grid[cy * cols + c] = blank; }
            }
            2 => {
                for c in 0..cols { grid[cy * cols + c] = blank; }
            }
            _ => {}
        }
        self.mark_dirty(cy as u16, cy as u16);
    }

    fn delete_chars(&mut self, n: u16) {
        let cy = self.cursor_y as usize;
        let cx = self.cursor_x as usize;
        let cols = self.cols as usize;
        let n = n as usize;
        let blank = Cell::blank();
        let grid: &mut Vec<Cell> = if self.on_alt { &mut self.alt_grid } else { &mut self.grid };
        for c in cx..cols {
            let src = c + n;
            grid[cy * cols + c] = if src < cols { grid[cy * cols + src] } else { blank };
        }
        self.mark_dirty(cy as u16, cy as u16);
    }

    fn insert_blanks(&mut self, n: u16) {
        let cy = self.cursor_y as usize;
        let cx = self.cursor_x as usize;
        let cols = self.cols as usize;
        let n = n as usize;
        let blank = Cell::blank();
        let grid: &mut Vec<Cell> = if self.on_alt { &mut self.alt_grid } else { &mut self.grid };
        for c in (cx + n..cols).rev() {
            grid[cy * cols + c] = grid[cy * cols + c - n];
        }
        for c in cx..(cx + n).min(cols) {
            grid[cy * cols + c] = blank;
        }
        self.mark_dirty(cy as u16, cy as u16);
    }

    fn handle_sgr(&mut self, nums: &[i32]) {
        if nums.is_empty() {
            self.cur_fg = 255;
            self.cur_bg = 255;
            self.cur_bold = false;
            self.cur_underline = false;
            self.cur_italic = false;
            self.cur_reverse = false;
            return;
        }
        let mut i = 0;
        while i < nums.len() {
            match nums[i] {
                0 => {
                    self.cur_fg = 255; self.cur_bg = 255;
                    self.cur_bold = false; self.cur_underline = false;
                    self.cur_italic = false; self.cur_reverse = false;
                }
                1 => self.cur_bold = true,
                3 => self.cur_italic = true,
                4 => self.cur_underline = true,
                7 => self.cur_reverse = true,
                22 => self.cur_bold = false,
                23 => self.cur_italic = false,
                24 => self.cur_underline = false,
                27 => self.cur_reverse = false,
                30..=37 => self.cur_fg = (nums[i] - 30) as u8,
                38 => {
                    if i + 1 < nums.len() {
                        if nums[i + 1] == 5 && i + 2 < nums.len() {
                            self.cur_fg = map_256(nums[i + 2] as u8);
                            i += 2;
                        } else if nums[i + 1] == 2 && i + 4 < nums.len() {
                            // Truecolor — аппроксимируем в 16-цветную палитру.
                            self.cur_fg = rgb_to_palette(nums[i+2] as u8, nums[i+3] as u8, nums[i+4] as u8);
                            i += 4;
                        }
                    }
                }
                39 => self.cur_fg = 255,
                40..=47 => self.cur_bg = (nums[i] - 40) as u8,
                48 => {
                    if i + 1 < nums.len() {
                        if nums[i + 1] == 5 && i + 2 < nums.len() {
                            self.cur_bg = map_256(nums[i + 2] as u8);
                            i += 2;
                        } else if nums[i + 1] == 2 && i + 4 < nums.len() {
                            self.cur_bg = rgb_to_palette(nums[i+2] as u8, nums[i+3] as u8, nums[i+4] as u8);
                            i += 4;
                        }
                    }
                }
                49 => self.cur_bg = 255,
                90..=97 => self.cur_fg = ((nums[i] - 90) as u8) + 8,
                100..=107 => self.cur_bg = ((nums[i] - 100) as u8) + 8,
                _ => {}
            }
            i += 1;
        }
    }
}

fn map_256(n: u8) -> u8 {
    if n < 16 { n }
    else if n < 232 { if n < 248 { 7 } else { 15 } }
    else {
        let r = ((n - 16) / 36) % 6;
        let g = ((n - 16) / 6) % 6;
        let b = (n - 16) % 6;
        if r > 2 && g > 2 && b > 2 { 15 }
        else if r > 2 { 9 }
        else if g > 2 { 10 }
        else if b > 2 { 14 }
        else { 8 }
    }
}

/// Аппроксимирует RGB цвет в 16-цветную ANSI палитру.
fn rgb_to_palette(r: u8, g: u8, b: u8) -> u8 {
    let mut best = 0u8;
    let mut best_d = u32::MAX;
    for (i, p) in ANSI_PALETTE.iter().enumerate() {
        let dr = r as i32 - p.0 as i32;
        let dg = g as i32 - p.1 as i32;
        let db = b as i32 - p.2 as i32;
        let d = (dr*dr + dg*dg + db*db) as u32;
        if d < best_d { best_d = d; best = i as u8; }
    }
    best
}

