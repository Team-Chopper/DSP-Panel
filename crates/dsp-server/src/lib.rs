//! DSP panel server: owns the TCP connections to the DSPs, serves the web
//! client and talks to the browsers over one WebSocket. Port of the Dart
//! server (dart/server/), with the same API, files and behaviour.

pub mod auth;
pub mod config;
pub mod http;
pub mod hub;
pub mod light;
pub mod log;
pub mod panel_data;
pub mod presets;
pub mod registry;
pub mod safety;
pub mod scan;
pub mod session;

use std::net::SocketAddr;
use std::time::Duration;

use axum_server::Handle;
use axum_server::tls_rustls::RustlsConfig;
use tokio::sync::{mpsc, watch};
use tokio::task::spawn_local;

use crate::auth::AuthService;
use crate::config::ServerConfig;
use crate::http::AppState;
use crate::hub::{Hub, HubStores};
use crate::light::LightSnapshot;
use crate::panel_data::PanelStore;
use crate::registry::DeviceRegistry;
use crate::safety::SafetyStore;
use crate::session::Timing;

/// A running server (inside a tokio `LocalSet`).
pub struct Server {
    pub hub: Hub,
    pub local_addr: SocketAddr,
    handle: Handle,
}

impl Server {
    /// Load the data, start the DSP sessions and listen. Must run inside a
    /// `LocalSet`.
    pub async fn start(config: ServerConfig, timing: Timing) -> Result<Server, String> {
        let mut auth = AuthService::new(config.auth_enabled, config.data_dir.clone());
        auth.init()?;
        if auth.enabled && !auth.has_password() {
            log::warn("Login is enabled but no password is set – run \"dsp-server passwd\".");
        }
        if !auth.enabled {
            log::warn("Login is disabled – everyone who can reach this server can control the DSPs.");
        }

        let registry = DeviceRegistry::new(config.data_dir.join("devices.json"));
        let (light_tx, light_rx) = watch::channel(LightSnapshot::default());
        let stores = HubStores {
            presets_file: config.data_dir.join("presets.json"),
            safety: SafetyStore::new(config.data_dir.join("safety.json")),
            panel: PanelStore::new(config.data_dir.join("panel.json")),
            light: light_tx,
        };
        let hub = Hub::new(registry, stores, timing);
        hub.start()?;
        let (hub_tx, hub_rx) = mpsc::unbounded_channel();
        let runner = hub.clone();
        spawn_local(async move { runner.run(hub_rx).await });

        let address: std::net::IpAddr =
            config.address.parse().map_err(|_| format!("Invalid listen address {}", config.address))?;
        let addr = SocketAddr::new(address, config.port);
        let tls = config.tls_enabled;
        let tls_config = if tls {
            let _ = rustls::crypto::ring::default_provider().install_default();
            let rustls = RustlsConfig::from_pem_file(&config.tls_cert, &config.tls_key)
                .await
                .map_err(|e| format!("TLS: {e}"))?;
            Some(rustls)
        } else {
            None
        };

        let app = http::router(AppState::new(config, auth, hub_tx, light_rx))
            .into_make_service_with_connect_info::<SocketAddr>();
        let handle = Handle::new();
        let serve_handle = handle.clone();
        let task = tokio::spawn(async move {
            let result = match tls_config {
                Some(tls_config) => axum_server::bind_rustls(addr, tls_config).handle(serve_handle).serve(app).await,
                None => axum_server::bind(addr).handle(serve_handle).serve(app).await,
            };
            if let Err(e) = result {
                log::warn(format!("Server stopped: {e}"));
            }
        });
        let local_addr = match handle.listening().await {
            Some(local) => local,
            None => {
                let _ = task.await;
                hub.shutdown().await;
                return Err(format!("Could not listen on {addr}"));
            }
        };
        log::info(format!("Listening on {}://{local_addr}", if tls { "https" } else { "http" }));
        Ok(Server { hub, local_addr, handle })
    }

    pub async fn stop(&self) {
        self.handle.graceful_shutdown(Some(Duration::from_secs(1)));
        self.hub.shutdown().await;
    }
}
