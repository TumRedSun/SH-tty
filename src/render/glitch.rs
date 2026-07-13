//! Glitch-анимации MCD-стиля для superhot-tty.
//!
//! Три типа анимаций, все рисуются поверх обычного кадра:
//!
//! 1. **WorkspaceTransition** — при переключении ws все символы экрана (терминальные
//!    ячейки, разделители и X11-окна как квадраты) перебираются случайными
//!    символами (A-Z + блоки ▒ ▓ █ ■ □ …) и параллельно новый ws "проявляется":
//!    добавляются недостающие символы и убираются лишние. Затем с левого верхнего
//!    угла в правый нижний символы фиксируются в финальном состоянии.
//!
//! 2. **NewWindow** — квадрат нового окна заливается перебором, через N мс
//!    corner-to-corner (TL → BR) перебор снимается.
//!
//! 3. **RandomGlitch** — спонтанный быстрый corner-to-corner глитч по всему экрану
//!    или по части. Вероятность срабатывания задаётся в конфиге.
//!
//! Все анимации включаются/выключаются/настраиваются в секции [animations] конфига.

use crate::config::AnimationsCfg;
use crate::render::canvas::Canvas;
use crate::render::font::Font;
use crate::render::text::TextRenderer;
use crate::ui::theme::Color;
use std::time::{Duration, Instant};

/// Случайный символ из набора для глитча.
pub fn random_glitch_char(cfg: &AnimationsCfg, rng: &mut impl FnMut(u32) -> u32) -> char {
    let mut pool: Vec<char> = Vec::with_capacity(64);
    if cfg.glitch_use_alpha {
        // A-Z
        for c in 'A'..='Z' { pool.push(c); }
    }
    if cfg.glitch_use_blocks {
        // Различные квадраты и блоки с разной заливкой.
        pool.extend_from_slice(&[
            '\u{2591}', '\u{2592}', '\u{2593}', '\u{2588}',  // ░ ▒ ▓ █
            '\u{25A0}', '\u{25A1}', '\u{25A2}', '\u{25A3}',  // ■ □ ▢ ▣
            '\u{25A4}', '\u{25A5}', '\u{25A6}', '\u{25A7}',  // ▤ ▥ ▦ ▧
            '\u{25A8}', '\u{25A9}',                          // ▨ ▩
            '\u{2580}', '\u{2584}', '\u{258C}', '\u{2590}',  // ▀ ▄ ▌ ▐
        ]);
    }
    if cfg.glitch_use_digits {
        for c in '0'..='9' { pool.push(c); }
    }
    if pool.is_empty() {
        return '?';
    }
    let idx = (rng(0) % pool.len() as u32) as usize;
    pool[idx]
}

/// Цвет глитча (из конфига или accent_cyan).
pub fn glitch_color(cfg: &AnimationsCfg, accent_cyan: Color) -> Color {
    if let Some(s) = &cfg.glitch_color {
        let (r, g, b) = crate::config::parse_color(s);
        Color(r, g, b)
    } else {
        accent_cyan
    }
}

// ===== Animation state machine =====

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnimationKind {
    WorkspaceTransition,
    NewWindow,
    RandomGlitch,
}

/// Активная анимация. Хранит состояние для отрисовки покадрово.
pub struct ActiveAnimation {
    pub kind: AnimationKind,
    pub started: Instant,
    pub total_duration: Duration,
    /// Для WorkspaceTransition: snapshot целевого ws (как char-grid, но мы
    /// не храним весь экран — мы рисуем на лету, сравнивая rect тайлов).
    /// Прямоугольник для new_window анимации.
    pub rect: Option<crate::layout::Rect>,
    /// Для WorkspaceTransition — сохраняем snapshot старого ws (chars) и нового.
    /// Это нужно для "manifest" фазы: добавлять недостающие символы, убирать лишние.
    pub old_snapshot: Option<CharSnapshot>,
    pub new_snapshot: Option<CharSnapshot>,
    /// Текущий псевдо-RNG сид (для стабильности кадра).
    pub rng_seed: u64,
}

/// Снапшот экрана как сетка символов. Используется для ws transition.
#[derive(Clone)]
pub struct CharSnapshot {
    pub cells: Vec<char>,
    pub cols: u32,
    pub rows: u32,
    pub cell_w: u32,
    pub cell_h: u32,
    pub origin_x: i32,
    pub origin_y: i32,
}

