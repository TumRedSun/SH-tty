//! PSF (PC Screen Font) + TTF font loader — загрузка шрифта для рендеринга терминала.
//!
//! ## Стратегия загрузки (v3)
//!
//! 1. **TTF через freetype** (основной путь):
//!    - WM находит системный monospace TTF через `fc-match monospace:spacing=100`
//!    - Через freetype пререндерит все codepoints из диапазона 0..=0xFFFF (BMP)
//!      в PSF-совместимую bitmap структуру (1 bit per pixel, packed).
//!    - Даёт полное Unicode покрытие: Cyrillic, Greek, CJK (если в шрифте),
//!      math symbols, box-drawing, Powerline symbols.
//!    - Типичный TTF (DejaVu Sans Mono) имеет ~3000 glyphs против 256-1000 у PSF.
//!
//! 2. **PSF fallback** — если freetype недоступен или TTF не найден:
//!    - Динамическое сканирование /usr/share/kbd/consolefonts/ и т.д.
//!    - Scoring по покрытию: Cyrillic > box-drawing > blocks > Greek > Powerline.
//!
//! 3. **User override** — файл в /etc/superhot-tty/:
//!    - `font.ttf` / `font.otf` — TTF через freetype
//!    - `font.psfu` / `font.psf` — PSF bitmap
//!
//! Если ничего не найдено — процедурный встроенный шрифт 8x16 (ASCII only).
//!
//! ## Зачем TTF вместо PSF
//!
//! PSF шрифты — bitmap шрифты фиксированного размера, ограничены 256-1024 glyphs.
//! Даже ter-u16n.psfu.gz (terminus-font) не покрывает многие Unicode диапазоны:
//!   - Нет emoji
//!   - Нет Nerd Font иконок
//!   - Ограниченный набор математических символов
//!   - Нет CJK (китайские/японские/корейские)
//!
//! TTF шрифты через freetype:
//!   - Поддерживают любой Unicode codepoint, который есть в шрифте
//!   - Hinting для лучшей читаемости
//!   - Любой размер (8px-32px)
//!   - Используют системные TTF (DejaVu, JetBrains Mono, Source Code Pro, etc.)
//!
//! ## Рекомендуемые TTF пакеты
//!
//!   Arch:    sudo pacman -S ttf-dejavu ttf-liberation
//!            sudo pacman -S ttf-jetbrains-mono ttf-nerd-fonts-symbols
//!   Debian:  sudo apt install fonts-dejavu fonts-liberation
//!   Fedora:  sudo dnf install dejavu-sans-mono-fonts
//!
//! ВАЖНО: PSF2-шрифты с unicode table (флаг PSF2_HAS_UNICODE_TABLE) содержат
//! секцию после glyphs, которая маппит Unicode codepoints на glyph indices.
//! Раньше мы это игнорировали и трактовали codepoint как прямой индекс —
//! из-за этого box-drawing chars (─ │ ┌ ┐ └ ┘, U+2500-U+257F) и Cyrillic
//! (U+0400-U+04FF) рендерились как мусорные символы (выглядело как "e с ~").
//! Теперь unicode table парсится в HashMap<u32, u32> для корректного lookup'а.

use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use anyhow::Context;

#[derive(Debug, Clone)]
pub struct Font {
    pub width: u32,
    pub height: u32,
    pub glyph_count: u32,
    pub bytes_per_glyph: u32,
    pub glyphs: Vec<u8>,
    pub has_unicode_table: bool,
    /// Codepoint → glyph index. Empty if font has no unicode table.
    /// Строится один раз при загрузке шрифта, используется в glyph_for().
    /// Для шрифтов без unicode table остаётся пустой — glyph_for использует
    /// legacy logic (cp < 0x80 → direct, иначе cp_to_index fallback).
    unicode_map: HashMap<u32, u32>,
}

impl Font {
    /// Загружает PSF2-шрифт из сырых байтов.
    pub fn from_psf2(data: &[u8]) -> anyhow::Result<Self> {
        if data.len() < 32 || &data[0..4] != &[0x72, 0xb5, 0x4a, 0x86] {
            anyhow::bail!("not a PSF2 font");
        }
        let version = u32::from_le_bytes(data[4..8].try_into().unwrap());
        if version != 0 { anyhow::bail!("unsupported PSF2 version {}", version); }
        let headersize   = u32::from_le_bytes(data[8..12].try_into().unwrap());
        let flags        = u32::from_le_bytes(data[12..16].try_into().unwrap());
        let length       = u32::from_le_bytes(data[16..20].try_into().unwrap());
        let _charsize   = u32::from_le_bytes(data[20..24].try_into().unwrap());
        let height       = u32::from_le_bytes(data[24..28].try_into().unwrap());
        let width        = u32::from_le_bytes(data[28..32].try_into().unwrap());

        let bytes_per_row = (width + 7) / 8;
        let bytes_per_glyph = bytes_per_row * height;
        let glyphs_len = (length * bytes_per_glyph) as usize;
        let glyphs_end = headersize as usize + glyphs_len;
        if data.len() < glyphs_end {
            anyhow::bail!("PSF2 truncated: need {} bytes, have {}", glyphs_end, data.len());
        }
        let glyphs = data[headersize as usize..glyphs_end].to_vec();
        let has_unicode_table = flags & 0x01 != 0;

        // Парсим unicode table если она есть. Без этого коды выше 0xFF
        // (включая UTF-8 русские/box-drawing) будут трактоваться как прямой
        // glyph index, что даёт мусор на экране.
        let unicode_map = if has_unicode_table {
            parse_psf2_unicode_table(&data[glyphs_end..], length)
        } else {
            HashMap::new()
        };

        Ok(Font {
            width, height,
            glyph_count: length,
            bytes_per_glyph,
            glyphs,
            has_unicode_table,
            unicode_map,
        })
    }

