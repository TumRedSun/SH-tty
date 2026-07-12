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

use anyhow::{Context, Result};
use std::process::{Child, Command};

pub struct AudioStack {
    pub pipewire: Option<Child>,
    pub pipewire_pulse: Option<Child>,
    pub wireplumber: Option<Child>,
}

impl AudioStack {
    /// Запускает полный PipeWire стек.
    pub fn start(start_pulse: bool, start_wireplumber: bool) -> Result<Self> {
        let mut stack = AudioStack { pipewire: None, pipewire_pulse: None, wireplumber: None };

        // 1. PipeWire daemon.
        match Command::new("pipewire").spawn() {
            Ok(c) => {
                log::info!("pipewire started (pid={})", c.id());
                stack.pipewire = Some(c);
            }
            Err(e) => log::warn!("failed to start pipewire: {} — install pipewire package", e),
        }
        // Даём pipewire 500ms на инициализацию.
        std::thread::sleep(std::time::Duration::from_millis(500));

        // 2. pipewire-pulse (PulseAudio replacement).
        if start_pulse {
            match Command::new("pipewire-pulse").spawn() {
                Ok(c) => {
                    log::info!("pipewire-pulse started (pid={})", c.id());
                    stack.pipewire_pulse = Some(c);
                }
                Err(e) => log::warn!("failed to start pipewire-pulse: {}", e),
            }
        }

        // 3. WirePlumber session manager.
        if start_wireplumber {
            match Command::new("wireplumber").spawn() {
                Ok(c) => {
                    log::info!("wireplumber started (pid={})", c.id());
                    stack.wireplumber = Some(c);
                }
                Err(e) => log::warn!("failed to start wireplumber: {}", e),
            }
        }

        // Устанавливаем XDG_RUNTIME_DIR для клиентов (pipewire требует).
        // Также устанавливаем PULSE_SERVER для pulseaudio-клиентов.
        if std::env::var("XDG_RUNTIME_DIR").is_err() {
            let runtime = format!("/run/user/{}", unsafe { libc::geteuid() });
            std::env::set_var("XDG_RUNTIME_DIR", &runtime);
        }

        log::info!("audio stack started");
        Ok(stack)
    }

    /// Устанавливает системную громкость (0..100).
    pub fn set_volume(volume: u32) -> Result<()> {
        let _ = Command::new("pactl")
            .args(["set-sink-volume", "@DEFAULT_SINK@", &format!("{}%", volume)])
            .output()
            .context("pactl set-sink-volume")?;
        Ok(())
    }

    /// Mute toggle.
    pub fn toggle_mute() -> Result<()> {
        let _ = Command::new("pactl")
            .args(["set-sink-mute", "@DEFAULT_SINK@", "toggle"])
            .output()
            .context("pactl set-sink-mute")?;
        Ok(())
    }
}

impl Drop for AudioStack {
    fn drop(&mut self) {
        for child in [&mut self.pipewire, &mut self.pipewire_pulse, &mut self.wireplumber] {
            if let Some(c) = child.as_mut() {
                let _ = c.kill();
                let _ = c.wait();
            }
        }
    }
}
