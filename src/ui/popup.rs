//! Popups — всплывающие уведомления поверх экрана.
//!
//! Виды popup:
//!   - Простой текст (Info/Alert)
//!   - Вывод скрипта (Script) — multiline текст
//!   - Prompt (односторонний input)

use crate::render::canvas::Canvas;
use crate::render::text::TextRenderer;
use crate::render::font::Font;
use crate::ui::theme::{Color, Theme};

#[derive(Debug, Clone)]
#[allow(dead_code)] // Alert/Prompt variants reserved for future popup types
pub enum PopupKind {
    /// Крупный текст по центру.
    Alert,
    /// Текстовый диалог (командная строка, имя файла).
    Prompt(String),
    /// Системное сообщение (лог, ошибка).
    Info(String),
    /// Вывод скрипта — multiline ASCII текст.
    Script(String),
}

pub struct Popup {
    pub kind: PopupKind,
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
    /// Ticks since appeared — для мигания курсора/таймингов.
    pub age: u32,
    /// Visibility.
    pub visible: bool,
}

impl Popup {
    #[allow(dead_code)] // reserved for future alerts
    pub fn alert(_text: &str, screen_w: u32, screen_h: u32) -> Self {
        let w = screen_w.min(800);
        let h = 120;
        Popup {
            kind: PopupKind::Alert,
            x: (screen_w as i32 - w as i32) / 2,
            y: (screen_h as i32 - h as i32) / 2,
            w, h,
            age: 0,
            visible: true,
        }
    }

    #[allow(dead_code)] // reserved for future text-input popups
    pub fn prompt(initial: &str, screen_w: u32, screen_h: u32) -> Self {
        let w = screen_w.min(600);
        let h = 60;
        Popup {
            kind: PopupKind::Prompt(initial.to_string()),
            x: (screen_w as i32 - w as i32) / 2,
            y: screen_h as i32 - h as i32 - 40,
            w, h,
            age: 0,
            visible: true,
        }
    }

    pub fn info(text: &str, screen_w: u32, _screen_h: u32) -> Self {
        let w = screen_w.min(500);
        let h = 80;
        Popup {
            kind: PopupKind::Info(text.to_string()),
            x: (screen_w as i32 - w as i32) / 2,
            y: 40,
            w, h,
            age: 0,
            visible: true,
        }
    }

    /// Создаёт popup с multiline контентом (например из скрипта).
    ///
    /// Размер popup рассчитывается по количеству Unicode-символов (chars),
    /// а не по байтам — иначе русские/CJK строки дают слишком широкий popup
    /// (UTF-8: "Привет" = 12 bytes, но 6 cells в fixed-width шрифте).
    pub fn script(content: &str, screen_w: u32, screen_h: u32) -> Self {
        let lines: Vec<&str> = content.lines().collect();
        // Считаем chars().count() — количество графем для fixed-width шрифта.
        // Для BMP chars это совпадает с cell width в PSF-шрифтах.
        let max_len = lines.iter()
            .map(|l| l.chars().count())
            .max()
            .unwrap_or(20)
            .max(20) as u32;
        let w = ((max_len + 4) * 8).min(screen_w * 2 / 3).max(300);
        let h = ((lines.len() as u32 + 4) * 16).min(screen_h * 3 / 4).max(100);
        Popup {
            kind: PopupKind::Script(content.to_string()),
            x: (screen_w as i32 - w as i32) / 2,
            y: (screen_h as i32 - h as i32) / 2,
            w, h,
            age: 0,
            visible: true,
        }
    }

    pub fn tick(&mut self) {
        self.age = self.age.saturating_add(1);
    }

    /// Рендерит popup на canvas.
    pub fn render(&self, canvas: &Canvas, theme: &Theme) {
        if !self.visible { return; }

        // BG.
        canvas.fill_rect(self.x, self.y, self.w, self.h, theme.popup_bg);

        let main_color = match self.kind {
            PopupKind::Alert    => theme.accent_magenta,
            PopupKind::Prompt(_) => theme.accent_cyan,
            PopupKind::Info(_)   => theme.accent_cyan,
            PopupKind::Script(_) => theme.accent_magenta,
        };

        // Main border.
        canvas.rect_outline(self.x, self.y, self.w, self.h, 2, main_color);

        // Угловые акценты.
        let cs = 12; // corner size
        canvas.fill_rect(self.x, self.y, cs, 2, main_color);
        canvas.fill_rect(self.x, self.y, 2, cs, main_color);
        canvas.fill_rect(self.x + self.w as i32 - cs as i32, self.y, cs, 2, main_color);
        canvas.fill_rect(self.x + self.w as i32 - 2, self.y, 2, cs, main_color);
        canvas.fill_rect(self.x, self.y + self.h as i32 - 2, cs, 2, main_color);
        canvas.fill_rect(self.x, self.y + self.h as i32 - cs as i32, 2, cs, main_color);
        canvas.fill_rect(self.x + self.w as i32 - cs as i32, self.y + self.h as i32 - 2, cs, 2, main_color);
        canvas.fill_rect(self.x + self.w as i32 - 2, self.y + self.h as i32 - cs as i32, 2, cs, main_color);
    }

    /// Рендерит текст popup с использованием шрифта.
    /// Должен вызываться после render() для отрисовки контента.
    pub fn render_content(&self, canvas: &Canvas, font: &Font, theme: &Theme) {
        if !self.visible { return; }
        let text = TextRenderer::new(canvas, font);
        let fw = font.width as i32;
        let fh = font.height as i32;
        match &self.kind {
            PopupKind::Info(t) | PopupKind::Prompt(t) => {
                text.draw_text(self.x + 10, self.y + 10, t, theme.fg_default, None);
            }
            PopupKind::Alert => {
                let title = "ALERT";
                let tx = self.x + (self.w as i32 - (title.len() as i32) * fw * 2) / 2;
                let ty = self.y + (self.h as i32 - fh * 2) / 2;
                draw_double_text(canvas, font, title, tx, ty, theme.accent_magenta);
            }
            PopupKind::Script(content) => {
                let lines: Vec<&str> = content.lines().collect();
                let start_y = self.y + 10;
                for (i, line) in lines.iter().enumerate() {
                    let ly = start_y + i as i32 * fh;
                    if ly + fh > self.y + self.h as i32 - 10 { break; }
                    text.draw_text(self.x + 10, ly, line, theme.fg_default, None);
                }
            }
        }
    }
}

/// Рисует текст двойного размера (каждый пиксель glyph = 2x2 block).
fn draw_double_text(canvas: &Canvas, font: &Font, text: &str, x: i32, y: i32, color: Color) {
    let fw = font.width as i32;
    let fh = font.height as i32;
    let mut cx = x;
    for ch in text.chars() {
        let glyph = font.glyph_for(ch as u32);
        let bytes_per_row = ((font.width + 7) / 8) as usize;
        for row in 0..fh {
            for col in 0..fw {
                let row_off = (row as usize) * bytes_per_row;
                let byte_off = row_off + (col as usize) / 8;
                if byte_off >= glyph.len() { break; }
                let bit_off = 7 - ((col as usize) % 8);
                if (glyph[byte_off] >> bit_off) & 1 == 1 {
                    canvas.fill_rect(cx + col * 2, y + row * 2, 2, 2, color);
                }
            }
        }
        cx += fw * 2;
    }
}
