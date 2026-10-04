#![allow(dead_code)]
#![allow(mutable_transmutes)]
#![allow(non_camel_case_types)]
#![allow(non_snake_case)]
#![allow(non_upper_case_globals)]
#![allow(unused_assignments)]
#![allow(unused_mut)]
// Keep the mechanically translated probe routines reviewable without a
// wholesale style rewrite when the vendored crate is checked as a workspace member.
#![allow(
    clippy::identity_op,
    clippy::needless_return,
    clippy::too_many_arguments,
    clippy::unnecessary_cast
)]
#![doc = include_str!("../README.md")]

extern crate libc;
pub mod engine_adapter;
mod tbprobe;

pub mod tablebases;
pub use engine_adapter::*;
pub use tablebases::*;

#[cfg(test)]
mod tests;
