#[cfg(not(target_os = "android"))]
pub mod file_decoder;
#[cfg(target_os = "ios")]
pub mod ios_file_decoder;
#[cfg(target_os = "android")]
pub mod android_file_decoder;
pub mod seek;
