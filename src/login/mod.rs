//! Login screen с PAM аутентификацией.
//!
//! Поток:
//!   1. При запуске WM показывает login screen (большой текст по центру, clock, hint).
//!   2. Пользователь нажимает Enter.
//!   3. Запрашивается логин (input field).
//!   4. Запрашивается пароль (input field, символы скрыты).
//!   5. PAM аутентификация через service "login".
//!   6. При успехе — переключаем UID/GID на пользователя, загружаем его конфиг,
//!      запускаем основное WM.
//!   7. При ошибке — показываем popup с сообщением, возвращаемся к шагу 1.
//!
//! PAM реализован через FFI к libpam (pam_authenticate, pam_acct_mgmt).

use anyhow::{Context, Result};
use std::ffi::{CString, CStr};
use std::os::unix::io::RawFd;
use crate::config::{Config, LoginCfg};
use crate::render::{Canvas, Font, TextRenderer};
use crate::ui::theme::Theme;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoginState {
    /// Показываем заголовок, ждём нажатия Enter.
    Welcome,
    /// Ввод логина.
    Username,
    /// Ввод пароля.
    Password,
    /// Аутентификация в процессе.
    Authenticating,
    /// Успех.
    Success,
    /// Ошибка, показываем сообщение.
    Error,
    /// Выйти из WM (пользователь нажал Esc на Welcome).
    Quit,
}

pub struct LoginScreen {
    pub state: LoginState,
    pub username: String,
    pub password: String,
    pub error_msg: String,
    pub cursor_blink: u32,
    /// Если Some — login успешен, пользователь аутентифицирован.
    pub authenticated_user: Option<String>,
    pub uid: u32,
    pub gid: u32,
    pub home_dir: String,
    pub shell: String,
}

impl LoginScreen {
    pub fn new() -> Self {
        LoginScreen {
            state: LoginState::Welcome,
            username: String::new(),
            password: String::new(),
            error_msg: String::new(),
            cursor_blink: 0,
            authenticated_user: None,
            uid: 0,
            gid: 0,
            home_dir: String::new(),
            shell: String::new(),
        }
    }

    pub fn handle_key(&mut self, key: &str, shift: bool, ctrl: bool) {
        let _ = (shift, ctrl);
        match self.state {
            LoginState::Welcome => {
                if key == "Return" || key == "space" {
                    self.state = LoginState::Username;
                } else if key == "Escape" {
                    self.state = LoginState::Quit;
                }
            }
            LoginState::Username => {
                match key {
                    "Return" => {
                        if !self.username.is_empty() {
                            self.state = LoginState::Password;
                        }
                    }
                    "BackSpace" => { self.username.pop(); }
                    "Escape" => {
                        self.username.clear();
                        self.state = LoginState::Welcome;
                    }
                    c if c.len() == 1 && c.chars().all(|ch| ch.is_ascii_graphic()) => {
                        if self.username.len() < 32 {
                            self.username.push(c.chars().next().unwrap());
                        }
                    }
                    _ => {}
                }
            }
            LoginState::Password => {
                match key {
                    "Return" => {
                        self.state = LoginState::Authenticating;
                        // Аутентификация произойдёт в основном потоке (см. try_authenticate).
                    }
                    "BackSpace" => { self.password.pop(); }
                    "Escape" => {
                        self.password.clear();
                        self.username.clear();
                        self.state = LoginState::Welcome;
                    }
                    c if c.len() == 1 && c.chars().all(|ch| ch.is_ascii_graphic()) => {
                        if self.password.len() < 64 {
                            self.password.push(c.chars().next().unwrap());
                        }
                    }
                    _ => {}
                }
            }
            LoginState::Error => {
                if key == "Return" || key == "Escape" {
                    self.username.clear();
                    self.password.clear();
                    self.error_msg.clear();
                    self.state = LoginState::Welcome;
                }
            }
            _ => {}
        }
    }

