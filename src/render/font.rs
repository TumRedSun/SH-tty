//! PSF (PC Screen Font) loader — загрузка шрифта для рендеринга терминала.
//!
//! Поддерживаются PSF1 и PSF2. Шрифт ищется в:
//!   1. /etc/superhot-tty/font.psfu (переопределение пользователя)
//!   2. /usr/share/kbd/consolefonts/Lat2-Terminus16.psfu.gz  (Arch default)
//!   3. /usr/share/consolefonts/Lat2-Terminus16.psfu.gz      (Debian/Fedora)
//! Если ничего не найдено — используется процедурный встроенный шрифт 8x16.

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

        Ok(Font {
            width, height,
            glyph_count: length,
            bytes_per_glyph,
            glyphs,
            has_unicode_table: flags & 0x01 != 0,
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
        Ok(Font {
            width, height,
            glyph_count: length,
            bytes_per_glyph,
            glyphs,
            has_unicode_table: mode & 0x02 != 0,
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
    pub fn load_default() -> Self {
        const CANDIDATES: &[&str] = &[
            "/etc/superhot-tty/font.psfu",
            "/etc/superhot-tty/font.psf",
            "/usr/share/kbd/consolefonts/Lat2-Terminus16.psfu.gz",
            "/usr/share/kbd/consolefonts/Lat2-Terminus16.psf",
            "/usr/share/consolefonts/Lat2-Terminus16.psfu.gz",
            "/usr/share/consolefonts/Lat2-Terminus16.psf",
            "/usr/share/kbd/consolefonts/default8x16.psfu.gz",
            "/usr/share/kbd/consolefonts/default8x16.psf",
        ];
        for path in CANDIDATES {
            if let Ok(data) = load_maybe_gz(path) {
                if let Ok(f) = Self::from_bytes(&data) {
                    log::info!("loaded font from {} ({}x{} glyphs={})",
                        path, f.width, f.height, f.glyph_count);
                    return f;
                }
            }
        }
        log::warn!("no system PSF font found, using builtin fallback 8x16");
        Self::builtin_8x16()
    }

    pub fn glyph_for(&self, cp: u32) -> &[u8] {
        if !self.has_unicode_table {
            let idx = if cp < 0x80 { cp } else { self.cp_to_index(cp).unwrap_or(b'?' as u32) };
            let idx = idx.min(self.glyph_count - 1);
            let off = (idx * self.bytes_per_glyph) as usize;
            &self.glyphs[off..off + self.bytes_per_glyph as usize]
        } else {
            let idx = (cp as usize).min(self.glyph_count as usize - 1) as u32;
            let off = (idx * self.bytes_per_glyph) as usize;
            &self.glyphs[off..off + self.bytes_per_glyph as usize]
        }
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
        }
    }
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
