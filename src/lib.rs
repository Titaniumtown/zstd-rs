#![deny(trivial_casts, trivial_numeric_casts, rust_2018_idioms)]
#![feature(const_slice_from_raw_parts)]
#![feature(const_mut_refs)]
#![feature(const_ptr_read)]
#![feature(const_default_impls)]
#![feature(const_trait_impl)]
#![feature(const_num_from_num)]


pub mod blocks;
pub mod decoding;
pub mod errors;
pub mod frame;
pub mod frame_decoder;
pub mod fse;
pub mod huff0;
pub mod streaming_decoder;
mod tests;

pub const VERBOSE: bool = false;
pub use frame_decoder::BlockDecodingStrategy;
pub use frame_decoder::FrameDecoder;
pub use streaming_decoder::StreamingDecoder;