    /// Вызывает PAM для аутентификации. Возвращает Ok(user_info) при успехе.
    pub fn try_authenticate(&mut self, pam_service: &str) -> Result<()> {
        let user = self.username.clone();
        let pass = self.password.clone();
        match pam_authenticate(&user, &pass, pam_service) {
            Ok(user_info) => {
                self.uid = user_info.uid;
                self.gid = user_info.gid;
                self.home_dir = user_info.home_dir;
                self.shell = user_info.shell;
                self.authenticated_user = Some(user);
                self.state = LoginState::Success;
                Ok(())
            }
            Err(e) => {
                self.error_msg = format!("Login failed: {}", e);
                self.state = LoginState::Error;
                Err(e)
            }
        }
    }

    /// Рендерит login screen на canvas.
    pub fn render(&mut self, canvas: &Canvas, font: &Font, theme: &Theme, cfg: &LoginCfg, screen_w: u32, screen_h: u32) {
        self.cursor_blink = self.cursor_blink.wrapping_add(1);
        let fw = font.width as i32;
        let fh = font.height as i32;

        // BG.
        canvas.fill(theme.bg);

        // Glitch effect на фоне.
        let glitch = 0.15f32;
        if glitch > 0.0 {
            for i in 0..20 {
                let y = (i * 40 + (self.cursor_blink as i32 / 4) as usize) as i32 % screen_h as i32;
                canvas.fill_rect(0, y, screen_w, 1, theme.tile_bg_inactive);
            }
        }

        let text = TextRenderer::new(canvas, font);
        let title = cfg.effective_title();
        let subtitle = cfg.effective_subtitle();

        // Title — large center text.
        let title_color = cfg.title_color.as_ref()
            .map(|s| { let (r,g,b) = crate::config::parse_color(s); crate::ui::theme::Color(r,g,b) })
            .unwrap_or(theme.accent_magenta);
        let title_x = (screen_w as i32 - (title.len() as i32) * fw * 3) / 2;
        let title_y = (screen_h as i32 / 2) - fh * 4;
        // Triple-size title (каждый символ = 3x3 block).
        draw_large_text(canvas, font, &title, title_x, title_y, title_color, 3);

        // Subtitle.
        let sub_x = (screen_w as i32 - (subtitle.len() as i32) * fw) / 2;
        let sub_y = title_y + fh * 3 + 10;
        text.draw_text(sub_x, sub_y, &subtitle, theme.fg_dim, None);

        // Clock.
        if cfg.show_clock {
            let now = chrono_now();
            let clock_x = (screen_w as i32 - (now.len() as i32) * fw) / 2;
            text.draw_text(clock_x, sub_y + fh + 8, &now, theme.accent_cyan, None);
        }

        // State-specific UI.
        let center_y = screen_h as i32 / 2 + fh * 2;
        match self.state {
            LoginState::Welcome => {
                if cfg.show_hint {
                    let hint = if cfg.language == "ru" { "Нажмите Enter для входа" } else { "Press Enter to login" };
                    let hint_x = (screen_w as i32 - (hint.len() as i32) * fw) / 2;
                    let blink = (self.cursor_blink / 30) % 2 == 0;
                    if blink {
                        text.draw_text(hint_x, center_y, hint, theme.accent_cyan, None);
                    }
                }
            }
            LoginState::Username => {
                let label = if cfg.language == "ru" { "Логин:" } else { "Login:" };
                let prompt = format!("{} {}", label, self.username);
                let prompt_x = (screen_w as i32 - (prompt.len() as i32 + 1) * fw) / 2;
                text.draw_text(prompt_x, center_y, &prompt, theme.fg_default, None);
                // Cursor.
                let cx = prompt_x + (prompt.len() as i32) * fw;
                if (self.cursor_blink / 30) % 2 == 0 {
                    canvas.fill_rect(cx, center_y, fw as u32, fh as u32, theme.accent_magenta);
                }
            }
            LoginState::Password => {
                let label = if cfg.language == "ru" { "Пароль:" } else { "Password:" };
                let hidden: String = "•".repeat(self.password.len());
                let prompt = format!("{} {}", label, hidden);
                let prompt_x = (screen_w as i32 - (prompt.len() as i32 + 1) * fw) / 2;
                text.draw_text(prompt_x, center_y, &prompt, theme.fg_default, None);
                let cx = prompt_x + (prompt.len() as i32) * fw;
                if (self.cursor_blink / 30) % 2 == 0 {
                    canvas.fill_rect(cx, center_y, fw as u32, fh as u32, theme.accent_magenta);
                }
            }
            LoginState::Authenticating => {
                let msg = if cfg.language == "ru" { "Проверка..." } else { "Authenticating..." };
                let mx = (screen_w as i32 - (msg.len() as i32) * fw) / 2;
                text.draw_text(mx, center_y, msg, theme.accent_cyan, None);
            }
            LoginState::Error => {
                let msg = &self.error_msg;
                let mx = (screen_w as i32 - (msg.len() as i32) * fw).max(0) / 2;
                // Red border around error.
                let box_w = ((msg.len() as i32 + 4) * fw).min(screen_w as i32 - 40);
                let box_x = (screen_w as i32 - box_w) / 2;
                canvas.fill_rect(box_x, center_y - 4, box_w as u32, fh as u32 + 8, theme.popup_bg);
                canvas.rect_outline(box_x, center_y - 4, box_w as u32, fh as u32 + 8, 2, theme.error);
                text.draw_text(mx, center_y, msg, theme.error, None);
                let hint = if cfg.language == "ru" { "Enter — повторить" } else { "Enter — retry" };
                let hx = (screen_w as i32 - (hint.len() as i32) * fw) / 2;
                text.draw_text(hx, center_y + fh + 12, hint, theme.fg_dim, None);
            }
            LoginState::Success => {
                let msg = if cfg.language == "ru" { "Добро пожаловать!" } else { "Welcome!" };
                let mx = (screen_w as i32 - (msg.len() as i32) * fw) / 2;
                text.draw_text(mx, center_y, msg, theme.accent_cyan, None);
            }
            LoginState::Quit => {}
        }

        // Corner brackets (MCD style).
        let cs: i32 = 32;
        let pad: i32 = 16;
        let sw = screen_w as i32;
        let sh = screen_h as i32;
        canvas.fill_rect(pad, pad, cs as u32, 3, theme.accent_magenta);
        canvas.fill_rect(pad, pad, 3, cs as u32, theme.accent_magenta);
        canvas.fill_rect(sw - pad - cs, pad, cs as u32, 3, theme.accent_magenta);
        canvas.fill_rect(sw - pad - 3, pad, 3, cs as u32, theme.accent_magenta);
        canvas.fill_rect(pad, sh - pad - 3, cs as u32, 3, theme.accent_magenta);
        canvas.fill_rect(pad, sh - pad - cs, 3, cs as u32, theme.accent_magenta);
        canvas.fill_rect(sw - pad - cs, sh - pad - 3, cs as u32, 3, theme.accent_magenta);
        canvas.fill_rect(sw - pad - 3, sh - pad - cs, 3, cs as u32, theme.accent_magenta);
    }
}

