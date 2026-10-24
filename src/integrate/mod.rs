//! Glue for sibling crates, each behind its own feature so the core depends
//! on neither.

#[cfg(feature = "ornisculpt")]
pub mod ornisculpt;
#[cfg(feature = "tarasaur")]
pub mod tarasaur;