impl CharSnapshot {
    /// Создаёт пустой snapshot (все пробелы).
    pub fn empty(cols: u32, rows: u32, cell_w: u32, cell_h: u32, origin_x: i32, origin_y: i32) -> Self {
        CharSnapshot {
            cells: vec![' '; (cols * rows) as usize],
            cols, rows, cell_w, cell_h, origin_x, origin_y,
        }
    }

    /// Заполняет snapshot рандомными символами из набора glitch.
    pub fn fill_random(&mut self, cfg: &AnimationsCfg) {
        let mut state = 0xDEADBEEFu64;
        for c in self.cells.iter_mut() {
            state = xorshift64(state);
            *c = random_glitch_char(cfg, &mut |_| state as u32);
        }
    }

    /// Возвращает символ в позиции (col, row) или ' ' если вне границ.
    pub fn get(&self, col: u32, row: u32) -> char {
        if col >= self.cols || row >= self.rows { return ' '; }
        self.cells[(row * self.cols + col) as usize]
    }

    /// Устанавливает символ в позиции (col, row).
    pub fn set(&mut self, col: u32, row: u32, ch: char) {
        if col < self.cols && row < self.rows {
            self.cells[(row * self.cols + col) as usize] = ch;
        }
    }
}

fn xorshift64(mut x: u64) -> u64 {
    if x == 0 { x = 0xDEADBEEF; }
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;
    x
}

impl ActiveAnimation {
    pub fn new_workspace_transition(
        old_snapshot: CharSnapshot,
        new_snapshot: CharSnapshot,
        cfg: &AnimationsCfg,
    ) -> Self {
        let total = Duration::from_millis(
            (cfg.ws_transition_ms + cfg.ws_manifest_ms + cfg.ws_reveal_ms) as u64
        );
        let now = Instant::now();
        ActiveAnimation {
            kind: AnimationKind::WorkspaceTransition,
            started: now,
            total_duration: total,
            rect: None,
            old_snapshot: Some(old_snapshot),
            new_snapshot: Some(new_snapshot),
            rng_seed: 0x1234_5678_9ABC_DEF0,
        }
    }

    pub fn new_new_window(rect: crate::layout::Rect, cfg: &AnimationsCfg) -> Self {
        let total = Duration::from_millis(
            (cfg.new_window_fill_ms + cfg.new_window_reveal_ms) as u64
        );
        let now = Instant::now();
        ActiveAnimation {
            kind: AnimationKind::NewWindow,
            started: now,
            total_duration: total,
            rect: Some(rect),
            old_snapshot: None,
            new_snapshot: None,
            rng_seed: 0xA5A5_5A5A_5A5A_5A5A,
        }
    }

    pub fn new_random_glitch(cfg: &AnimationsCfg, rect: Option<crate::layout::Rect>) -> Self {
        let total = Duration::from_millis(cfg.random_glitch_ms as u64);
        let now = Instant::now();
        ActiveAnimation {
            kind: AnimationKind::RandomGlitch,
            started: now,
            total_duration: total,
            rect,
            old_snapshot: None,
            new_snapshot: None,
            rng_seed: 0xF00D_FEED_C0FF_EE11,
        }
    }

    pub fn is_finished(&self) -> bool {
        self.started.elapsed() >= self.total_duration
    }

    pub fn elapsed_ms(&self) -> u64 {
        self.started.elapsed().as_millis() as u64
    }

    /// Прогресс 0.0..1.0 внутри всей анимации.
    pub fn progress(&self) -> f32 {
        let e = self.started.elapsed().as_secs_f32();
        let t = self.total_duration.as_secs_f32();
        if t <= 0.0 { 1.0 } else { (e / t).clamp(0.0, 1.0) }
    }

    /// Для corner-to-corner reveal: какая доля ячеек уже зафиксирована.
    /// 0.0 = ни одной, 1.0 = все.
    pub fn reveal_progress(&self, cfg: &AnimationsCfg) -> f32 {
        let elapsed = self.elapsed_ms();
        let reveal_start = match self.kind {
            AnimationKind::WorkspaceTransition => {
                (cfg.ws_transition_ms + cfg.ws_manifest_ms) as u64
            }
            AnimationKind::NewWindow => cfg.new_window_fill_ms as u64,
            AnimationKind::RandomGlitch => 0,
        };
        let reveal_dur = match self.kind {
            AnimationKind::WorkspaceTransition => cfg.ws_reveal_ms as u64,
            AnimationKind::NewWindow => cfg.new_window_reveal_ms as u64,
            AnimationKind::RandomGlitch => cfg.random_glitch_ms as u64,
        };
        if reveal_dur == 0 { return 1.0; }
        if elapsed < reveal_start { return 0.0; }
        let p = (elapsed - reveal_start) as f32 / reveal_dur as f32;
        p.clamp(0.0, 1.0)
    }

