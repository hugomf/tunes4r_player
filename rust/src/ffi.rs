//! FFI bindings for Flutter integration
//!
//! Exposes the audio engine functionality through a C-compatible API.

#![allow(clippy::not_unsafe_ptr_arg_deref)]

use log::{error, info};

use crate::audio::engine::types::GLOBAL_SPECTRUM;
use crate::audio::{PlaybackEngine, PlaybackError};
use crate::dsp::SpectrumAnalyzer;
use crate::models::{PlaybackPosition, PlaybackState, SpectrumData};
use std::ffi::{CStr, CString};
use std::os::raw::c_char;
use std::sync::RwLock;

// Simple stderr logger for non-Android platforms.
#[cfg(not(target_os = "android"))]
struct StderrLogger;

#[cfg(not(target_os = "android"))]
impl log::Log for StderrLogger {
    fn enabled(&self, metadata: &log::Metadata) -> bool {
        metadata.level() <= log::Level::Info
    }

    fn log(&self, record: &log::Record) {
        if self.enabled(record.metadata()) {
            eprintln!("{}", record.args());
        }
    }

    fn flush(&self) {}
}

#[cfg(not(target_os = "android"))]
fn init_logger() {
    use log::LevelFilter;
    static LOGGER: StderrLogger = StderrLogger;
    log::set_logger(&LOGGER).ok();
    log::set_max_level(LevelFilter::Info);
}

// ============================================================================
// rustls-platform-verifier FFI bindings
// ============================================================================

/// Check if platform verifier is available
#[cfg(feature = "rustls-platform-verifier")]
#[no_mangle]
pub extern "C" fn tunes4r_platform_verifier_available() -> bool {
    true
}

#[cfg(not(feature = "rustls-platform-verifier"))]
#[no_mangle]
pub extern "C" fn tunes4r_platform_verifier_available() -> bool {
    false
}

#[cfg(target_os = "android")]
#[no_mangle]
pub extern "C" fn Java_com_ocelot_tunes4r_MainActivity_initRustlsPlatformVerifier(
    mut env: jni::JNIEnv,
    _class: jni::sys::jobject,
    context: jni::sys::jobject,
) {
    use rustls_platform_verifier::android;
    let context = unsafe { jni::objects::JObject::from_raw(context) };
    let _ = android::init_hosted(&mut env, context);
}

#[no_mangle]
pub extern "C" fn tunes4r_init_android_verifier(
    _env: *mut std::os::raw::c_void,
    _context: *mut std::os::raw::c_void,
) {
    error!("[ffi] tunes4r_init_android_verifier: not needed on this platform");
}

/// This function is required when building Rust code that interfaces with C++
/// code (like cpal/rodio) for Android. It handles pure virtual function calls.
/// This is a known issue when cross-compiling Rust + C++ for Android.
#[no_mangle]
pub extern "C" fn __cxa_pure_virtual() {
    panic!("__cxa_pure_virtual called - this should never happen");
}

/// JNI_OnLoad is called by the JVM when System.loadLibrary("tunes4r") executes
/// (see MainActivity.java static block). This initializes ndk_context so that
/// background threads can later attach to the JVM — required by cpal's AAudio
/// backend for creating audio output streams.
///
/// NOTE: When the library is loaded via Dart FFI (DynamicLibrary.open), this
/// is NOT called. Use Java_com_tunes4r_1player_tunes4r_1player_Tunes4rPlayerPlugin_nativeInit
/// instead, which is invoked from the Flutter plugin's onAttachedToEngine.
#[cfg(target_os = "android")]
#[no_mangle]
pub extern "C" fn JNI_OnLoad(vm: *mut std::ffi::c_void, _reserved: *mut std::ffi::c_void) -> i32 {
    unsafe {
        ndk_context::initialize_android_context(vm, std::ptr::null_mut());
    }
    // Initialize Android logger so all log::info! / log::error! calls appear in logcat
    #[cfg(target_os = "android")]
    android_logger::init_once(
        android_logger::Config::default()
            .with_max_level(log::LevelFilter::Debug)
            .with_tag("tunes4r"),
    );
    log::info!("[ffi] JNI_OnLoad: ndk_context initialized");
    jni::sys::JNI_VERSION_1_6
}

