//! PipeWire audio integration.
//!
//! Запускаем pipewire daemon + pipewire-pulse (PulseAudio совместимость) +
//! wireplumber (session manager). Это даёт нам:
//!   - Звук для X11 приложений через pipewire-pulse
//!   - Поддержку Bluetooth наушников
//!   - Прокрутку громкости через pactl
//!
//! Приложения (Discord, браузеры) видят PulseAudio API и работают нативно.
//! Steam тоже работает через pulseaudio.
//!
//! Если PipeWire уже запущен (например через systemd --user services),
//! новые экземпляры не создаются — это предотвращает конфликты сокетов
//! и duplicate daemon processes.

use anyhow::{Context, Result};
use std::process::{Child, Command};

pub struct AudioStack {
    pub pipewire: Option<Child>,
    pub pipewire_pulse: Option<Child>,
    pub wireplumber: Option<Child>,
}

impl AudioStack {
    /// Запускает полный PipeWire стек.
    ///
    /// Каждый daemon запускается только если он ещё не работает. Проверка
    /// делается через `pgrep -x <name>` — это быстрее и надёжнее чем проверка
    /// сокетов, и не требует парсинга /proc.
    pub fn start(start_pulse: bool, start_wireplumber: bool) -> Result<Self> {
        let mut stack = AudioStack { pipewire: None, pipewire_pulse: None, wireplumber: None };

        // 1. PipeWire daemon.
        stack.pipewire = spawn_if_not_running("pipewire");
        // Даём pipewire 500ms на инициализацию перед зависимыми сервисами.
        if stack.pipewire.is_some() {
            std::thread::sleep(std::time::Duration::from_millis(500));
        }

        // 2. pipewire-pulse (PulseAudio replacement).
        if start_pulse {
            stack.pipewire_pulse = spawn_if_not_running("pipewire-pulse");
        }

        // 3. WirePlumber session manager.
        if start_wireplumber {
            stack.wireplumber = spawn_if_not_running("wireplumber");
        }

        // Устанавливаем XDG_RUNTIME_DIR для клиентов (pipewire требует).
        if std::env::var("XDG_RUNTIME_DIR").is_err() {
            let runtime = format!("/run/user/{}", unsafe { libc::geteuid() });
            std::env::set_var("XDG_RUNTIME_DIR", &runtime);
        }

        log::info!("audio stack ready (pipewire={}, pulse={}, wireplumber={})",
            stack.pipewire.is_some(), stack.pipewire_pulse.is_some(), stack.wireplumber.is_some());
        Ok(stack)
    }

    /// Устанавливает системную громкость (0..100).
    /// Возвращает ошибку если pactl не смог выполнить команду.
    #[allow(dead_code)] // not wired to keybindings yet
    pub fn set_volume(volume: u32) -> Result<()> {
        let output = Command::new("pactl")
            .args(["set-sink-volume", "@DEFAULT_SINK@", &format!("{}%", volume)])
            .output()
            .context("failed to spawn pactl")?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("pactl set-sink-volume failed: {}", stderr.trim());
        }
        Ok(())
    }

    /// Mute toggle.
    #[allow(dead_code)] // not wired to keybindings yet
    pub fn toggle_mute() -> Result<()> {
        let output = Command::new("pactl")
            .args(["set-sink-mute", "@DEFAULT_SINK@", "toggle"])
            .output()
            .context("failed to spawn pactl")?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("pactl set-sink-mute failed: {}", stderr.trim());
        }
        Ok(())
    }
}

/// Запускает daemon если он ещё не работает.
///
/// Проверка через `pgrep -x` — exact match по имени процесса. pgrep
/// возвращает exit code 0 если найдён, 1 если не найден. Если сам pgrep
/// недоступен (маловероятно на Linux), пропускаем проверку и запускаем.
fn spawn_if_not_running(name: &str) -> Option<Child> {
    if is_process_running(name) {
        log::info!("{} already running — not starting duplicate", name);
        return None;
    }
    match Command::new(name).spawn() {
        Ok(c) => {
            log::info!("{} started (pid={})", name, c.id());
            Some(c)
        }
        Err(e) => {
            log::warn!("failed to start {}: {} — install {} package", name, e, name);
            None
        }
    }
}

/// Проверяет, запущен ли процесс с заданным именем (exact match).
/// Использует `pgrep -x` — если pgrep недоступен, возвращает false
/// (лучше запустить duplicate чем молча пропустить запуск daemon'а).
fn is_process_running(name: &str) -> bool {
    match Command::new("pgrep")
        .args(["-x", name])
        .output()
    {
        Ok(output) => output.status.success(),
        Err(_) => false, // pgrep not available — assume not running
    }
}

impl Drop for AudioStack {
    fn drop(&mut self) {
        // Корректно завершаем только те daemon'ы, которые мы сами запустили.
        // kill() + wait() гарантирует что child не останется zombie.
        for child in [&mut self.pipewire, &mut self.pipewire_pulse, &mut self.wireplumber] {
            if let Some(c) = child.as_mut() {
                let _ = c.kill();
                let _ = c.wait(); // reap zombie
            }
        }
    }
}
