#![deny(unsafe_code)]
#![doc = include_str!("../README.md")]

pub mod engine_adapter;
#[cfg(feature = "fuzzing")]
pub mod fuzz_support;
#[allow(unsafe_code)]
mod storage;
mod table_decoder;
mod table_encoder;
mod table_lookup;
mod table_moves;
mod table_parser;
mod table_position;
mod table_probe;
mod tbprobe;

pub mod tablebases;
pub use engine_adapter::*;
pub use tablebases::*;

#[cfg(test)]
mod tests;