/// Called from Java Tunes4rPlayerPlugin.nativeInit() during Flutter plugin
/// registration (onAttachedToEngine). This captures the JVM pointer via JNI
/// and initializes ndk_context so that Rust background threads (like
/// "playback-decode") can later attach to the JVM.
///
/// Fallback in case JNI_OnLoad wasn't triggered (library loaded via Dart FFI
/// before class static initializer runs).
#[cfg(target_os = "android")]
#[no_mangle]
pub unsafe extern "system" fn Java_com_tunes4r_1player_tunes4r_1player_Tunes4rPlayerPlugin_nativeInit(
    env: *mut jni::sys::JNIEnv,
    _class: jni::sys::jclass,
) {
    let env_wrapper = match jni::JNIEnv::from_raw(env) {
        Ok(e) => e,
        Err(e) => {
            log::warn!("[ffi] nativeInit: JNIEnv::from_raw failed: {:?}", e);
            return;
        }
    };
    let vm = match env_wrapper.get_java_vm() {
        Ok(v) => v,
        Err(e) => {
            log::warn!("[ffi] nativeInit: get_java_vm failed: {:?}", e);
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
        log::info!("[ffi] ndk_context initialized via nativeInit");
    }
}

/// Opaque handle to the audio engine
pub struct AudioEngineHandle {
    playback: RwLock<PlaybackEngine>,
    spectrum: RwLock<SpectrumAnalyzer>,
}

impl AudioEngineHandle {
    fn new() -> Result<Self, PlaybackError> {
        Ok(Self {
            playback: RwLock::new(PlaybackEngine::new_without_device()?),
            spectrum: RwLock::new(SpectrumAnalyzer::default()),
        })
    }

    pub fn playback(&self) -> &RwLock<PlaybackEngine> {
        &self.playback
    }
}

// ============================================================================
// Engine lifecycle
// ============================================================================

/// Create a new audio engine instance
#[no_mangle]
pub extern "C" fn audio_engine_create() -> *mut AudioEngineHandle {
    #[cfg(not(target_os = "android"))]
    init_logger();

    let result = std::panic::catch_unwind(AudioEngineHandle::new);
    match result {
        Ok(Ok(engine)) => Box::into_raw(Box::new(engine)),
        Ok(Err(e)) => {
            error!("[ffi] audio_engine_create failed: {}", e);
            std::ptr::null_mut()
        }
        Err(panic_info) => {
            error!("[ffi] audio_engine_create PANIC: {:?}", panic_info);
            std::ptr::null_mut()
        }
    }
}

/// Destroy an audio engine instance
///
/// # Safety
/// The handle must have been created by `audio_engine_create` and not previously destroyed.
#[no_mangle]
pub unsafe extern "C" fn audio_engine_destroy(handle: *mut AudioEngineHandle) {
    if !handle.is_null() {
        drop(Box::from_raw(handle));
    }
}

/// Set the number of spectrum bands to compute.
/// This should be called before starting playback.
#[no_mangle]
pub extern "C" fn audio_engine_set_spectrum_band_count(handle: *mut AudioEngineHandle, count: i32) {
    if handle.is_null() {
        return;
    }
    unsafe {
        let handle = &*handle;
        handle
            .playback
            .write()
            .unwrap()
            .set_spectrum_band_count(count as usize);
    }
}

/// Set the number of spectrum bands to compute (global, for Android).
/// This should be called before starting playback.
#[no_mangle]
pub extern "C" fn audio_engine_set_spectrum_band_count_global(count: i32) {
    crate::audio::engine::set_band_count(count as usize);
}

// ============================================================================
// Playback control
// ============================================================================

/// Unified play: auto-detects source type from URI and starts playback.
///
/// Accepts: file paths, HTTP URLs, YouTube URLs/IDs/search queries.
///
/// # Safety
/// The uri must be a valid null-terminated UTF-8 string.
#[no_mangle]
pub unsafe extern "C" fn audio_engine_play(
    handle: *mut AudioEngineHandle,
    uri: *const c_char,
) -> i32 {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        if handle.is_null() || uri.is_null() {
            return -1;
        }

        let handle = &*handle;
        let uri = match CStr::from_ptr(uri).to_str() {
            Ok(s) => s,
            Err(_) => return -2,
        };

        match handle.playback.write().unwrap().play(uri) {
            Ok(()) => 0,
            Err(e) => {
                error!("[ffi] audio_engine_play error: {}", e);
                -3
            }
        }
    }));

    match result {
        Ok(code) => code,
        Err(panic_info) => {
            let msg = if let Some(s) = panic_info.downcast_ref::<&str>() {
                s.to_string()
            } else if let Some(s) = panic_info.downcast_ref::<String>() {
                s.clone()
            } else {
                "Unknown panic".to_string()
            };
            error!("[ffi] PANIC in audio_engine_play: {}", msg);
            -99
        }
    }
}

/// Check whether the current playback source supports seeking.
#[no_mangle]
pub extern "C" fn audio_engine_can_seek(handle: *const AudioEngineHandle) -> bool {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        if handle.is_null() {
            return false;
        }
        unsafe { &*handle }
            .playback
            .read()
            .unwrap()
            .source_supports(crate::audio::stream::source::Capability::Seek)
    }));

    result.unwrap_or(false)
}

/// Check whether the current playback source supports downloading.
#[no_mangle]
pub extern "C" fn audio_engine_can_download(handle: *const AudioEngineHandle) -> bool {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        if handle.is_null() {
            return false;
        }
        unsafe { &*handle }
            .playback
            .read()
            .unwrap()
            .source_supports(crate::audio::stream::source::Capability::Download)
    }));

    result.unwrap_or(false)
}

