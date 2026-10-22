//! Development tool: simulate DSP 206 / DSP 408 devices.
//!
//! ```text
//! fake-dsp --port 19761 --models 408,206
//! ```
//!
//! starts a simulated DSP 408 on port 19761 and a DSP 206 on port 19762.

use std::net::{IpAddr, Ipv4Addr};
use std::time::Duration;

use clap::Parser;
use dsp_core::DeviceModel;
use fake_dsp::{FakeDsp, FakeOptions};

#[derive(Parser)]
#[command(about = "Simulated DSP 206/408 devices for development")]
struct Args {
    /// TCP port of the first device
    #[arg(long, default_value_t = 19761)]
    port: u16,
    /// Comma-separated models, one device each (408, 206)
    #[arg(long, default_value = "408")]
    models: String,
    /// Response delay in ms
    #[arg(long, default_value_t = 5)]
    latency: u64,
    /// Do not print received set commands
    #[arg(short, long)]
    quiet: bool,
}

#[tokio::main]
async fn main() -> std::io::Result<()> {
    let args = Args::parse();
    let models: Vec<DeviceModel> = args
        .models
        .split(',')
        .map(|m| {
            let digits: String = m.chars().filter(char::is_ascii_digit).collect();
            DeviceModel::by_id(Some(&format!("dsp{digits}")))
        })
        .collect();

    let mut devices = Vec::new();
    for (i, model) in models.into_iter().enumerate() {
        let mut dsp = FakeDsp::new(FakeOptions {
            model,
            latency: Duration::from_millis(args.latency),
            verbose: !args.quiet,
            device_id: 1,
            label: Some(format!("Fake {} #{}", model.name, i + 1)),
        });
        dsp.start_on(IpAddr::V4(Ipv4Addr::UNSPECIFIED), args.port + i as u16).await?;
        println!("{} listening on port {}", dsp.device().label, dsp.port());
        devices.push(dsp);
    }
    tokio::signal::ctrl_c().await?;
    for mut d in devices {
        d.stop().await;
    }
    Ok(())
}
