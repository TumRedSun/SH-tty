//! Тайловый layout manager в стиле i3/BSP.
//!
//! Дерево узлов: либо лист (terminal или X11), либо split (horizontal/vertical)
//! с двумя дочерними поддеревьями и ratio (0.05..0.95) разделения.
//!
//! Прямоугольник корневого узла = вся доступная область экрана.
//! Каждый узел вычисляет прямоугольники для детей на основе ratio.

pub mod workspaces;

use crate::ui::theme::{Color, Theme};

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct LeafId(pub u64);

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Direction { Horizontal, Vertical }

impl Direction {
    pub fn opposite(self) -> Self {
        match self { Direction::Horizontal => Direction::Vertical, Direction::Vertical => Direction::Horizontal }
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum FocusDir { Up, Down, Left, Right }

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum TileKind { Terminal, X11 }

#[derive(Debug, Clone)]
pub enum Node {
    Leaf {
        id: LeafId,
        kind: TileKind,
    },
    Split {
        dir: Direction,
        ratio: f32,
        a: Box<Node>,
        b: Box<Node>,
        // Какая из веток содержит сфокусированный лист (для пересчёта ratio).
        // Не храним фокус здесь — он в Layout::focused.
    },
}

impl Node {
    pub fn leaf(id: LeafId, kind: TileKind) -> Self { Node::Leaf { id, kind } }
}

#[derive(Debug, Clone, Copy)]
pub struct Rect { pub x: i32, pub y: i32, pub w: u32, pub h: u32 }

impl Rect {
    pub fn shrink(&self, px: i32) -> Rect {
        Rect {
            x: self.x + px,
            y: self.y + px,
            w: (self.w as i32 - 2 * px).max(0) as u32,
            h: (self.h as i32 - 2 * px).max(0) as u32,
        }
    }
}

pub struct Layout {
    pub root: Option<Node>,
    pub focused: Option<LeafId>,
    pub next_id: u64,
    pub fullscreen: Option<LeafId>,
    pub gap: i32,
    pub border: i32,
    pub padding_outer: i32,
}

impl Layout {
    pub fn new() -> Self {
        Layout {
            root: None,
            focused: None,
            next_id: 1,
            fullscreen: None,
            gap: 4,
            border: 1,
            padding_outer: 8,
        }
    }

    pub fn new_leaf_id(&mut self) -> LeafId {
        let id = LeafId(self.next_id);
        self.next_id += 1;
        id
    }

    /// Добавляет новую плитку в заданном направлении от текущего фокуса.
    /// Возвращает (id_new_leaf, id_parent_or_self_to_replace).
    pub fn open_tile(&mut self, kind: TileKind, split_dir: Direction) -> LeafId {
        let new_id = self.new_leaf_id();
        let new_leaf = Node::leaf(new_id, kind);

        match &mut self.root {
            None => {
                self.root = Some(new_leaf);
                self.focused = Some(new_id);
            }
            Some(root) => {
                // Находим текущий сфокусированный лист в дереве и заменяем
                // его на split (old_leaf | new_leaf).
                if let Some(focused) = self.focused {
                    let focused_owned = focused;
                    let new_leaf_owned = new_leaf;
                    let sd = split_dir;
                    Self::replace_leaf(root, focused_owned, move |old_leaf| {
                        Node::Split {
                            dir: sd,
                            ratio: 0.5,
                            a: Box::new(old_leaf),
                            b: Box::new(new_leaf_owned),
                        }
                    });
                }
                self.focused = Some(new_id);
            }
        }
        new_id
    }

    fn replace_leaf<F>(node: &mut Node, target: LeafId, f: F) -> bool
    where F: FnOnce(Node) -> Node
    {
        match node {
            Node::Leaf { id, .. } if *id == target => {
                let old = std::mem::replace(node, Node::leaf(LeafId(0), TileKind::Terminal));
                *node = f(old);
                true
            }
            Node::Leaf { .. } => false,
            Node::Split { a, b, .. } => {
                if Self::contains(a, target) {
                    Self::replace_leaf(a, target, f)
                } else if Self::contains(b, target) {
                    Self::replace_leaf(b, target, f)
                } else {
                    false
                }
            }
        }
    }

    /// Закрывает лист. Если у листа был сиблинг, заменяет родительский split
    /// на сиблинга. Если это был корень, обнуляет root.
    pub fn close_leaf(&mut self, id: LeafId) {
        let mut new_root = self.root.take();
        if let Some(root) = new_root.as_mut() {
            let _ = Self::remove_leaf(root, id);
        }
        // Если корень стал пустым листом — обнуляем.
        if let Some(Node::Leaf { id: LeafId(0), .. }) = new_root {
            new_root = None;
        }
        self.root = new_root;
        if self.focused == Some(id) {
            self.focused = self.root.as_ref().and_then(|n| Self::first_leaf(n));
        }
        if self.fullscreen == Some(id) {
            self.fullscreen = None;
        }
    }

    fn remove_leaf(node: &mut Node, target: LeafId) -> bool {
        match node {
            Node::Leaf { id, .. } if *id == target => {
                // Помечаем как удалённый (LeafId(0)).
                *node = Node::leaf(LeafId(0), TileKind::Terminal);
                true
            }
            Node::Leaf { .. } => false,
            Node::Split { a, b, .. } => {
                let a_removed = Self::remove_leaf(a, target);
                let b_removed = if !a_removed { Self::remove_leaf(b, target) } else { false };
                if a_removed || b_removed {
                    // Если одна из веток стала "нулевой" — заменяем весь split на другую.
                    let a_is_null = matches!(a.as_ref(), Node::Leaf { id: LeafId(0), .. });
                    let b_is_null = matches!(b.as_ref(), Node::Leaf { id: LeafId(0), .. });
                    if a_is_null {
                        let other = std::mem::replace(b.as_mut(), Node::leaf(LeafId(0), TileKind::Terminal));
                        *node = other;
                    } else if b_is_null {
                        let other = std::mem::replace(a.as_mut(), Node::leaf(LeafId(0), TileKind::Terminal));
                        *node = other;
                    }
                    true
                } else {
                    false
                }
            }
        }
    }

    fn first_leaf(node: &Node) -> Option<LeafId> {
        match node {
            Node::Leaf { id, .. } if id.0 != 0 => Some(*id),
            Node::Leaf { .. } => None,
            Node::Split { a, b, .. } => Self::first_leaf(a).or_else(|| Self::first_leaf(b))
        }
    }

    pub fn focus(&mut self, dir: FocusDir) {
        // Упрощённо: находим все листья в порядке, выбираем ближайший в направлении.
        if self.fullscreen.is_some() { return; }
        let Some(focused_id) = self.focused else { return; };
        let root = match &self.root { Some(r) => r, None => return };

        // Собираем (id, rect) для всех листьев.
        let root_rect = Rect { x: 0, y: 0, w: 1, h: 1 }; // относительный
        let leaves = Self::collect_leaves_relative(root, root_rect);
        let Some((_id, cur_rect)) = leaves.iter().find(|(id, _)| *id == focused_id).copied() else { return; };

        let next = leaves.iter()
            .filter(|(id, _)| *id != focused_id)
            .filter(|(_, r)| match dir {
                FocusDir::Left   => r.x + r.w as i32 <= cur_rect.x,
                FocusDir::Right  => r.x >= cur_rect.x + cur_rect.w as i32,
                FocusDir::Up     => r.y + r.h as i32 <= cur_rect.y,
                FocusDir::Down   => r.y >= cur_rect.y + cur_rect.h as i32,
            })
            .min_by_key(|(_, r)| {
                let cx = r.x + r.w as i32 / 2;
                let cy = r.y + r.h as i32 / 2;
                let my_x = cur_rect.x + cur_rect.w as i32 / 2;
                let my_y = cur_rect.y + cur_rect.h as i32 / 2;
                let dx = cx - my_x;
                let dy = cy - my_y;
                let primary = match dir {
                    FocusDir::Left => -dx, FocusDir::Right => dx,
                    FocusDir::Up => -dy, FocusDir::Down => dy,
                };
                let secondary = match dir {
                    FocusDir::Left | FocusDir::Right => dy.abs(),
                    FocusDir::Up | FocusDir::Down => dx.abs(),
                };
                primary * 1000 - secondary  // максимизируем primary, минимизируем secondary
            })
            .map(|(id, _)| *id);

        if let Some(next_id) = next {
            self.focused = Some(next_id);
        }
    }

    pub fn focus_cycle(&mut self) {
        let root = match &self.root { Some(r) => r, None => return };
        let leaves = Self::collect_leaves_ids(root);
        if leaves.is_empty() { return; }
        let cur = self.focused.unwrap_or(leaves[0]);
        let pos = leaves.iter().position(|&x| x == cur).unwrap_or(0);
        let next = leaves[(pos + 1) % leaves.len()];
        self.focused = Some(next);
    }

    pub fn resize_focused(&mut self, dir: FocusDir, delta: f32) {
        let Some(focused_id) = self.focused else { return; };
        let Some(root) = self.root.as_mut() else { return; };
        Self::resize_leaf_split(root, focused_id, dir, delta);
    }

    fn resize_leaf_split(node: &mut Node, target: LeafId, dir: FocusDir, delta: f32) {
        if let Node::Split { dir: ref split_dir, ratio, a, b } = node {
            // Сначала спускаемся — найдём split, где target в одной из веток.
            let a_has = Self::contains(a, target);
            let b_has = Self::contains(b, target);
            if a_has {
                let horizontal_match = matches!((split_dir, dir), (Direction::Horizontal, FocusDir::Right) | (Direction::Horizontal, FocusDir::Left));
                let vertical_match = matches!((split_dir, dir), (Direction::Vertical, FocusDir::Down) | (Direction::Vertical, FocusDir::Up));
                if horizontal_match || vertical_match {
                    let factor = if (a_has && matches!(dir, FocusDir::Left | FocusDir::Up))
                        || (b_has && matches!(dir, FocusDir::Right | FocusDir::Down))
                    { -delta } else { delta };
                    *ratio = (*ratio + factor).clamp(0.1, 0.9);
                }
                Self::resize_leaf_split(a, target, dir, delta);
            } else if b_has {
                Self::resize_leaf_split(b, target, dir, delta);
            }
        }
    }

    fn contains(node: &Node, target: LeafId) -> bool {
        match node {
            Node::Leaf { id, .. } => *id == target,
            Node::Split { a, b, .. } => Self::contains(a, target) || Self::contains(b, target),
        }
    }

    pub fn toggle_fullscreen(&mut self) {
        if let Some(fs) = self.fullscreen {
            if self.focused == Some(fs) {
                self.fullscreen = None;
                return;
            }
        }
        self.fullscreen = self.focused;
    }

    /// Возвращает список (LeafId, TileKind, Rect) для рендеринга.
    pub fn tile_rects(&self, screen: Rect) -> Vec<(LeafId, TileKind, Rect)> {
        let mut out = Vec::new();
        if let Some(fs) = self.fullscreen {
            if let Some(root) = &self.root {
                if let Some((_, kind)) = Self::find_leaf(root, fs) {
                    out.push((fs, kind, screen));
                    return out;
                }
            }
            return out;
        }
        if let Some(root) = &self.root {
            let inner = screen.shrink(self.padding_outer);
            Self::layout_node(root, inner, &mut out, self.gap, self.border);
        }
        out
    }

    fn find_leaf(node: &Node, id: LeafId) -> Option<(LeafId, TileKind)> {
        match node {
            Node::Leaf { id: lid, kind } if *lid == id => Some((*lid, *kind)),
            Node::Leaf { .. } => None,
            Node::Split { a, b, .. } => Self::find_leaf(a, id).or_else(|| Self::find_leaf(b, id)),
        }
    }

    fn layout_node(node: &Node, rect: Rect, out: &mut Vec<(LeafId, TileKind, Rect)>, gap: i32, border: i32) {
        match node {
            Node::Leaf { id, kind } if id.0 != 0 => {
                let inner = rect.shrink(border);
                out.push((*id, *kind, inner));
            }
            Node::Leaf { .. } => {} // удалённый
            Node::Split { dir, ratio, a, b } => {
                let (rect_a, rect_b) = split_rect(rect, *dir, *ratio, gap);
                Self::layout_node(a, rect_a, out, gap, border);
                Self::layout_node(b, rect_b, out, gap, border);
            }
        }
    }

    fn collect_leaves_relative(node: &Node, rect: Rect) -> Vec<(LeafId, Rect)> {
        let mut out = Vec::new();
        Self::collect_leaves_inner(node, rect, &mut out, 4, 1);
        out
    }

    fn collect_leaves_ids(node: &Node) -> Vec<LeafId> {
        let mut out = Vec::new();
        Self::collect_leaves_ids_inner(node, &mut out);
        out
    }

    fn collect_leaves_ids_inner(node: &Node, out: &mut Vec<LeafId>) {
        match node {
            Node::Leaf { id, .. } if id.0 != 0 => out.push(*id),
            Node::Leaf { .. } => {}
            Node::Split { a, b, .. } => {
                Self::collect_leaves_ids_inner(a, out);
                Self::collect_leaves_ids_inner(b, out);
            }
        }
    }

    fn collect_leaves_inner(node: &Node, rect: Rect, out: &mut Vec<(LeafId, Rect)>, gap: i32, border: i32) {
        match node {
            Node::Leaf { id, .. } if id.0 != 0 => {
                out.push((*id, rect.shrink(border)));
            }
            Node::Leaf { .. } => {}
            Node::Split { dir, ratio, a, b } => {
                let (ra, rb) = split_rect(rect, *dir, *ratio, gap);
                Self::collect_leaves_inner(a, ra, out, gap, border);
                Self::collect_leaves_inner(b, rb, out, gap, border);
            }
        }
    }

    pub fn focused_kind(&self) -> Option<TileKind> {
        let focused = self.focused?;
        let root = self.root.as_ref()?;
        Self::find_leaf(root, focused).map(|(_, k)| k)
    }

    pub fn all_leaf_ids(&self) -> Vec<LeafId> {
        match &self.root {
            Some(r) => {
                let mut v = Vec::new();
                Self::collect_leaves_ids_inner(r, &mut v);
                v
            }
            None => Vec::new(),
        }
    }

    /// Перемещает сфокусированный тайл в заданном направлении (меняет местами
    /// с соседним тайлом в направлении). Это эквивалент Mod4+Shift+H/J/K/L.
    pub fn move_focused(&mut self, dir: FocusDir) {
        let Some(focused_id) = self.focused else { return; };
        let Some(root) = self.root.as_mut() else { return; };
        let leaves = Self::collect_leaves_relative(root, Rect { x: 0, y: 0, w: 1, h: 1 });
        let Some((_id, cur_rect)) = leaves.iter().find(|(id, _)| *id == focused_id).copied() else { return; };
        let next = leaves.iter()
            .filter(|(id, _)| *id != focused_id)
            .filter(|(_, r)| match dir {
                FocusDir::Left   => r.x + r.w as i32 <= cur_rect.x,
                FocusDir::Right  => r.x >= cur_rect.x + cur_rect.w as i32,
                FocusDir::Up     => r.y + r.h as i32 <= cur_rect.y,
                FocusDir::Down   => r.y >= cur_rect.y + cur_rect.h as i32,
            })
            .min_by_key(|(_, r)| {
                let cx = r.x + r.w as i32 / 2;
                let cy = r.y + r.h as i32 / 2;
                let my_x = cur_rect.x + cur_rect.w as i32 / 2;
                let my_y = cur_rect.y + cur_rect.h as i32 / 2;
                let dx = cx - my_x;
                let dy = cy - my_y;
                let primary = match dir {
                    FocusDir::Left => -dx, FocusDir::Right => dx,
                    FocusDir::Up => -dy, FocusDir::Down => dy,
                };
                let secondary = match dir {
                    FocusDir::Left | FocusDir::Right => dy.abs(),
                    FocusDir::Up | FocusDir::Down => dx.abs(),
                };
                primary * 1000 - secondary
            })
            .map(|(id, _)| *id);
        if let Some(other) = next {
            // Меняем местами leaf IDs в дереве.
            Self::swap_leaf_ids(root, focused_id, other);
        }
    }

    /// Меняет местами два leaf-узла в дереве (по их ID).
    pub fn swap_focused(&mut self, dir: FocusDir) {
        // Аналог move_focused — фактически swap. Используется Mod4+Ctrl+HJKL.
        self.move_focused(dir);
    }

    fn swap_leaf_ids(node: &mut Node, a: LeafId, b: LeafId) {
        match node {
            Node::Leaf { id, .. } => {
                if *id == a { *id = b; }
                else if *id == b { *id = a; }
            }
            Node::Split { a: na, b: nb, .. } => {
                Self::swap_leaf_ids(na, a, b);
                Self::swap_leaf_ids(nb, a, b);
            }
        }
    }
}

fn split_rect(rect: Rect, dir: Direction, ratio: f32, gap: i32) -> (Rect, Rect) {
    match dir {
        Direction::Horizontal => {
            // Делим по вертикали: A слева, B справа.
            let total_w = rect.w as i32;
            let a_w = (((total_w as f32 * ratio) - gap as f32 / 2.0).max(1.0)) as i32;
            let b_w = (total_w - a_w - gap).max(1);
            let a = Rect { x: rect.x, y: rect.y, w: a_w as u32, h: rect.h };
            let b = Rect { x: rect.x + a_w + gap, y: rect.y, w: b_w as u32, h: rect.h };
            (a, b)
        }
        Direction::Vertical => {
            let total_h = rect.h as i32;
            let a_h = (((total_h as f32 * ratio) - gap as f32 / 2.0).max(1.0)) as i32;
            let b_h = (total_h - a_h - gap).max(1);
            let a = Rect { x: rect.x, y: rect.y, w: rect.w, h: a_h as u32 };
            let b = Rect { x: rect.x, y: rect.y + a_h + gap, w: rect.w, h: b_h as u32 };
            (a, b)
        }
    }
}

/// Возвращает цвет бордера для тайла в зависимости от состояния.
pub fn border_color_for(tile_kind: TileKind, focused: bool, theme: &Theme) -> Color {
    match (tile_kind, focused) {
        (TileKind::Terminal, true)  => theme.border_active,
        (TileKind::Terminal, false) => theme.border_inactive,
        (TileKind::X11, true)       => theme.border_x11,
        (TileKind::X11, false)      => Color(
            theme.border_x11.0 / 2,
            theme.border_x11.1 / 2,
            theme.border_x11.2 / 2,
        ),
    }
}