/// Start pipe-based playback (decoder waits for bytes from Dart)
///
/// # Safety
/// The handle must be a valid engine handle.
#[no_mangle]
pub unsafe extern "C" fn audio_engine_play_stream_from_bytes(
    handle: *mut AudioEngineHandle,
    url: *const c_char,
) -> i32 {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        if handle.is_null() || url.is_null() {
            return -1;
        }

        let handle = &*handle;
        let url = match CStr::from_ptr(url).to_str() {
            Ok(s) => s,
            Err(_) => return -2,
        };

        match handle
            .playback
            .write()
            .unwrap()
            .play_stream_from_bytes_internal(url)
        {
            Ok(()) => 0,
            Err(e) => {
                error!("[ffi] play_stream_from_bytes error: {}", e);
                -3
            }
        }
    }));

    match result {
        Ok(code) => code,
        Err(panic_info) => {
            let msg = if let Some(s) = panic_info.downcast_ref::<&str>() {
                s.to_string()
            } else if let Some(s) = panic_info.downcast_ref::<String>() {
                s.clone()
            } else {
                "Unknown panic".to_string()
            };
            error!(
                "[ffi] PANIC in audio_engine_play_stream_from_bytes: {}",
                msg
            );
            -99
        }
    }
}

/// Start playback from bytes piped from Dart (bypasses Rust HTTP client)
///
/// # Safety
/// The handle must be a valid engine handle.
#[no_mangle]
pub unsafe extern "C" fn audio_engine_fetch_and_pipe(
    handle: *mut AudioEngineHandle,
    url: *const c_char,
) -> i32 {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        if handle.is_null() || url.is_null() {
            return -1;
        }
        let handle = &*handle;
        let url_str = match CStr::from_ptr(url).to_str() {
            Ok(s) => s.to_string(),
            Err(_) => return -2,
        };

        std::thread::spawn(move || {
            if let Err(e) = crate::audio_http_fetch::fetch_and_pipe(&url_str, handle) {
                error!("[ffi] HTTP fetch failed: {}", e);
            }
        });

        0
    }));

    result.unwrap_or(-99)
}

/// Push audio bytes to the active stream pipe (called from Dart HTTP fetch)
///
/// # Safety
/// The handle must be a valid engine handle, and data must point to `len` valid bytes.
#[no_mangle]
pub unsafe extern "C" fn audio_engine_push_audio_bytes(
    handle: *mut AudioEngineHandle,
    data: *const u8,
    len: i32,
) {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        if handle.is_null() || data.is_null() || len <= 0 {
            return;
        }
        let handle = &*handle;
        let bytes = std::slice::from_raw_parts(data, len as usize);
        handle.playback.read().unwrap().push_audio_bytes(bytes);
    }));
    if result.is_err() {
        error!("[ffi] PANIC in audio_engine_push_audio_bytes");
    }
}

/// Signal end of piped audio stream
///
/// # Safety
/// The handle must be a valid engine handle.
#[no_mangle]
pub unsafe extern "C" fn audio_engine_end_audio_stream(handle: *mut AudioEngineHandle) {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        if handle.is_null() {
            return;
        }
        let handle = &*handle;
        handle.playback.read().unwrap().end_audio_stream();
    }));
    if result.is_err() {
        error!("[ffi] PANIC in audio_engine_end_audio_stream");
    }
}

/// Signal an error in the piped stream (e.g., HTTP 403)
///
/// # Safety
/// The handle must be a valid engine handle.
#[no_mangle]
pub unsafe extern "C" fn audio_engine_set_stream_error(
    handle: *mut AudioEngineHandle,
    message: *const c_char,
) {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        if handle.is_null() || message.is_null() {
            return;
        }
        let handle = &*handle;
        let message_str = match CStr::from_ptr(message).to_str() {
            Ok(s) => s,
            Err(_) => return,
        };
        handle
            .playback
            .write()
            .unwrap()
            .set_stream_error(message_str);
    }));
    if result.is_err() {
        error!("[ffi] PANIC in audio_engine_set_stream_error");
    }
}

/// Set total bytes for the piped stream
///
/// # Safety
/// The handle must be a valid engine handle.
#[no_mangle]
pub unsafe extern "C" fn audio_engine_set_pipe_total_bytes(
    handle: *mut AudioEngineHandle,
    total_bytes: u64,
) {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        if handle.is_null() {
            return;
        }
        let handle = &*handle;
        handle
            .playback
            .read()
            .unwrap()
            .set_pipe_total_bytes(total_bytes);
    }));
    if result.is_err() {
        error!("[ffi] PANIC in audio_engine_set_pipe_total_bytes");
    }
}

/// Pause playback
#[no_mangle]
pub extern "C" fn audio_engine_pause(handle: *const AudioEngineHandle) {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        if !handle.is_null() {
            unsafe { &*handle }.playback.write().unwrap().pause();
        }
    }));
    if result.is_err() {
        error!("[ffi] PANIC in audio_engine_pause");
    }
}

/// Resume playback
#[no_mangle]
pub extern "C" fn audio_engine_resume(handle: *const AudioEngineHandle) {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        if !handle.is_null() {
            unsafe { &*handle }.playback.write().unwrap().resume();
        }
    }));
    if result.is_err() {
        error!("[ffi] PANIC in audio_engine_resume");
    }
}

