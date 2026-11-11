//! rustBOX qube model.
//!
//! A rustBOX system is described by one TOML file ([`spec::SystemSpec`]):
//! dom0, the service qubes that own hardware (`sys-net`, `sys-usb`,
//! `sys-gui`), and the qubes that do work. This crate
//!
//! 1. parses that description,
//! 2. checks it against the isolation policy ([`policy`]) — e.g. dom0 owns
//!    no devices and has no network, a PCI device has exactly one owner, USB
//!    controllers only ever go to a USB qube — and
//! 3. renders each qube into a Xen domain configuration ([`xl`]).
//!
//! Rendering takes a [`policy::ValidSystem`], which only exists for
//! descriptions that passed every check, so an unchecked topology cannot be
//! turned into running domains by accident.

pub mod pci;
pub mod plan;
pub mod policy;
pub mod spec;
pub mod xl;

pub use policy::{check, Issue, Report, Severity, ValidSystem};
pub use spec::SystemSpec;
