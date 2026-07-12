//! SuperHot MCD-styled popups.
//!
//! В MCD всплывающие окна появляются с глитч-эффектом: сначала «призрак» рамки
//! со смещением по RGB-каналам, потом раскрытие. Мы эмулируем это через
//! многослойный рендер: 3 рамки со смещением (-2,0,2)x с цветами R/G/B.

use crate::render::canvas::Canvas;
use crate::ui::theme::{Color, Theme};

#[derive(Debug, Clone)]
pub enum PopupKind {
    /// «SYSTEM ALERT» в стиле MCD — крупный текст по центру.
    Alert,
    /// Текстовый диалог (командная строка, имя файла).
    Prompt(String),
    /// Системное сообщение (лог, ошибка).
    Info(String),
    /// Kill cam (когда закрывается окно).
    KillCam,
}

pub struct Popup {
    pub kind: PopupKind,
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
    /// Ticks since appeared — для анимации.
    pub age: u32,
    /// Visibility.
    pub visible: bool,
}

impl Popup {
    pub fn alert(text: &str, screen_w: u32, screen_h: u32) -> Self {
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

    pub fn info(text: &str, screen_w: u32, screen_h: u32) -> Self {
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

    pub fn killcam(screen_w: u32, screen_h: u32) -> Self {
        Popup {
            kind: PopupKind::KillCam,
            x: 0, y: 0, w: screen_w, h: screen_h,
            age: 0,
            visible: true,
        }
    }

    pub fn tick(&mut self) {
        self.age = self.age.saturating_add(1);
    }

    /// Рендерит popup на canvas. text_renderer используется если передан.
    pub fn render(&self, canvas: &Canvas, theme: &Theme) {
        if !self.visible { return; }
        // Глитч-анимация: первые 10 тиков — расширяющиеся RGB-рамки.
        let glitch_phase = self.age.min(10);

        // BG.
        canvas.fill_rect(self.x, self.y, self.w, self.h, theme.popup_bg);

        // Triple-rendered glitch border.
        let main_color = match self.kind {
            PopupKind::Alert    => theme.accent_magenta,
            PopupKind::Prompt(_) => theme.accent_cyan,
            PopupKind::Info(_)   => theme.accent_cyan,
            PopupKind::KillCam  => theme.error,
        };

        for (offset, color) in [
            (-2i32, Color(0xFF, 0x00, 0x00)), // R
            ( 0,    Color(0x00, 0xFF, 0x00)), // G
            ( 2,    Color(0x00, 0xC0, 0xFF)), // B
        ] {
            let alpha = if glitch_phase < 8 { 200 } else { 120 };
            let _ = alpha;
            let dx = self.x + offset;
            let dy = self.y + (offset / 2);
            canvas.rect_outline(dx, dy, self.w, self.h, 1, color);
        }
        // Main bright border.
        canvas.rect_outline(self.x, self.y, self.w, self.h, 2, main_color);

        // Угловые акценты (как MCD corner brackets).
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
}