/// Stop playback
#[no_mangle]
pub extern "C" fn audio_engine_stop(handle: *mut AudioEngineHandle) {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        if !handle.is_null() {
            unsafe { &mut *handle }.playback.write().unwrap().stop();
        }
    }));
    if result.is_err() {
        error!("[ffi] PANIC in audio_engine_stop");
    }
}

/// Seek to position in milliseconds
#[no_mangle]
pub extern "C" fn audio_engine_seek(handle: *mut AudioEngineHandle, position_ms: u64) -> i32 {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        if handle.is_null() {
            return -1;
        }

        let seek_result = unsafe { &mut *handle }
            .playback
            .write()
            .unwrap()
            .seek(position_ms);
        match seek_result {
            Ok(()) => 0,
            Err(e) => {
                error!("[ffi] audio_engine_seek: failed with error: {:?}", e);
                -2
            }
        }
    }));

    match result {
        Ok(code) => code,
        Err(panic_info) => {
            error!("[ffi] audio_engine_seek: PANIC: {:?}", panic_info);
            -99
        }
    }
}

/// Check if a pipe seek is pending and return the offset in milliseconds
/// Returns 0 if no seek is pending, -1 on error
#[no_mangle]
pub extern "C" fn audio_engine_get_pipe_seek_offset(handle: *mut AudioEngineHandle) -> i64 {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        if handle.is_null() {
            return -1;
        }

        let engine = unsafe { &*handle }.playback.read().unwrap();
        match engine.get_pipe_seek_request() {
            Some((_url, offset_ms)) => offset_ms as i64,
            None => 0,
        }
    }));

    result.unwrap_or(-1)
}

/// Get the byte offset for a pending pipe seek
/// Returns -1 on error, 0 if no seek pending
#[no_mangle]
pub extern "C" fn audio_engine_get_pipe_seek_byte_offset(handle: *mut AudioEngineHandle) -> i64 {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        if handle.is_null() {
            return -1;
        }

        let engine = unsafe { &*handle }.playback.read().unwrap();
        match engine.get_pipe_seek_info() {
            Some((_url, _offset_ms, byte_offset)) => byte_offset as i64,
            None => 0,
        }
    }));

    result.unwrap_or(-1)
}

/// Get the pipe URL for re-fetching
/// Returns null if not in pipe mode or no pending seek
#[no_mangle]
pub extern "C" fn audio_engine_get_pipe_url_for_seek(
    handle: *mut AudioEngineHandle,
) -> *mut c_char {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        if handle.is_null() {
            return std::ptr::null_mut();
        }

        let engine = unsafe { &*handle }.playback.read().unwrap();
        match engine.get_pipe_seek_info() {
            Some((url, _offset_ms, _byte_offset)) => CString::new(url).unwrap().into_raw(),
            None => std::ptr::null_mut(),
        }
    }));

    result.unwrap_or(std::ptr::null_mut())
}

/// Poll for a seek request from the Symphonia decoder via the pipe.
/// This is for internal probing seeks, not user-initiated seeks.
/// Returns the byte offset of the seek if one is pending and significant (> 10 bytes),
/// otherwise returns -1. This function should be called by Dart periodically.
/// It will clear the seek request from the pipe once retrieved.
#[no_mangle]
pub extern "C" fn audio_engine_poll_pipe_seek_byte_offset(handle: *mut AudioEngineHandle) -> i64 {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        if handle.is_null() {
            return -1;
        }

        let engine = unsafe { &mut *handle }.playback.write().unwrap();

        // Check if we have a pipe writer
        if let Some(pipe_writer) = &engine.stream_pipe {
            if let Some(seek_byte_offset) = pipe_writer.take_seek_request() {
                // Differentiate from probing seeks. If it's a small offset, it's likely
                // Symphonia probing, ignore it for Dart's re-fetch.
                // Real seeks (from user or Symphonia after initial probe) will be larger.
                const PROBE_SEEK_THRESHOLD_BYTES: u64 = 10;
                if seek_byte_offset > PROBE_SEEK_THRESHOLD_BYTES {
                    error!(
                        "[ffi] Polling pipe: significant seek request to {} bytes",
                        seek_byte_offset
                    );
                    return seek_byte_offset as i64;
                } else {
                    error!(
                        "[ffi] Polling pipe: ignoring small probe seek to {} bytes",
                        seek_byte_offset
                    );
                }
            }
        }
        -1 // No significant seek request
    }));

    result.unwrap_or(-1)
}

/// Clear the pending pipe seek request
#[no_mangle]
pub extern "C" fn audio_engine_clear_pipe_seek_request(handle: *mut AudioEngineHandle) {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        if handle.is_null() {
            return;
        }
        unsafe { &mut *handle }
            .playback
            .write()
            .unwrap()
            .clear_pipe_seek_request();
    }));
    if result.is_err() {
        error!("[ffi] PANIC in audio_engine_clear_pipe_seek_request");
    }
}

/// Skip forward by milliseconds
#[no_mangle]
pub extern "C" fn audio_engine_skip_forward(handle: *mut AudioEngineHandle, ms: u64) -> i32 {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        if handle.is_null() {
            return -1;
        }

        match unsafe { &mut *handle }
            .playback
            .write()
            .unwrap()
            .skip_forward(ms)
        {
            Ok(()) => 0,
            Err(_) => -2,
        }
    }));

    result.unwrap_or(-99)
}

