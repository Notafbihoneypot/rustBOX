//! Xen public ABI definitions for x86_64 PVH/HVM guests.
//!
//! This crate is `no_std` and contains only constants and `#[repr(C)]`
//! layouts taken from Xen's stable public interface (`xen/include/public`).
//! It performs no hypercalls itself, so it can be shared by the freestanding
//! kernel and by host-side tooling and tests.
#![no_std]

pub mod cpuid;
pub mod elfnote;
pub mod hypercall;
pub mod start_info;
