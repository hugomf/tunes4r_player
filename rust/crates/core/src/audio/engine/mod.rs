//! The audio playback engine module.

pub mod commands;
pub mod context;
pub mod state;
pub mod types;

pub use types::{
    get_band_count, set_band_count, update_global_spectrum, PlaybackEngine, PlaybackType,
    GLOBAL_SPECTRUM,
};