/// Skip backward by milliseconds
#[no_mangle]
pub extern "C" fn audio_engine_skip_backward(handle: *mut AudioEngineHandle, ms: u64) -> i32 {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        if handle.is_null() {
            return -1;
        }

        match unsafe { &mut *handle }
            .playback
            .write()
            .unwrap()
            .skip_backward(ms)
        {
            Ok(()) => 0,
            Err(_) => -2,
        }
    }));

    result.unwrap_or(-99)
}

// ============================================================================
// Volume control
// ============================================================================

/// Set volume (0.0 to 1.0)
#[no_mangle]
pub extern "C" fn audio_engine_set_volume(handle: *const AudioEngineHandle, volume: f32) {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        if !handle.is_null() {
            unsafe { &*handle }
                .playback
                .read()
                .unwrap()
                .set_volume(volume);
        }
    }));
    if result.is_err() {
        error!("[ffi] PANIC in audio_engine_set_volume");
    }
}

/// Get current volume
#[no_mangle]
pub extern "C" fn audio_engine_get_volume(handle: *const AudioEngineHandle) -> f32 {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        if handle.is_null() {
            1.0
        } else {
            unsafe { &*handle }.playback.read().unwrap().get_volume()
        }
    }));
    result.unwrap_or(1.0)
}

// ============================================================================
// State queries
// ============================================================================

/// Get current playback state
#[no_mangle]
pub extern "C" fn audio_engine_get_state(handle: *const AudioEngineHandle) -> i32 {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        if handle.is_null() {
            return PlaybackState::default().to_i32();
        }

        unsafe { &*handle }
            .playback
            .read()
            .unwrap()
            .get_state()
            .to_i32()
    }));

    match result {
        Ok(state) => state,
        Err(e) => {
            error!("[ffi] PANIC in audio_engine_get_state: {:?}", e);
            PlaybackState::default().to_i32()
        }
    }
}

/// Get current playback position
/// Returns a struct with current_ms and total_ms
#[no_mangle]
pub extern "C" fn audio_engine_get_position(handle: *const AudioEngineHandle) -> PlaybackPosition {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        if handle.is_null() {
            return PlaybackPosition::default();
        }

        unsafe { &*handle }.playback.read().unwrap().get_position()
    }));

    result.unwrap_or_default()
}

/// Check if currently playing
#[no_mangle]
pub extern "C" fn audio_engine_is_playing(handle: *const AudioEngineHandle) -> bool {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        if handle.is_null() {
            return false;
        }
        unsafe { &*handle }.playback.read().unwrap().is_playing()
    }));

    if result.is_err() {
        error!("[ffi] PANIC in audio_engine_is_playing");
    }
    result.unwrap_or(false)
}

// ============================================================================
// Spectrum analysis
// ============================================================================

/// Analyze audio samples and return spectrum data
///
/// The returned SpectrumData must be freed with `spectrum_data_free`.
#[allow(improper_ctypes_definitions)]
#[no_mangle]
pub extern "C" fn audio_engine_analyze_spectrum(
    handle: *mut AudioEngineHandle,
    samples: *const f32,
    sample_count: usize,
) -> SpectrumData {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        if handle.is_null() || samples.is_null() || sample_count == 0 {
            return SpectrumData::default();
        }

        let handle = unsafe { &*handle };
        let samples = unsafe { std::slice::from_raw_parts(samples, sample_count) };

        handle.spectrum.write().unwrap().analyze(samples)
    }));

    result.unwrap_or_default()
}

/// Free spectrum data returned from analysis
#[no_mangle]
pub extern "C" fn spectrum_data_free(data: *mut SpectrumData) {
    if !data.is_null() {
        unsafe {
            drop(Box::from_raw(data));
        }
    }
}

/// Get real-time spectrum data from the playback engine
///
/// Copies up to 32 frequency bands into the provided output buffer.
/// Returns true if spectrum data was available, false otherwise.
///
/// # Safety
/// The out pointer must point to a valid buffer of at least `max_bands` f32 values.
#[no_mangle]
pub unsafe extern "C" fn audio_engine_get_spectrum(
    _handle: *mut AudioEngineHandle,
    out: *mut f32,
    max_bands: usize,
) -> bool {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        if out.is_null() || max_bands == 0 {
            return false;
        }

        // Use global spectrum data (works for both Android and desktop)
        let spectrum = GLOBAL_SPECTRUM.read().unwrap();
        let n = spectrum.len().min(max_bands).min(32);
        if n == 0 {
            return false;
        }
        std::ptr::copy_nonoverlapping(spectrum.as_ptr(), out, n);
        true
    }));

    result.unwrap_or(false)
}

/// Get the number of spectrum bands used by the DSP engine.
/// This value should be used to allocate the output buffer for `audio_engine_get_spectrum`.
#[no_mangle]
pub extern "C" fn audio_engine_get_spectrum_band_count() -> i32 {
    crate::dsp::DEFAULT_SPECTRUM_BANDS as i32
}