    /// PSF1 (magic 0x36 0x04).
    pub fn from_psf1(data: &[u8]) -> anyhow::Result<Self> {
        if data.len() < 4 || data[0] != 0x36 || data[1] != 0x04 {
            anyhow::bail!("not a PSF1 font");
        }
        let mode = data[2];
        let charsize = data[3] as u32;
        let height = charsize;
        let width = 8u32;
        let bytes_per_glyph = charsize;
        let length: u32 = if mode & 0x01 != 0 { 512 } else { 256 };
        let glyphs_len = (length * bytes_per_glyph) as usize;
        if data.len() < 4 + glyphs_len { anyhow::bail!("PSF1 truncated"); }
        let glyphs = data[4..4 + glyphs_len].to_vec();
        let has_unicode_table = mode & 0x02 != 0;
        let unicode_map = if has_unicode_table {
            parse_psf1_unicode_table(&data[4 + glyphs_len..], length)
        } else {
            HashMap::new()
        };
        Ok(Font {
            width, height,
            glyph_count: length,
            bytes_per_glyph,
            glyphs,
            has_unicode_table,
            unicode_map,
        })
    }

    pub fn from_bytes(data: &[u8]) -> anyhow::Result<Self> {
        if data.len() >= 4 && data[0..2] == [0x36, 0x04] {
            Self::from_psf1(data)
        } else if data.len() >= 4 && data[0..4] == [0x72, 0xb5, 0x4a, 0x86] {
            Self::from_psf2(data)
        } else {
            anyhow::bail!("unknown font format")
        }
    }

    /// Пробует стандартные пути и возвращает загруженный шрифт.
    ///
    /// Стратегия (v3): TTF через freetype → PSF fallback.
    ///
    /// 1. TTF через freetype + fontconfig. WM находит системный monospace TTF
    ///    (DejaVu Sans Mono, JetBrains Mono, etc.) через `fc-match` и пререндерит
    ///    все BMP codepoints в PSF-совместимую структуру. Это даёт полное Unicode
    ///    покрытие: Cyrillic, Greek, CJK (если в шрифте), math symbols, box-drawing.
    ///    TTF предпочтительнее PSF — у TTF шрифтов обычно 2000-3000 glyphs против
    ///    256-1000 у PSF.
    ///
    /// 2. PSF fallback — если freetype недоступен или TTF не найден, используем
    ///    динамическое сканирование каталогов консольных шрифтов (как в v2).
    pub fn load_default() -> Self {
        // 0. User override через /etc/superhot-tty/font.ttf — TTF файл.
        // Если есть — используем freetype для рендеринга.
        for user_path in ["/etc/superhot-tty/font.ttf", "/etc/superhot-tty/font.otf"] {
            if std::path::Path::new(user_path).exists() {
                match Self::from_ttf(user_path, 16) {
                    Ok(f) => {
                        log::info!(
                            "loaded user TTF font from {} ({}x{} glyphs={} cyrillic={} box_drawing={})",
                            user_path, f.width, f.height, f.glyph_count,
                            f.has_cyrillic(), f.has_box_drawing()
                        );
                        return f;
                    }
                    Err(e) => {
                        log::warn!("failed to load user TTF {}: {} — falling back", user_path, e);
                    }
                }
            }
        }

        // 1. User override через /etc/superhot-tty/font.psfu — PSF файл (legacy).
        for user_path in ["/etc/superhot-tty/font.psfu", "/etc/superhot-tty/font.psf"] {
            if let Ok(data) = load_maybe_gz(user_path) {
                if let Ok(f) = Self::from_bytes(&data) {
                    log::info!(
                        "loaded user PSF font from {} ({}x{} glyphs={} cyrillic={} box_drawing={})",
                        user_path, f.width, f.height, f.glyph_count,
                        f.has_cyrillic(), f.has_box_drawing()
                    );
                    return f;
                }
            }
        }

        // 2. TTF через fontconfig — основной путь. Ищем системный monospace TTF.
        match find_ttf_via_fontconfig() {
            Some((path, family)) => {
                log::info!("fontconfig selected TTF: {} ({})", path, family);
                match Self::from_ttf(&path, 16) {
                    Ok(f) => {
                        log::info!(
                            "loaded TTF font from {} ({}x{} glyphs={} cyrillic={} box_drawing={} greek={} powerline={})",
                            path, f.width, f.height, f.glyph_count,
                            f.has_cyrillic(), f.has_box_drawing(), f.has_greek(), f.has_powerline()
                        );
                        if !f.has_cyrillic() {
                            log::warn!("TTF font does NOT contain Cyrillic — install a font with Cyrillic coverage");
                            log::warn!("  Arch:    sudo pacman -S ttf-dejavu   (or any ttf-* package)");
                        }
                        return f;
                    }
                    Err(e) => {
                        log::warn!("failed to load TTF {}: {} — falling back to PSF", path, e);
                    }
                }
            }
            None => {
                log::warn!("fontconfig (fc-match) not available or returned nothing — falling back to PSF");
            }
        }

        // 3. PSF fallback — dynamic scan of console font directories.
        Self::load_psf_fallback()
    }

