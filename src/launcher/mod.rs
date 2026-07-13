//! Rofi-подобный launcher в стиле SuperHot MCD.
//!
   //! При активации (Super+D) открывает popup, читает .desktop файлы из
//! стандартных директорий + кастомные записи из конфига. Пользователь
//! навигирует стрелками, Enter запускает выбранную программу.
//!
//! Программы запускаются на нашем Xephyr-дисплее как X-клиенты, и
//! появляются в новой X11-плитке.

use crate::ui::theme::Theme;
use crate::render::canvas::Canvas;
use crate::render::text::TextRenderer;
use crate::render::font::Font;
use crate::ui::Color;
use std::collections::HashMap;
use std::path::Path;
use std::process::Command;

#[derive(Debug, Clone)]
pub struct Entry {
    pub name: String,
    pub exec: String,
    pub icon: Option<String>,
    pub is_terminal: bool,
    pub category: String,
}

pub struct Launcher {
    pub entries: Vec<Entry>,
    pub filtered: Vec<usize>,
    pub selected: usize,
    pub query: String,
    pub visible: bool,
    pub scroll_offset: usize,
}

impl Launcher {
    pub fn new(desktop_paths: &[String], custom: &HashMap<String, String>) -> Self {
        let mut entries = Vec::new();
        for path in desktop_paths {
            let p = expand_tilde(path);
            entries.extend(scan_desktop_files(&p));
        }
        for (name, cmd) in custom {
            entries.push(Entry {
                name: name.clone(),
                exec: cmd.clone(),
                icon: None,
                is_terminal: false,
                category: "custom".into(),
            });
        }
        entries.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
        entries.dedup_by(|a, b| a.name == b.name && a.exec == b.exec);
        let filtered: Vec<usize> = (0..entries.len()).collect();
        Launcher {
            entries,
            filtered,
            selected: 0,
            query: String::new(),
            visible: false,
            scroll_offset: 0,
        }
    }

    pub fn open(&mut self) {
        self.visible = true;
        self.query.clear();
        self.selected = 0;
        self.scroll_offset = 0;
        self.filtered = (0..self.entries.len()).collect();
    }

    pub fn close(&mut self) {
        self.visible = false;
    }

    pub fn toggle(&mut self) {
        if self.visible { self.close(); } else { self.open(); }
    }

    /// Возвращает индекс выбранной записи или None.
    pub fn handle_key(&mut self, key: &str) -> Option<usize> {
        if !self.visible { return None; }
        match key {
            "Escape" => { self.close(); None }
            "Up" | "k" => {
                if self.selected > 0 { self.selected -= 1; }
                self.adjust_scroll();
                None
            }
            "Down" | "j" => {
                if self.selected + 1 < self.filtered.len() { self.selected += 1; }
                self.adjust_scroll();
                None
            }
            "Return" => {
                if let Some(&idx) = self.filtered.get(self.selected) {
                    self.close();
                    Some(idx)
                } else { None }
            }
            "BackSpace" => {
                self.query.pop();
                self.refilter();
                None
            }
            _ => {
                if key.chars().count() == 1 {
                    self.query.push(key.chars().next().unwrap());
                    self.refilter();
                }
                None
            }
        }
    }

    fn refilter(&mut self) {
        let q = self.query.to_lowercase();
        self.filtered = (0..self.entries.len())
            .filter(|&i| {
                if q.is_empty() { return true; }
                let name = self.entries[i].name.to_lowercase();
                let exec = self.entries[i].exec.to_lowercase();
                name.contains(&q) || exec.contains(&q)
            })
            .collect();
        self.selected = 0;
        self.scroll_offset = 0;
    }

    fn adjust_scroll(&mut self) {
        let visible = 12usize;
        if self.selected < self.scroll_offset {
            self.scroll_offset = self.selected;
        } else if self.selected >= self.scroll_offset + visible {
            self.scroll_offset = self.selected - visible + 1;
        }
    }

