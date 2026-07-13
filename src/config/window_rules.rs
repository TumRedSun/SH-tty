//! Window rules engine — применяет правила размещения к новым X11 окнам.
//!
//! При CreateNotify:
//!   1. Получаем WM_CLASS и WM_NAME окна.
//!   2. Проходим по всем window_rules из конфига.
//!   3. Первое совпавшее правило применяется:
//!      - workspace: поместить на указанный workspace
//!      - monitor: поместить на указанный монитор
//!      - size/position: задать размер и позицию плитки
//!      - focus: сделать окно сфокусированным
//!      - fullscreen: открыть в полноэкранном режиме
//!      - skip_auto_place: не размещать автоматически
//!   4. Если ни одно правило не совпало:
//!      - Открываем на текущем активном workspace, в фокусе.

use crate::config::{WindowRule, Config};
use std::collections::HashMap;

pub struct WindowRuleEngine {
    rules: Vec<WindowRule>,
}

#[derive(Debug, Clone)]
pub struct WindowInfo {
    pub class: String,
    pub title: String,
    pub app_id: String,
}

#[derive(Debug, Clone)]
pub struct Placement {
    pub workspace: Option<u8>,
    pub monitor: Option<String>,
    pub size: Option<(u32, u32)>,
    pub position: Option<(u32, u32)>,
    pub focus: bool,
    pub fullscreen: bool,
    pub skip_auto_place: bool,
}

impl Default for Placement {
    fn default() -> Self {
        Placement {
            workspace: None,
            monitor: None,
            size: None,
            position: None,
            focus: true,
            fullscreen: false,
            skip_auto_place: false,
        }
    }
}

impl WindowRuleEngine {
    pub fn new(cfg: &Config) -> Self {
        WindowRuleEngine { rules: cfg.window_rules.clone() }
    }

    /// Находит первое совпавшее правило и возвращает placement.
    /// Если ни одно правило не совпало — возвращает default placement (active ws, focus).
    pub fn match_window(&self, info: &WindowInfo) -> Placement {
        for rule in &self.rules {
            if self.rule_matches(rule, info) {
                return Placement {
                    workspace: rule.workspace,
                    monitor: rule.monitor.clone(),
                    size: rule.size,
                    position: rule.position,
                    focus: rule.focus,
                    fullscreen: rule.fullscreen,
                    skip_auto_place: rule.skip_auto_place,
                };
            }
        }
        Placement::default()
    }

    fn rule_matches(&self, rule: &WindowRule, info: &WindowInfo) -> bool {
        let mut matched_any = false;
        let mut matched_all = true;

        if let Some(class_pattern) = &rule.match_class {
            matched_any = true;
            if !self.match_str(class_pattern, &info.class, rule.regex) {
                matched_all = false;
            }
        }
        if let Some(title_pattern) = &rule.match_title {
            matched_any = true;
            if !self.match_str(title_pattern, &info.title, rule.regex) {
                matched_all = false;
            }
        }
        if let Some(app_id_pattern) = &rule.match_app_id {
            matched_any = true;
            if !self.match_str(app_id_pattern, &info.app_id, rule.regex) {
                matched_all = false;
            }
        }

        matched_any && matched_all
    }

    fn match_str(&self, pattern: &str, value: &str, use_regex: bool) -> bool {
        if use_regex {
            // Простая regex-реализация через str::matches + wildcards.
            // Полноценный regex добавил бы зависимость regex crate.
            // Для MVP — поддерживаем '*' как wildcard.
            if pattern.contains('*') {
                self.wildcard_match(pattern, value)
            } else {
                value == pattern
            }
        } else {
            // Case-insensitive contains.
            value.to_lowercase().contains(&pattern.to_lowercase())
        }
    }

    /// Простой wildcard matcher: '*' = любая последовательность, '?' = один символ.
    fn wildcard_match(&self, pattern: &str, value: &str) -> bool {
        let p: Vec<char> = pattern.chars().collect();
        let v: Vec<char> = value.chars().collect();
        self.wildcard_recursive(&p, 0, &v, 0)
    }

    fn wildcard_recursive(&self, p: &[char], pi: usize, v: &[char], vi: usize) -> bool {
        if pi == p.len() { return vi == v.len(); }
        if p[pi] == '*' {
            // '*' matches zero or more chars.
            if pi + 1 == p.len() { return true; }
            for i in vi..=v.len() {
                if self.wildcard_recursive(p, pi + 1, v, i) { return true; }
            }
            false
        } else if vi < v.len() && (p[pi] == '?' || p[pi].eq_ignore_ascii_case(&v[vi])) {
            self.wildcard_recursive(p, pi + 1, v, vi + 1)
        } else {
            false
        }
    }
}

/// Cache: X11 window ID → Placement (для отслеживания уже размещённых окон).
pub struct PlacementCache {
    pub placed: HashMap<u32, Placement>,
}

impl PlacementCache {
    pub fn new() -> Self {
        PlacementCache { placed: HashMap::new() }
    }

    pub fn mark_placed(&mut self, xid: u32, placement: Placement) {
        self.placed.insert(xid, placement);
    }

    pub fn is_placed(&self, xid: u32) -> bool {
        self.placed.contains_key(&xid)
    }

    pub fn unplace(&mut self, xid: u32) {
        self.placed.remove(&xid);
    }
}

impl Default for PlacementCache {
    fn default() -> Self { Self::new() }
}