    /// PSF fallback: динамическое сканирование каталогов консольных шрифтов.
    /// Используется если TTF недоступен. Логика идентична v2 load_default().
    fn load_psf_fallback() -> Self {
        let mut candidates: Vec<(PathBuf, Vec<u8>)> = Vec::new();

        const PRIORITY_PATHS: &[&str] = &[
            "/usr/share/kbd/consolefonts/ter-u16n.psfu.gz",
            "/usr/share/kbd/consolefonts/ter-u20n.psfu.gz",
            "/usr/share/kbd/consolefonts/ter-u24n.psfu.gz",
            "/usr/share/kbd/consolefonts/ter-u28n.psfu.gz",
            "/usr/share/kbd/consolefonts/ter-u32n.psfu.gz",
            "/usr/share/kbd/consolefonts/Uni3-Terminus16.psfu.gz",
            "/usr/share/kbd/consolefonts/Uni3-Fixed16.psfu.gz",
            "/usr/share/kbd/consolefonts/UniCox_14.psfu.gz",
            "/usr/share/kbd/consolefonts/UniCortex_14.psfu.gz",
            "/usr/share/kbd/consolefonts/UniFont-Terminus16.psfu.gz",
        ];
        for path in PRIORITY_PATHS {
            if let Ok(data) = load_maybe_gz(path) {
                candidates.push((PathBuf::from(path), data));
            }
        }

        const SCAN_DIRS: &[&str] = &[
            "/usr/share/kbd/consolefonts",
            "/usr/share/consolefonts",
            "/usr/lib/kbd/consolefonts",
            "/lib/kbd/consolefonts",
        ];
        for dir in SCAN_DIRS {
            let Ok(entries) = fs::read_dir(dir) else { continue };
            for entry in entries.flatten() {
                let path = entry.path();
                let Some(fname) = path.file_name().and_then(|n| n.to_str()) else { continue };
                let is_psf = fname.ends_with(".psfu.gz")
                    || fname.ends_with(".psfu")
                    || fname.ends_with(".psf.gz")
                    || fname.ends_with(".psf");
                if !is_psf { continue; }
                if candidates.iter().any(|(p, _)| p == &path) { continue; }
                if let Ok(data) = load_maybe_gz(path.to_str().unwrap_or("")) {
                    candidates.push((path, data));
                }
            }
        }

        log::debug!("PSF fallback: {} candidates found", candidates.len());

        let mut best: Option<(PathBuf, Self, (u32, u32, u32, u32, u32, u32, u32))> = None;
        for (path, data) in &candidates {
            let Ok(f) = Self::from_bytes(data) else { continue };
            let score = (
                f.has_cyrillic() as u32,
                f.has_box_drawing() as u32,
                f.has_block_elements() as u32,
                f.has_greek() as u32,
                f.has_powerline() as u32,
                f.has_unicode_table as u32,
                f.glyph_count,
            );
            log::debug!(
                "PSF candidate {}: {}x{} glyphs={} uni_table={} cyrillic={} box={} blk={} greek={} pwr={} score={:?}",
                path.display(), f.width, f.height, f.glyph_count, f.has_unicode_table,
                f.has_cyrillic(), f.has_box_drawing(), f.has_block_elements(),
                f.has_greek(), f.has_powerline(), score
            );
            let better = match &best {
                None => true,
                Some((_, _, bs)) => score > *bs,
            };
            if better {
                best = Some((path.clone(), f, score));
            }
        }

        if let Some((path, f, score)) = best {
            let has_cyr = f.has_cyrillic();
            log::info!(
                "loaded PSF font from {} ({}x{} glyphs={} unicode_table={} cyrillic={} box_drawing={} greek={} powerline={} score={:?})",
                path.display(), f.width, f.height, f.glyph_count, f.has_unicode_table,
                f.has_cyrillic(), f.has_box_drawing(), f.has_greek(), f.has_powerline(), score
            );
            if !has_cyr {
                log::warn!("PSF font does NOT contain Cyrillic glyphs — Russian text will render as '?'");
                log::warn!("install a Unicode-capable font package to fix:");
                log::warn!("  Arch:    sudo pacman -S terminus-font   (provides ter-u16n.psfu.gz)");
                log::warn!("  Debian:  sudo apt install fonts-terminus console-setup");
                log::warn!("  Fedora:  sudo dnf install terminus-fonts-pcf");
                log::warn!("or copy a .psfu font to /etc/superhot-tty/font.psfu");
            }
            return f;
        }

        log::warn!("no system PSF font found — using builtin fallback 8x16 (NO Cyrillic, NO box-drawing)");
        Self::builtin_8x16()
    }