    /// Рендерит launcher popup.
    pub fn render(&self, canvas: &Canvas, font: &Font, theme: &Theme, screen_w: u32, screen_h: u32) {
        if !self.visible { return; }
        let fw = font.width as i32;
        let fh = font.height as i32;

        let popup_w = (screen_w as i32 * 2 / 3).max(400) as u32;
        let popup_h = (font.height as u32 * 16 + 40).max(360);
        let px = (screen_w as i32 - popup_w as i32) / 2;
        let py = (screen_h as i32 - popup_h as i32) / 2;
        let popup_w_i = popup_w as i32;
        let popup_h_i = popup_h as i32;

        // BG.
        canvas.fill_rect(px, py, popup_w, popup_h, theme.popup_bg);

        // Glitch border (RGB-сдвиг).
        for (offset, color) in [
            (-2i32, Color(0xFF, 0x00, 0x00)),
            (0,    Color(0x00, 0xFF, 0x00)),
            (2,    Color(0x00, 0xC0, 0xFF)),
        ] {
            canvas.rect_outline(px + offset, py, popup_w, popup_h, 1, color);
        }
        // Main border.
        canvas.rect_outline(px, py, popup_w, popup_h, 2, theme.accent_magenta);

        // Corner brackets (MCD style).
        let cs: u32 = 16;
        canvas.fill_rect(px, py, cs, 3, theme.accent_magenta);
        canvas.fill_rect(px, py, 3, cs, theme.accent_magenta);
        canvas.fill_rect(px + popup_w_i - cs as i32, py, cs, 3, theme.accent_magenta);
        canvas.fill_rect(px + popup_w_i - 3, py, 3, cs, theme.accent_magenta);
        canvas.fill_rect(px, py + popup_h_i - 3, cs, 3, theme.accent_magenta);
        canvas.fill_rect(px, py + popup_h_i - cs as i32, 3, cs, theme.accent_magenta);
        canvas.fill_rect(px + popup_w_i - cs as i32, py + popup_h_i - 3, cs, 3, theme.accent_magenta);
        canvas.fill_rect(px + popup_w_i - 3, py + popup_h_i - cs as i32, 3, cs, theme.accent_magenta);

        // Header.
        let text = TextRenderer::new(canvas, font);
        let header_y = py + 10;
        text.draw_text(px + 12, header_y, "RUN", theme.accent_cyan, None);
        text.draw_text(px + 50, header_y, "// superhot launcher", theme.fg_dim, None);

        // Query line.
        let qy = header_y + fh + 4;
        canvas.fill_rect(px + 8, qy - 2, popup_w - 16, fh as u32 + 4, Color(0x05, 0x03, 0x10));
        let prompt = format!("> {}", self.query);
        text.draw_text(px + 12, qy, &prompt, theme.accent_magenta, None);
        // blinking cursor
        let cursor_x = px + 12 + (prompt.len() as i32 + 1) * fw;
        canvas.fill_rect(cursor_x, qy, fw as u32, fh as u32, theme.accent_magenta);

        // Entries.
        let entry_start_y = qy + fh + 8;
        let visible_count = 12usize;
        let max_show = self.filtered.len().min(visible_count);
        for i in 0..max_show {
            let filtered_idx = i + self.scroll_offset;
            if filtered_idx >= self.filtered.len() { break; }
            let entry_idx = self.filtered[filtered_idx];
            let entry = &self.entries[entry_idx];
            let ey = entry_start_y + i as i32 * fh;
            let is_sel = filtered_idx == self.selected;
            if is_sel {
                canvas.fill_rect(px + 8, ey - 1, popup_w - 16, fh as u32 + 2, Color(0x20, 0x10, 0x40));
                // left accent
                canvas.fill_rect(px + 8, ey - 1, 3, fh as u32 + 2, theme.accent_magenta);
            }
            let color = if is_sel { theme.accent_cyan } else { theme.fg_default };
            let display_name = if entry.name.len() > 50 { &entry.name[..50] } else { &entry.name };
            text.draw_text(px + 16, ey, display_name, color, None);
            // category tag справа
            let cat = &entry.category;
            let cat_x = px + popup_w_i - 12 - (cat.len() as i32 + 2) * fw;
            text.draw_text(cat_x, ey, cat, theme.fg_dim, None);
        }

        // Footer hint.
        let fy = py + popup_h_i - fh - 8;
        text.draw_text(px + 12, fy, "↑↓ navigate  Enter run  Esc close", theme.fg_dim, None);
        // count
        let count = format!("[{}/{}]", self.filtered.len(), self.entries.len());
        let cx = px + popup_w_i - 12 - (count.len() as i32) * fw;
        text.draw_text(cx, fy, &count, theme.accent_cyan, None);
    }

