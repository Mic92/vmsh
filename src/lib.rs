#![deny(clippy::print_stdout, clippy::print_stderr, clippy::unwrap_used)]
// TODO: more checks
//#![warn(
//    clippy::pedantic,
//)]
//#![allow(
//    clippy::similar_names,
//    cast_sign_loss,
//    missing_errors_doc,
//    cast_possible_truncation,
//    cast_possible_wrap
//)]

pub mod attach;
pub mod coredump;
pub mod cpu;
pub mod debug;
pub mod devices;
pub mod elf;
pub mod guest_mem;
pub mod inspect;
pub mod interrutable_thread;
pub mod kernel;
pub mod kvm;
pub mod loader;
pub mod page_math;
pub mod page_table;
pub mod result;
pub mod signal_handler;
pub mod stage1;
pub mod tracer;