    /// Загружает TTF/OTF шрифт через freetype и пререндерит все codepoints из
    /// диапазона 0..=0xFFFF (BMP) в PSF-совместимую bitmap структуру.
    ///
    /// Это позволяет WM рендерить ЛЮБОЙ Unicode символ, который есть в шрифте —
    /// кириллица, греческий, box-drawing, математические операторы, и т.д.
    /// TTF предпочтительнее PSF — у TTF обычно 2000-3000+ glyphs против 256-1000
    /// у PSF, плюс TTF поддерживает любые размеры и hinting.
    ///
    /// `pixel_height` — желаемая высота глифа в пикселях (16, 20, 24).
    /// Ширина подбирается автоматически по advance width (для monospace это
    /// одинаково для всех глифов).
    pub fn from_ttf(path: &str, pixel_height: u32) -> anyhow::Result<Self> {
        use freetype::Library;
        use freetype::face::LoadFlag;

        let lib = Library::init()
            .context("freetype Library::init failed — is libfreetype installed?")?;
        let face = lib.new_face(path, 0)
            .with_context(|| format!("failed to open TTF face: {}", path))?;

        // Set pixel size. Freetype uses 26.6 fixed point internally.
        // set_char_size(width, height, h_res, v_res) — width=0 means auto.
        let char_size = (pixel_height * 64) as isize;
        face.set_char_size(0, char_size, 0, 0)
            .with_context(|| format!("set_char_size({}px) failed for {}", pixel_height, path))?;

        // Determine target glyph width from advance of common monospace chars.
        // For monospace fonts, all chars have same advance.
        let target_width = determine_ttf_width(&face);
        let bytes_per_row = ((target_width + 7) / 8) as usize;
        let bytes_per_glyph = bytes_per_row * pixel_height as usize;

        log::debug!(
            "TTF {}: target_width={}px height={}px bytes_per_glyph={}",
            path, target_width, pixel_height, bytes_per_glyph
        );

        // Pre-allocate for full BMP (65536 codepoints).
        // Total memory: 65536 * bytes_per_glyph (e.g., 65536 * 32 = 2MB for 16x16).
        // Acceptable — startup cost is ~50-200ms for typical fonts with ~3000 glyphs.
        let glyph_capacity = 0x10000usize;
        let mut glyphs = vec![0u8; glyph_capacity * bytes_per_glyph];
        let mut unicode_map: HashMap<u32, u32> = HashMap::new();
        let mut max_idx: u32 = 0;
        let mut rendered_count = 0u32;
        let mut failed_count = 0u32;

        // Iterate all BMP codepoints. For each:
        //   - Try to load the glyph via freetype.
        //   - If glyph exists (char_index != 0), render it as monochrome bitmap.
        //   - Copy bitmap into PSF-packed format with proper vertical centering.
        //   - Map codepoint → glyph index (direct: cp = idx for BMP).
        for cp in 0u32..=0xFFFF {
            // Fast path: check if char exists in font via get_char_index.
            // This avoids the expensive load_char for undefined codepoints.
            // get_char_index returns Option<u32> — None means char not in font.
            if face.get_char_index(cp as usize).is_none() {
                continue; // Codepoint not in font — leave as empty glyph (zeros).
            }

            // Load + render. MONOCHROME produces 1-bit-per-pixel packed bitmap
            // (compatible with PSF format). RENDER forces rasterization.
            let load_result = face.load_char(cp as usize, LoadFlag::RENDER | LoadFlag::MONOCHROME);
            if load_result.is_err() {
                failed_count += 1;
                continue;
            }

            let glyph = face.glyph();
            let bitmap = glyph.bitmap();
            let bm_width = bitmap.width() as usize;
            let bm_rows = bitmap.rows() as usize;
            let bm_left = glyph.bitmap_left();
            let bm_top = glyph.bitmap_top();
            let buffer = bitmap.buffer();
            let pitch = bitmap.pitch().unsigned_abs() as usize;

            // Use codepoint as glyph index (direct mapping, since we have 65536 slots).
            let idx = cp;
            let glyph_off = idx as usize * bytes_per_glyph;
            if glyph_off + bytes_per_glyph > glyphs.len() {
                continue; // Shouldn't happen, but be safe.
            }

            // Empty glyph (e.g., space) — leave as zeros, but still map it.
            if buffer.is_empty() || bm_width == 0 || bm_rows == 0 {
                unicode_map.insert(cp, idx);
                if idx > max_idx { max_idx = idx; }
                rendered_count += 1;
                continue;
            }

            // Vertical centering: align glyph baseline with cell baseline.
            // For a cell of height H, baseline is typically at row H-3 (with
            // 3px descent for chars like 'g', 'p', 'y').
            let baseline = pixel_height as i32 - 3;
            let y_offset = baseline - bm_top;

            // Horizontal centering: place glyph at bm_left offset (already
            // provided by freetype), but clamp to cell width.
            let x_offset = bm_left.max(0).min(target_width as i32 - 1);

            // Copy freetype's monochrome bitmap into our PSF-packed buffer.
            // Both formats are 1bpp MSB-first, but freetype's pitch may be
            // larger (padded to 32-bit) while PSF packs to byte boundaries.
            for row in 0..bm_rows {
                let target_row = row as i32 + y_offset;
                if target_row < 0 || target_row >= pixel_height as i32 { continue; }
                let src_row_off = row * pitch;
                let dst_row_off = glyph_off + (target_row as usize) * bytes_per_row;
                if dst_row_off + bytes_per_row > glyphs.len() { break; }

                // Copy bits, respecting horizontal offset and cell width.
                for col in 0..(bm_width as i32) {
                    let target_col = col + x_offset;
                    if target_col < 0 || target_col >= target_width as i32 { continue; }
                    let src_byte_off = src_row_off + (col as usize) / 8;
                    if src_byte_off >= buffer.len() { break; }
                    let src_bit = 7 - ((col as usize) % 8);
                    let set = (buffer[src_byte_off] >> src_bit) & 1 == 1;
                    if set {
                        let dst_byte_off = dst_row_off + (target_col as usize) / 8;
                        let dst_bit = 7 - ((target_col as usize) % 8);
                        if dst_byte_off < glyphs.len() {
                            glyphs[dst_byte_off] |= 1 << dst_bit;
                        }
                    }
                }
            }

            unicode_map.insert(cp, idx);
            if idx > max_idx { max_idx = idx; }
            rendered_count += 1;
        }

        // Ensure '?' (U+003F) is mapped — it's the fallback in glyph_for().
        if !unicode_map.contains_key(&(b'?' as u32)) {
            // Draw a simple '?' glyph manually if font doesn't have it.
            // (Extremely unlikely for any real font, but be safe.)
            let q_off = (b'?' as u32) as usize * bytes_per_glyph;
            if q_off + bytes_per_glyph <= glyphs.len() {
                // Top arc + descender — minimal '?' shape.
                let mid = bytes_per_row / 2;
                glyphs[q_off + 0 * bytes_per_row + mid] = 0x3C;
                glyphs[q_off + 1 * bytes_per_row + mid] = 0x42;
                glyphs[q_off + 2 * bytes_per_row + mid] = 0x02;
                glyphs[q_off + 3 * bytes_per_row + mid] = 0x04;
                glyphs[q_off + 4 * bytes_per_row + mid] = 0x08;
                glyphs[q_off + 5 * bytes_per_row + mid] = 0x08;
                glyphs[q_off + 7 * bytes_per_row + mid] = 0x08;
            }
            unicode_map.insert(b'?' as u32, b'?' as u32);
            if (b'?' as u32) > max_idx { max_idx = b'?' as u32; }
        }

        log::info!(
            "TTF rendered: {} glyphs ({} failed) from {}, max_idx={}, cell={}x{}, mem={}KB",
            rendered_count, failed_count, path, max_idx, target_width, pixel_height,
            (glyphs.len() + 1023) / 1024
        );

        // Trim glyphs vec to actual used size to save memory.
        // We keep slots 0..=max_idx, drop the rest.
        let trimmed_len = (max_idx as usize + 1) * bytes_per_glyph;
        glyphs.truncate(trimmed_len);

        Ok(Font {
            width: target_width,
            height: pixel_height,
            glyph_count: max_idx + 1,
            bytes_per_glyph: bytes_per_glyph as u32,
            glyphs,
            has_unicode_table: true,
            unicode_map,
        })
    }

