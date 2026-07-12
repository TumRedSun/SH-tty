//! MCD (Mind Control Delete) neon palette.
//!
//! Базовый фон — глубокий фиолетовый, акценты — неоновый циан и магента.
//! Глитч-эффекты достигаются смещением слоёв с разной альфой.

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct Color(pub u8, pub u8, pub u8);

impl Color {
    pub const fn rgb(r: u8, g: u8, b: u8) -> Self { Color(r, g, b) }
    pub const fn as_u32(&self, fmt: PixelFmt) -> u32 {
        match fmt {
            PixelFmt::Xrgb8888 | PixelFmt::Argb8888 =>
                ((self.0 as u32) << 16) | ((self.1 as u32) << 8) | (self.2 as u32),
            PixelFmt::Bgrx8888 =>
                ((self.2 as u32) << 16) | ((self.1 as u32) << 8) | (self.0 as u32),
            PixelFmt::Rgb565 =>
                (((self.0 as u32 >> 3) << 11) | ((self.1 as u32 >> 2) << 5) | (self.2 as u32 >> 3)),
        }
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum PixelFmt {
    Xrgb8888,
    Argb8888,
    Bgrx8888,
    Rgb565,
}

impl PixelFmt {
    pub fn bytes_per_pixel(&self) -> usize {
        match self { PixelFmt::Rgb565 => 2, _ => 4 }
    }
}

/// Палитра MCD.
pub struct Theme {
    // Базовый фон экрана — глубокий ночной фиолет.
    pub bg: Color,
    // Фон неактивной плитки — чуть темнее основного.
    pub tile_bg_inactive: Color,
    // Фон активной плитки — почти чёрный с фиолетовым отливом.
    pub tile_bg_active: Color,
    // Бордер неактивной плитки — тусклый циан.
    pub border_inactive: Color,
    // Бордер активной плитки — неоновая магента.
    pub border_active: Color,
    // Бордер плитки с X11-окном — неоновый циан.
    pub border_x11: Color,
    // Текст терминала по умолчанию — холодный белый.
    pub fg_default: Color,
    // Текст тусклый — для non-focused.
    pub fg_dim: Color,
    // Цвет акцента — неоновая магента (как «CORE» в MCD).
    pub accent_magenta: Color,
    // Цвет акцента — неоновый циан (как «HACK» в MCD).
    pub accent_cyan: Color,
    // Цвет popup фона.
    pub popup_bg: Color,
    // Цвет popup границы.
    pub popup_border: Color,
    // Цвет ошибки/предупреждения — горячий красный.
    pub error: Color,
}

impl Default for Theme {
    fn default() -> Self {
        Theme {
            bg:                Color(0x0A, 0x07, 0x16),
            tile_bg_inactive:  Color(0x12, 0x0E, 0x24),
            tile_bg_active:    Color(0x0F, 0x0A, 0x1E),
            border_inactive:   Color(0x3A, 0x2D, 0x5C),
            border_active:     Color(0xFF, 0x2E, 0x97),
            border_x11:        Color(0x00, 0xF0, 0xFF),
            fg_default:        Color(0xE6, 0xE1, 0xF0),
            fg_dim:            Color(0x7A, 0x6F, 0x96),
            accent_magenta:    Color(0xFF, 0x2E, 0x97),
            accent_cyan:       Color(0x00, 0xF0, 0xFF),
            popup_bg:          Color(0x14, 0x0B, 0x2E),
            popup_border:      Color(0xFF, 0x2E, 0x97),
            error:             Color(0xFF, 0x4D, 0x4D),
        }
    }
}

/// ANSI 16-color palette, mapped to MCD-coherent hues.
pub const ANSI_PALETTE: [Color; 16] = [
    Color(0x12, 0x0E, 0x24), // black
    Color(0xFF, 0x2E, 0x97), // red — magenta
    Color(0x00, 0xF0, 0xFF), // green — cyan
    Color(0xFF, 0xD1, 0x66), // yellow — warm yellow
    Color(0x6A, 0x4C, 0xFF), // blue — electric indigo
    Color(0xC5, 0x4B, 0xE8), // magenta — purple
    Color(0x4D, 0xD0, 0xE1), // cyan — pale cyan
    Color(0xE6, 0xE1, 0xF0), // white
    Color(0x7A, 0x6F, 0x96), // bright black (dim grey)
    Color(0xFF, 0x5C, 0xB0), // bright red
    Color(0x66, 0xF5, 0xFF), // bright green
    Color(0xFF, 0xE0, 0x99), // bright yellow
    Color(0x8C, 0x77, 0xFF), // bright blue
    Color(0xD7, 0x84, 0xF0), // bright magenta
    Color(0x9F, 0xE5, 0xF0), // bright cyan
    Color(0xFF, 0xFF, 0xFF), // bright white
];
