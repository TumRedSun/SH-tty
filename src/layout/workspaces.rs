//! Workspaces — набор независимых layout-деревьев.
//!
//! Каждый workspace 1..9 имеет своё дерево тайлов. При переключении между
//! workspaces сохраняется фокус и положение каждого тайла.
//!
//! Окна можно перемещать между workspaces через Mod4+Shift+N.

use crate::layout::{Layout, LeafId, Direction, Node};
use std::collections::HashMap;

pub struct Workspaces {
    pub current: u8,
    pub layouts: HashMap<u8, Layout>,
    /// Метаданные тайлов, которые сохраняются при перемещении между workspaces:
    /// (workspace_n, leaf_id) → TileMeta.
    /// На практике: когда мы move_to_workspace, мы вынимаем leaf из текущего
    /// дерева и кладём в дерево целевого workspace.
    pub names: HashMap<u8, String>,
    pub max: u8,
}

impl Workspaces {
    pub fn new(max: u8, names: HashMap<u8, String>) -> Self {
        // Гарантируем минимум 1 workspace — иначе current=1 не найдётся в layouts
        // и current_layout() запаникует. max=0 возможен только при невалидном
        // конфиге (workspace_count=0), фиксим на 1 с предупреждением.
        let max = if max < 1 {
            log::warn!("workspace_count < 1 is invalid, defaulting to 1");
            1
        } else {
            max
        };
        let mut layouts = HashMap::new();
        for n in 1..=max {
            layouts.insert(n, Layout::new());
        }
        Workspaces { current: 1, layouts, names, max }
    }

    pub fn current_layout(&self) -> &Layout {
        self.layouts.get(&self.current).expect("missing current workspace")
    }

    pub fn current_layout_mut(&mut self) -> &mut Layout {
        self.layouts.get_mut(&self.current).expect("missing current workspace")
    }

    pub fn switch_to(&mut self, n: u8) {
        if n >= 1 && n <= self.max && n != self.current {
            self.current = n;
            log::info!("switched to workspace {} ({})", n, self.names.get(&n).map(|s| s.as_str()).unwrap_or(""));
        }
    }

    pub fn next(&mut self) {
        let n = if self.current == self.max { 1 } else { self.current + 1 };
        self.switch_to(n);
    }

    pub fn prev(&mut self) {
        let n = if self.current == 1 { self.max } else { self.current - 1 };
        self.switch_to(n);
    }

    /// Переносит активный тайл на другой workspace.
    /// Возвращает LeafId если перенос успешен.
    pub fn move_focused_to(&mut self, target_ws: u8) -> Option<LeafId> {
        if target_ws < 1 || target_ws > self.max || target_ws == self.current { return None; }
        let focused_id = self.current_layout().focused?;
        let kind = self.current_layout().focused_kind()?;
        // Удаляем leaf из текущего дерева.
        self.current_layout_mut().close_leaf(focused_id);

        // Открываем leaf с тем же ID в целевом workspace.
        // get_mut может вернуть None только если layouts не содержит target_ws,
        // что возможно при max=0 (невалидный конфиг) — возвращаем None вместо panic.
        let target = self.layouts.get_mut(&target_ws)?;

        match target.root.take() {
            None => {
                // Пустой target — создаём root leaf.
                target.root = Some(Node::leaf(focused_id, kind));
                target.focused = Some(focused_id);
            }
            Some(existing_root) => {
                // В target уже есть дерево. Если есть focused leaf — split его,
                // иначе — добавляем как новый root split.
                let cur_focused = target.focused;
                if let Some(cf) = cur_focused {
                    let owned_id = focused_id;
                    let owned_kind = kind;
                    let mut new_root = existing_root;
                    Layout::replace_leaf_external(&mut new_root, cf, move |old_leaf| {
                        Node::Split {
                            dir: Direction::Horizontal,
                            ratio: 0.5,
                            a: Box::new(old_leaf),
                            b: Box::new(Node::leaf(owned_id, owned_kind)),
                        }
                    });
                    target.root = Some(new_root);
                } else {
                    // No focused target — append as root split.
                    target.root = Some(Node::Split {
                        dir: Direction::Horizontal,
                        ratio: 0.5,
                        a: Box::new(existing_root),
                        b: Box::new(Node::leaf(focused_id, kind)),
                    });
                }
                target.focused = Some(focused_id);
            }
        }
        log::info!("moved tile {:?} from ws {} to ws {}", focused_id, self.current, target_ws);
        Some(focused_id)
    }
}

impl Layout {
    /// Публичная обёртка над replace_leaf для использования из Workspaces.
    pub fn replace_leaf_external<F>(node: &mut Node, target: LeafId, f: F)
    where F: FnOnce(Node) -> Node
    {
        Self::replace_leaf(node, target, f);
    }
}
