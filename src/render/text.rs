//! Рендеринг текста на canvas с использованием PSF-шрифта.

use crate::render::canvas::Canvas;
use crate::render::font::Font;
use crate::ui::theme::Color;

pub struct TextRenderer<'a> {
    pub canvas: &'a Canvas,
    pub font: &'a Font,
}

impl<'a> TextRenderer<'a> {
    pub fn new(canvas: &'a Canvas, font: &'a Font) -> Self {
        TextRenderer { canvas, font }
    }

    /// Рисует один глиф в позиции (px, py). Возвращает следующую x-позицию.
    pub fn draw_glyph(&self, px: i32, py: i32, cp: u32, fg: Color, bg: Option<Color>) -> i32 {
        let glyph = self.font.glyph_for(cp);
        let fw = self.font.width as i32;
        let fh = self.font.height as i32;
        let bytes_per_row = ((self.font.width + 7) / 8) as usize;
        for row in 0..fh {
            for col in 0..fw {
                let row_off = (row as usize) * bytes_per_row;
                let byte_off = row_off + (col as usize) / 8;
                if byte_off >= glyph.len() { break; }
                let bit_off = 7 - ((col as usize) % 8);
                let set = (glyph[byte_off] >> bit_off) & 1 == 1;
                let c = if set { fg } else { bg.unwrap_or(fg) };
                if set || bg.is_some() {
                    self.canvas.put_pixel(px + col, py + row, c);
                }
            }
        }
        px + fw
    }

    /// Рисует строку UTF-8.
    pub fn draw_text(&self, mut px: i32, py: i32, text: &str, fg: Color, bg: Option<Color>) {
        for ch in text.chars() {
            px = self.draw_glyph(px, py, ch as u32, fg, bg);
        }
    }

    /// Рисует текст в прямоугольнике с переносом по словам (упрощённо).
    #[allow(dead_code)] // not currently used, kept for future popup rendering
    pub fn draw_text_wrapped(&self, x: i32, y: i32, max_w: u32, text: &str, fg: Color) {
        let fw = self.font.width as i32;
        let fh = self.font.height as i32;
        let max_cols = (max_w as i32 / fw).max(1) as usize;
        let mut cur_x = x;
        let mut cur_y = y;
        for word in text.split_whitespace() {
            if cur_x + (word.len() as i32 + 1) * fw > x + (max_cols as i32) * fw {
                cur_x = x;
                cur_y += fh + 2;
            }
            self.draw_text(cur_x, cur_y, word, fg, None);
            cur_x += (word.len() as i32 + 1) * fw;
        }
    }
}