    /// Проверяет, покрывает ли шрифт базовый Cyrillic диапазон (U+0410–U+044F).
    /// Это заглавные и строчные русские буквы. Без этого русский текст рендерится как '?'.
    pub fn has_cyrillic(&self) -> bool {
        [0x0410, 0x0411, 0x0412, 0x0415, 0x041F, 0x0420, 0x0430, 0x0435, 0x043F, 0x0440]
            .iter()
            .filter(|cp| self.unicode_map.contains_key(cp))
            .count() >= 5
    }

    /// Проверяет, покрывает ли шрифт box-drawing диапазон (U+2500–U+257F).
    /// Нужен для htop, btop, ncurses UI, рамок вокруг окон.
    pub fn has_box_drawing(&self) -> bool {
        [0x2500, 0x2502, 0x250C, 0x2510, 0x2514, 0x2518, 0x251C, 0x2524, 0x252C, 0x2534]
            .iter()
            .filter(|cp| self.unicode_map.contains_key(cp))
            .count() >= 5
    }

    /// Проверяет, покрывает ли шрифт block elements (U+2580–U+259F).
    /// ▀ ▄ █ ▒ ▓ — для прогресс-баров и заливки.
    pub fn has_block_elements(&self) -> bool {
        [0x2580, 0x2584, 0x2588, 0x258C, 0x2590, 0x2591, 0x2592, 0x2593]
            .iter()
            .filter(|cp| self.unicode_map.contains_key(cp))
            .count() >= 4
    }

    /// Проверяет, покрывает ли шрифт греческий алфавит (U+0391–U+03C9).
    pub fn has_greek(&self) -> bool {
        [0x0391, 0x0392, 0x0395, 0x03A0, 0x03A3, 0x03B1, 0x03B5, 0x03C0]
            .iter()
            .filter(|cp| self.unicode_map.contains_key(cp))
            .count() >= 4
    }