/// Get the current spectrum band count from a specific engine instance.
/// This returns the configured band count, not the default.
#[no_mangle]
pub extern "C" fn audio_engine_get_spectrum_band_count_for_engine(
    handle: *mut AudioEngineHandle,
) -> i32 {
    if handle.is_null() {
        return crate::dsp::DEFAULT_SPECTRUM_BANDS as i32;
    }
    // Return the global band count for Android
    crate::audio::engine::get_band_count() as i32
}

// ============================================================================
// Utility functions
// ============================================================================

/// Get the last error message (if any)
///
/// Returns a newly allocated string that must be freed with `rust_string_free`.
#[no_mangle]
pub extern "C" fn audio_engine_get_load_error(handle: *const AudioEngineHandle) -> *mut c_char {
    if handle.is_null() {
        return std::ptr::null_mut();
    }

    let error = unsafe { &*handle }
        .playback
        .read()
        .unwrap()
        .get_load_error()
        .map(|s| s.to_string());
    match error {
        Some(msg) => CString::new(msg).unwrap().into_raw(),
        None => std::ptr::null_mut(),
    }
}

/// Get last error from the engine (alias for load_error for clarity)
#[no_mangle]
pub extern "C" fn audio_engine_get_last_error(handle: *const AudioEngineHandle) -> *mut c_char {
    audio_engine_get_load_error(handle)
}

/// Get buffered bytes during Buffering state
/// Returns 0 if not currently buffering
#[no_mangle]
pub extern "C" fn audio_engine_get_buffered_bytes(handle: *const AudioEngineHandle) -> u64 {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        if handle.is_null() {
            return 0;
        }

        let state = unsafe { &*handle }.playback.write().unwrap().get_state();
        match state {
            PlaybackState::Buffering { buffered_bytes, .. } => buffered_bytes,
            _ => 0,
        }
    }));

    result.unwrap_or(0)
}

/// Get buffered position in milliseconds
/// This is the current playback position plus the audio queue length
/// Returns 0 if engine is not initialized
#[no_mangle]
pub extern "C" fn audio_engine_get_buffered_position(handle: *const AudioEngineHandle) -> u64 {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        if handle.is_null() {
            return 0;
        }

        unsafe { &*handle }
            .playback
            .read()
            .unwrap()
            .get_buffered_position()
    }));

    result.unwrap_or(0)
}

/// Set total bytes during Buffering state (for progress display)
#[no_mangle]
pub extern "C" fn audio_engine_get_total_bytes(handle: *const AudioEngineHandle) -> i64 {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        if handle.is_null() {
            return -1;
        }

        let state = unsafe { &*handle }.playback.read().unwrap().get_state();
        match state {
            PlaybackState::Buffering {
                total_bytes: Some(t),
                ..
            } => t as i64,
            _ => -1,
        }
    }));

    result.unwrap_or(-1)
}

/// Free a string allocated by Rust
///
/// # Safety
/// The pointer must have been allocated by Rust and not previously freed.
#[no_mangle]
pub unsafe extern "C" fn rust_string_free(s: *mut c_char) {
    if !s.is_null() {
        drop(CString::from_raw(s));
    }
}

/// Configure iOS audio session for playback.
/// Must be called before starting playback on iOS.
#[no_mangle]
pub extern "C" fn tunes4r_configure_audio_session() {
    #[cfg(target_os = "ios")]
    {
        error!("[ffi] tunes4r_configure_audio_session: called (iOS)");
    }
    #[cfg(not(target_os = "ios"))]
    {
        error!("[ffi] tunes4r_configure_audio_session: no-op on non-iOS platform",);
    }
}

#[no_mangle]
pub extern "C" fn audio_engine_get_sample_rate(handle: *const AudioEngineHandle) -> u64 {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        if handle.is_null() {
            return 0;
        }

        unsafe { &*handle }.playback.read().unwrap().sample_rate()
    }));

    result.unwrap_or(0)
}

#[no_mangle]
pub extern "C" fn audio_engine_get_channels(handle: *const AudioEngineHandle) -> u64 {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        if handle.is_null() {
            return 0;
        }

        unsafe { &*handle }.playback.read().unwrap().channels()
    }));

    result.unwrap_or(0)
}

// ============================================================================
// YouTube Service FFI bindings
// ============================================================================

/// Opaque handle to a YouTube service instance
pub struct YoutubeServiceHandle(pub crate::youtube::YouTubeService);

/// Create a new YouTube service instance
#[no_mangle]
pub extern "C" fn youtube_service_create() -> *mut YoutubeServiceHandle {
    let result = std::panic::catch_unwind(|| {
        Box::into_raw(Box::new(YoutubeServiceHandle(
            crate::youtube::YouTubeService::new(),
        )))
    });
    match result {
        Ok(handle) => handle,
        Err(panic_info) => {
            error!("[ffi] youtube_service_create PANIC: {:?}", panic_info);
            std::ptr::null_mut()
        }
    }
}

/// Destroy a YouTube service instance
#[no_mangle]
pub unsafe extern "C" fn youtube_service_destroy(handle: *mut YoutubeServiceHandle) {
    if !handle.is_null() {
        drop(Box::from_raw(handle));
    }
}