    /// Перебираем ли сейчас символы в ячейке (col, row)?
    /// Для corner-to-corner reveal: ячейка в "diagonal distance" от TL > reveal_progress.
    pub fn cell_is_glitching(&self, col: u32, row: u32, cols: u32, rows: u32, cfg: &AnimationsCfg) -> bool {
        let rp = self.reveal_progress(cfg);
        if rp >= 1.0 { return false; }
        // Диагональное расстояние нормализованное в [0..1].
        let nx = if cols > 0 { col as f32 / (cols - 1).max(1) as f32 } else { 0.0 };
        let ny = if rows > 0 { row as f32 / (rows - 1).max(1) as f32 } else { 0.0 };
        let diag = (nx + ny) * 0.5; // упрощённо — диагональ
        diag > rp
    }

    /// Тики перебора для конкретной ячейки — сколько раз символ сменился.
    /// Зависит от chars_per_sec и времени с начала анимации.
    pub fn cell_tick(&self, col: u32, row: u32, cfg: &AnimationsCfg) -> u32 {
        let chars_per_sec = match self.kind {
            AnimationKind::RandomGlitch => cfg.random_chars_per_sec,
            _ => cfg.chars_per_sec,
        };
        let elapsed = self.elapsed_ms();
        // Псевдо-случайный сдвиг по ячейке, чтобы ячейки менялись не одновременно.
        let cell_hash = (col as u64).wrapping_mul(0x9E3779B97F4A7C15)
            ^ (row as u64).wrapping_mul(0xBF58476D1CE4E5B9);
        let per_cell_offset = (cell_hash % 100) as u64;
        let t = elapsed + per_cell_offset;
        ((t * chars_per_sec as u64) / 1000) as u32
    }
}

/// Менеджер всех активных анимаций. Главный цикл WM вызывает
/// `tick()` каждый кадр и `render()` после обычного рендера.
pub struct AnimationManager {
    pub active: Vec<ActiveAnimation>,
    /// Счётчик кадров для random glitch.
    pub frame_counter: u32,
}

impl Default for AnimationManager {
    fn default() -> Self {
        AnimationManager { active: Vec::new(), frame_counter: 0 }
    }
}

impl AnimationManager {
    pub fn new() -> Self { Self::default() }

    /// Запускает анимацию перехода между ws.
    pub fn start_ws_transition(
        &mut self,
        old_snapshot: CharSnapshot,
        new_snapshot: CharSnapshot,
        cfg: &AnimationsCfg,
    ) {
        if !cfg.workspace_transition { return; }
        // Завершаем любые предыдущие ws-переходы.
        self.active.retain(|a| a.kind != AnimationKind::WorkspaceTransition);
        self.active.push(ActiveAnimation::new_workspace_transition(
            old_snapshot, new_snapshot, cfg,
        ));
    }

    /// Запускает анимацию появления нового окна в заданном прямоугольнике.
    pub fn start_new_window(&mut self, rect: crate::layout::Rect, cfg: &AnimationsCfg) {
        if !cfg.new_window { return; }
        self.active.push(ActiveAnimation::new_new_window(rect, cfg));
    }

    /// Возможно запускает случайный глитч (вызывается каждый кадр).
    pub fn maybe_random_glitch(
        &mut self,
        cfg: &AnimationsCfg,
        glitch_intensity: f32,
        canvas_w: u32,
        canvas_h: u32,
    ) {
        if !cfg.random_glitch || cfg.random_glitch_every_frames == 0 { return; }
        self.frame_counter = self.frame_counter.wrapping_add(1);
        // Вероятность срабатывания: 1 / every_frames, умножить на intensity.
        let p = glitch_intensity / cfg.random_glitch_every_frames as f32;
        // Простой LCG для случайности.
        let r = (self.frame_counter as u64).wrapping_mul(0x9E3779B97F4A7C15) % 1000000;
        let threshold = (p * 1_000_000.0) as u64;
        if r < threshold {
            // Случайный под-прямоугольник для glitch.
            let w = canvas_w / 4;
            let h = canvas_h / 4;
            let r2 = (r.wrapping_mul(31)) % 1000;
            let x = (r2 as u32 * canvas_w / 1000) % (canvas_w - w);
            let r3 = (r.wrapping_mul(37)) % 1000;
            let y = (r3 as u32 * canvas_h / 1000) % (canvas_h - h);
            let rect = crate::layout::Rect {
                x: x as i32, y: y as i32, w, h,
            };
            self.active.push(ActiveAnimation::new_random_glitch(cfg, Some(rect)));
        }
    }

