//! Numeric conversions that `From`/`TryFrom` can't express.
//!
//! The crate denies `as` casts, so each conversion it needs lives here once,
//! with the reason it is sound for how the crate uses it.
#![allow(
    clippy::as_conversions,
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss
)]

// `u32 -> usize` is lossless on every target this crate builds for.
const _: () = assert!(usize::BITS >= u32::BITS);

/// Lossless: `usize` is at least 32 bits (asserted above).
#[inline]
#[must_use]
pub const fn usize_from(v: u32) -> usize {
    v as usize
}

/// Exact below 2^24, which covers every count, index and grid cell the crate
/// converts.
#[inline]
#[must_use]
pub const fn f32_from_u32(v: u32) -> f32 {
    v as f32
}

/// Exact for `|v| < 2^24`; see [`f32_from_u32`].
#[inline]
#[must_use]
pub const fn f32_from_i32(v: i32) -> f32 {
    v as f32
}

/// Exact below 2^24; see [`f32_from_u32`].
#[inline]
#[must_use]
pub const fn f32_from_usize(v: usize) -> f32 {
    v as f32
}

/// The top 24 bits of `v` as a float in `0..1`: every value is exact.
#[inline]
#[must_use]
pub const fn unit_f32_from_u64(v: u64) -> f32 {
    (v >> 40) as f32 * (1.0 / 16_777_216.0)
}

/// Saturating float -> int: out-of-range values clamp to the type's bounds
/// and NaN becomes 0. Floor first; this truncates toward zero.
#[inline]
#[must_use]
pub const fn i32_from_f32(v: f32) -> i32 {
    v as i32
}

/// Saturating like [`i32_from_f32`]; negative values become 0.
#[inline]
#[must_use]
pub const fn u32_from_f32(v: f32) -> u32 {
    v as u32
}

/// Saturating at `u32::MAX`.
#[inline]
#[must_use]
pub fn u32_from_usize(v: usize) -> u32 {
    u32::try_from(v).unwrap_or(u32::MAX)
}

/// Bit-for-bit reinterpretation, for hashing.
#[inline]
#[must_use]
pub const fn u64_from_i64_bits(v: i64) -> u64 {
    v as u64
}