/// Рисует текст большим кеглем (каждый символ = NxN block).
fn draw_large_text(canvas: &Canvas, font: &Font, text: &str, x: i32, y: i32, color: crate::ui::theme::Color, scale: u32) {
    let fw = font.width as i32;
    let fh = font.height as i32;
    let mut cx = x;
    for ch in text.chars() {
        let glyph = font.glyph_for(ch as u32);
        let bytes_per_row = ((font.width + 7) / 8) as usize;
        for row in 0..fh {
            for col in 0..fw {
                let row_off = (row as usize) * bytes_per_row;
                let byte_off = row_off + (col as usize) / 8;
                if byte_off >= glyph.len() { break; }
                let bit_off = 7 - ((col as usize) % 8);
                if (glyph[byte_off] >> bit_off) & 1 == 1 {
                    canvas.fill_rect(cx + col * scale as i32, y + row * scale as i32, scale, scale, color);
                }
            }
        }
        cx += fw * scale as i32;
    }
}

/// Простая реализация времени (без chrono).
fn chrono_now() -> String {
    let mut t: libc::time_t = 0;
    unsafe { libc::time(&mut t); }
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    unsafe { libc::localtime_r(&t, &mut tm); }
    format!("{:02}:{:02}:{:02}", tm.tm_hour, tm.tm_min, tm.tm_sec)
}

