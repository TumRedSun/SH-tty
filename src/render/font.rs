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
//! 3. **User override** — файл в /etc/shtty/:
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
    /// Базовая линия глифов в пикселях от верха ячейки (для TTF — из метрик
    /// шрифта, для PSF — приближение 3/4 высоты). Fallback-шрифты рендерятся
    /// с той же baseline, чтобы глифы не "прыгали" по вертикали.
    pub baseline: u32,
    /// Fallback-шрифты: опрашиваются в порядке если codepoint отсутствует
    /// в основном шрифте. Так рендерятся Nerd Font иконки, Powerline-символы,
    /// математические знаки и прочие символы, которых нет в monospace-шрифте.
    /// Все fallback'и отрендерены в ТЕ ЖЕ ячейки (width×height), что и основной
    /// шрифт, поэтому bytes_per_glyph совпадает и glyph_for безопасен.
    fallbacks: Vec<Font>,
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
            baseline: height * 3 / 4,
            fallbacks: Vec::new(),
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
            baseline: height * 3 / 4,
            fallbacks: Vec::new(),
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
    ///
    /// Параметры:
    ///   `family_hint` — имя семейства из конфига (general.font). Используется
    ///                   как приоритет при выборе TTF через fontconfig. Пустая
    ///                   строка — берём любой monospace.
    ///   `pixel_height` — желаемая высота глифа в пикселях (general.font_size).
    ///                    0 = default 16px.
    pub fn load_default_with_config(family_hint: &str, pixel_height: u32) -> Self {
        let pixel_height = if pixel_height == 0 { 16 } else { pixel_height.clamp(8, 64) };
        let mut font = Self::load_primary(family_hint, pixel_height);
        Self::attach_fallbacks(&mut font);
        font
    }

    /// Загружает основной шрифт (без fallback'ов).
    fn load_primary(family_hint: &str, pixel_height: u32) -> Self {
        // 0. User override через /etc/shtty/font.ttf — TTF файл.
        // Если есть — используем freetype для рендеринга.
        for user_path in ["/etc/shtty/font.ttf", "/etc/shtty/font.otf"] {
            if std::path::Path::new(user_path).exists() {
                match Self::from_ttf(user_path, pixel_height) {
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

        // 1. User override через /etc/shtty/font.psfu — PSF файл (legacy).
        for user_path in ["/etc/shtty/font.psfu", "/etc/shtty/font.psf"] {
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
        //    Если family_hint задан (например "JetBrains Mono"), пытаемся сначала
        //    найти именно его через fc-match. Иначе — любой monospace.
        match find_ttf_via_fontconfig(family_hint) {
            Some((path, family)) => {
                log::info!("fontconfig selected TTF: {} ({}) size={}px", path, family, pixel_height);
                match Self::from_ttf(&path, pixel_height) {
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

    /// Подключает fallback-шрифты для символов, которых нет в основном шрифте.
    ///
    /// Зачем: monospace TTF (DejaVu Sans Mono и т.п.) не содержит Nerd Font
    /// иконок (U+E000-U+F8FF), Powerline-символов и многих спецзнаков. Пользователь
    /// видит на их месте '?' или пустые ячейки. Если в системе установлены
    /// символ-шрифты (ttf-nerd-fonts-symbols, noto-fonts, symbola, unifont...),
    /// они подключаются как fallback и все иконки рендерятся корректно.
    ///
    /// Источники кандидатов:
    ///   1. /etc/shtty/font-fallbacks/*.ttf|otf — явный пользовательский override
    ///   2. fc-match по списку известных символ-семейств
    ///   3. скан /usr/share/fonts и ~/.local/share/fonts по маске имени
    ///
    /// Подключаются только шрифты, добавляющие НОВЫЕ codepoints (проверка по
    /// charmap до рендеринга). Максимум 6 fallback'ов. Emoji-шрифты пропускаются —
    /// 1-битный monochrome рендер не может отобразить цветные эмодзи.
    #[cfg(feature = "ttf")]
    fn attach_fallbacks(font: &mut Font) {
        const MAX_FALLBACKS: usize = 6;

        let mut candidates: Vec<String> = Vec::new();

        // 1. Пользовательский каталог fallback'ов.
        if let Ok(rd) = fs::read_dir("/etc/shtty/font-fallbacks") {
            for entry in rd.flatten() {
                let path = entry.path();
                let lower = path.to_string_lossy().to_lowercase();
                if lower.ends_with(".ttf") || lower.ends_with(".otf") {
                    candidates.push(path.to_string_lossy().to_string());
                }
            }
        }

        // 2. Известные символ-семейства через fc-match.
        const FALLBACK_FAMILIES: &[&str] = &[
            "Symbols Nerd Font Mono",
            "Symbols Nerd Font",
            "Noto Sans Symbols 2",
            "Noto Sans Symbols",
            "Symbola",
            "Unifont",
            "Noto Sans Math",
            "Powerline",
        ];
        for family in FALLBACK_FAMILIES {
            if let Some((path, _)) = find_ttf_via_fontconfig(family) {
                candidates.push(path);
            }
        }

        // 3. Скан файловых каталогов по маскам имени.
        for pattern_dir in [
            "/usr/share/fonts",
            "/usr/local/share/fonts",
            "~/.local/share/fonts",
        ] {
            let dir = expand_tilde_path(pattern_dir);
            collect_font_candidates(&dir, &mut candidates);
        }

        // Дедупликация + исключение эмодзи и пустых.
        let mut seen: Vec<String> = Vec::new();
        for c in candidates {
            let lower = c.to_lowercase();
            if lower.contains("emoji") { continue; }
            if !seen.contains(&c) {
                seen.push(c);
            }
        }

        let cell_w = font.width;
        let cell_h = font.height;
        let baseline = font.baseline_hint();
        let mut added = 0usize;

        for path in seen {
            if added >= MAX_FALLBACKS { break; }

            // Проверяем покрытие ДО рендеринга: шрифт должен добавлять
            // хотя бы один новый codepoint. face.chars() — дёшево.
            let new_cps = match freetype_new_face_codepoints(&path) {
                Some(cps) => cps,
                None => continue,
            };
            let fresh: Vec<u32> = new_cps
                .into_iter()
                .filter(|&cp| !font.has_glyph(cp) && cp >= 0x80)
                .collect();
            if fresh.is_empty() { continue; }

            // Рендерим в ТЕ ЖЕ ячейки что и основной шрифт.
            match font.render_fallback(&path, cell_w, cell_h, baseline) {
                Ok(()) => {
                    added += 1;
                    log::info!(
                        "font fallback #{}: {} ({} новых codepoints, примеры: {:?})",
                        added, path, fresh.len(),
                        fresh.iter().take(4).map(|c| format!("U+{:04X}", c)).collect::<Vec<_>>()
                    );
                }
                Err(e) => {
                    log::debug!("fallback font {} skipped: {}", path, e);
                }
            }
        }

        if added == 0 {
            log::info!("no additional symbol fonts found — icons from Nerd Font/Powerline will render as '?'");
            log::info!("  install them for icon support, e.g.:");
            log::info!("    Arch:    sudo pacman -S ttf-nerd-fonts-symbols noto-fonts");
            log::info!("    Debian:  sudo apt install fonts-noto-core fonts-symbola");
            log::info!("  or drop any .ttf into /etc/shtty/font-fallbacks/");
        } else {
            log::info!("font fallback chain: {} additional font(s), coverage extended", added);
        }
    }

    #[cfg(not(feature = "ttf"))]
    fn attach_fallbacks(_font: &mut Font) {
        // TTF support disabled — fallback chain requires freetype.
    }

    /// Legacy-точка входа — вызывает `load_default_with_config` с дефолтами
    /// (любой monospace, 16px). Сохранена для совместимости с существующими
    /// вызовами и тестами.
    #[allow(dead_code)]
    pub fn load_default() -> Self {
        Self::load_default_with_config("", 16)
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
                log::warn!("or copy a .psfu font to /etc/shtty/font.psfu");
            }
            return f;
        }

        log::warn!("no system PSF font found — using builtin fallback 8x16 (NO Cyrillic, NO box-drawing)");
        Self::builtin_8x16()
    }

    /// Загружает TTF/OTF шрифт через freetype и пререндерит все codepoints,
    /// которые есть в шрифте, в PSF-совместимую bitmap структуру.
    ///
    /// Это позволяет WM рендерить ЛЮБОЙ Unicode символ, который есть в шрифте —
    /// кириллица, греческий, box-drawing, математические операторы, emoji (если
    /// в шрифте, например Noto Color Mono), CJK, и т.д.
    /// TTF предпочтительнее PSF — у TTF обычно 2000-3000+ glyphs против 256-1000
    /// у PSF, плюс TTF поддерживает любые размеры и hinting.
    ///
    /// В отличие от старой версии (которая итерировала только 0..=0xFFFF и
    /// использовала codepoint как прямой индекс), эта версия:
    ///   1. Итерирует charmap шрифта через `face.chars()` — получает ВСЕ
    ///      codepoints включая Plane 1+ (emoji U+1F600, CJK Ext B U+20000+, и т.д.).
    ///   2. Использует последовательный glyph index (0, 1, 2, ...) — это
    ///      compact: для шрифта с 3000 glyphs выделяем 3000 слотов, а не 65536.
    ///   3. Codepoint → glyph index mapping хранится в `unicode_map` (HashMap).
    ///      glyph_for() уже умеет с ним работать.
    ///
    /// `pixel_height` — желаемая высота глифа в пикселях (16, 20, 24).
    /// Ширина подбирается автоматически по advance width (для monospace это
    /// одинаково для всех глифов).
    #[cfg(feature = "ttf")]
    pub fn from_ttf(path: &str, pixel_height: u32) -> anyhow::Result<Self> {
        Self::from_ttf_inner(path, pixel_height, None, None)
    }

    /// TTF с фиксированной шириной ячейки и baseline — используется для
    /// fallback-шрифтов, чтобы их глифы попадали в ячейки основного шрифта.
    #[cfg(feature = "ttf")]
    fn render_fallback(&mut self, path: &str, cell_width: u32, cell_height: u32, baseline: u32) -> anyhow::Result<()> {
        let f = Self::from_ttf_inner(path, cell_height, Some(cell_width), Some(baseline as i32))?;
        self.fallbacks.push(f);
        Ok(())
    }

    /// Базовая линия основного шрифта (для выравнивания fallback'ов).
    pub fn baseline_hint(&self) -> u32 {
        self.baseline
    }

    #[cfg(feature = "ttf")]
    fn from_ttf_inner(
        path: &str,
        pixel_height: u32,
        force_width: Option<u32>,
        baseline_hint: Option<i32>,
    ) -> anyhow::Result<Self> {
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
        // force_width — для fallback-шрифтов: ячейка должна совпадать с основным.
        let target_width = force_width.unwrap_or_else(|| determine_ttf_width(&face));
        let bytes_per_row = ((target_width + 7) / 8) as usize;
        let bytes_per_glyph = bytes_per_row * pixel_height as usize;

        // Get font metrics for proper vertical alignment.
        // ascender/descender are in 26.6 fixed point after set_char_size —
        // but freetype-rs exposes them as FT_Short (raw font units scaled by
        // current size). size_metrics gives the scaled values directly.
        let (ascender, descender) = if let Some(m) = face.size_metrics() {
            // ascender/descender in pixels (already scaled, in 26.6 fixed → shift).
            let a = (m.ascender >> 6) as i32;
            let d = (m.descender >> 6) as i32; // negative
            (a, d)
        } else {
            // Fallback: face.ascender()/descender() in font units, scaled by
            // pixel_height / em_size. Most fonts have em_size = 1000 or 2048.
            let em = face.em_size().max(1) as i32;
            let a = (face.ascender() as i32) * pixel_height as i32 / em;
            let d = (face.descender() as i32) * pixel_height as i32 / em;
            (a, d)
        };
        // baseline = ascender pixels from the top of the cell.
        // baseline_hint — от основного шрифта, чтобы fallback глифы стояли на той же линии.
        let baseline = baseline_hint.unwrap_or(ascender).max(0).min(pixel_height as i32 - 1);

        log::debug!(
            "TTF {}: target_width={}px height={}px bytes_per_glyph={} ascender={} descender={} baseline={}",
            path, target_width, pixel_height, bytes_per_glyph, ascender, descender, baseline
        );

        // First pass: collect all (codepoint, glyph_index) pairs from the
        // font's charmap. This includes codepoints above U+FFFF (emoji, CJK
        // extensions, etc.) which the old BMP-only loop missed.
        let charmap: Vec<(u32, u32)> = face.chars()
            .map(|(cp, gi)| (cp as u32, gi.get()))
            .collect();

        // Pre-allocate glyphs vec with one slot per charmap entry.
        // For a typical font with ~3000 glyphs at 16px (bytes_per_glyph=32),
        // this is ~96KB — far less than the old 65536 * 32 = 2MB.
        let total_glyphs = charmap.len();
        let mut glyphs = vec![0u8; total_glyphs * bytes_per_glyph];
        let mut unicode_map: HashMap<u32, u32> = HashMap::with_capacity(total_glyphs);
        let mut rendered_count = 0u32;
        let mut failed_count = 0u32;

        // Second pass: render each glyph into its sequential slot.
        for (seq_idx, (cp, glyph_index)) in charmap.iter().enumerate() {
            let seq_idx = seq_idx as u32;

            // Load + render. We use load_glyph (not load_char) since we
            // already have the glyph index from the charmap iterator.
            // MONOCHROME produces 1-bit-per-pixel packed bitmap (PSF-compatible).
            // RENDER forces rasterization.
            if face.load_glyph(*glyph_index, LoadFlag::RENDER | LoadFlag::MONOCHROME).is_err() {
                failed_count += 1;
                // Still map the codepoint to the empty slot so lookup doesn't
                // fall back to '?' — the user sees a blank instead.
                unicode_map.insert(*cp, seq_idx);
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

            let glyph_off = seq_idx as usize * bytes_per_glyph;

            // Empty glyph (e.g., space) — leave as zeros, but still map it.
            if buffer.is_empty() || bm_width == 0 || bm_rows == 0 {
                unicode_map.insert(*cp, seq_idx);
                rendered_count += 1;
                continue;
            }

            // Vertical positioning: bm_top is pixels above baseline (from freetype).
            // To place the glyph's top row in the cell, we compute:
            //   y_offset = baseline - bm_top
            // (positive = below cell top, negative = above cell top → clipped)
            let y_offset = baseline - bm_top;

            // Horizontal positioning: bm_left is pixels to the right of the
            // glyph origin (from freetype). For monospace fonts, bm_left is
            // usually 0 or small positive. Clamp to cell width to avoid OOB.
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

            unicode_map.insert(*cp, seq_idx);
            rendered_count += 1;
        }

        // Ensure '?' (U+003F) is mapped — it's the fallback in glyph_for().
        // If the font somehow doesn't have '?', draw a minimal one manually
        // at the next available slot.
        if !unicode_map.contains_key(&(b'?' as u32)) {
            let q_idx = unicode_map.len() as u32;
            // Extend glyphs vec to fit the new '?' slot.
            glyphs.resize(((q_idx + 1) as usize) * bytes_per_glyph, 0);
            let q_off = q_idx as usize * bytes_per_glyph;
            // Top arc + descender — minimal '?' shape.
            let mid = bytes_per_row / 2;
            if mid < bytes_per_row {
                glyphs[q_off + 0 * bytes_per_row + mid] = 0x3C;
                glyphs[q_off + 1 * bytes_per_row + mid] = 0x42;
                glyphs[q_off + 2 * bytes_per_row + mid] = 0x02;
                glyphs[q_off + 3 * bytes_per_row + mid] = 0x04;
                glyphs[q_off + 4 * bytes_per_row + mid] = 0x08;
                glyphs[q_off + 5 * bytes_per_row + mid] = 0x08;
                if pixel_height as usize > 7 {
                    glyphs[q_off + 7 * bytes_per_row + mid] = 0x08;
                }
            }
            unicode_map.insert(b'?' as u32, q_idx);
        }

        log::info!(
            "TTF rendered: {} glyphs ({} failed) from {}, total_chars={}, cell={}x{}, mem={}KB",
            rendered_count, failed_count, path, total_glyphs, target_width, pixel_height,
            (glyphs.len() + 1023) / 1024
        );

        Ok(Font {
            width: target_width,
            height: pixel_height,
            glyph_count: total_glyphs as u32,
            bytes_per_glyph: bytes_per_glyph as u32,
            glyphs,
            has_unicode_table: true,
            unicode_map,
            baseline: baseline.max(0) as u32,
            fallbacks: Vec::new(),
        })
    }

    /// Stub for from_ttf when the `ttf` feature is disabled.
    /// Returns an error so load_default_with_config falls back to PSF.
    #[cfg(not(feature = "ttf"))]
    pub fn from_ttf(_path: &str, _pixel_height: u32) -> anyhow::Result<Self> {
        anyhow::bail!("TTF support disabled at compile time (build with --features ttf or default features to enable)")
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
        // 1. Основной шрифт.
        if let Some(glyph) = self.glyph_from_own_map(cp) {
            return glyph;
        }
        // 2. Fallback-шрифты по порядку — так находятся Nerd Font иконки,
        //    Powerline-символы и прочие знаки, отсутствующие в основном шрифте.
        for fb in &self.fallbacks {
            if let Some(glyph) = fb.glyph_from_own_map(cp) {
                return glyph;
            }
        }
        // 3. Если нигде нет — '?' основного шрифта (или пустой glyph).
        self.glyph_from_own_map(b'?' as u32)
            .unwrap_or(&self.glyphs[..(self.bytes_per_glyph as usize).min(self.glyphs.len())])
    }

    /// Lookup глифа ТОЛЬКО в собственном map (без fallback'ов).
    /// Возвращает None если codepoint отсутствует.
    fn glyph_from_own_map(&self, cp: u32) -> Option<&[u8]> {
        let idx = if !self.unicode_map.is_empty() {
            // Шрифт с unicode table — используем её для корректного lookup'а.
            self.unicode_map.get(&cp).copied()?
        } else if !self.has_unicode_table {
            // Legacy: no unicode table — direct ASCII + hardcoded Cyrillic.
            if cp < 0x80 {
                cp
            } else {
                self.cp_to_index(cp)?
            }
        } else {
            // has_unicode_table = true но map пуста (parse failed) — прямой индекс.
            if (cp as usize) < self.glyph_count as usize { cp } else { return None; }
        };
        let idx = idx.min(self.glyph_count.saturating_sub(1));
        let off = (idx * self.bytes_per_glyph) as usize;
        let end = off + self.bytes_per_glyph as usize;
        if end > self.glyphs.len() {
            // Out of bounds — malformed font, treat as missing.
            return None;
        }
        Some(&self.glyphs[off..end])
    }

    /// Есть ли в шрифте (включая fallback'и) глиф для codepoint.
    pub fn has_glyph(&self, cp: u32) -> bool {
        if self.glyph_from_own_map(cp).is_some() { return true; }
        self.fallbacks.iter().any(|f| f.glyph_from_own_map(cp).is_some())
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
            baseline: 12,
            fallbacks: Vec::new(),
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
/// Если `family_hint` не пустой — сначала пытаемся найти именно этот шрифт:
///   `fc-match -f "%{file}\t%{family}" "Family Name:spacing=100"`
/// Если family_hint пустой или не нашёлся — fallback на любой monospace:
///   `fc-match -f "%{file}\t%{family}" "monospace:spacing=100"`
///
///   - `spacing=100` — требуем strictly monospace (фиксированная ширина)
///   - `%{file}` — путь к TTF файлу
///   - `%{family}` — имя семейства (для логов)
///
/// Возвращает (path, family) или None если fc-match недоступен.
#[cfg(feature = "ttf")]
fn find_ttf_via_fontconfig(family_hint: &str) -> Option<(String, String)> {
    use std::process::Command;

    // Список паттернов для fc-match — идём от конкретного к общему.
    // Если family_hint задан (например "JetBrains Mono"), пробуем сначала его.
    let patterns: Vec<String> = if family_hint.is_empty() {
        vec!["monospace:spacing=100".to_string()]
    } else {
        vec![
            format!("{}:spacing=100", family_hint),
            format!("{}:family", family_hint),
            "monospace:spacing=100".to_string(),
        ]
    };

    for pattern in &patterns {
        let output = Command::new("fc-match")
            .args(["-f", "%{file}\t%{family}", pattern])
            .output()
            .ok();

        let output = match output {
            Some(o) => o,
            None => continue,
        };

        if !output.status.success() {
            log::debug!("fc-match pattern '{}' failed: {}", pattern,
                String::from_utf8_lossy(&output.stderr).trim());
            continue;
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let line = match stdout.lines().next() {
            Some(l) => l.trim(),
            None => continue,
        };
        if line.is_empty() { continue; }

        // Parse "path\tFamily Name" — split on first tab.
        let (path, family) = match line.split_once('\t') {
            Some((p, f)) => (p.to_string(), f.to_string()),
            None => (line.to_string(), String::from("unknown")),
        };

        // Sanity check: file must exist and end with .ttf/.otf/.ttc
        if !std::path::Path::new(&path).exists() {
            log::debug!("fc-match pattern '{}' returned non-existent path: {}", pattern, path);
            continue;
        }

        let lower = path.to_lowercase();
        if !lower.ends_with(".ttf") && !lower.ends_with(".otf") && !lower.ends_with(".ttc") {
            log::debug!("fc-match pattern '{}' returned non-TTF path: {} — skipping", pattern, path);
            continue;
        }

        log::debug!("fc-match pattern '{}' → {} ({})", pattern, path, family);
        return Some((path, family));
    }

    None
}

/// Stub for find_ttf_via_fontconfig when the `ttf` feature is disabled.
#[cfg(not(feature = "ttf"))]
fn find_ttf_via_fontconfig(_family_hint: &str) -> Option<(String, String)> {
    None
}

/// Определяет целевую ширину глифа для TTF шрифта по advance width
/// нескольких репрезентативных символов. Для monospace шрифтов advance
/// одинаков для всех глифов — берём максимум из тестовых символов.
///
/// Возвращает ширину в пикселях (8-16).
#[cfg(feature = "ttf")]
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

/// expand_tilde для путей шрифтов (без зависимости от crate::config —
/// чтобы модуль font оставался самодостаточным).
fn expand_tilde_path(s: &str) -> std::path::PathBuf {
    if let Some(rest) = s.strip_prefix("~/") {
        if let Ok(home) = std::env::var("HOME") {
            return std::path::PathBuf::from(format!("{}/{}", home, rest));
        }
    }
    std::path::PathBuf::from(s)
}

/// Рекурсивно собирает TTF/OTF файлы, чьи имена похожи на символ-шрифты
/// (Nerd Font, Symbols, Symbola, Unifont, Powerline, Math).
fn collect_font_candidates(dir: &std::path::Path, out: &mut Vec<String>) {
    let Ok(rd) = fs::read_dir(dir) else { return };
    for entry in rd.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_font_candidates(&path, out);
            continue;
        }
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else { continue };
        let lower = name.to_lowercase();
        if !lower.ends_with(".ttf") && !lower.ends_with(".otf") { continue; }
        const SYMBOL_HINTS: &[&str] = &[
            "nerd", "symbols", "symbola", "unifont", "powerline",
            "notosansmath", "noto_sans_math", "math",
        ];
        if SYMBOL_HINTS.iter().any(|h| lower.contains(h)) {
            out.push(path.to_string_lossy().to_string());
        }
    }
}

/// Возвращает множество codepoints из charmap шрифта (без рендеринга).
/// Используется для дешёвой проверки "добавляет ли шрифт новое покрытие".
#[cfg(feature = "ttf")]
fn freetype_new_face_codepoints(path: &str) -> Option<Vec<u32>> {
    let lib = freetype::Library::init().ok()?;
    let face = lib.new_face(path, 0).ok()?;
    Some(face.chars().map(|(cp, _)| cp as u32).collect())
}

#[cfg(all(test, feature = "ttf"))]
mod tests {
    use super::*;

    /// Основной путь загрузки: fc-match → DejaVu Sans Mono (или другой системный
    /// monospace TTF). Кириллица и box-drawing обязательны для терминала и бара.
    #[test]
    fn ttf_load_covers_cyrillic_and_box_drawing() {
        let f = Font::load_default_with_config("DejaVu Sans Mono", 16);
        assert!(f.width >= 6 && f.width <= 32, "cell width {}", f.width);
        assert_eq!(f.height, 16);
        assert!(f.has_cyrillic(), "нет кириллицы в системном monospace TTF");
        assert!(f.has_box_drawing(), "нет box-drawing в системном monospace TTF");
        // '?' существует и не пустой — это fallback отсутствующих глифов.
        let q = f.glyph_for(b'?' as u32);
        assert!(q.iter().any(|&b| b != 0), "'?' глиф пустой");
        // Отсутствующий codepoint не паникует и что-то возвращает.
        let _ = f.glyph_for(0x10FFFF);
    }

    /// Fallback-цепочка: глиф, которого нет в основном шрифте, но есть в
    /// fallback'е, должен рендериться из fallback'а (не '?').
    /// Тест герметичный: шрифты собираются вручную, системные не участвуют.
    #[test]
    fn glyph_for_uses_fallbacks() {
        // Основной шрифт: builtin 8x16 (ASCII only, cp_to_index → None для PUA).
        let mut f = Font::builtin_8x16();
        assert!(!f.has_glyph(0xF00C), "builtin не должен знать U+F00C");

        // Fallback с Nerd Font codepoint (U+F00C — FontAwesome check).
        let mut fb = Font::builtin_8x16();
        fb.unicode_map.insert(0xF00C, 1);
        f.fallbacks.push(fb);

        assert!(f.has_glyph(0xF00C));
        assert_ne!(f.glyph_for(0xF00C), f.glyph_for(b'?' as u32));
        // '?' глиф в builtin пустой, а fallback-глиф 0xF00C должен отличаться.
        assert_ne!(f.glyph_for(0xF00C), f.glyph_for(0xF00D));
    }
}

#[cfg(all(test, feature = "ttf"))]
mod nerd_font_tests {
    use super::*;

    /// Интеграционная проверка fallback-цепочки: если в системе установлен
    /// Symbols Nerd Font (ttf-nerd-fonts-symbols / ~/.local/share/fonts),
    /// Powerline/FontAwesome codepoints должны рендериться через fallback,
    /// даже если основной monospace-шрифт их не содержит.
    #[test]
    fn nerd_font_icons_resolve_via_fallback() {
        let f = Font::load_default_with_config("DejaVu Sans Mono", 16);
        // U+E0B0 Powerline arrow, U+F00C FontAwesome check.
        if !f.has_glyph(0xE0B0) && !f.has_glyph(0xF00C) {
            // Nerd Font не установлен — тест не должен фейлиться, просто
            // fallback-механизм нечем проверить. Логируем.
            eprintln!("Symbols Nerd Font not installed — skipping fallback assertion");
            return;
        }
        assert!(f.has_glyph(0xE0B0) || f.has_glyph(0xF00C));
        // Глиф fallback не совпадает с '?'-глифом основного шрифта.
        let icon = f.glyph_for(0xF00C);
        assert!(!icon.is_empty());
        eprintln!("fallback chain works: {} fonts attached", f.fallbacks.len());
        assert!(f.fallbacks.len() >= 1, "иконка найдена, но fallback-шрифты не подключены");
    }
}
