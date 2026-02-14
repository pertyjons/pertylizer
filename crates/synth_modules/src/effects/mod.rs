//! Audio effects modules.
//!
//! This module contains effect processors that operate on the mixed
//! audio output:
//! - BBD Delay (analog bucket-brigade emulation)
//! - Delay (mono, stereo, ping-pong)
//! - Reverb (Schroeder algorithm)
//! - Distortion (soft clip, hard clip, foldback, bitcrush)
//! - Chorus
//! - Phaser (cascaded all-pass filters)
//! - Flanger (modulated delay with feedback)
//! - Compressor (dynamics processor)
//! - EQ (3-band parametric equalizer)
//! - Mid/Side (stereo width and mid/side gain processing)

pub mod bbd_delay;
pub mod chorus;
pub mod compressor;
pub mod delay;
pub mod distortion;
pub mod eq;
pub mod flanger;
pub mod limiter;
pub mod mid_side;
pub mod phaser;
pub mod reverb;
pub mod waveshaper;

pub use bbd_delay::BbdDelay;
pub use chorus::Chorus;
pub use compressor::Compressor;
pub use delay::Delay;
pub use distortion::{Distortion, DistortionType};
pub use eq::Eq;
pub use flanger::Flanger;
pub use limiter::Limiter;
pub use mid_side::MidSide;
pub use phaser::Phaser;
pub use reverb::Reverb;
pub use waveshaper::Waveshaper;