    /// Проверяет, покрывает ли шрифт Powerline symbols (U+E0A0–U+E0D4).
    pub fn has_powerline(&self) -> bool {
        [0xE0A0, 0xE0A1, 0xE0A2, 0xE0B0, 0xE0B2, 0xE0B3, 0xE0D4]
            .iter()
            .any(|cp| self.unicode_map.contains_key(cp))
    }

    /// Возвращает bitmap глифа для codepoint `cp`.
    /// Для шрифтов с unicode table — lookup через unicode_map.
    /// Для шрифтов без unicode table — legacy logic (ASCII direct, Cyrillic hardcoded).
    /// Для неизвестных codepoints — glyph для '?' (или последний glyph как fallback).
    pub fn glyph_for(&self, cp: u32) -> &[u8] {
        let idx = if !self.unicode_map.is_empty() {
            // Шрифт с unicode table — используем её для корректного lookup'а.
            // Если codepoint не найден — fallback на '?'.
            self.unicode_map.get(&cp).copied()
                .or_else(|| if cp < 0x80 { Some(cp) } else { None })
                .unwrap_or(b'?' as u32)
                .min(self.glyph_count.saturating_sub(1))
        } else if !self.has_unicode_table {
            // Legacy: no unicode table — direct ASCII + hardcoded Cyrillic.
            if cp < 0x80 {
                cp
            } else {
                self.cp_to_index(cp).unwrap_or(b'?' as u32)
            }.min(self.glyph_count.saturating_sub(1))
        } else {
            // has_unicode_table = true но map пуста (parse failed) — fallback.
            (cp as usize).min(self.glyph_count as usize - 1) as u32
        };
        let off = (idx * self.bytes_per_glyph) as usize;
        let end = off + self.bytes_per_glyph as usize;
        if end > self.glyphs.len() {
            // Out of bounds — return empty glyph (avoids panic on malformed font).
            return &self.glyphs[..(self.bytes_per_glyph as usize).min(self.glyphs.len())];
        }
        &self.glyphs[off..end]
    }

    fn cp_to_index(&self, cp: u32) -> Option<u32> {
        if      (0x0410..=0x042F).contains(&cp) { Some(cp - 0x0410 + 0x80) }
        else if (0x0430..=0x044F).contains(&cp) { Some(cp - 0x0430 + 0xA0) }
        else if cp == 0x0401 { Some(0xF0) }
        else if cp == 0x0451 { Some(0xF1) }
        else { None }
    }

    /// Процедурно сгенерированный 8x16 шрифт с минимальным набором символов.
    /// Глифы рисуются простыми алгоритмами. Используется только если ничего
    /// другого нет — на реальной Arch-системе всегда будет Lat2-Terminus16.psfu.gz.
    pub fn builtin_8x16() -> Self {
        let mut glyphs = vec![0u8; 256 * 16];
        // Рамка для каждого символа (как заглушка), потом перерисовываем нужные.
        for i in 0..256u32 {
            let g = &mut glyphs[(i * 16) as usize..((i + 1) * 16) as usize];
            for row in g.iter_mut() { *row = 0; }
        }
        // Пробел — пустой.
        // '!' (0x21)
        let exclaim: [u8; 16] = [0x18,0x18,0x18,0x18,0x18,0x18,0x18,0x18,0x00,0x18,0x18,0x00,0,0,0,0];
        glyphs[0x21*16..0x21*16+16].copy_from_slice(&exclaim);
        // '#' (0x23)
        let hash: [u8; 16] = [0x00,0x6C,0x6C,0xFE,0x6C,0xFE,0x6C,0x6C,0x00,0x00,0x00,0x00,0,0,0,0];
        glyphs[0x23*16..0x23*16+16].copy_from_slice(&hash);
        // Простые прямоугольники для остальных печатных ASCII.
        for c in 0x20..0x7Fu32 {
            if c == 0x21 || c == 0x23 { continue; }
            let g = &mut glyphs[(c * 16) as usize..((c + 1) * 16) as usize];
            // Рамка 5x7 начиная с row=4 col=1.
            g[4] = 0x7C; g[10] = 0x7C;
            for r in 5..=9 { g[r] = 0x44; }
            g[5] |= 0x38; g[9] |= 0x38;
            // Внутри — символ из 4px высоты.
            let ch = c as u8 as char;
            let bit = match ch {
                '0' => 0x10, '1' => 0x20, '2' => 0x30, '3' => 0x40, '4' => 0x50,
                _ => 0x00,
            };
            if bit != 0 {
                for r in 6..=8 { g[r] = bit; }
            }
        }
        Font {
            width: 8, height: 16,
            glyph_count: 256,
            bytes_per_glyph: 16,
            glyphs,
            has_unicode_table: false,
            unicode_map: HashMap::new(),
        }
    }
}