// ===== PAM via FFI =====

pub struct UserInfo {
    pub uid: u32,
    pub gid: u32,
    pub home_dir: String,
    pub shell: String,
}

#[cfg(feature = "pam")]
#[link(name = "pam")]
extern "C" {
    fn pam_start(service_name: *const libc::c_char, user: *const libc::c_char, pam_conv: *const PamConv, ph: *mut *mut PamHandle) -> i32;
    fn pam_end(ph: *mut PamHandle, status: i32) -> i32;
    fn pam_authenticate_raw(ph: *mut PamHandle, flags: i32) -> i32;
    fn pam_acct_mgmt(ph: *mut PamHandle, flags: i32) -> i32;
}

#[allow(non_camel_case_types)]
type PamHandle = libc::c_void;

#[repr(C)]
struct PamConv {
    conv: extern "C" fn(num_msg: i32, msg: *mut *const PamMessage, resp: *mut *mut PamResponse, appdata: *mut libc::c_void) -> i32,
    appdata: *mut libc::c_void,
}

#[repr(C)]
struct PamMessage {
    msg_style: i32,
    msg: *const libc::c_char,
}

#[repr(C)]
struct PamResponse {
    resp: *mut libc::c_char,
    resp_retcode: i32,
}

// Static password storage for PAM callback.
static mut PAM_PASSWORD: Option<String> = None;

extern "C" fn pam_conv_fn(num_msg: i32, msg: *mut *const PamMessage, resp: *mut *mut PamResponse, _appdata: *mut libc::c_void) -> i32 {
    unsafe {
        let responses = libc::calloc(num_msg as usize, std::mem::size_of::<PamResponse>()) as *mut PamResponse;
        if responses.is_null() { return 1; }
        for i in 0..num_msg {
            let m = *msg.offset(i as isize);
            if m.is_null() { continue; }
            let style = (*m).msg_style;
            match style {
                1 => { // PAM_PROMPT_ECHO_OFF — password
                    if let Some(pwd) = &PAM_PASSWORD {
                        let cstr = CString::new(pwd.as_str()).unwrap();
                        let len = pwd.len() + 1;
                        let buf = libc::calloc(len, 1) as *mut libc::c_char;
                        libc::strcpy(buf, cstr.as_ptr());
                        (*responses.offset(i as isize)).resp = buf;
                    }
                }
                2 => { // PAM_PROMPT_ECHO_ON — username (skip, we have it)
                }
                _ => {}
            }
        }
        *resp = responses;
        0
    }
}

/// Аутентифицирует пользователя через PAM.
#[cfg(feature = "pam")]
pub fn pam_authenticate(username: &str, password: &str, service: &str) -> Result<UserInfo> {
    unsafe {
        PAM_PASSWORD = Some(password.to_string());
    }
    let service_c = CString::new(service).unwrap();
    let user_c = CString::new(username).unwrap();
    let conv = PamConv {
        conv: pam_conv_fn,
        appdata: std::ptr::null_mut(),
    };
    let mut ph: *mut PamHandle = std::ptr::null_mut();
    let r = unsafe { pam_start(service_c.as_ptr(), user_c.as_ptr(), &conv, &mut ph) };
    if r != 0 {
        unsafe { PAM_PASSWORD = None; }
        anyhow::bail!("pam_start failed: {}", r);
    }
    let r = unsafe { pam_authenticate_raw(ph, 0) };
    if r != 0 {
        unsafe { pam_end(ph, r); PAM_PASSWORD = None; }
        anyhow::bail!("authentication failed (code {})", r);
    }
    let r = unsafe { pam_acct_mgmt(ph, 0) };
    if r != 0 {
        unsafe { pam_end(ph, r); PAM_PASSWORD = None; }
        anyhow::bail!("account management failed (code {})", r);
    }
    unsafe { pam_end(ph, 0); PAM_PASSWORD = None; }

    // Получаем user info из /etc/passwd через getpwnam.
    let user_c2 = CString::new(username).unwrap();
    let pw = unsafe { libc::getpwnam(user_c2.as_ptr()) };
    if pw.is_null() {
        anyhow::bail!("user '{}' not found in passwd", username);
    }
    let pw_ref = unsafe { &*pw };
    let home_dir = unsafe { CStr::from_ptr(pw_ref.pw_dir).to_string_lossy().to_string() };
    let shell = unsafe { CStr::from_ptr(pw_ref.pw_shell).to_string_lossy().to_string() };
    Ok(UserInfo {
        uid: pw_ref.pw_uid,
        gid: pw_ref.pw_gid,
        home_dir,
        shell,
    })
}

