//! PCI addresses, device classes and host device discovery.

use std::fmt;
use std::path::Path;
use std::str::FromStr;

use serde::Deserialize;

use crate::spec::Role;

/// A PCI device address: `domain:bus:device.function`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Deserialize)]
#[serde(try_from = "String")]
pub struct PciAddress {
    pub domain: u16,
    pub bus: u8,
    pub device: u8,
    pub function: u8,
}

#[derive(Debug, PartialEq, Eq)]
pub struct ParsePciAddressError(String);

impl fmt::Display for ParsePciAddressError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "invalid PCI address `{}` (expected [dddd:]bb:dd.f in hex)",
            self.0
        )
    }
}

impl std::error::Error for ParsePciAddressError {}

impl FromStr for PciAddress {
    type Err = ParsePciAddressError;

    /// Accepts `dddd:bb:dd.f` and the short `bb:dd.f` (domain 0).
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let err = || ParsePciAddressError(s.to_string());
        // Exact field widths, hex digits only: this string ends up in domain
        // configs and sysfs paths, so nothing looser is accepted.
        let hex = |field: &str, width: usize| -> Result<u16, ParsePciAddressError> {
            if field.len() != width || !field.bytes().all(|b| b.is_ascii_hexdigit()) {
                return Err(err());
            }
            u16::from_str_radix(field, 16).map_err(|_| err())
        };
        let (rest, function) = s.rsplit_once('.').ok_or_else(err)?;
        let mut parts: Vec<&str> = rest.split(':').collect();
        let device = parts.pop().ok_or_else(err)?;
        let bus = parts.pop().ok_or_else(err)?;
        let domain = match parts.as_slice() {
            [] => 0,
            [d] => hex(d, 4)?,
            _ => return Err(err()),
        };
        let addr = PciAddress {
            domain,
            bus: hex(bus, 2)? as u8,
            device: hex(device, 2)? as u8,
            function: hex(function, 1)? as u8,
        };
        if addr.device > 0x1f || addr.function > 7 {
            return Err(err());
        }
        Ok(addr)
    }
}

impl TryFrom<String> for PciAddress {
    type Error = ParsePciAddressError;

    fn try_from(s: String) -> Result<Self, Self::Error> {
        s.parse()
    }
}

impl fmt::Display for PciAddress {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{:04x}:{:02x}:{:02x}.{:x}",
            self.domain, self.bus, self.device, self.function
        )
    }
}

/// What a PCI device is, as far as the isolation policy cares.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DeviceClass {
    /// Network controller (wired or wireless).
    Nic,
    /// USB host controller.
    Usb,
    /// Display controller.
    Gpu,
    /// Audio device (typically a GPU's HDMI audio function).
    Audio,
    /// Anything else.
    Other,
}

impl DeviceClass {
    /// Classifies a 24-bit PCI class code (`base << 16 | sub << 8 | prog-if`).
    pub fn from_class_code(code: u32) -> DeviceClass {
        let base = (code >> 16) & 0xff;
        let sub = (code >> 8) & 0xff;
        match (base, sub) {
            (0x02, _) => DeviceClass::Nic,
            (0x03, _) => DeviceClass::Gpu,
            (0x04, 0x01 | 0x03) => DeviceClass::Audio,
            (0x0c, 0x03) => DeviceClass::Usb,
            _ => DeviceClass::Other,
        }
    }

    /// Whether the policy lets a qube with `role` own a device of this class.
    pub fn allowed_for(self, role: Role) -> bool {
        match self {
            DeviceClass::Nic => role == Role::Net,
            DeviceClass::Usb => role == Role::Usb,
            DeviceClass::Gpu | DeviceClass::Audio => matches!(role, Role::Gui | Role::Standalone),
            DeviceClass::Other => role == Role::Standalone,
        }
    }

    /// The roles that may own this class, for messages.
    pub fn allowed_roles(self) -> &'static str {
        match self {
            DeviceClass::Nic => "net",
            DeviceClass::Usb => "usb",
            DeviceClass::Gpu | DeviceClass::Audio => "gui or standalone",
            DeviceClass::Other => "standalone",
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            DeviceClass::Nic => "nic",
            DeviceClass::Usb => "usb",
            DeviceClass::Gpu => "gpu",
            DeviceClass::Audio => "audio",
            DeviceClass::Other => "other",
        }
    }
}

/// A PCI device found on the host.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HostDevice {
    pub address: PciAddress,
    pub vendor: u16,
    pub device: u16,
    pub class_code: u32,
}

