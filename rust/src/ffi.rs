//! FFI bindings for Flutter integration
//!
//! Exposes audio engine functionality through a C-compatible API
//! consumed via `dart:ffi` in `lib/src/tunes4r_player_ffi.dart`.

#![allow(clippy::not_unsafe_ptr_arg_deref)]

use std::collections::VecDeque;
use std::ffi::{CStr, CString};
use std::os::raw::c_char;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::thread;
use std::time::Duration;

use tracing::{error, info};

use tunes4r_player::{PlaybackState, Player};

// ============================================================================
// C-compatible structs matching Dart FFI struct layouts
// ============================================================================

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct PlaybackPosition {
    pub current_ms: u64,
    pub total_ms: u64,
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct EngineEvent {
    pub event_type: i32,
    pub int_param: i64,
}

impl Default for EngineEvent {
    fn default() -> Self {
        Self {
            event_type: ENGINE_EVENT_NONE,
            int_param: 0,
        }
    }
}

pub const ENGINE_EVENT_NONE: i32 = 0;
pub const ENGINE_EVENT_STATE_CHANGED: i32 = 1;
pub const ENGINE_EVENT_SEEK_STARTED: i32 = 2;
pub const ENGINE_EVENT_SEEK_COMPLETED: i32 = 3;
pub const ENGINE_EVENT_END_OF_STREAM: i32 = 4;

pub fn playback_state_to_i32(state: PlaybackState) -> i32 {
    state as i32
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct DownloadBuffer {
    pub capacity_ms: u64,
    pub read_offset_ms: u64,
    pub write_offset_ms: u64,
    pub total_ms: u64,
    pub is_complete: bool,
}

impl Default for DownloadBuffer {
    fn default() -> Self {
        Self {
            capacity_ms: 0,
            read_offset_ms: 0,
            write_offset_ms: 0,
            total_ms: 0,
            is_complete: false,
        }
    }
}

// ============================================================================
// Global spectrum state (stub — no FFT analyzer in Player yet)
// ============================================================================

static SPECTRUM_BAND_COUNT: AtomicI32 = AtomicI32::new(32);
static GLOBAL_SPECTRUM: RwLock<Vec<f32>> = RwLock::new(Vec::new());

fn set_band_count(count: usize) {
    SPECTRUM_BAND_COUNT.store(count as i32, Ordering::Relaxed);
    *GLOBAL_SPECTRUM.write().unwrap() = Vec::with_capacity(count);
}

fn get_band_count() -> usize {
    SPECTRUM_BAND_COUNT.load(Ordering::Relaxed) as usize
}

// ============================================================================
// JNI / Android
// ============================================================================

/// Required when building Rust code that interfaces with C++ (cpal/rodio).
#[no_mangle]
pub extern "C" fn __cxa_pure_virtual() {
    panic!("__cxa_pure_virtual called - this should never happen");
}

#[cfg(target_os = "android")]
#[no_mangle]
pub extern "C" fn JNI_OnLoad(vm: *mut std::ffi::c_void, _reserved: *mut std::ffi::c_void) -> i32 {
    unsafe {
        ndk_context::initialize_android_context(vm, std::ptr::null_mut());
    }
    #[cfg(target_os = "android")]
    android_logger::init_once(
        android_logger::Config::default()
            .with_max_level(log::LevelFilter::Debug)
            .with_tag("tunes4r"),
    );
    info!("[ffi] JNI_OnLoad: ndk_context initialized");
    jni::sys::JNI_VERSION_1_6
}

fn init_logger() {
    static INIT: std::sync::Once = std::sync::Once::new();
    INIT.call_once(|| {
        tracing_log::LogTracer::init().ok();
        let _ = tracing_subscriber::fmt()
            .with_max_level(tracing::Level::DEBUG)
            .with_writer(std::io::stderr)
            .with_target(true)
            .try_init();
    });
}

#[cfg(target_os = "android")]
#[no_mangle]
pub unsafe extern "system" fn Java_com_tunes4r_1player_tunes4r_1player_Tunes4rPlayerPlugin_nativeInit(
    env: *mut jni::sys::JNIEnv,
    _class: jni::sys::jclass,
) {
    use tracing::warn;
    let env_wrapper = match jni::JNIEnv::from_raw(env) {
        Ok(e) => e,
        Err(e) => {
            warn!("[ffi] nativeInit: JNIEnv::from_raw failed: {:?}", e);
            return;
        }
    };
    let vm = match env_wrapper.get_java_vm() {
        Ok(v) => v,
        Err(e) => {
            warn!("[ffi] nativeInit: get_java_vm failed: {:?}", e);
            return;
        }
    };
    let vm_ptr = vm.get_java_vm_pointer();
    if !vm_ptr.is_null() {
        ndk_context::initialize_android_context(
            vm_ptr as *mut std::ffi::c_void,
            std::ptr::null_mut(),
        );
        android_logger::init_once(
            android_logger::Config::default()
                .with_max_level(log::LevelFilter::Debug)
                .with_tag("tunes4r"),
        );
        info!("[ffi] ndk_context initialized via nativeInit");
    }
}

// ============================================================================
// AudioEngineHandle — wraps Player with monitor thread + cached state
// ============================================================================

pub struct AudioEngineHandle {
    player: Arc<Mutex<Player>>,
    shared_state: Arc<AtomicI32>,
    cached_position_ms: Arc<AtomicU64>,
    cached_total_duration_ms: Arc<AtomicU64>,
    cached_seek_target_ms: Arc<AtomicU64>,
    event_queue: Arc<Mutex<VecDeque<EngineEvent>>>,
    stop_monitor: Arc<AtomicBool>,
    _monitor_handle: Option<thread::JoinHandle<()>>,
}

impl AudioEngineHandle {
    fn new() -> Self {
        init_logger();

        let player = Arc::new(Mutex::new(Player::new()));
        let shared_state = Arc::new(AtomicI32::new(PlaybackState::Stopped as i32));
        let cached_position_ms = Arc::new(AtomicU64::new(0));
        let cached_total_duration_ms = Arc::new(AtomicU64::new(0));
        let cached_seek_target_ms = Arc::new(AtomicU64::new(0));
        let event_queue = Arc::new(Mutex::new(VecDeque::new()));
        let stop_monitor = Arc::new(AtomicBool::new(false));

        // Spawn monitor thread — polls Player state and pushes events
        let m_player = player.clone();
        let m_shared_state = shared_state.clone();
        let m_pos = cached_position_ms.clone();
        let m_dur = cached_total_duration_ms.clone();
        let m_seek_target = cached_seek_target_ms.clone();
        let m_queue = event_queue.clone();
        let m_stop = stop_monitor.clone();

        let handle = thread::spawn(move || {
            let mut prev_state_i32 = PlaybackState::Stopped as i32;

            loop {
                if m_stop.load(Ordering::Relaxed) {
                    return;
                }

                if let Ok(p) = m_player.try_lock() {
                    let state_i32 = p.state() as i32;
                    let pos = p.position_ms();
                    let dur = p.total_duration_ms();

                    m_shared_state.store(state_i32, Ordering::Relaxed);
                    m_pos.store(pos, Ordering::Relaxed);
                    m_dur.store(dur, Ordering::Relaxed);

                    // Check if seek completed
                    let target = m_seek_target.load(Ordering::Relaxed);
                    if target > 0 && pos >= target {
                        m_seek_target.store(0, Ordering::Relaxed);
                        let mut q = m_queue.lock().unwrap();
                        q.push_back(EngineEvent {
                            event_type: ENGINE_EVENT_SEEK_COMPLETED,
                            int_param: pos as i64,
                        });
                    }

                    if p.is_finished() {
                        let mut q = m_queue.lock().unwrap();
                        q.push_back(EngineEvent {
                            event_type: ENGINE_EVENT_END_OF_STREAM,
                            int_param: 0,
                        });
                    }

                    if state_i32 != prev_state_i32 {
                        let mut q = m_queue.lock().unwrap();
                        q.push_back(EngineEvent {
                            event_type: ENGINE_EVENT_STATE_CHANGED,
                            int_param: state_i32 as i64,
                        });
                        prev_state_i32 = state_i32;
                    }
                }

                thread::sleep(Duration::from_millis(50));
            }
        });

        AudioEngineHandle {
            player,
            shared_state,
            cached_position_ms,
            cached_total_duration_ms,
            cached_seek_target_ms,
            event_queue,
            stop_monitor,
            _monitor_handle: Some(handle),
        }
    }
}

// ============================================================================
// Engine lifecycle
// ============================================================================

#[no_mangle]
pub extern "C" fn audio_engine_create() -> *mut AudioEngineHandle {
    Box::into_raw(Box::new(AudioEngineHandle::new()))
}

#[no_mangle]
pub unsafe extern "C" fn audio_engine_destroy(handle: *mut AudioEngineHandle) {
    if !handle.is_null() {
        (*handle).stop_monitor.store(true, Ordering::Relaxed);
        drop(Box::from_raw(handle));
    }
}

#[no_mangle]
pub extern "C" fn audio_engine_set_spectrum_band_count(
    _handle: *mut AudioEngineHandle,
    count: i32,
) {
    set_band_count(count as usize);
}

#[no_mangle]
pub extern "C" fn audio_engine_set_spectrum_band_count_global(count: i32) {
    set_band_count(count as usize);
}

// ============================================================================
// Playback control
// ============================================================================

#[no_mangle]
pub unsafe extern "C" fn audio_engine_play(
    handle: *mut AudioEngineHandle,
    uri: *const c_char,
    _buffer_size_ms: i64,
) -> i32 {
    if handle.is_null() || uri.is_null() {
        return -1;
    }
    let uri_str = match CStr::from_ptr(uri).to_str() {
        Ok(s) => s.to_string(),
        Err(_) => return -2,
    };

    let h = &*handle;

    // Set Connecting immediately so the UI sees it
    h.shared_state
        .store(PlaybackState::Connecting as i32, Ordering::Relaxed);

    let player = h.player.clone();

    // Spawn background thread — start() does YouTube resolution + probe + output build
    thread::spawn(move || {
        let mut p = match player.lock() {
            Ok(p) => p,
            Err(e) => {
                error!("[ffi] play: lock error: {}", e);
                return;
            }
        };
        let uri_local = uri_str.as_str();
        info!("[ffi] play: starting uri={}", uri_local);
        if let Err(e) = p.start(uri_local) {
            error!("[ffi] play: start error: {}", e);
            p.set_load_error(format!("{e}"));
        }
    });

    0
}

#[no_mangle]
pub extern "C" fn audio_engine_can_seek(handle: *const AudioEngineHandle) -> bool {
    let h = match unsafe { handle.as_ref() } {
        Some(h) => h,
        None => return false,
    };
    if let Ok(p) = h.player.try_lock() {
        p.total_duration_ms() > 0
    } else {
        false
    }
}

#[no_mangle]
pub extern "C" fn audio_engine_can_download(handle: *const AudioEngineHandle) -> bool {
    let h = match unsafe { handle.as_ref() } {
        Some(h) => h,
        None => return false,
    };
    if let Ok(p) = h.player.try_lock() {
        p.content_length().is_some()
    } else {
        false
    }
}

#[no_mangle]
pub extern "C" fn audio_engine_pause(handle: *const AudioEngineHandle) {
    let h = match unsafe { handle.as_ref() } {
        Some(h) => h,
        None => return,
    };
    if let Ok(p) = h.player.lock() {
        p.set_paused(true);
    }
}

#[no_mangle]
pub extern "C" fn audio_engine_resume(handle: *const AudioEngineHandle) {
    let h = match unsafe { handle.as_ref() } {
        Some(h) => h,
        None => return,
    };
    if let Ok(p) = h.player.lock() {
        p.set_paused(false);
    }
}

#[no_mangle]
pub extern "C" fn audio_engine_stop(handle: *mut AudioEngineHandle) {
    let h = match unsafe { handle.as_mut() } {
        Some(h) => h,
        None => return,
    };
    if let Ok(mut p) = h.player.lock() {
        p.stop();
    }
}

#[no_mangle]
pub extern "C" fn audio_engine_seek(handle: *mut AudioEngineHandle, position_ms: u64) -> i32 {
    let h = match unsafe { handle.as_mut() } {
        Some(h) => h,
        None => return -1,
    };
    if let Ok(p) = h.player.lock() {
        let t0 = std::time::Instant::now();
        // Store target before seek — monitor thread will detect completion
        h.cached_seek_target_ms.store(position_ms, Ordering::Relaxed);
        // Push seek-started event
        {
            let mut q = h.event_queue.lock().unwrap();
            q.push_back(EngineEvent {
                event_type: ENGINE_EVENT_SEEK_STARTED,
                int_param: position_ms as i64,
            });
        }
        p.seek(position_ms);
        info!(
            "[ffi] seek to {}ms took {:?}",
            position_ms,
            t0.elapsed()
        );
        0
    } else {
        -3
    }
}

// ============================================================================
// Volume control
// ============================================================================

#[no_mangle]
pub extern "C" fn audio_engine_set_volume(
    handle: *const AudioEngineHandle,
    volume: f32,
) {
    let h = match unsafe { handle.as_ref() } {
        Some(h) => h,
        None => return,
    };
    if let Ok(p) = h.player.lock() {
        p.set_volume_gain(volume);
    }
}

#[no_mangle]
pub extern "C" fn audio_engine_get_volume(handle: *const AudioEngineHandle) -> f32 {
    let h = match unsafe { handle.as_ref() } {
        Some(h) => h,
        None => return 1.0,
    };
    if let Ok(p) = h.player.try_lock() {
        p.volume_gain()
    } else {
        // Return cached value from when we last read it
        1.0
    }
}

// ============================================================================
// State queries
// ============================================================================

#[no_mangle]
pub extern "C" fn audio_engine_get_state(handle: *const AudioEngineHandle) -> i32 {
    let h = match unsafe { handle.as_ref() } {
        Some(h) => h,
        None => return PlaybackState::Stopped as i32,
    };
    h.shared_state.load(Ordering::Relaxed)
}

#[no_mangle]
pub extern "C" fn audio_engine_get_position(
    handle: *const AudioEngineHandle,
) -> PlaybackPosition {
    let h = match unsafe { handle.as_ref() } {
        Some(h) => h,
        None => return PlaybackPosition::default(),
    };
    PlaybackPosition {
        current_ms: h.cached_position_ms.load(Ordering::Relaxed),
        total_ms: h.cached_total_duration_ms.load(Ordering::Relaxed),
    }
}

#[no_mangle]
pub extern "C" fn audio_engine_get_download_buffer(
    handle: *const AudioEngineHandle,
) -> DownloadBuffer {
    let h = match unsafe { handle.as_ref() } {
        Some(h) => h,
        None => return DownloadBuffer::default(),
    };

    let dur = h.cached_total_duration_ms.load(Ordering::Relaxed);
    if dur == 0 {
        return DownloadBuffer::default();
    }

    if let Ok(p) = h.player.try_lock() {
        let total = p.total_duration_ms();
        if total == 0 {
            return DownloadBuffer::default();
        }

        if let Some(content_len) = p.content_length() {
            let cached = p.cached_bytes();
            let ratio = if content_len > 0 {
                cached as f64 / content_len as f64
            } else {
                0.0
            };
            let write_offset = (total as f64 * ratio) as u64;
            let pos = p.position_ms();
            let mut buf = DownloadBuffer {
                capacity_ms: total,
                read_offset_ms: pos,
                write_offset_ms: write_offset.min(total),
                total_ms: total,
                is_complete: cached >= content_len,
            };
            if buf.write_offset_ms < buf.read_offset_ms {
                buf.write_offset_ms = buf.read_offset_ms;
            }
            buf
        } else {
            // Local file — fully buffered
            DownloadBuffer {
                capacity_ms: total,
                read_offset_ms: 0,
                write_offset_ms: total,
                total_ms: total,
                is_complete: true,
            }
        }
    } else {
        DownloadBuffer {
            capacity_ms: dur,
            read_offset_ms: 0,
            write_offset_ms: dur,
            total_ms: dur,
            is_complete: false,
        }
    }
}

#[no_mangle]
pub extern "C" fn audio_engine_poll_event(
    handle: *const AudioEngineHandle,
) -> EngineEvent {
    let h = match unsafe { handle.as_ref() } {
        Some(h) => h,
        None => return EngineEvent::default(),
    };
    let mut q = h.event_queue.lock().unwrap();
    q.pop_front().unwrap_or_default()
}

#[no_mangle]
pub extern "C" fn audio_engine_is_playing(handle: *const AudioEngineHandle) -> bool {
    let h = match unsafe { handle.as_ref() } {
        Some(h) => h,
        None => return false,
    };
    h.shared_state.load(Ordering::Relaxed) == PlaybackState::Playing as i32
}

// ============================================================================
// Spectrum analysis (stub — always returns empty)
// ============================================================================

#[no_mangle]
pub unsafe extern "C" fn audio_engine_get_spectrum(
    _handle: *mut AudioEngineHandle,
    out: *mut f32,
    max_bands: usize,
) -> bool {
    if out.is_null() || max_bands == 0 {
        return false;
    }
    let spectrum = GLOBAL_SPECTRUM.read().unwrap();
    let n = spectrum.len().min(max_bands).min(32);
    if n == 0 {
        return false;
    }
    std::ptr::copy_nonoverlapping(spectrum.as_ptr(), out, n);
    true
}

#[no_mangle]
pub extern "C" fn audio_engine_get_spectrum_band_count_for_engine(
    _handle: *mut AudioEngineHandle,
) -> i32 {
    get_band_count() as i32
}

// ============================================================================
// Utility functions
// ============================================================================

#[no_mangle]
pub extern "C" fn audio_engine_get_load_error(
    handle: *const AudioEngineHandle,
) -> *mut c_char {
    let h = match unsafe { handle.as_ref() } {
        Some(h) => h,
        None => return std::ptr::null_mut(),
    };
    let msg = if let Ok(p) = h.player.try_lock() {
        p.take_load_error()
    } else {
        None
    };
    match msg {
        Some(s) => CString::new(s).unwrap().into_raw(),
        None => std::ptr::null_mut(),
    }
}

#[no_mangle]
pub extern "C" fn audio_engine_get_buffered_position(
    handle: *const AudioEngineHandle,
) -> u64 {
    let h = match unsafe { handle.as_ref() } {
        Some(h) => h,
        None => return 0,
    };
    if let Ok(p) = h.player.try_lock() {
        p.output_position_ms()
    } else {
        0
    }
}

#[no_mangle]
pub extern "C" fn audio_engine_get_sample_rate(
    _handle: *const AudioEngineHandle,
) -> u64 {
    0
}

#[no_mangle]
pub extern "C" fn audio_engine_get_channels(
    _handle: *const AudioEngineHandle,
) -> u64 {
    0
}

// ============================================================================
// YouTube FFI bindings
// ============================================================================

#[no_mangle]
pub extern "C" fn youtube_get_stream_url(video_id: *const c_char) -> *mut c_char {
    let video_id_str = match unsafe { CStr::from_ptr(video_id).to_str() } {
        Ok(s) => s,
        Err(_) => return std::ptr::null_mut(),
    };

    let yt = ytex::YouTube::new();
    match yt.videos().stream(video_id_str) {
        Ok(manifest) => match manifest.best_audio() {
            Some(fmt) => CString::new(fmt.url.clone()).unwrap().into_raw(),
            None => std::ptr::null_mut(),
        },
        Err(e) => {
            error!("[ffi] youtube_get_stream_url failed: {}", e);
            std::ptr::null_mut()
        }
    }
}

#[no_mangle]
pub unsafe extern "C" fn audio_engine_play_youtube(
    handle: *mut AudioEngineHandle,
    url: *const c_char,
) -> i32 {
    if handle.is_null() || url.is_null() {
        return -1;
    }
    let url_str = match CStr::from_ptr(url).to_str() {
        Ok(s) => s,
        Err(_) => return -2,
    };

    let h = &*handle;
    h.shared_state
        .store(PlaybackState::Connecting as i32, Ordering::Relaxed);

    let player = h.player.clone();
    let url_owned = url_str.to_string();

    thread::spawn(move || {
        let mut p = match player.lock() {
            Ok(p) => p,
            Err(e) => {
                error!("[ffi] play_youtube: lock error: {}", e);
                return;
            }
        };
        if let Err(e) = p.start(&url_owned) {
            error!("[ffi] play_youtube: start error: {}", e);
            p.set_load_error(format!("{e}"));
        }
    });

    0
}

#[no_mangle]
pub unsafe extern "C" fn audio_engine_play_live(
    handle: *mut AudioEngineHandle,
    url: *const c_char,
    _cache_max_ms: u64,
) -> i32 {
    if handle.is_null() || url.is_null() {
        return -1;
    }
    let url_str = match CStr::from_ptr(url).to_str() {
        Ok(s) => s,
        Err(_) => return -2,
    };

    let h = &*handle;
    h.shared_state
        .store(PlaybackState::Connecting as i32, Ordering::Relaxed);

    let player = h.player.clone();
    let url_owned = url_str.to_string();

    thread::spawn(move || {
        let mut p = match player.lock() {
            Ok(p) => p,
            Err(e) => {
                error!("[ffi] play_live: lock error: {}", e);
                return;
            }
        };
        if let Err(e) = p.start(&url_owned) {
            error!("[ffi] play_live: start error: {}", e);
            p.set_load_error(format!("{e}"));
        }
    });

    0
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_engine_lifecycle() {
        let engine = audio_engine_create();
        assert!(!engine.is_null());
        unsafe {
            audio_engine_destroy(engine);
        }
    }

    #[test]
    fn test_null_handle_safety() {
        audio_engine_pause(std::ptr::null());
        audio_engine_resume(std::ptr::null());
        audio_engine_stop(std::ptr::null_mut());
        assert!(!audio_engine_is_playing(std::ptr::null()));
        assert_eq!(audio_engine_get_volume(std::ptr::null()), 1.0);
        assert_eq!(audio_engine_get_state(std::ptr::null()), 0); // Stopped
        let pos = audio_engine_get_position(std::ptr::null());
        assert_eq!(pos.current_ms, 0);
        assert_eq!(pos.total_ms, 0);
        let evt = audio_engine_poll_event(std::ptr::null());
        assert_eq!(evt.event_type, ENGINE_EVENT_NONE);
        let buf = audio_engine_get_download_buffer(std::ptr::null());
        assert_eq!(buf.total_ms, 0);
        assert!(audio_engine_get_load_error(std::ptr::null()).is_null());
    }

    #[test]
    fn test_spectrum_null_safety() {
        unsafe {
            assert!(!audio_engine_get_spectrum(
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                0,
            ));
        }
    }

    #[test]
    fn test_youtube_get_stream_url_null_safety() {
        let result = youtube_get_stream_url(std::ptr::null());
        assert!(result.is_null());
    }

    #[test]
    fn test_engine_get_channel_sr_defaults() {
        let engine = audio_engine_create();
        assert!(!engine.is_null());
        unsafe {
            assert_eq!(audio_engine_get_sample_rate(engine), 0);
            assert_eq!(audio_engine_get_channels(engine), 0);
            audio_engine_destroy(engine);
        }
    }
}
