//! Audio backend implementations.

mod cpal_backend;
mod null_backend;

pub use cpal_backend::CpalBackend;
pub use null_backend::NullBackend;
