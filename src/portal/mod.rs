//! xdg-desktop-portal backend для screen sharing.
//!
//! Реализует интерфейс org.freedesktop.impl.portal.ScreenCast через DBus.
//! Когда OBS/Discord запрашивают share screen через xdg-desktop-portal,
//! наш портал предоставляет PipeWire stream нашего DRM framebuffer.
//!
//! MVP: регистрируем DBus service name и логируем запросы.
//! Полная реализация: создать PipeWire node который читает DRM framebuffer
//! и отдаёт кадры клиентам.

use anyhow::{Context, Result};
use zbus::{interface, Connection};

pub struct PortalBackend {
    #[allow(dead_code)] // stored for future DBus introspection
    pub service_name: String,
    #[allow(dead_code)] // stored for future DBus introspection
    pub object_path: String,
}

/// org.freedesktop.impl.portal.ScreenCast interface implementation.
pub struct ScreenCast {
    session_counter: std::sync::atomic::AtomicU32,
}

#[interface(name = "org.freedesktop.impl.portal.ScreenCast")]
impl ScreenCast {
    async fn create_session(&self, _handle: u32) -> u32 {
        let id = self.session_counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        log::info!("ScreenCast.CreateSession → {}", id);
        id
    }

    async fn select_sources(&self, _session: u32) {
        log::info!("ScreenCast.SelectSources — auto-select monitor");
    }

    async fn start(&self, session: u32) -> u32 {
        log::info!("ScreenCast.Start session={}", session);
        1000 + session
    }
}

impl PortalBackend {
    pub async fn start(service_name: String, object_path: String) -> Result<Self> {
        let screencast = ScreenCast {
            session_counter: std::sync::atomic::AtomicU32::new(1),
        };

        let conn = Connection::session().await.context("DBus session connection")?;
        conn.object_server().at(object_path.as_str(), screencast).await.context("register ScreenCast object")?;
        conn.request_name(service_name.as_str()).await.context("request DBus name")?;
        log::info!("portal backend registered: {} at {}", service_name, object_path);

        Ok(PortalBackend { service_name, object_path })
    }
}
