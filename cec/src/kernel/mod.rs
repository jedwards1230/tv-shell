//! The kernel CEC backend — the only place this crate speaks `linux-cec`.
//!
//! Linux-only, as `core/`'s evdev backend is, and for the same reason: every
//! DECISION lives in the pure modules beside it ([`crate::config`],
//! [`crate::protocol`], [`crate::state`]), which build and test on any host. A
//! runner with no `/dev/cecN` still covers the rules.
//!
//! **Nothing here transmits except on a client's request.** [`device::open`]
//! configures the adapter without putting a message on the bus — see its docs
//! for the one call that would, if it were made in the wrong order — and
//! [`follower`] only listens. Every transmit this daemon makes comes from a
//! [`crate::action::Plan`], built by a pure function and translated to
//! `linux-cec` messages in [`ops`].

pub mod device;
pub mod follower;
pub mod ops;

pub use device::KernelBackend;