/// Fallback аутентификация без PAM — проверяем через /etc/passwd + /etc/shadow (crypt).
/// Используется когда libpam недоступен. Менее безопасно (не учитывает PAM политики).
#[cfg(not(feature = "pam"))]
pub fn pam_authenticate(username: &str, password: &str, _service: &str) -> Result<UserInfo> {
    use std::io::Read;
    // Получаем user info из /etc/passwd.
    let user_c = CString::new(username).unwrap();
    let pw = unsafe { libc::getpwnam(user_c.as_ptr()) };
    if pw.is_null() {
        anyhow::bail!("user '{}' not found in passwd", username);
    }
    let pw_ref = unsafe { &*pw };
    let uid = pw_ref.pw_uid;
    let gid = pw_ref.pw_gid;
    let home_dir = unsafe { CStr::from_ptr(pw_ref.pw_dir).to_string_lossy().to_string() };
    let shell = unsafe { CStr::from_ptr(pw_ref.pw_shell).to_string_lossy().to_string() };

    // Читаем /etc/shadow для hash пароля.
    let shadow = std::fs::read_to_string("/etc/shadow")
        .context("cannot read /etc/shadow (need root)")?;
    for line in shadow.lines() {
        let parts: Vec<&str> = line.split(':').collect();
        if parts.len() < 2 || parts[0] != username { continue; }
        let hash = parts[1];
        if hash.is_empty() || hash == "*" || hash == "!" {
            anyhow::bail!("account locked or no password");
        }
        // Используем libc crypt() для проверки.
        let hash_c = CString::new(hash).unwrap();
        let pass_c = CString::new(password).unwrap();
        let result = unsafe { crypt(pass_c.as_ptr(), hash_c.as_ptr()) };
        if result.is_null() {
            anyhow::bail!("crypt() failed");
        }
        let result_str = unsafe { CStr::from_ptr(result).to_string_lossy().to_string() };
        if result_str != hash {
            anyhow::bail!("invalid password");
        }
        return Ok(UserInfo { uid, gid, home_dir, shell });
    }
    anyhow::bail!("user '{}' not found in shadow", username);
}

#[cfg(not(feature = "pam"))]
#[allow(dead_code)]
fn _pam_conv_fn_stub() -> i32 { 0 }

#[cfg(not(feature = "pam"))]
#[link(name = "crypt")]
extern "C" {
    fn crypt(key: *const libc::c_char, salt: *const libc::c_char) -> *mut libc::c_char;
}

/// Переключает процесс на UID/GID пользователя (после успешной аутентификации).
pub fn switch_to_user(uid: u32, gid: u32, home_dir: &str) -> Result<()> {
    // setgid + setuid + initgroups.
    unsafe {
        if libc::setgid(gid) != 0 {
            anyhow::bail!("setgid failed: {}", std::io::Error::last_os_error());
        }
        if libc::initgroups(std::ptr::null(), gid) != 0 {
            log::warn!("initgroups failed: {}", std::io::Error::last_os_error());
        }
        if libc::setuid(uid) != 0 {
            anyhow::bail!("setuid failed: {}", std::io::Error::last_os_error());
        }
    }
    std::env::set_var("HOME", home_dir);
    std::env::set_var("USER", &format!("{}", uid));
    if std::env::var("SHELL").is_err() {
        std::env::set_var("SHELL", "/bin/zsh");
    }
    // XDG_RUNTIME_DIR для pipewire/portal.
    let xdg_runtime = format!("/run/user/{}", uid);
    std::env::set_var("XDG_RUNTIME_DIR", &xdg_runtime);
    // Создаём если нет.
    let _ = std::fs::create_dir_all(&xdg_runtime);
    Ok(())
}
