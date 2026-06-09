//! Platform-specific shims for audio playback.
//!
//! Consolidates all `#[cfg(target_os)]` into one module so the rest of
//! the codebase can use simple type aliases and function calls.
//!
//! ## `platform_init()`
//! - On Android: initialises `android_logger` and attaches the current
//!   thread to the JVM.
//! - On other platforms: no-op.
//!
//! ## `HttpFetcher` trait + platform type alias
//! Abstracts over blocking (desktop/iOS) and async (Android) HTTP fetching,
//! pushing bytes into a `PipeWriter` for consumption by the decode loop.

use crate::audio::stream::pipe::PipeWriter;
use std::sync::Arc;

// ── HttpFetcher trait ─────────────────────────────────────────────────────

/// Platform-specific HTTP fetch that pushes bytes into a PipeWriter.
/// Android uses async Tokio; desktop/iOS use blocking reqwest.
pub trait HttpFetcher: Send + 'static {
    fn fetch(url: &str, range_start: u64, writer: Arc<PipeWriter>);
}

// ── Platform type alias ───────────────────────────────────────────────────

/// The HTTP fetcher for the current compilation target.
#[cfg(target_os = "android")]
pub type PlatformFetcher = AndroidFetcher;

#[cfg(not(target_os = "android"))]
pub type PlatformFetcher = BlockingFetcher;

// ── Platform init ─────────────────────────────────────────────────────────

#[cfg(target_os = "android")]
pub fn platform_init() {
    use jni::JavaVM;
    use std::sync::OnceLock;

    static JVM: OnceLock<JavaVM> = OnceLock::new();

    let _ = android_logger::init_once(
        android_logger::Config::default()
            .with_max_level(log::LevelFilter::Debug)
            .with_tag("tunes4r"),
    );

    let ctx = ndk_context::android_context();
    let vm = unsafe { JavaVM::from_raw(ctx.vm() as *mut jni::sys::JavaVM) }.ok();
    if let Some(jvm) = vm {
        let _ = JVM.set(jvm);
    }
    let _ = JVM.get().map(|jvm| jvm.attach_current_thread());
}

#[cfg(not(target_os = "android"))]
pub fn platform_init() {}

// ── BlockingFetcher (desktop / iOS) ───────────────────────────────────────

pub struct BlockingFetcher;

impl HttpFetcher for BlockingFetcher {
    fn fetch(url: &str, range_start: u64, writer: Arc<PipeWriter>) {
        use log::debug;
        use std::io::Read;

        let client = crate::audio::http::build_blocking_http_client();

        let mut req = client.get(url);
        if range_start > 0 {
            req = req.header("Range", format!("bytes={}-", range_start));
        }

        let mut resp = match req.send() {
            Ok(r) => r,
            Err(e) => {
                writer.set_error(format!("HTTP fetch failed: {}", e));
                return;
            }
        };

        let status = resp.status();
        if !status.is_success() && status != reqwest::StatusCode::PARTIAL_CONTENT {
            writer.set_error(format!("HTTP {}", status.as_u16()));
            return;
        }

        if let Some(cl) = resp.content_length() {
            writer.set_total_bytes(cl);
        }

        let mut buf = [0u8; 65536];
        loop {
            match resp.read(&mut buf) {
                Ok(0) => {
                    writer.end();
                    return;
                }
                Ok(n) => {
                    writer.push(&buf[..n]);
                }
                Err(e) => {
                    debug!("[platform] BlockingFetcher read error: {}", e);
                    writer.set_error(format!("Read error: {}", e));
                    return;
                }
            }
        }
    }
}

// ── AndroidFetcher (async Tokio) ──────────────────────────────────────────

#[cfg(target_os = "android")]
pub struct AndroidFetcher;

#[cfg(target_os = "android")]
impl HttpFetcher for AndroidFetcher {
    fn fetch(url: &str, range_start: u64, writer: Arc<PipeWriter>) {
        use log::debug;
        use std::thread;

        let url = url.to_string();
        thread::spawn(move || {
            let rt = match tokio::runtime::Runtime::new() {
                Ok(r) => r,
                Err(e) => {
                    writer.set_error(format!("Failed to create tokio runtime: {}", e));
                    return;
                }
            };
            rt.block_on(async move {
                let client = reqwest::Client::new();
                let mut req = client.get(&url);
                if range_start > 0 {
                    req = req.header("Range", format!("bytes={}-", range_start));
                }

                let mut resp = match req.send().await {
                    Ok(r) => r,
                    Err(e) => {
                        writer.set_error(format!("HTTP fetch failed: {}", e));
                        return;
                    }
                };

                let status = resp.status();
                if !status.is_success() && status != reqwest::StatusCode::PARTIAL_CONTENT {
                    writer.set_error(format!("HTTP {}", status.as_u16()));
                    return;
                }

                if let Some(cl) = resp.content_length() {
                    writer.set_total_bytes(cl);
                }

                loop {
                    match resp.chunk().await {
                        Ok(Some(data)) => {
                            writer.push(&data);
                        }
                        Ok(None) => {
                            writer.end();
                            return;
                        }
                        Err(e) => {
                            debug!("[platform] AndroidFetcher error: {}", e);
                            writer.set_error(format!("Stream error: {}", e));
                            return;
                        }
                    }
                }
            });
        });
    }
}