    /// Удаляет завершённые анимации.
    pub fn tick(&mut self) {
        self.active.retain(|a| !a.is_finished());
    }

    pub fn is_animating(&self) -> bool {
        !self.active.is_empty()
    }

    pub fn is_in_ws_transition(&self) -> bool {
        self.active.iter().any(|a| a.kind == AnimationKind::WorkspaceTransition)
    }

    /// Рисует все активные анимации поверх canvas.
    pub fn render(
        &self,
        canvas: &Canvas,
        font: &Font,
        cfg: &AnimationsCfg,
        accent_cyan: Color,
    ) {
        let text = TextRenderer::new(canvas, font);
        let glitch_col = glitch_color(cfg, accent_cyan);
        for anim in &self.active {
            match anim.kind {
                AnimationKind::WorkspaceTransition => {
                    render_ws_transition(canvas, &text, font, anim, cfg, glitch_col);
                }
                AnimationKind::NewWindow => {
                    if let Some(rect) = anim.rect {
                        render_new_window(canvas, &text, font, anim, rect, cfg, glitch_col);
                    }
                }
                AnimationKind::RandomGlitch => {
                    if let Some(rect) = anim.rect {
                        render_random_glitch(canvas, &text, font, anim, rect, cfg, glitch_col);
                    }
                }
            }
        }
    }
}

// ===== Renderers =====

fn render_ws_transition(
    _canvas: &Canvas,
    text: &TextRenderer,
    font: &Font,
    anim: &ActiveAnimation,
    cfg: &AnimationsCfg,
    glitch_col: Color,
) {
    let elapsed = anim.elapsed_ms();
    let phase1_end = cfg.ws_transition_ms as u64;
    let phase2_end = phase1_end + cfg.ws_manifest_ms as u64;
    let phase3_end = phase2_end + cfg.ws_reveal_ms as u64;

    // Базовые параметры сетки из new_snapshot.
    let new_snap = match &anim.new_snapshot {
        Some(s) => s,
        None => return,
    };
    let old_snap = anim.old_snapshot.as_ref();

    let cols = new_snap.cols;
    let rows = new_snap.rows;
    let cw = new_snap.cell_w as i32;
    let ch = new_snap.cell_h as i32;
    let ox = new_snap.origin_x;
    let oy = new_snap.origin_y;

    // Сколько символов проявилось из нового ws (фаза manifest).
    // В фазе 1 (transition) — 0%. В фазе 2 (manifest) — растёт до 100%. В фазе 3 — 100%.
    let manifest_ratio: f32 = if elapsed < phase1_end {
        0.0
    } else if elapsed < phase2_end {
        (elapsed - phase1_end) as f32 / cfg.ws_manifest_ms.max(1) as f32
    } else {
        1.0
    };

    // reveal_progress: какая доля ячеек уже зафиксирована (TL → BR диагональ).
    let reveal = anim.reveal_progress(cfg);

    for row in 0..rows {
        for col in 0..cols {
            let px = ox + col as i32 * cw;
            let py = oy + row as i32 * ch;
            // Ячейка уже зафиксирована?
            let nx = if cols > 1 { col as f32 / (cols - 1) as f32 } else { 0.0 };
            let ny = if rows > 1 { row as f32 / (rows - 1) as f32 } else { 0.0 };
            let diag = (nx + ny) * 0.5;
            let is_revealed = diag <= reveal;

            if is_revealed {
                // Финальный символ из нового ws.
                let ch_final = new_snap.get(col, row);
                if ch_final != ' ' {
                    text.draw_glyph(px, py, ch_final as u32, glitch_col, None);
                }
                continue;
            }

            // Ячейка ещё перебирается. Определяем источник символа:
            // В фазе manifest — постепенно проявляем новый ws.
            let cell_hash = (col as u64).wrapping_mul(0x9E3779B97F4A7C15)
                ^ (row as u64).wrapping_mul(0xBF58476D1CE4E5B9);
            let cell_rand = (cell_hash % 1000) as f32 / 1000.0;

            let glitch_tick = anim.cell_tick(col, row, cfg);
            // Случайный символ глитча, зависящий от tick.
            let mut state = anim.rng_seed
                .wrapping_add(cell_hash)
                .wrapping_add(glitch_tick as u64);
            state = xorshift64(state);
            let glitch_ch = random_glitch_char(cfg, &mut |_| state as u32);

            // Если manifest уже дошёл до этой ячейки — показываем финальный символ
            // из нового ws поверх перебора. Это создаёт эффект "появления".
            if cell_rand < manifest_ratio {
                let ch_final = new_snap.get(col, row);
                if ch_final != ' ' {
                    text.draw_glyph(px, py, ch_final as u32, glitch_col, None);
                } else if let Some(old) = old_snap {
                    // Если в новом ws пусто, но в старом было что-то — оставляем
                    // glitch (эффект "исчезновения").
                    let old_ch = old.get(col, row);
                    if old_ch != ' ' {
                        text.draw_glyph(px, py, glitch_ch as u32, glitch_col, None);
                    }
                }
            } else {
                // Полный glitch.
                if cfg.glitch_use_blocks || cfg.glitch_use_alpha || cfg.glitch_use_digits {
                    text.draw_glyph(px, py, glitch_ch as u32, glitch_col, None);
                }
            }
        }
    }
}