/// Парсит PSF2 unicode table. Формат (после glyphs section):
///   Для каждого glyph index 0..N, последовательность UTF-8 codepoints:
///     - 0xFF: разделитель между glyph'ами (end of entry for current glyph).
///     - 0xFE: разделитель между альтернативными последовательностями для
///             одного glyph (combining chars). Игнорируем — берём только первую.
///   Каждый codepoint кодируется как UTF-8 (1-4 bytes).
///
/// Возвращает HashMap<u32, u32> (codepoint → glyph index).
/// Для combining sequences берём первый codepoint последовательности.
fn parse_psf2_unicode_table(data: &[u8], glyph_count: u32) -> HashMap<u32, u32> {
    let mut map: HashMap<u32, u32> = HashMap::new();
    let mut pos = 0usize;
    let mut glyph_idx = 0u32;

    while pos < data.len() && glyph_idx < glyph_count {
        // Начало entry для текущего glyph. Читаем codepoints до 0xFF/0xFE.
        let mut first_cp: Option<u32> = None;
        while pos < data.len() {
            let b = data[pos];
            if b == 0xFF {
                // End of glyph entry.
                pos += 1;
                break;
            }
            if b == 0xFE {
                // Separator between alternative sequences for the same glyph
                // (combining chars). Skip the rest of this sequence.
                // Advance until we hit 0xFF (end of glyph entry).
                while pos < data.len() && data[pos] != 0xFF {
                    pos += 1;
                }
                if pos < data.len() { pos += 1; } // skip 0xFF
                break;
            }
            // Decode UTF-8 codepoint starting at `b`.
            let (cp_opt, advance) = decode_utf8(&data[pos..]);
            pos += advance;
            if let Some(cp) = cp_opt {
                if first_cp.is_none() {
                    first_cp = Some(cp);
                    // Only record the first codepoint for this glyph index.
                    // Multiple codepoints mapping to the same glyph are
                    // already covered (we'd just overwrite with same index).
                    map.entry(cp).or_insert(glyph_idx);
                }
            }
        }
        // If we hit EOF without seeing 0xFF, glyph_idx is still incremented below.
        let _ = first_cp; // silence unused warning if no codepoint was decoded
        glyph_idx += 1;
    }

    log::debug!("PSF2 unicode table: {} codepoints mapped to {} glyphs",
        map.len(), glyph_count);
    map
}

/// Парсит PSF1 unicode table. Формат похож на PSF2:
///   Для каждого glyph index 0..N, последовательность 16-bit Unicode codepoints
///   (little-endian), завершающаяся 0xFFFF.
///   0xFFFE — separator between alternative sequences (combining chars).
fn parse_psf1_unicode_table(data: &[u8], glyph_count: u32) -> HashMap<u32, u32> {
    let mut map: HashMap<u32, u32> = HashMap::new();
    let mut pos = 0usize;
    let mut glyph_idx = 0u32;

    while pos + 1 < data.len() && glyph_idx < glyph_count {
        let cp = u16::from_le_bytes([data[pos], data[pos + 1]]) as u32;
        pos += 2;
        if cp == 0xFFFF {
            glyph_idx += 1;
            continue;
        }
        if cp == 0xFFFE {
            // Skip rest of alternatives for this glyph.
            while pos + 1 < data.len() {
                let v = u16::from_le_bytes([data[pos], data[pos + 1]]);
                pos += 2;
                if v == 0xFFFF { break; }
            }
            glyph_idx += 1;
            continue;
        }
        // Map first codepoint of glyph → glyph index.
        map.entry(cp).or_insert(glyph_idx);
    }

    log::debug!("PSF1 unicode table: {} codepoints mapped", map.len());
    map
}

/// Декодирует один UTF-8 codepoint из начала slice. Возвращает (Some(cp), length)
/// при успехе или (None, advance) при ошибке (advance = сколько байт пропустить).
fn decode_utf8(data: &[u8]) -> (Option<u32>, usize) {
    if data.is_empty() { return (None, 0); }
    let b0 = data[0];
    if b0 < 0x80 {
        return (Some(b0 as u32), 1);
    }
    if b0 & 0xE0 == 0xC0 {
        // 2-byte: 110xxxxx 10xxxxxx
        if data.len() < 2 { return (None, data.len()); }
        let b1 = data[1];
        if b1 & 0xC0 != 0x80 { return (None, 1); }
        let cp = ((b0 as u32 & 0x1F) << 6) | (b1 as u32 & 0x3F);
        return (Some(cp), 2);
    }
    if b0 & 0xF0 == 0xE0 {
        // 3-byte: 1110xxxx 10xxxxxx 10xxxxxx
        if data.len() < 3 { return (None, data.len()); }
        let b1 = data[1];
        let b2 = data[2];
        if b1 & 0xC0 != 0x80 || b2 & 0xC0 != 0x80 { return (None, 1); }
        let cp = ((b0 as u32 & 0x0F) << 12)
               | ((b1 as u32 & 0x3F) << 6)
               | (b2 as u32 & 0x3F);
        return (Some(cp), 3);
    }
    if b0 & 0xF8 == 0xF0 {
        // 4-byte: 11110xxx 10xxxxxx 10xxxxxx 10xxxxxx
        if data.len() < 4 { return (None, data.len()); }
        let b1 = data[1];
        let b2 = data[2];
        let b3 = data[3];
        if b1 & 0xC0 != 0x80 || b2 & 0xC0 != 0x80 || b3 & 0xC0 != 0x80 {
            return (None, 1);
        }
        let cp = ((b0 as u32 & 0x07) << 18)
               | ((b1 as u32 & 0x3F) << 12)
               | ((b2 as u32 & 0x3F) << 6)
               | (b3 as u32 & 0x3F);
        return (Some(cp), 4);
    }
    // Invalid lead byte.
    (None, 1)
}

