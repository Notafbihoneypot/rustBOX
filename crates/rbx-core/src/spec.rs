//! The system description: what `rustbox.toml` deserialises into.

use serde::Deserialize;

use crate::pci::{DeviceClass, PciAddress};

/// A whole rustBOX system: dom0 plus every qube.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SystemSpec {
    pub system: SystemMeta,
    #[serde(default, rename = "qube")]
    pub qubes: Vec<QubeSpec>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SystemMeta {
    pub name: String,
    /// GUI qube that displays windows of app and standalone qubes, unless a
    /// qube names its own.
    pub guivm: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QubeSpec {
    pub name: String,
    pub role: Role,
    /// Virtualisation mode. Defaults to HVM for standalone qubes and qubes
    /// with PCI devices, PVH otherwise.
    pub virt: Option<Virt>,
    pub memory_mb: u32,
    #[serde(default = "one")]
    pub vcpus: u32,
    /// Trust colour the GUI qube paints around this qube's windows.
    pub label: Option<Label>,
    /// Qube providing this qube's network. `None` means no network at all.
    pub netvm: Option<String>,
    /// Overrides [`SystemMeta::guivm`] for this qube.
    pub guivm: Option<String>,
    #[serde(default)]
    pub pci: Vec<PciAssignment>,
    /// Direct kernel boot (required for PVH qubes).
    pub kernel: Option<String>,
    pub ramdisk: Option<String>,
    pub cmdline: Option<String>,
    #[serde(default)]
    pub disks: Vec<DiskSpec>,
    /// Installation medium for a bring-your-own-ISO HVM.
    pub iso: Option<String>,
    #[serde(default)]
    pub autostart: bool,
}

fn one() -> u32 {
    1
}

impl QubeSpec {
    /// The virtualisation mode this qube will actually run in.
    pub fn effective_virt(&self) -> Virt {
        match self.virt {
            Some(v) => v,
            None if self.role == Role::Dom0 => Virt::Pvh,
            None if self.role == Role::Standalone || !self.pci.is_empty() => Virt::Hvm,
            None => Virt::Pvh,
        }
    }

    pub fn owns(&self, class: DeviceClass) -> bool {
        self.pci.iter().any(|p| p.class == class)
    }
}

impl SystemSpec {
    pub fn parse(text: &str) -> Result<SystemSpec, toml::de::Error> {
        toml::from_str(text)
    }

    pub fn qube(&self, name: &str) -> Option<&QubeSpec> {
        self.qubes.iter().find(|q| q.name == name)
    }

    pub fn dom0(&self) -> Option<&QubeSpec> {
        self.qubes.iter().find(|q| q.role == Role::Dom0)
    }

    /// The GUI qube that shows `qube`'s windows, if it has any to show.
    pub fn guivm_of<'a>(&'a self, qube: &'a QubeSpec) -> Option<&'a str> {
        if !qube.role.has_windows() {
            return None;
        }
        qube.guivm.as_deref().or(self.system.guivm.as_deref())
    }
}

/// What a qube is for. The role decides which devices it may own and how it
/// may be connected.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Role {
    /// The control domain: builds and supervises qubes. No devices, no
    /// network, no user applications.
    Dom0,
    /// Owns the network cards and provides the uplink (`sys-net`).
    Net,
    /// Filters traffic between its clients and its own netvm.
    Firewall,
    /// Owns the USB controllers (`sys-usb`).
    Usb,
    /// Owns the GPU and runs the Wayland compositor (`sys-gui`).
    Gui,
    /// Runs user applications.
    App,
    /// Self-contained HVM: bring your own ISO, optionally a passed-through
    /// GPU.
    Standalone,
}

impl Role {
    pub fn provides_network(self) -> bool {
        matches!(self, Role::Net | Role::Firewall)
    }

    /// Whether qubes of this role display windows through a GUI qube.
    pub fn has_windows(self) -> bool {
        matches!(self, Role::App | Role::Standalone)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Role::Dom0 => "dom0",
            Role::Net => "net",
            Role::Firewall => "firewall",
            Role::Usb => "usb",
            Role::Gui => "gui",
            Role::App => "app",
            Role::Standalone => "standalone",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Virt {
    /// Hardware-virtualised container, no emulated devices. Smallest attack
    /// surface; needs a PVH-capable kernel.
    Pvh,
    /// Full machine emulation (device model in a stub domain). Needed for
    /// PCI passthrough and for unmodified guest systems.
    Hvm,
}

impl Virt {
    pub fn as_str(self) -> &'static str {
        match self {
            Virt::Pvh => "pvh",
            Virt::Hvm => "hvm",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Label {
    Red,
    Orange,
    Yellow,
    Green,
    Gray,
    Blue,
    Purple,
    Black,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PciAssignment {
    pub bdf: PciAddress,
    pub class: DeviceClass,
    /// Lets the guest write otherwise-filtered config space registers. Some
    /// devices need it; it widens the attack surface, so it is opt-in.
    #[serde(default)]
    pub permissive: bool,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DiskSpec {
    pub path: String,
    #[serde(default)]
    pub readonly: bool,
}
