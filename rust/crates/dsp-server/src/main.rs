//! DSP panel server (DSP 408 / DSP 206)
//!
//! ```text
//! dsp-server [serve] --config /etc/dsp-panel/config.yaml
//! dsp-server passwd  --config /etc/dsp-panel/config.yaml
//! ```

use std::io::{BufRead, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::Parser;
use dsp_server::auth::AuthService;
use dsp_server::config::{ServerConfig, absolute};
use dsp_server::session::Timing;
use dsp_server::{Server, log};

const DEFAULT_CONFIG_PATH: &str = "/etc/dsp-panel/config.yaml";

#[derive(Parser)]
#[command(name = "dsp-server", version, about = "DSP panel server (DSP 408 / DSP 206)")]
struct Args {
    /// serve (default) or passwd
    command: Option<String>,
    /// Config file (default: /etc/dsp-panel/config.yaml if it exists)
    #[arg(short, long)]
    config: Option<PathBuf>,
    /// Override listen port
    #[arg(long)]
    port: Option<u16>,
    /// Override listen address
    #[arg(long)]
    address: Option<String>,
    /// Override data directory
    #[arg(long = "data-dir")]
    data_dir: Option<PathBuf>,
    /// Override web client directory
    #[arg(long = "web-root")]
    web_root: Option<PathBuf>,
    /// Override TLS: on
    #[arg(long, overrides_with = "no_tls")]
    tls: bool,
    /// Override TLS: off
    #[arg(long = "no-tls", overrides_with = "tls")]
    no_tls: bool,
    /// Override login: on
    #[arg(long, overrides_with = "no_auth")]
    auth: bool,
    /// Override login: off
    #[arg(long = "no-auth", overrides_with = "auth")]
    no_auth: bool,
    /// Debug output
    #[arg(short, long)]
    verbose: bool,
}

fn flag(on: bool, off: bool) -> Option<bool> {
    if on {
        Some(true)
    } else if off {
        Some(false)
    } else {
        None
    }
}

fn load_config(args: &Args) -> Result<ServerConfig, String> {
    let path = args.config.clone().or_else(|| {
        let default = Path::new(DEFAULT_CONFIG_PATH);
        default.exists().then(|| default.to_path_buf())
    });
    let mut config = match path {
        Some(path) => ServerConfig::load(&path)?,
        None => ServerConfig::default(),
    };
    if let Some(port) = args.port {
        config.port = port;
    }
    if let Some(address) = &args.address {
        config.address = address.clone();
    }
    if let Some(dir) = &args.data_dir {
        config.data_dir = absolute(dir);
    }
    if let Some(dir) = &args.web_root {
        config.web_root = absolute(dir);
    }
    if let Some(tls) = flag(args.tls, args.no_tls) {
        config.tls_enabled = tls;
    }
    if let Some(auth) = flag(args.auth, args.no_auth) {
        config.auth_enabled = auth;
    }
    Ok(config)
}

fn main() -> ExitCode {
    let args = match Args::try_parse() {
        Ok(args) => args,
        Err(e) if e.use_stderr() => {
            let _ = e.print();
            return ExitCode::from(64);
        }
        Err(e) => {
            let _ = e.print();
            return ExitCode::SUCCESS;
        }
    };
    log::set_verbose(args.verbose);
    let config = match load_config(&args) {
        Ok(config) => config,
        Err(e) => {
            eprintln!("Config error: {e}");
            return ExitCode::from(78);
        }
    };
    match args.command.as_deref().unwrap_or("serve") {
        "serve" => serve(config),
        "passwd" => passwd(&config),
        other => {
            eprintln!("Unknown command \"{other}\" (serve or passwd)");
            ExitCode::from(64)
        }
    }
}

fn serve(config: ServerConfig) -> ExitCode {
    log::info(format!("Starting DSP panel server ({config})"));
    let runtime = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
        Ok(runtime) => runtime,
        Err(e) => {
            eprintln!("Could not start: {e}");
            return ExitCode::FAILURE;
        }
    };
    let local = tokio::task::LocalSet::new();
    local.block_on(&runtime, async move {
        let tls = config.tls_enabled;
        let (cert, key) = (config.tls_cert.clone(), config.tls_key.clone());
        let server = match Server::start(config, Timing::default()).await {
            Ok(server) => server,
            Err(e) => {
                eprintln!("Could not start server: {e}");
                if tls {
                    eprintln!("Check tls.cert ({}) and tls.key ({}).", cert.display(), key.display());
                }
                return ExitCode::FAILURE;
            }
        };
        let signal = shutdown_signal().await;
        log::info(format!("Received {signal}, shutting down"));
        server.stop().await;
        ExitCode::SUCCESS
    })
}

async fn shutdown_signal() -> &'static str {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};
        match signal(SignalKind::terminate()) {
            Ok(mut term) => tokio::select! {
                _ = tokio::signal::ctrl_c() => "SIGINT",
                _ = term.recv() => "SIGTERM",
            },
            Err(_) => {
                let _ = tokio::signal::ctrl_c().await;
                "SIGINT"
            }
        }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
        "SIGINT"
    }
}

fn passwd(config: &ServerConfig) -> ExitCode {
    let password = if !std::io::stdin().is_terminal() {
        // Allow scripting: read the password from the first line of stdin.
        let mut line = String::new();
        let _ = std::io::stdin().lock().read_line(&mut line);
        line.trim_end_matches(['\r', '\n']).to_string()
    } else {
        let prompt = |text: &str| {
            let _ = std::io::stdout().flush();
            rpassword::prompt_password(text).unwrap_or_default()
        };
        let password = prompt("New password: ");
        if password.chars().count() < 8 {
            eprintln!("Password must have at least 8 characters.");
            return ExitCode::FAILURE;
        }
        if prompt("Repeat password: ") != password {
            eprintln!("Passwords do not match.");
            return ExitCode::FAILURE;
        }
        password
    };
    if password.chars().count() < 8 {
        eprintln!("Password must have at least 8 characters.");
        return ExitCode::FAILURE;
    }
    let mut auth = AuthService::new(true, config.data_dir.clone());
    if let Err(e) = auth.init().and_then(|()| auth.set_password(&password)) {
        eprintln!("Could not save the password: {e}");
        return ExitCode::FAILURE;
    }
    println!(
        "Password saved in {}. Existing logins are now invalid.",
        config.data_dir.join("auth.json").display()
    );
    if !config.auth_enabled {
        println!("Note: login is currently disabled (auth.enabled: false).");
    }
    ExitCode::SUCCESS
}