fn render_new_window(
    canvas: &Canvas,
    text: &TextRenderer,
    font: &Font,
    anim: &ActiveAnimation,
    rect: crate::layout::Rect,
    cfg: &AnimationsCfg,
    glitch_col: Color,
) {
    let elapsed = anim.elapsed_ms();
    let phase1_end = cfg.new_window_fill_ms as u64;
    let phase2_end = phase1_end + cfg.new_window_reveal_ms as u64;
    let _ = phase2_end;

    // Заливаем rect фоном глитча (чтобы скрыть содержимое окна).
    canvas.fill_rect(rect.x, rect.y, rect.w, rect.h, Color(0x05, 0x03, 0x10));

    let fw = font.width as i32;
    let fh = font.height as i32;
    if fw <= 0 || fh <= 0 { return; }
    let cols = (rect.w as i32 / fw).max(1) as u32;
    let rows = (rect.h as i32 / fh).max(1) as u32;
    let reveal = anim.reveal_progress(cfg);

    for row in 0..rows {
        for col in 0..cols {
            let px = rect.x + col as i32 * fw;
            let py = rect.y + row as i32 * fh;
            let nx = if cols > 1 { col as f32 / (cols - 1) as f32 } else { 0.0 };
            let ny = if rows > 1 { row as f32 / (rows - 1) as f32 } else { 0.0 };
            let diag = (nx + ny) * 0.5;
            if diag <= reveal && elapsed > phase1_end {
                // Уже проявилось — пропускаем (окно видно нормально).
                continue;
            }
            // Перебор символов.
            let glitch_tick = anim.cell_tick(col, row, cfg);
            let cell_hash = (col as u64).wrapping_mul(0x9E3779B97F4A7C15)
                ^ (row as u64).wrapping_mul(0xBF58476D1CE4E5B9);
            let mut state = anim.rng_seed
                .wrapping_add(cell_hash)
                .wrapping_add(glitch_tick as u64);
            state = xorshift64(state);
            let ch = random_glitch_char(cfg, &mut |_| state as u32);
            text.draw_glyph(px, py, ch as u32, glitch_col, None);
        }
    }
}

