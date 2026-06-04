use std::io::Read;
use std::marker::{Send, Sync};
use std::thread;

use crate::ffi::AudioEngineHandle;
use log::info;

struct ThreadSafeEngineHandle(*mut AudioEngineHandle);

unsafe impl Send for ThreadSafeEngineHandle {}
unsafe impl Sync for ThreadSafeEngineHandle {}

pub fn fetch_and_pipe(url: &str, engine: &AudioEngineHandle) -> Result<(), String> {
    info!("[fetch] fetch_and_pipe called with URL: {}", url);

    engine
        .playback()
        .write()
        .unwrap()
        .play_stream_from_bytes_internal(url)
        .map_err(|e| format!("Failed to start pipe playback: {}", e))?;

    let url_owned = url.to_string();
    let engine_ptr =
        ThreadSafeEngineHandle(engine as *const AudioEngineHandle as *mut AudioEngineHandle);

    thread::spawn(move || {
        info!("[fetch] Background thread starting with URL: {}", url_owned);
        if let Err(e) = fetch_and_pipe_internal(&url_owned, engine_ptr) {
            info!("[fetch] Error: {}", e);
        }
    });

    Ok(())
}

fn fetch_and_pipe_internal(url: &str, engine: ThreadSafeEngineHandle) -> Result<(), String> {
    info!("[fetch] fetch_and_pipe_internal with URL: {}", url);

    let client = crate::audio::http::build_blocking_http_client();
    let mut response = client
        .get(url)
        .send()
        .map_err(|e| format!("HTTP request failed: {}", e))?;

    if !response.status().is_success() {
        return Err(format!("HTTP {}", response.status()));
    }

    unsafe {
        (*engine.0)
            .playback()
            .write()
            .unwrap()
            .set_pipe_total_bytes(
                response
                    .headers()
                    .get("content-range")
                    .and_then(|v| v.to_str().ok())
                    .and_then(|v| v.split('/').next_back().and_then(|s| s.parse().ok()))
                    .unwrap_or(0),
            );

        let mut buffer = [0u8; 8192];
        loop {
            match response.read(&mut buffer) {
                Ok(0) => break,
                Ok(n) => {
                    (*engine.0)
                        .playback()
                        .write()
                        .unwrap()
                        .push_audio_bytes(&buffer[..n]);
                }
                Err(e) => return Err(e.to_string()),
            }
        }

        (*engine.0).playback().write().unwrap().end_audio_stream();
    }
    Ok(())
}