/// Search YouTube videos
#[no_mangle]
pub unsafe extern "C" fn youtube_search(
    handle: *mut YoutubeServiceHandle,
    query: *const c_char,
    limit: i32,
) -> *mut c_char {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        if handle.is_null() || query.is_null() {
            return std::ptr::null_mut();
        }

        let query_str = match CStr::from_ptr(query).to_str() {
            Ok(s) => s,
            Err(_) => return std::ptr::null_mut(),
        };

        let yt = crate::youtube::YouTube::new();
        match crate::youtube::search::search(yt.client().http(), query_str, limit as usize) {
            Ok(results) => {
                let json = serde_json::to_string(&results).unwrap_or_default();
                CString::new(json).unwrap().into_raw()
            }
            Err(e) => {
                error!("[ffi] youtube_search failed: {}", e);
                CString::new(format!(r#"{{"error":"{}"}}"#, e))
                    .unwrap()
                    .into_raw()
            }
        }
    }));

    result.unwrap_or(std::ptr::null_mut())
}

/// Get audio stream URL for a video
#[no_mangle]
pub unsafe extern "C" fn youtube_get_stream_url(
    handle: *mut YoutubeServiceHandle,
    video_id: *const c_char,
) -> *mut c_char {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        if handle.is_null() || video_id.is_null() {
            return std::ptr::null_mut();
        }

        let video_id_str = match CStr::from_ptr(video_id).to_str() {
            Ok(s) => s,
            Err(_) => return std::ptr::null_mut(),
        };

        log::info!(
            "[ffi] youtube_get_stream_url: resolving video_id={}",
            video_id_str
        );
        let start = std::time::Instant::now();

        let yt = crate::youtube::YouTube::new();
        match yt.videos().stream(video_id_str) {
            Ok(manifest) => {
                let elapsed = start.elapsed();
                log::info!(
                    "[ffi] youtube_get_stream_url: resolved in {}ms, formats: {} audio, {} video",
                    elapsed.as_millis(),
                    manifest.audio.len(),
                    manifest.video.len()
                );
                match manifest.best_audio() {
                    Some(format) => CString::new(format.url.clone()).unwrap().into_raw(),
                    None => {
                        error!("[ffi] youtube_get_stream_url: no audio formats found");
                        CString::new("").unwrap().into_raw()
                    }
                }
            }
            Err(e) => {
                let elapsed = start.elapsed();
                error!(
                    "[ffi] youtube_get_stream_url failed: {} ({}ms)",
                    e,
                    elapsed.as_millis()
                );
                CString::new("").unwrap().into_raw()
            }
        }
    }));

    result.unwrap_or(std::ptr::null_mut())
}

/// Get video metadata
#[no_mangle]
pub unsafe extern "C" fn youtube_get_video_info(
    handle: *mut YoutubeServiceHandle,
    video_id: *const c_char,
) -> *mut c_char {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        if handle.is_null() || video_id.is_null() {
            return std::ptr::null_mut();
        }

        let video_id_str = match CStr::from_ptr(video_id).to_str() {
            Ok(s) => s,
            Err(_) => return std::ptr::null_mut(),
        };

        let yt = crate::youtube::YouTube::new();
        match yt.videos().get(video_id_str) {
            Ok(info) => {
                let json = serde_json::json!({
                    "id": info.id,
                    "title": info.title,
                    "author": info.author,
                    "duration": info.duration
                });
                CString::new(json.to_string()).unwrap().into_raw()
            }
            Err(e) => {
                error!("[ffi] youtube_get_video_info failed: {}", e);
                CString::new(format!(r#"{{"error":"{}"}}"#, e))
                    .unwrap()
                    .into_raw()
            }
        }
    }));

    result.unwrap_or(std::ptr::null_mut())
}

/// Download audio file
#[no_mangle]
pub unsafe extern "C" fn youtube_download_audio(
    handle: *mut YoutubeServiceHandle,
    video_id: *const c_char,
    output_path: *const c_char,
) -> i32 {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        if handle.is_null() || video_id.is_null() || output_path.is_null() {
            return -1;
        }

        let video_id_str = match CStr::from_ptr(video_id).to_str() {
            Ok(s) => s,
            Err(_) => return -2,
        };

        let output_path_str = match CStr::from_ptr(output_path).to_str() {
            Ok(s) => s,
            Err(_) => return -3,
        };

        let yt = crate::youtube::YouTube::new();
        let manifest = match yt.videos().stream(video_id_str) {
            Ok(m) => m,
            Err(e) => {
                error!("[ffi] youtube_download_audio: failed to extract: {}", e);
                return -4;
            }
        };

        let audio_format = match manifest.best_audio() {
            Some(a) => a,
            None => {
                error!("[ffi] youtube_download_audio: no audio formats found");
                return -5;
            }
        };

        let client = match reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(120))
            .build()
        {
            Ok(c) => c,
            Err(e) => {
                error!(
                    "[ffi] youtube_download_audio: failed to build client: {}",
                    e
                );
                return -6;
            }
        };

        let response = match client.get(&audio_format.url).send() {
            Ok(r) => r,
            Err(e) => {
                error!("[ffi] youtube_download_audio: HTTP request failed: {}", e);
                return -7;
            }
        };

        if !response.status().is_success() {
            error!("[ffi] youtube_download_audio: HTTP {}", response.status());
            return -8;
        }

        use std::io::{Read, Write};
        let mut stream = response;
        let mut file = match std::fs::File::create(output_path_str) {
            Ok(f) => f,
            Err(e) => {
                error!("[ffi] youtube_download_audio: create file error: {}", e);
                return -9;
            }
        };

        let mut total: usize = 0;
        let mut buf = [0u8; 65536];
        loop {
            match stream.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    total += n;
                    let _ = file.write_all(&buf[..n]);
                }
                Err(e) => {
                    error!("[ffi] youtube_download_audio: read error: {}", e);
                    return -10;
                }
            }
        }

        info!(
            "[ffi] youtube_download_audio: downloaded {} bytes to {}",
            total, output_path_str
        );
        0
    }));

    result.unwrap_or(-99)
}

