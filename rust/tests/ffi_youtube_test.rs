#![cfg(not(target_os = "android"))]

use std::ffi::{CStr, CString};

use tunes4r::ffi::{
    audio_engine_create, audio_engine_destroy, audio_engine_get_load_error,
    audio_engine_get_position, audio_engine_get_state, audio_engine_play, audio_engine_stop,
    AudioEngineHandle,
};

use tunes4r::PlaybackState;

struct EngineGuard(*mut AudioEngineHandle);

impl Drop for EngineGuard {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { audio_engine_destroy(self.0) };
        }
    }
}

fn make_engine() -> EngineGuard {
    let raw = audio_engine_create();
    assert!(!raw.is_null(), "audio_engine_create returned null");
    EngineGuard(raw)
}

fn cstr(s: &str) -> CString {
    CString::new(s).expect("CString::new")
}

fn ffi_state(h: *mut AudioEngineHandle) -> PlaybackState {
    PlaybackState::from_u8(audio_engine_get_state(h) as u8)
}

fn load_error(h: *mut AudioEngineHandle) -> Option<String> {
    let ptr = audio_engine_get_load_error(h);
    if ptr.is_null() {
        return None;
    }
    let s = unsafe { CStr::from_ptr(ptr) }.to_str().unwrap_or("").to_string();
    unsafe { drop(CString::from_raw(ptr)); }
    if s.is_empty() { None } else { Some(s) }
}

#[test]
fn ffi_play_youtube_via_id() {
    let engine = make_engine();
    let video_id = std::env::var("YT_VIDEO").unwrap_or_else(|_| "dQw4w9WgXcQ".to_string());
    let url = format!("https://www.youtube.com/watch?v={video_id}");

    println!("\n=== FFI YouTube Test ===");
    println!("Playing: {url}");

    let rc = unsafe { audio_engine_play(engine.0, cstr(&url).as_ptr(), -1) };
    assert_eq!(rc, 0, "audio_engine_play should return 0, got {rc}");

    let start = Instant::now();
    let timeout = Duration::from_secs(60);
    let mut reached_playing = false;

    while start.elapsed() < timeout {
        thread::sleep(Duration::from_millis(500));

        let state = ffi_state(engine.0);
        let pos = audio_engine_get_position(engine.0);
        let elapsed = start.elapsed().as_secs();

        print!(
            "[{elapsed:>4}s] state={state:12}  pos={:>8}ms / total={:>8}ms",
            pos.current_ms, pos.total_ms
        );

        if let Some(err) = load_error(engine.0) {
            print!("  ERROR: {err}");
        }
        println!();

        if state == PlaybackState::Playing {
            reached_playing = true;
            println!(">>> Reached Playing state! YouTube FFI works.");
            thread::sleep(Duration::from_secs(3));
            break;
        }

        if state == PlaybackState::Stopped || state == PlaybackState::Finished {
            let err = load_error(engine.0);
            if let Some(e) = err {
                panic!("Player stopped with error: {e}");
            }
            println!(">>> Player stopped (no error). Breaking.");
            break;
        }
    }

    assert!(reached_playing, "Never reached Playing state");
    audio_engine_stop(engine.0);
    println!("=== Test passed ===\n");
}
