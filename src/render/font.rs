//! PSF (PC Screen Font) loader — загрузка шрифта для рендеринга терминала.
//!
//! Поддерживаются PSF1 и PSF2. Шрифт ищется в:
//!   1. /etc/superhot-tty/font.psfu (переопределение пользователя)
//!   2. /usr/share/kbd/consolefonts/Lat2-Terminus16.psfu.gz  (Arch default)
//!   3. /usr/share/consolefonts/Lat2-Terminus16.psfu.gz      (Debian/Fedora)
//! Если ничего не найдено — используется процедурный встроенный шрифт 8x16.
//!
//! ВАЖНО: PSF2-шрифты с unicode table (флаг PSF2_HAS_UNICODE_TABLE) содержат
//! секцию после glyphs, которая маппит Unicode codepoints на glyph indices.
//! Раньше мы это игнорировали и трактовали codepoint как прямой индекс —
//! из-за этого box-drawing chars (─ │ ┌ ┐ └ ┘, U+2500-U+257F) и Cyrillic
//! (U+0400-U+04FF) рендерились как мусорные символы (выглядело как "e с ~").
//! Теперь unicode table парсится в HashMap<u32, u32> для корректного lookup'а.

use std::collections::HashMap;
use std::fs;
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
    /// Стратегия: сканируем ВСЕ кандидаты и выбираем шрифт с наибольшим
    /// glyph_count (т.е. лучшим Unicode покрытием). Раньше мы просто брали
    /// первый найденный — это был Lat2-Terminus16 (256 glyphs), который не
    /// содержит Cyrillic (U+0410-U+044F), Greek, Powerline (U+E0A0-E0D4),
    /// многих математических символов. Все неизвестные codepoints рендерятся
    /// как '?', что приводило к "e → ?" и "btop symbols → ?".
    ///
    /// Шрифты с полным Unicode покрытием (терминус-variants, UniCox, Uni3,
    /// sun12x22) обычно имеют 1000-6000 glyphs и корректно отображают и
    /// кириллицу, и box-drawing, и Powerline symbols.
    pub fn load_default() -> Self {
        const CANDIDATES: &[&str] = &[
            // User override — максимальный приоритет, не сравниваем с остальными.
            "/etc/superhot-tty/font.psfu",
            "/etc/superhot-tty/font.psf",
            // Шрифты с большим Unicode покрытием — лучше рендерят box-drawing,
            // кириллицу, Powerline symbols. Если доступны, предпочтительнее
            // Lat2-Terminus16 (который имеет всего 256 glyphs).
            "/usr/share/kbd/consolefonts/Uni3-Terminus16.psfu.gz",
            "/usr/share/kbd/consolefonts/Uni3-Fixed16.psfu.gz",
            "/usr/share/kbd/consolefonts/UniCox_14.psfu.gz",
            "/usr/share/kbd/consolefonts/UniCortex_14.psfu.gz",
            "/usr/share/kbd/consolefonts/UniFont-Terminus16.psfu.gz",
            "/usr/share/kbd/consolefonts/ter-v16n.psfu.gz",
            "/usr/share/kbd/consolefonts/ter-v14n.psfu.gz",
            "/usr/share/kbd/consolefonts/ter-u16n.psfu.gz",
            "/usr/share/kbd/consolefonts/Lat2-Terminus16.psfu.gz",
            "/usr/share/kbd/consolefonts/Lat2-Terminus16.psf",
            "/usr/share/consolefonts/Lat2-Terminus16.psfu.gz",
            "/usr/share/consolefonts/Lat2-Terminus16.psf",
            "/usr/share/kbd/consolefonts/default8x16.psfu.gz",
            "/usr/share/kbd/consolefonts/default8x16.psf",
            "/usr/share/kbd/consolefonts/sun12x22.psfu.gz",
        ];

        // Сначала проверяем user override — если есть, используем без сравнения.
        if let Some(path) = CANDIDATES.iter().take(2).find_map(|p| {
            load_maybe_gz(p).ok().and_then(|data| Self::from_bytes(&data).ok().map(|f| (p, f)))
        }) {
            let (path, f) = path;
            log::info!("loaded user font from {} ({}x{} glyphs={} unicode_table={})",
                path, f.width, f.height, f.glyph_count, f.has_unicode_table);
            return f;
        }

        // Сканируем системные шрифты и выбираем с наибольшим glyph_count.
        let mut best: Option<(&str, Self)> = None;
        for path in CANDIDATES.iter().skip(2) {
            let Ok(data) = load_maybe_gz(path) else { continue };
            let Ok(f) = Self::from_bytes(&data) else { continue };
            // Prefer fonts with unicode_table (corректный lookup codepoint → glyph).
            // Among fonts with same unicode_table status, prefer more glyphs.
            let score = (f.has_unicode_table as u32, f.glyph_count);
            let better = match &best {
                None => true,
                Some((_, b)) => (f.has_unicode_table as u32, f.glyph_count)
                    > (b.has_unicode_table as u32, b.glyph_count),
            };
            log::debug!("font candidate {}: {}x{} glyphs={} unicode_table={} score={:?} best_so_far={}",
                path, f.width, f.height, f.glyph_count, f.has_unicode_table, score, better);
            if better {
                best = Some((path, f));
            }
        }

        if let Some((path, f)) = best {
            log::info!("loaded font from {} ({}x{} glyphs={} unicode_table={})",
                path, f.width, f.height, f.glyph_count, f.has_unicode_table);
            return f;
        }

        log::warn!("no system PSF font found, using builtin fallback 8x16");
        Self::builtin_8x16()
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
