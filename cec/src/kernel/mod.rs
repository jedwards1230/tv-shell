//! The kernel CEC backend — the only place this crate speaks `linux-cec`.
//!
//! Linux-only, as `core/`'s evdev backend is, and for the same reason: every
//! DECISION lives in the pure modules beside it ([`crate::config`],
//! [`crate::protocol`], [`crate::state`]), which build and test on any host. A
//! runner with no `/dev/cecN` still covers the rules.
//!
//! **Nothing here transmits on the CEC bus.** See [`device::open`] for the
//! ordering that keeps that true, including the one call that would transmit if
//! it were made in the wrong order.

pub mod device;
pub mod follower;

pub use device::KernelBackend;