/// Play audio from a YouTube URL, video ID, search query, or direct CDN URL.
/// Uses adaptive buffering internally.
#[no_mangle]
pub unsafe extern "C" fn audio_engine_play_youtube(
    handle: *mut AudioEngineHandle,
    url: *const c_char,
    cache_dir: *const c_char,
) -> i32 {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        if handle.is_null() || url.is_null() || cache_dir.is_null() {
            return -1;
        }

        let url_str = match CStr::from_ptr(url).to_str() {
            Ok(s) => s,
            Err(_) => return -2,
        };

        let cache_dir_str = match CStr::from_ptr(cache_dir).to_str() {
            Ok(s) => s,
            Err(_) => return -3,
        };

        let engine = unsafe { &mut *handle };
        match engine
            .playback
            .write()
            .unwrap()
            .play_adaptive_buffer(url_str, cache_dir_str)
        {
            Ok(()) => 0,
            Err(e) => {
                error!("[ffi] audio_engine_play_youtube failed: {}", e);
                -4
            }
        }
    }));

    result.unwrap_or(-99)
}

/// Play a stream using stream_download crate for progressive download.
/// This is useful for streams that don't support range requests or for
/// simpler streaming scenarios.
#[no_mangle]
pub unsafe extern "C" fn audio_engine_play_stream_with_downloader(
    handle: *mut AudioEngineHandle,
    url: *const c_char,
) -> i32 {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        if handle.is_null() || url.is_null() {
            return -1;
        }

        let url_str = match CStr::from_ptr(url).to_str() {
            Ok(s) => s,
            Err(_) => return -2,
        };

        let engine = unsafe { &mut *handle };
        match engine
            .playback
            .write()
            .unwrap()
            .play_stream_with_downloader(url_str)
        {
            Ok(()) => 0,
            Err(e) => {
                error!(
                    "[ffi] audio_engine_play_stream_with_downloader failed: {}",
                    e
                );
                -3
            }
        }
    }));

    result.unwrap_or(-99)
}

#[cfg(test)]
#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::engine::{set_band_count, types::update_global_spectrum};
    use crate::dsp::RmsSpectrumAnalyzer;

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
        assert_eq!(audio_engine_get_volume(std::ptr::null()), 1.0);
        assert!(!audio_engine_is_playing(std::ptr::null()));
    }

    #[test]
    fn test_spectrum_pipeline_nonzero_from_audio() {
        // Set band count to 16
        set_band_count(16);

        // Create analyzer for 44100 Hz, 16 bands
        let mut analyzer = RmsSpectrumAnalyzer::new(44100, 16);

        // Generate synthetic audio: 1024 samples of a 440 Hz sine wave at amplitude 0.5
        let sample_rate = 44100.0;
        let freq = 440.0;
        let num_samples = 2048;
        let mut mono = Vec::with_capacity(num_samples);
        for i in 0..num_samples {
            let t = i as f32 / sample_rate;
            mono.push((2.0 * std::f32::consts::PI * freq * t).sin() * 0.5);
        }

        // Run analyzer
        let spectrum = analyzer.analyze(&mono);
        assert_eq!(spectrum.len(), 16, "Spectrum should have 16 bands");

        // Verify at least some bands have non-zero values
        let max_val = spectrum.iter().cloned().fold(0.0_f32, f32::max);
        assert!(
            max_val > 0.0,
            "Spectrum should have non-zero values for audio input, got max={}",
            max_val
        );

        // Write to global spectrum and verify readback via FFI
        update_global_spectrum(spectrum);
        let mut output = [0.0f32; 16];
        unsafe {
            let ok = audio_engine_get_spectrum(std::ptr::null_mut(), output.as_mut_ptr(), 16);
            assert!(ok, "audio_engine_get_spectrum should return true");
        }
        let readback_max = output.iter().cloned().fold(0.0_f32, f32::max);
        assert!(
            readback_max > 0.0,
            "Readback spectrum should have non-zero values, got max={}",
            readback_max
        );
    }
}