    /// Запускает выбранную программу.
    /// Если entry.is_terminal == true → запускаем Exec в нашем нативном терминале
    /// (через shell -c "exec ...").
    /// Иначе — запускаем как X11 приложение.
    pub fn launch(entry: &Entry, display: &str, terminal_shell: &str) -> std::io::Result<()> {
        if entry.is_terminal {
            // Запускаем в нашем нативном терминале: shell -c "exec <cmd>"
            let full_cmd = format!("exec {}", entry.exec);
            let mut cmd = Command::new(terminal_shell);
            cmd.args(["-c", &full_cmd]);
            cmd.env("DISPLAY", display);
            cmd.env("XDG_SESSION_TYPE", "x11");
            cmd.env("XDG_CURRENT_DESKTOP", "superhot");
            cmd.env("TERM", "xterm-256color");
            cmd.spawn()?;
            log::info!("launched (terminal) '{}' via {}", entry.exec, terminal_shell);
        } else {
            let mut cmd = Command::new(&entry.exec);
            cmd.env("DISPLAY", display);
            cmd.env("XDG_SESSION_TYPE", "x11");
            cmd.env("XDG_CURRENT_DESKTOP", "superhot");
            cmd.spawn()?;
            log::info!("launched (x11) '{}'", entry.exec);
        }
        Ok(())
    }
}

fn scan_desktop_files(dir: &str) -> Vec<Entry> {
    let mut out = Vec::new();
    if !Path::new(dir).is_dir() { return out; }
    if let Ok(rd) = std::fs::read_dir(dir) {
        for entry in rd.flatten() {
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) != Some("desktop") { continue; }
            if let Ok(content) = std::fs::read_to_string(&path) {
                if let Some(e) = parse_desktop_file(&content) { out.push(e); }
            }
        }
    }
    out
}

fn parse_desktop_file(content: &str) -> Option<Entry> {
    let mut name = None;
    let mut exec = None;
    let mut icon = None;
    let mut terminal = false;
    let mut nodisplay = false;
    let mut category = "app".to_string();
    let mut in_desktop_entry = false;
    for line in content.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_desktop_entry = line == "[Desktop Entry]";
            continue;
        }
        if !in_desktop_entry { continue; }
        if let Some(eq) = line.find('=') {
            let k = line[..eq].trim();
            let v = line[eq+1..].trim();
            match k {
                "Name" => name = Some(v.to_string()),
                "Exec" => {
                    // Strip % placeholders.
                    let clean = v.split_whitespace()
                        .filter(|t| !t.starts_with('%'))
                        .collect::<Vec<_>>()
                        .join(" ");
                    exec = Some(clean);
                }
                "Icon" => icon = Some(v.to_string()),
                "Terminal" => terminal = v == "true",
                "NoDisplay" => nodisplay = v == "true",
                "Categories" => category = v.split(';').next().unwrap_or("app").to_string(),
                _ => {}
            }
        }
    }
    if nodisplay { return None; }
    let name = name?;
    let exec = exec?;
    Some(Entry { name, exec, icon, is_terminal: terminal, category })
}

fn expand_tilde(s: &str) -> String {
    if let Some(rest) = s.strip_prefix("~/") {
        if let Ok(home) = std::env::var("HOME") {
            return format!("{}/{}", home, rest);
        }
    }
    s.to_string()
}
