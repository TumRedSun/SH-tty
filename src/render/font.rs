//! PSF (PC Screen Font) loader — загрузка шрифта для рендеринга терминала.
//!
//! Поддерживаются PSF1 и PSF2. Шрифт ищется в:
//!   1. /etc/superhot-tty/font.psfu (переопределение пользователя — приоритет)
//!   2. Динамическое сканирование /usr/share/kbd/consolefonts/,
//!      /usr/share/consolefonts/, /usr/lib/kbd/consolefonts/ — ВСЕ .psfu* файлы.
//!      Scoring по покрытию: Cyrillic > box-drawing > blocks > Greek > Powerline.
//!   3. Если ничего не найдено — процедурный встроенный шрифт 8x16.
//!
//! Рекомендуемый шрифт для русского языка: `ter-u16n.psfu.gz` из пакета
//! `terminus-font` (Arch) / `fonts-terminus` (Debian).
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
    /// Стратегия (v2): динамически сканируем ВСЕ `.psfu*` файлы в стандартных
    /// каталогах консольных шрифтов, а не только hardcoded paths. Это важно
    /// потому что на разных дистрибутивах имена файлов отличаются:
    ///   - Arch:      /usr/share/kbd/consolefonts/ter-u16n.psfu.gz  (terminus-font)
    ///   - Debian:    /usr/share/consolefonts/Uni3-Terminus16.psfu.gz
    ///   - Fedora:    /usr/lib/kbd/consolefonts/
    ///
    /// Затем scoring не просто по glyph_count (как раньше), а по coverage:
    ///   1. Cyrillic (U+0410–U+044F, U+0401, U+0451)  — критично для русского
    ///   2. Box-drawing (U+2500–U+257F)               — для btop, htop UI
    ///   3. Block elements (U+2580–U+259F)             — для прогресс-баров
    ///   4. Greek (U+0391–U+03C9)                      — математика
    ///   5. Powerline symbols (U+E0A0–U+E0D4)          — для zsh/powerline
    ///   6. has_unicode_table                          — корректный lookup
    ///   7. glyph_count                                — tiebreaker
    ///
    /// Это исправляет баг, когда выбирался `Lat2-Terminus16.psfu.gz` (256 glyphs,
    /// Latin-2 only, NO Cyrillic) — у него была unicode_table, поэтому он
    /// побеждал старый scoring. Теперь его обходят шрифты с Cyrillic покрытием.
    ///
    /// Если ни один шрифт не содержит Cyrillic — логируем warning с инструкцией
    /// установить `terminus-font` пакет.
    pub fn load_default() -> Self {
        // 1. User override — максимальный приоритет, без сравнения.
        for user_path in ["/etc/superhot-tty/font.psfu", "/etc/superhot-tty/font.psf"] {
            if let Ok(data) = load_maybe_gz(user_path) {
                if let Ok(f) = Self::from_bytes(&data) {
                    log::info!(
                        "loaded user font from {} ({}x{} glyphs={} cyrillic={} box_drawing={})",
                        user_path, f.width, f.height, f.glyph_count,
                        f.has_cyrillic(), f.has_box_drawing()
                    );
                    return f;
                }
            }
        }

        // 2. Собираем кандидатов из всех источников.
        let mut candidates: Vec<(PathBuf, Vec<u8>)> = Vec::new();

        // 2a. Hardcoded high-priority paths (если существуют — пробуем первыми).
        const PRIORITY_PATHS: &[&str] = &[
            // Шрифты с полным Unicode покрытием — лучшее качество.
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

        // 2b. Динамическое сканирование стандартных каталогов.
        // Это подхватит ЛЮБОЙ установленный шрифт — даже если его имени нет
        // в hardcoded списке выше.
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
                // Принимаем .psfu / .psfu.gz / .psf / .psf.gz
                let is_psf = fname.ends_with(".psfu.gz")
                    || fname.ends_with(".psfu")
                    || fname.ends_with(".psf.gz")
                    || fname.ends_with(".psf");
                if !is_psf { continue; }
                // Не дублируем уже добавленные пути.
                if candidates.iter().any(|(p, _)| p == &path) { continue; }
                if let Ok(data) = load_maybe_gz(path.to_str().unwrap_or("")) {
                    candidates.push((path, data));
                }
            }
        }

        log::debug!("font scan: {} candidates found", candidates.len());

        // 3. Парсим и scoring. Score tuple сравнивается лексикографически:
        //    (cyrillic, box_drawing, blocks, greek, powerline, has_unicode_table, glyph_count)
        //    Шрифт с Cyrillic всегда побеждает шрифт без Cyrillic, и т.д.
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
                "font candidate {}: {}x{} glyphs={} uni_table={} cyrillic={} box={} blk={} greek={} pwr={} score={:?}",
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
                "loaded font from {} ({}x{} glyphs={} unicode_table={} cyrillic={} box_drawing={} greek={} powerline={} score={:?})",
                path.display(), f.width, f.height, f.glyph_count, f.has_unicode_table,
                f.has_cyrillic(), f.has_box_drawing(), f.has_greek(), f.has_powerline(), score
            );
            if !has_cyr {
                log::warn!("loaded font does NOT contain Cyrillic glyphs — Russian text will render as '?'");
                log::warn!("install a Unicode-capable font package to fix:");
                log::warn!("  Arch:    sudo pacman -S terminus-font   (provides ter-u16n.psfu.gz)");
                log::warn!("  Debian:  sudo apt install fonts-terminus console-setup");
                log::warn!("  Fedora:  sudo dnf install terminus-fonts-pcf");
                log::warn!("or copy a .psfu font to /etc/superhot-tty/font.psfu");
            }
            return f;
        }

        log::warn!("no system PSF font found in any standard directory");
        log::warn!("install one of:");
        log::warn!("  Arch:    sudo pacman -S kbd terminus-font");
        log::warn!("  Debian:  sudo apt install kbd console-setup fonts-terminus");
        log::warn!("  Fedora:  sudo dnf install kbd terminus-fonts-pcf");
        log::warn!("using builtin fallback 8x16 (NO Cyrillic, NO box-drawing)");
        Self::builtin_8x16()
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