fn render_random_glitch(
    canvas: &Canvas,
    text: &TextRenderer,
    font: &Font,
    anim: &ActiveAnimation,
    rect: crate::layout::Rect,
    cfg: &AnimationsCfg,
    glitch_col: Color,
) {
    let _ = canvas;
    let fw = font.width as i32;
    let fh = font.height as i32;
    if fw <= 0 || fh <= 0 { return; }
    let cols = (rect.w as i32 / fw).max(1) as u32;
    let rows = (rect.h as i32 / fh).max(1) as u32;
    let reveal = anim.reveal_progress(cfg);

    for row in 0..rows {
        for col in 0..cols {
            let px = rect.x + col as i32 * fw;
            let py = rect.y + row as i32 * fh;
            let nx = if cols > 1 { col as f32 / (cols - 1) as f32 } else { 0.0 };
            let ny = if rows > 1 { row as f32 / (rows - 1) as f32 } else { 0.0 };
            let diag = (nx + ny) * 0.5;
            if diag <= reveal {
                continue;
            }
            let glitch_tick = anim.cell_tick(col, row, cfg);
            let cell_hash = (col as u64).wrapping_mul(0x9E3779B97F4A7C15)
                ^ (row as u64).wrapping_mul(0xBF58476D1CE4E5B9);
            let mut state = anim.rng_seed
                .wrapping_add(cell_hash)
                .wrapping_add(glitch_tick as u64);
            state = xorshift64(state);
            let ch = random_glitch_char(cfg, &mut |_| state as u32);
            // Случайный glitch: мигаем яркими цветами.
            let color = if (state & 0x80) != 0 { glitch_col } else { Color(0xFF, 0x2E, 0x97) };
            text.draw_glyph(px, py, ch as u32, color, None);
        }
    }
}

// ===== Snapshot helpers =====

/// Создаёт CharSnapshot из текущего состояния workspace (терминалы + X11 как квадраты).
/// cell_w / cell_h = размер шрифта. cols / rows = число ячеек в canvas.
pub fn snapshot_workspace(
    workspaces: &crate::layout::workspaces::Workspaces,
    terminals: &std::collections::HashMap<crate::layout::LeafId, crate::TerminalTile>,
    x11: &Option<crate::x11::X11Compositor>,
    canvas: &Canvas,
    font: &Font,
    theme: &crate::ui::Theme,
) -> CharSnapshot {
    let fw = font.width.max(1);
    let fh = font.height.max(1);
    let cols = (canvas.width / fw).max(1);
    let rows = (canvas.height / fh).max(1);
    let mut snap = CharSnapshot::empty(cols, rows, fw, fh, 0, 0);

    let layout = workspaces.current_layout();
    let screen_rect = crate::layout::Rect { x: 0, y: 0, w: canvas.width, h: canvas.height };
    let tiles = layout.tile_rects(screen_rect);

    for (leaf_id, kind, rect) in &tiles {
        match kind {
            crate::layout::TileKind::Terminal => {
                if let Some(tile) = terminals.get(leaf_id) {
                    if tile.workspace != workspaces.current { continue; }
                    let grid = tile.vterm.grid_slice();
                    let t_cols = tile.vterm.cols as usize;
                    let t_rows = tile.vterm.rows as usize;
                    let term_x = rect.x + 4;
                    let term_y = rect.y + 4 + fh as i32;
                    for row in 0..t_rows {
                        for col in 0..t_cols {
                            let cell = &grid[row * t_cols + col];
                            if cell.ch == ' ' { continue; }
                            let px = term_x + col as i32 * fw as i32;
                            let py = term_y + row as i32 * fh as i32;
                            let snap_col = (px / fw as i32).max(0) as u32;
                            let snap_row = (py / fh as i32).max(0) as u32;
                            if snap_col < cols && snap_row < rows {
                                snap.set(snap_col, snap_row, cell.ch);
                            }
                        }
                    }
                }
            }
            crate::layout::TileKind::X11 => {
                // Для X11 окна — заполняем квадрат символами блока (███).
                // Это нужно чтобы анимация воспринимала X11 окно как "текстовый" квадрат.
                let x0 = (rect.x / fw as i32).max(0) as u32;
                let y0 = (rect.y / fh as i32).max(0) as u32;
                let x1 = ((rect.x + rect.w as i32) / fw as i32).min(cols as i32 - 1).max(0) as u32;
                let y1 = ((rect.y + rect.h as i32) / fh as i32).min(rows as i32 - 1).max(0) as u32;
                for r in y0..=y1 {
                    for c in x0..=x1 {
                        // Чередуем блоки для визуальной текстуры.
                        let ch = if (c + r) % 2 == 0 { '\u{2588}' } else { '\u{2593}' };
                        snap.set(c, r, ch);
                    }
                }
            }
        }
    }

    // Добавляем символы статус-бара (нижняя строка).
    let status_y = (canvas.height as i32 - 24) / fh as i32;
    if status_y >= 0 && (status_y as u32) < rows {
        for c in 0..cols.min(40) {
            snap.set(c, status_y as u32, '\u{2580}');
        }
    }

    let _ = theme; // unused now, placeholder for future theming
    snap
}
