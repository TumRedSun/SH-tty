//! Геймпады.
//!
//! Для максимальной совместимости со Steam и играми используется гибрид:
//!   - evdev passthrough в Steam (Steam Input сам обрабатывает геймпад
//!     через /dev/input/event*). Это даёт нативную поддержку в Steam играх.
//!   - SDL2 опционально (через feature `gamepad-sdl2`) для не-Steam сценария
//!     — маппим кнопки в клавиши.
//!
//! Без feature `gamepad-sdl2` модуль работает в режиме "evdev passthrough only":
//! мы не вмешиваемся в геймпад, Steam может его захватить.
//!
//! При активации SDL2 (cargo build --features gamepad-sdl2):
//!   - Поддержка Xbox/PS4/PS5/Switch Pro/8BitDo/Steam Deck через SDL_GameController
//!   - Встроенный database контроллеров (gamecontrollerdb.txt)
//!   - Маппинг кнопок на клавиши через конфиг

use anyhow::Result;
use std::collections::HashMap;

#[cfg(not(feature = "gamepad-sdl2"))]
pub struct GamepadManager {
    pub passthrough_note: String,
}

#[cfg(not(feature = "gamepad-sdl2"))]
impl GamepadManager {
    pub fn new(_keymap: HashMap<String, String>, _stick_sensitivity: u32, _enabled: bool) -> Result<Self> {
        log::info!("gamepad: evdev passthrough mode (no SDL2). Steam Input works natively.");
        Ok(GamepadManager { passthrough_note: "evdev passthrough".into() })
    }

    pub fn poll(&mut self) -> Vec<GamepadKey> {
        Vec::new()
    }
}

#[cfg(feature = "gamepad-sdl2")]
pub struct GamepadManager {
    sdl: Option<sdl2::Sdl>,
    controller_subsystem: Option<sdl2::controller::GameControllerSubsystem>,
    controllers: Vec<sdl2::controller::GameController>,
    keymap: HashMap<String, String>,
    button_state: HashMap<sdl2::controller::Button, bool>,
    stick_state: HashMap<sdl2::controller::Axis, i16>,
    last_input: std::time::Instant,
    stick_sensitivity: u32,
}

#[cfg(feature = "gamepad-sdl2")]
impl GamepadManager {
    pub fn new(keymap: HashMap<String, String>, stick_sensitivity: u32, enabled: bool) -> Result<Self> {
        use anyhow::Context;
        if !enabled {
            return Ok(GamepadManager {
                sdl: None, controller_subsystem: None, controllers: Vec::new(),
                keymap, button_state: HashMap::new(), stick_state: HashMap::new(),
                last_input: std::time::Instant::now(), stick_sensitivity,
            });
        }
        let sdl = sdl2::init().context("SDL2 init")?;
        let controller_subsystem = sdl.game_controller().context("SDL2 game_controller subsystem")?;
        let mut controllers = Vec::new();
        for i in 0..controller_subsystem.num_joysticks().unwrap_or(0) {
            if controller_subsystem.is_game_controller(i) {
                match controller_subsystem.open(i) {
                    Ok(c) => {
                        log::info!("game controller connected: {}", c.name());
                        controllers.push(c);
                    }
                    Err(e) => log::warn!("failed to open controller {}: {}", i, e),
                }
            }
        }
        log::info!("gamepad manager initialized (SDL2), {} controllers active", controllers.len());
        Ok(GamepadManager {
            sdl: Some(sdl),
            controller_subsystem: Some(controller_subsystem),
            controllers,
            keymap,
            button_state: HashMap::new(),
            stick_state: HashMap::new(),
            last_input: std::time::Instant::now(),
            stick_sensitivity,
        })
    }

    pub fn poll(&mut self) -> Vec<GamepadKey> {
        let mut keys = Vec::new();
        if let Some(sdl) = &self.sdl {
            if let Some(mut pump) = sdl.event_pump() {
                use sdl2::controller::Axis;
                use sdl2::controller::Button;
                use sdl2::event::Event;
                for event in pump.poll_iter() {
                    match event {
                        Event::ControllerButtonDown { button, .. } => {
                            self.button_state.insert(button, true);
                            self.last_input = std::time::Instant::now();
                            if let Some(key) = self.button_to_key(button) {
                                keys.push(GamepadKey::Press(key));
                            }
                        }
                        Event::ControllerButtonUp { button, .. } => {
                            self.button_state.insert(button, false);
                            if let Some(key) = self.button_to_key(button) {
                                keys.push(GamepadKey::Release(key));
                            }
                        }
                        Event::ControllerAxisMotion { axis, value, .. } => {
                            self.stick_state.insert(axis, value);
                            let threshold = ((self.stick_sensitivity as i32) * 320).clamp(8000, 28000) as i16;
                            match axis {
                                Axis::LeftX | Axis::RightX => {
                                    if value > threshold { keys.push(GamepadKey::Press("Right".into())); }
                                    else if value < -threshold { keys.push(GamepadKey::Press("Left".into())); }
                                }
                                Axis::LeftY | Axis::RightY => {
                                    if value > threshold { keys.push(GamepadKey::Press("Down".into())); }
                                    else if value < -threshold { keys.push(GamepadKey::Press("Up".into())); }
                                }
                                _ => {}
                            }
                            self.last_input = std::time::Instant::now();
                        }
                        Event::ControllerAdded { which, .. } => {
                            if let Some(cs) = &self.controller_subsystem {
                                if let Ok(c) = cs.open(which) {
                                    log::info!("controller connected: {}", c.name());
                                    self.controllers.push(c);
                                }
                            }
                        }
                        Event::ControllerRemoved { which, .. } => {
                            self.controllers.retain(|c| c.instance_id() != which);
                        }
                        _ => {}
                    }
                }
            }
        }
        keys
    }

    fn button_to_key(&self, button: sdl2::controller::Button) -> Option<String> {
        use sdl2::controller::Button;
        let name = match button {
            Button::A => "a", Button::B => "b", Button::X => "x", Button::Y => "y",
            Button::DPadUp => "dpad_up", Button::DPadDown => "dpad_down",
            Button::DPadLeft => "dpad_left", Button::DPadRight => "dpad_right",
            Button::Start => "start", Button::Back => "back",
            Button::LeftShoulder => "left_shoulder", Button::RightShoulder => "right_shoulder",
            Button::LeftStick => "left_stick", Button::RightStick => "right_stick",
            Button::Guide => "guide",
        };
        self.keymap.get(name).cloned()
    }
}

#[derive(Debug, Clone)]
pub enum GamepadKey {
    Press(String),
    Release(String),
}