impl HostDevice {
    pub fn class(&self) -> DeviceClass {
        DeviceClass::from_class_code(self.class_code)
    }
}

/// Lists PCI devices from a Linux-style sysfs tree (`/sys` on a real host),
/// sorted by address. Entries that are not PCI functions are skipped.
pub fn scan_sysfs(sysfs: &Path) -> std::io::Result<Vec<HostDevice>> {
    let mut devices = Vec::new();
    for entry in std::fs::read_dir(sysfs.join("bus/pci/devices"))? {
        let entry = entry?;
        let Some(address) = entry
            .file_name()
            .to_str()
            .and_then(|n| n.parse::<PciAddress>().ok())
        else {
            continue;
        };
        let attr = |name: &str| -> std::io::Result<u32> {
            let text = std::fs::read_to_string(entry.path().join(name))?;
            let text = text.trim();
            u32::from_str_radix(text.strip_prefix("0x").unwrap_or(text), 16).map_err(|e| {
                std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    format!("{address} {name}: {e}"),
                )
            })
        };
        devices.push(HostDevice {
            address,
            vendor: attr("vendor")? as u16,
            device: attr("device")? as u16,
            class_code: attr("class")?,
        });
    }
    devices.sort_by_key(|d| d.address);
    Ok(devices)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_full_and_short_addresses() {
        let full: PciAddress = "0000:03:00.1".parse().unwrap();
        assert_eq!(
            full,
            PciAddress {
                domain: 0,
                bus: 3,
                device: 0,
                function: 1
            }
        );
        assert_eq!("03:00.1".parse::<PciAddress>().unwrap(), full);
        assert_eq!(
            "00a1:ff:1f.7".parse::<PciAddress>().unwrap().to_string(),
            "00a1:ff:1f.7"
        );
    }

    #[test]
    fn rejects_malformed_and_out_of_range_addresses() {
        for bad in [
            "",
            "03:00",
            "0000:03:00.8",   // function > 7
            "0000:03:20.0",   // device > 0x1f
            "0000:3:00.0",    // short bus
            "00000:03:00.0",  // long domain
            "0:0000:03:00.0", // too many fields
            "0000:03:00.0,permissive=1",
            "0000:03:0g.0",
            "+000:03:00.0",
            " 03:00.0",
        ] {
            assert!(
                bad.parse::<PciAddress>().is_err(),
                "{bad:?} should not parse"
            );
        }
    }

    #[test]
    fn classifies_class_codes() {
        assert_eq!(DeviceClass::from_class_code(0x020000), DeviceClass::Nic); // ethernet
        assert_eq!(DeviceClass::from_class_code(0x028000), DeviceClass::Nic); // wifi
        assert_eq!(DeviceClass::from_class_code(0x030000), DeviceClass::Gpu); // VGA
        assert_eq!(DeviceClass::from_class_code(0x038000), DeviceClass::Gpu); // display, other
        assert_eq!(DeviceClass::from_class_code(0x040300), DeviceClass::Audio);
        assert_eq!(DeviceClass::from_class_code(0x0c0330), DeviceClass::Usb); // xHCI
        assert_eq!(DeviceClass::from_class_code(0x0c0500), DeviceClass::Other); // SMBus
        assert_eq!(DeviceClass::from_class_code(0x010802), DeviceClass::Other); // NVMe
    }

    #[test]
    fn scans_a_sysfs_tree() {
        let root = std::env::temp_dir().join(format!("rbx-sysfs-{}", std::process::id()));
        let devs = root.join("bus/pci/devices");
        let add = |bdf: &str, class: &str| {
            let d = devs.join(bdf);
            std::fs::create_dir_all(&d).unwrap();
            std::fs::write(d.join("class"), format!("{class}\n")).unwrap();
            std::fs::write(d.join("vendor"), "0x8086\n").unwrap();
            std::fs::write(d.join("device"), "0x15b8\n").unwrap();
        };
        add("0000:00:1f.6", "0x020000");
        add("0000:00:02.0", "0x030000");
        std::fs::create_dir_all(devs.join("not-a-device")).unwrap();

        let found = scan_sysfs(&root).unwrap();
        std::fs::remove_dir_all(&root).unwrap();

        assert_eq!(found.len(), 2);
        assert_eq!(found[0].address.to_string(), "0000:00:02.0");
        assert_eq!(found[0].class(), DeviceClass::Gpu);
        assert_eq!(found[1].class(), DeviceClass::Nic);
        assert_eq!((found[1].vendor, found[1].device), (0x8086, 0x15b8));
    }
}