/// Загружает файл, возможно gzip-сжатый (с расширением .gz).
///
/// Для .gz файлов вызывает внешний `gunzip -c`. Альтернатива — зависимость
/// `flate2`, но для PSF шрифтов это избыточно. Корректно завершает child
/// процесс и проверяет его exit status.
fn load_maybe_gz(path: &str) -> anyhow::Result<Vec<u8>> {
    let raw = fs::read(path)?;
    let is_gz = path.ends_with(".gz")
        || (raw.len() >= 2 && raw[0] == 0x1f && raw[1] == 0x8b); // gzip magic
    if !is_gz {
        return Ok(raw);
    }

    use std::io::Write;
    use std::process::{Command, Stdio};

    let mut child = Command::new("gunzip")
        .arg("-c")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped()) // подавляем stderr gunzip в логи WM
        .spawn()
        .context("failed to spawn gunzip — install gzip package")?;

    // Записываем данные в stdin, затем закрываем pipe (drop stdin handle).
    // Это сигнализирует gunzip что ввод окончен.
    {
        let mut stdin = child.stdin.take()
            .context("gunzip stdin not piped (should not happen)")?;
        stdin.write_all(&raw)
            .context("failed to write to gunzip stdin")?;
        // stdin drops here → pipe closed → gunzip sees EOF
    }

    // wait_with_output() дочитывает stdout/stderr и дожидается завершения,
    // предотвращая zombie. Возвращает Output со статусом.
    let output = child.wait_with_output()
        .context("failed to wait for gunzip")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("gunzip failed (exit {:?}): {}",
            output.status.code(), stderr.trim());
    }

    Ok(output.stdout)
}

/// Запрашивает fontconfig (`fc-match`) для поиска системного monospace TTF.
///
/// Команда: `fc-match -f "%{file}:%{family}\n" monospace:spacing=100`
///   - `monospace` — запрашиваем семейство monospace
///   - `spacing=100` — требуем strictly monospace (фиксированная ширина)
///   - `%{file}` — путь к TTF файлу
///   - `%{family}` — имя семейства (для логов)
///
/// Возвращает (path, family) или None если fc-match недоступен.
fn find_ttf_via_fontconfig() -> Option<(String, String)> {
    use std::process::Command;

    let output = Command::new("fc-match")
        .args(["-f", "%{file}\t%{family}", "monospace:spacing=100"])
        .output()
        .ok()?;

    if !output.status.success() {
        log::debug!("fc-match failed: {}", String::from_utf8_lossy(&output.stderr).trim());
        return None;
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let line = stdout.lines().next()?.trim();
    if line.is_empty() {
        return None;
    }

    // Parse "path\tFamily Name" — split on first tab.
    let (path, family) = match line.split_once('\t') {
        Some((p, f)) => (p.to_string(), f.to_string()),
        None => (line.to_string(), String::from("unknown")),
    };

    // Sanity check: file must exist and end with .ttf/.otf/.ttc
    if !std::path::Path::new(&path).exists() {
        log::warn!("fc-match returned non-existent path: {}", path);
        return None;
    }

    let lower = path.to_lowercase();
    if !lower.ends_with(".ttf") && !lower.ends_with(".otf") && !lower.ends_with(".ttc") {
        log::warn!("fc-match returned non-TTF path: {} — skipping", path);
        return None;
    }

    Some((path, family))
}

/// Определяет целевую ширину глифа для TTF шрифта по advance width
/// нескольких репрезентативных символов. Для monospace шрифтов advance
/// одинаков для всех глифов — берём максимум из тестовых символов.
///
/// Возвращает ширину в пикселях (8-16).
fn determine_ttf_width(face: &freetype::Face) -> u32 {
    use freetype::face::LoadFlag;

    // Тестовые символы: латиница, кириллица (для проверки что шрифт не узкий).
    let test_chars: &[u32] = &[
        b'M' as u32, b'W' as u32, b'm' as u32, b'w' as u32,
        b'@' as u32, b'#' as u32, b'8' as u32,
        0x0410, // А (кириллица)
        0x044F, // я
    ];
    let mut max_width: u32 = 8; // default for 16px height

    for &cp in test_chars {
        if face.load_char(cp as usize, LoadFlag::DEFAULT).is_err() { continue; }
        let advance = face.glyph().advance().x;
        if advance > 0 {
            // Freetype advance is in 26.6 fixed point — shift right by 6 to get pixels.
            let pixel_advance = (advance >> 6) as u32;
            if pixel_advance > max_width {
                max_width = pixel_advance;
            }
        }
    }

    // Cap at reasonable width to avoid huge cells for non-monospace fonts.
    max_width.min(16).max(6)
}
