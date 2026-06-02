//! HTTP infrastructure for audio streaming

use log::debug;

use std::time::Duration;
use tokio::runtime::Runtime;

pub fn get_runtime() -> &'static Runtime {
    static RUNTIME: std::sync::OnceLock<Runtime> = std::sync::OnceLock::new();
    RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("tunes4r-http")
            .enable_all()
            .build()
            .expect("Failed to build Tokio runtime")
    })
}

pub fn run_async<F, T>(future: F) -> T
where
    F: std::future::Future<Output = T> + Send + 'static,
    T: Send + 'static,
{
    let (tx, rx) = std::sync::mpsc::channel();
    get_runtime().spawn(async move {
        let result = future.await;
        let _ = tx.send(result);
    });
    rx.recv().expect("async task panicked or was dropped")
}

#[cfg(feature = "rustls-platform-verifier")]
pub fn build_http_client() -> reqwest::Client {
    use rustls::crypto::ring::default_provider;
    use rustls::ClientConfig;
    use rustls_platform_verifier::ConfigVerifierExt;

    let _ = get_runtime();

    debug!("[http] Building HTTP client...");

    run_async(async {
        let provider = default_provider();
        debug!("[http] Got default ring provider");

        let install_result = rustls::crypto::CryptoProvider::install_default(provider);
        match install_result {
            Ok(_) => debug!("[http] Installed default crypto provider"),
            Err(e) => debug!("[http] Crypto provider already installed: {:?}", e),
        }

        let config_result = ClientConfig::with_platform_verifier();

        match config_result {
            Ok(cfg) => {
                debug!("[http] Using platform verifier for TLS");
                reqwest::Client::builder()
                    .connect_timeout(Duration::from_secs(15))
                    .tcp_keepalive(Duration::from_secs(30))
                    .use_preconfigured_tls(cfg)
                    .build()
                    .expect("Failed to build HTTP client with platform verifier")
            }
            Err(e) => {
                debug!(
                    "[http] Platform verifier failed: {:?}, using default TLS",
                    e
                );
                match reqwest::Client::builder()
                    .connect_timeout(Duration::from_secs(15))
                    .tcp_keepalive(Duration::from_secs(30))
                    .build()
                {
                    Ok(client) => client,
                    Err(e) => panic!("Failed to build HTTP client: {:?}", e),
                }
            }
        }
    })
}

#[cfg(feature = "rustls-platform-verifier")]
pub fn build_blocking_http_client() -> reqwest::blocking::Client {
    use rustls::crypto::ring::default_provider;
    use rustls::ClientConfig;
    use rustls_platform_verifier::ConfigVerifierExt;

    debug!("[http] Building blocking HTTP client with rustls-platform-verifier...");

    let provider = default_provider();
    let _ = rustls::crypto::CryptoProvider::install_default(provider);
    debug!("[http] Installed ring crypto provider");

    let cfg = ClientConfig::with_platform_verifier()
        .expect("Failed to create TLS config with platform verifier");

    reqwest::blocking::Client::builder()
        .connect_timeout(Duration::from_secs(15))
        .tcp_keepalive(Duration::from_secs(30))
        .use_preconfigured_tls(cfg)
        .build()
        .expect("Failed to build blocking HTTP client")
}

#[cfg(not(feature = "rustls-platform-verifier"))]
pub fn build_blocking_http_client() -> reqwest::blocking::Client {
    reqwest::blocking::Client::builder()
        .connect_timeout(Duration::from_secs(15))
        .tcp_keepalive(Duration::from_secs(30))
        .build()
        .unwrap_or_default()
}

#[cfg(not(feature = "rustls-platform-verifier"))]
pub fn build_http_client() -> reqwest::Client {
    run_async(async {
        reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(15))
            .tcp_keepalive(Duration::from_secs(30))
            .build()
            .unwrap_or_default()
    })
}
