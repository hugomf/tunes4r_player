//! Decorators wrap a StreamSource to add cross-cutting capabilities.
//!
//! Each decorator implements `StreamSource` and delegates most methods to
//! an inner source, overriding only the behavior it modifies.
//!
//! Available decorators:
//! - `cache::CacheDecorator` — caches downloaded bytes to disk
//! - `adaptive::AdaptiveBufferDecorator` — background pre-fetch buffering

pub mod adaptive;
pub mod cache;
pub mod retry;
