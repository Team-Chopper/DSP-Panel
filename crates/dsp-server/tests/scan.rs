//! Network search against the fake DSP.

use std::collections::HashSet;
use std::net::Ipv4Addr;
use std::time::Duration;

use dsp_core::DeviceModel;
use dsp_server::scan::{Found, ScanTiming, probe, scan};
use fake_dsp::{FakeDsp, FakeOptions};

fn timing() -> ScanTiming {
    ScanTiming { connect: Duration::from_millis(300), answer: Duration::from_millis(500) }
}

#[tokio::test]
async fn a_dsp_answers_with_its_model_and_free_ports_are_skipped() {
    let mut fake = FakeDsp::new(FakeOptions { model: DeviceModel::DSP204, ..FakeOptions::default() });
    fake.start(0).await.unwrap();
    let port = fake.port();
    let found = probe(Ipv4Addr::LOCALHOST, port, timing()).await;
    assert_eq!(
        found,
        Some(Found { host: "127.0.0.1".into(), port, info: "DSP204 V0104P".into(), model: Some("dsp204".into()) })
    );
    // The connection was closed: the fake accepts the next one.
    assert!(probe(Ipv4Addr::LOCALHOST, port, timing()).await.is_some());
    fake.stop().await;
    assert_eq!(probe(Ipv4Addr::LOCALHOST, port, timing()).await, None);
}

#[tokio::test]
async fn scan_leaves_out_skipped_hosts() {
    let mut fake = FakeDsp::new(FakeOptions::default());
    fake.start(0).await.unwrap();
    let port = fake.port();
    let skip: HashSet<String> = (2..=254).map(|h| format!("127.0.0.{h}")).collect();
    let found = scan(&[[127, 0, 0]], &skip, port, timing()).await;
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].model.as_deref(), Some("dsp408"));
    let skip: HashSet<String> = (1..=254).map(|h| format!("127.0.0.{h}")).collect();
    assert!(scan(&[[127, 0, 0]], &skip, port, timing()).await.is_empty());
    fake.stop().await;
}
