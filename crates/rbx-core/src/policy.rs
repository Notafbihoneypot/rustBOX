//! The isolation policy: every rule a system description must satisfy
//! before any qube is built from it.

use std::collections::BTreeMap;
use std::fmt;

use crate::pci::{DeviceClass, PciAddress};
use crate::spec::{QubeSpec, Role, SystemSpec, Virt};

/// Smallest memory size accepted for any domain.
pub const MIN_MEMORY_MB: u32 = 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Severity {
    /// The description is unsafe or cannot work; nothing is rendered.
    Error,
    /// Suspicious but allowed.
    Warning,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Issue {
    pub severity: Severity,
    /// Stable machine-readable identifier of the rule.
    pub code: &'static str,
    /// The qube the issue is about, if it is about one.
    pub qube: Option<String>,
    pub message: String,
}

impl fmt::Display for Issue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let severity = match self.severity {
            Severity::Error => "error",
            Severity::Warning => "warning",
        };
        match &self.qube {
            Some(q) => write!(f, "{severity}[{}]: {q}: {}", self.code, self.message),
            None => write!(f, "{severity}[{}]: {}", self.code, self.message),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Report {
    pub issues: Vec<Issue>,
}

impl Report {
    pub fn has_errors(&self) -> bool {
        self.issues.iter().any(|i| i.severity == Severity::Error)
    }

    pub fn errors(&self) -> impl Iterator<Item = &Issue> {
        self.issues.iter().filter(|i| i.severity == Severity::Error)
    }

    pub fn warnings(&self) -> impl Iterator<Item = &Issue> {
        self.issues
            .iter()
            .filter(|i| i.severity == Severity::Warning)
    }

    fn error(&mut self, code: &'static str, qube: Option<&QubeSpec>, message: String) {
        self.push(Severity::Error, code, qube, message);
    }

    fn warning(&mut self, code: &'static str, qube: Option<&QubeSpec>, message: String) {
        self.push(Severity::Warning, code, qube, message);
    }

    fn push(
        &mut self,
        severity: Severity,
        code: &'static str,
        qube: Option<&QubeSpec>,
        message: String,
    ) {
        self.issues.push(Issue {
            severity,
            code,
            qube: qube.map(|q| q.name.clone()),
            message,
        });
    }
}

/// A system description that passed [`check`] without errors. The only way
/// to get one is [`ValidSystem::new`], and it is what rendering requires.
#[derive(Clone, Copy, Debug)]
pub struct ValidSystem<'a> {
    spec: &'a SystemSpec,
}

impl<'a> ValidSystem<'a> {
    /// Checks `spec`; on success also returns the warnings.
    pub fn new(spec: &'a SystemSpec) -> Result<(ValidSystem<'a>, Report), Report> {
        let report = check(spec);
        if report.has_errors() {
            Err(report)
        } else {
            Ok((ValidSystem { spec }, report))
        }
    }

    pub fn spec(&self) -> &'a SystemSpec {
        self.spec
    }

    pub fn dom0(&self) -> &'a QubeSpec {
        self.spec
            .dom0()
            .expect("a valid system has exactly one dom0")
    }
}

/// Checks `spec` against every policy rule and reports all violations.
pub fn check(spec: &SystemSpec) -> Report {
    let mut r = Report::default();

    if !is_valid_name(&spec.system.name) {
        r.error(
            "bad-system-name",
            None,
            format!(
                "system name `{}` {NAME_RULE}",
                spec.system.name.escape_debug()
            ),
        );
    }
    check_names(spec, &mut r);
    match spec.qubes.iter().filter(|q| q.role == Role::Dom0).count() {
        0 => r.error(
            "no-dom0",
            None,
            "the system needs a qube with role `dom0`".into(),
        ),
        1 => {}
        n => r.error(
            "multiple-dom0",
            None,
            format!("{n} qubes have role `dom0`; there is exactly one control domain"),
        ),
    }
    for q in &spec.qubes {
        check_resources(q, &mut r);
        if q.role == Role::Dom0 {
            check_dom0(q, &mut r);
        } else {
            check_boot(q, &mut r);
            check_devices(q, &mut r);
            check_network(spec, q, &mut r);
            check_gui(spec, q, &mut r);
        }
    }
    check_device_ownership(spec, &mut r);
    if let Some(gui) = &spec.system.guivm {
        check_guivm_ref(spec, None, gui, &mut r);
    }
    r
}

const NAME_RULE: &str = "must be 1-31 characters of a-z, 0-9 and `-`, starting with a letter";

/// Names end up in domain configs, xenstore paths and file names, so the
/// alphabet is deliberately tiny.
fn is_valid_name(name: &str) -> bool {
    let mut bytes = name.bytes();
    matches!(bytes.next(), Some(b'a'..=b'z'))
        && name.len() <= 31
        && bytes.all(|b| matches!(b, b'a'..=b'z' | b'0'..=b'9' | b'-'))
}

fn check_names(spec: &SystemSpec, r: &mut Report) {
    let mut seen: BTreeMap<&str, ()> = BTreeMap::new();
    for q in &spec.qubes {
        if !is_valid_name(&q.name) {
            r.error(
                "bad-name",
                None,
                format!("qube name `{}` {NAME_RULE}", q.name.escape_debug()),
            );
        }
        if seen.insert(&q.name, ()).is_some() {
            r.error(
                "duplicate-name",
                Some(q),
                "more than one qube has this name".into(),
            );
        }
        if (q.name == "dom0") != (q.role == Role::Dom0) {
            r.error(
                "dom0-name",
                Some(q),
                "the name `dom0` and the role `dom0` go together".into(),
            );
        }
    }
}

fn check_resources(q: &QubeSpec, r: &mut Report) {
    if q.memory_mb < MIN_MEMORY_MB {
        r.error(
            "memory-too-small",
            Some(q),
            format!(
                "memory_mb = {} is below the {MIN_MEMORY_MB} MiB minimum",
                q.memory_mb
            ),
        );
    }
    if q.vcpus == 0 {
        r.error("no-vcpus", Some(q), "vcpus must be at least 1".into());
    }
}

/// dom0 is the root of trust: it gets CPU and memory and nothing else.
fn check_dom0(q: &QubeSpec, r: &mut Report) {
    if q.netvm.is_some() {
        r.error(
            "dom0-networked",
            Some(q),
            "dom0 must not have a network connection".into(),
        );
    }
    if !q.pci.is_empty() {
        r.error(
            "dom0-owns-device",
            Some(q),
            "dom0 must not own PCI devices; give them to a service qube".into(),
        );
    }
    if q.virt == Some(Virt::Hvm) {
        r.error("dom0-virt", Some(q), "dom0 runs as a PVH domain".into());
    }
    let extras = [
        ("kernel", q.kernel.is_some()),
        ("ramdisk", q.ramdisk.is_some()),
        ("cmdline", q.cmdline.is_some()),
        ("disks", !q.disks.is_empty()),
        ("iso", q.iso.is_some()),
        ("guivm", q.guivm.is_some()),
        ("label", q.label.is_some()),
        ("autostart", q.autostart),
    ];
    for (field, set) in extras {
        if set {
            r.error(
                "dom0-field",
                Some(q),
                format!("`{field}` does not apply to dom0 (it is booted by Xen and described by memory_mb and vcpus only)"),
            );
        }
    }
}

fn check_boot(q: &QubeSpec, r: &mut Report) {
    let virt = q.effective_virt();
    if q.virt == Some(Virt::Pvh) {
        if !q.pci.is_empty() {
            r.error(
                "pci-needs-hvm",
                Some(q),
                "PCI passthrough needs virt = \"hvm\"".into(),
            );
        }
        if q.iso.is_some() {
            r.error(
                "iso-needs-hvm",
                Some(q),
                "booting an ISO needs virt = \"hvm\"".into(),
            );
        }
    }
    match virt {
        Virt::Pvh if q.kernel.is_none() => {
            r.error(
                "no-boot-source",
                Some(q),
                "a PVH qube boots a `kernel` directly; none is set".into(),
            );
        }
        Virt::Hvm if q.kernel.is_none() && q.iso.is_none() && q.disks.is_empty() => {
            r.error(
                "no-boot-source",
                Some(q),
                "an HVM qube needs an `iso`, a disk or a `kernel` to boot from".into(),
            );
        }
        _ => {}
    }
    if q.kernel.is_none() && (q.ramdisk.is_some() || q.cmdline.is_some()) {
        r.error(
            "orphan-boot-arg",
            Some(q),
            "`ramdisk` and `cmdline` only make sense together with `kernel`".into(),
        );
    }

    let paths = [
        ("kernel", &q.kernel),
        ("ramdisk", &q.ramdisk),
        ("iso", &q.iso),
    ];
    let paths = paths
        .iter()
        .filter_map(|(what, p)| p.as_deref().map(|p| (*what, p)));
    for (what, path) in paths.chain(q.disks.iter().map(|d| ("disk", d.path.as_str()))) {
        if !is_safe_path(path) {
            r.error(
                "unsafe-path",
                Some(q),
                format!("{what} path `{}` must be absolute, without `..`, using only a-z A-Z 0-9 / . _ + -", path.escape_debug()),
            );
        }
    }
    if let Some(cmdline) = &q.cmdline {
        if !is_safe_cmdline(cmdline) {
            r.error(
                "unsafe-cmdline",
                Some(q),
                "cmdline must be printable ASCII without `\"` or `\\`".into(),
            );
        }
    }
}

/// Paths are spliced into comma-separated disk specifications and quoted
/// config strings, so anything that could end a field early is refused
/// rather than escaped.
fn is_safe_path(path: &str) -> bool {
    path.starts_with('/')
        && path.len() <= 255
        && path
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'/' | b'.' | b'_' | b'+' | b'-'))
        && !path.split('/').any(|part| part == "..")
}

fn is_safe_cmdline(cmdline: &str) -> bool {
    cmdline.len() <= 1024
        && cmdline
            .bytes()
            .all(|b| matches!(b, b' '..=b'~') && b != b'"' && b != b'\\')
}

fn check_devices(q: &QubeSpec, r: &mut Report) {
    for dev in &q.pci {
        if !dev.class.allowed_for(q.role) {
            r.error(
                "device-role-mismatch",
                Some(q),
                format!(
                    "{} is a {} device; only {} qubes may own one, and this is a {} qube",
                    dev.bdf,
                    dev.class.as_str(),
                    dev.class.allowed_roles(),
                    q.role.as_str()
                ),
            );
        }
    }
    let expected = match q.role {
        Role::Net => Some((
            DeviceClass::Nic,
            "net-without-nic",
            "a net qube without a network card has no uplink",
        )),
        Role::Usb => Some((
            DeviceClass::Usb,
            "usb-without-controller",
            "a usb qube without a USB controller does nothing",
        )),
        Role::Gui => Some((
            DeviceClass::Gpu,
            "gui-without-gpu",
            "a gui qube without a GPU has nothing to draw on",
        )),
        _ => None,
    };
    if let Some((class, code, message)) = expected {
        if !q.owns(class) {
            r.warning(code, Some(q), message.into());
        }
    }
}

/// Every PCI function has exactly one owner.
fn check_device_ownership(spec: &SystemSpec, r: &mut Report) {
    let mut owners: BTreeMap<PciAddress, &QubeSpec> = BTreeMap::new();
    for q in &spec.qubes {
        for dev in &q.pci {
            if let Some(first) = owners.insert(dev.bdf, q) {
                let message = if first.name == q.name {
                    format!("{} is listed twice", dev.bdf)
                } else {
                    format!("{} is already assigned to `{}`", dev.bdf, first.name)
                };
                r.error("device-shared", Some(q), message);
                owners.insert(dev.bdf, first);
            }
        }
    }
}

fn check_network(spec: &SystemSpec, q: &QubeSpec, r: &mut Report) {
    let Some(netvm) = q.netvm.as_deref() else {
        if q.role == Role::Firewall {
            r.warning(
                "firewall-without-uplink",
                Some(q),
                "a firewall qube without a netvm forwards nowhere".into(),
            );
        }
        return;
    };
    match q.role {
        // These hold the keys to the machine (input devices, the screen):
        // they stay offline so a compromise cannot phone home.
        Role::Usb | Role::Gui => {
            r.error(
                "isolated-role-networked",
                Some(q),
                format!("a {} qube must not have a netvm", q.role.as_str()),
            );
            return;
        }
        Role::Net => {
            r.error(
                "net-qube-has-netvm",
                Some(q),
                "a net qube is the uplink; it cannot have a netvm itself".into(),
            );
            return;
        }
        _ => {}
    }
    let Some(target) = spec.qube(netvm) else {
        r.error(
            "unknown-netvm",
            Some(q),
            format!(
                "netvm `{}` is not a qube in this system",
                netvm.escape_debug()
            ),
        );
        return;
    };
    if !target.role.provides_network() {
        r.error(
            "netvm-not-a-provider",
            Some(q),
            format!(
                "netvm `{}` is a {} qube; only net and firewall qubes provide network",
                target.name,
                target.role.as_str()
            ),
        );
        return;
    }
    // Walk the provider chain; it must end at a qube without a netvm.
    let mut hops = 0;
    let mut cur = target;
    while let Some(next) = cur.netvm.as_deref().and_then(|n| spec.qube(n)) {
        hops += 1;
        if next.name == q.name || hops > spec.qubes.len() {
            r.error(
                "netvm-cycle",
                Some(q),
                "the netvm chain loops back instead of ending at a net qube".into(),
            );
            return;
        }
        cur = next;
    }
}

fn check_gui(spec: &SystemSpec, q: &QubeSpec, r: &mut Report) {
    if let Some(gui) = q.guivm.as_deref() {
        if !q.role.has_windows() {
            r.error(
                "guivm-not-applicable",
                Some(q),
                format!(
                    "a {} qube shows no windows; remove `guivm`",
                    q.role.as_str()
                ),
            );
            return;
        }
        check_guivm_ref(spec, Some(q), gui, r);
    }
    // The label is the user's only trustworthy cue for where a window comes
    // from, so nothing reaches the screen without one.
    if spec.guivm_of(q).is_some() && q.label.is_none() {
        r.error(
            "missing-label",
            Some(q),
            "qubes that show windows need a `label` colour".into(),
        );
    }
}

fn check_guivm_ref(spec: &SystemSpec, q: Option<&QubeSpec>, gui: &str, r: &mut Report) {
    match spec.qube(gui) {
        None => r.error(
            "unknown-guivm",
            q,
            format!(
                "guivm `{}` is not a qube in this system",
                gui.escape_debug()
            ),
        ),
        Some(target) if target.role != Role::Gui => {
            r.error(
                "guivm-not-gui",
                q,
                format!(
                    "guivm `{}` is a {} qube, not a gui qube",
                    target.name,
                    target.role.as_str()
                ),
            );
        }
        Some(_) => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A small but complete system used as the starting point of each test.
    const BASE: &str = r#"
        [system]
        name = "test"
        guivm = "sys-gui"

        [[qube]]
        name = "dom0"
        role = "dom0"
        memory_mb = 1024

        [[qube]]
        name = "sys-net"
        role = "net"
        memory_mb = 512
        pci = [{ bdf = "0000:02:00.0", class = "nic" }]
        disks = [{ path = "/var/lib/rbx/sys-net.img" }]

        [[qube]]
        name = "sys-firewall"
        role = "firewall"
        memory_mb = 256
        netvm = "sys-net"
        kernel = "/boot/rbx/vmlinuz"

        [[qube]]
        name = "sys-usb"
        role = "usb"
        memory_mb = 512
        pci = [{ bdf = "0000:00:14.0", class = "usb" }]
        disks = [{ path = "/var/lib/rbx/sys-usb.img" }]

        [[qube]]
        name = "sys-gui"
        role = "gui"
        memory_mb = 2048
        pci = [{ bdf = "0000:00:02.0", class = "gpu" }]
        disks = [{ path = "/var/lib/rbx/sys-gui.img" }]

        [[qube]]
        name = "work"
        role = "app"
        label = "blue"
        memory_mb = 2048
        netvm = "sys-firewall"
        kernel = "/boot/rbx/vmlinuz"
    "#;

    fn base() -> SystemSpec {
        SystemSpec::parse(BASE).unwrap()
    }

    fn qube<'a>(spec: &'a mut SystemSpec, name: &str) -> &'a mut QubeSpec {
        spec.qubes.iter_mut().find(|q| q.name == name).unwrap()
    }

    fn codes(spec: &SystemSpec) -> Vec<&'static str> {
        check(spec).issues.iter().map(|i| i.code).collect()
    }

    fn pci(bdf: &str, class: DeviceClass) -> crate::spec::PciAssignment {
        crate::spec::PciAssignment {
            bdf: bdf.parse().unwrap(),
            class,
            permissive: false,
        }
    }

    #[test]
    fn the_base_system_is_clean() {
        assert_eq!(check(&base()).issues, vec![]);
        assert!(ValidSystem::new(&base()).is_ok());
    }

    #[test]
    fn dom0_gets_no_devices_no_network_and_no_guest_fields() {
        let mut s = base();
        let d = qube(&mut s, "dom0");
        d.netvm = Some("sys-net".into());
        d.pci.push(pci("0000:05:00.0", DeviceClass::Other));
        d.iso = Some("/x.iso".into());
        assert_eq!(
            codes(&s),
            ["dom0-networked", "dom0-owns-device", "dom0-field"]
        );
        assert!(ValidSystem::new(&s).is_err());
    }

    #[test]
    fn there_is_exactly_one_dom0_and_it_is_called_dom0() {
        let mut s = base();
        s.qubes.retain(|q| q.role != Role::Dom0);
        assert_eq!(codes(&s), ["no-dom0"]);

        let mut s = base();
        qube(&mut s, "work").role = Role::Dom0;
        assert!(codes(&s).contains(&"multiple-dom0"));
        assert!(codes(&s).contains(&"dom0-name"));
    }

    #[test]
    fn a_device_has_one_owner() {
        let mut s = base();
        qube(&mut s, "sys-usb")
            .pci
            .push(pci("0000:02:00.0", DeviceClass::Usb));
        let report = check(&s);
        assert_eq!(report.issues.len(), 1);
        assert_eq!(report.issues[0].code, "device-shared");
        assert_eq!(report.issues[0].qube.as_deref(), Some("sys-usb"));
        assert!(report.issues[0].message.contains("`sys-net`"));
    }

    #[test]
    fn device_classes_are_tied_to_roles() {
        // A NIC in an app qube, a USB controller in the net qube, a GPU in
        // the firewall: each bypasses the service qube that should own it.
        for (target, class) in [
            ("work", DeviceClass::Nic),
            ("sys-net", DeviceClass::Usb),
            ("sys-firewall", DeviceClass::Gpu),
        ] {
            let mut s = base();
            qube(&mut s, target).pci.push(pci("0000:09:00.0", class));
            assert_eq!(
                codes(&s),
                ["device-role-mismatch"],
                "{target} with {class:?}"
            );
        }
        // A second GPU for a standalone HVM is the supported way to get one.
        let mut s = base();
        let w = qube(&mut s, "work");
        w.role = Role::Standalone;
        w.pci.push(pci("0000:01:00.0", DeviceClass::Gpu));
        w.pci.push(pci("0000:01:00.1", DeviceClass::Audio));
        assert_eq!(codes(&s), [] as [&str; 0]);
    }

    #[test]
    fn usb_and_gui_qubes_stay_offline() {
        for name in ["sys-usb", "sys-gui"] {
            let mut s = base();
            qube(&mut s, name).netvm = Some("sys-firewall".into());
            assert_eq!(codes(&s), ["isolated-role-networked"], "{name}");
        }
    }

    #[test]
    fn netvm_must_exist_provide_network_and_not_loop() {
        let mut s = base();
        qube(&mut s, "work").netvm = Some("nope".into());
        assert_eq!(codes(&s), ["unknown-netvm"]);

        let mut s = base();
        qube(&mut s, "work").netvm = Some("sys-usb".into());
        assert_eq!(codes(&s), ["netvm-not-a-provider"]);

        // A second uplink qube chained behind the first one.
        let mut s = base();
        let mut wifi = qube(&mut s, "sys-net").clone();
        wifi.name = "sys-wifi".into();
        wifi.pci = vec![pci("0000:04:00.0", DeviceClass::Nic)];
        wifi.netvm = Some("sys-net".into());
        s.qubes.push(wifi);
        assert_eq!(codes(&s), ["net-qube-has-netvm"]);

        let mut s = base();
        let mut fw2 = qube(&mut s, "sys-firewall").clone();
        fw2.name = "sys-firewall-b".into();
        fw2.netvm = Some("sys-firewall".into());
        s.qubes.push(fw2);
        qube(&mut s, "sys-firewall").netvm = Some("sys-firewall-b".into());
        let found = codes(&s);
        assert!(found.iter().all(|c| *c == "netvm-cycle"), "{found:?}");
        // Both firewalls and their client `work` are caught in the loop.
        assert_eq!(found.len(), 3);
    }

    #[test]
    fn passthrough_and_isos_need_hvm_and_everything_needs_a_boot_source() {
        let mut s = base();
        qube(&mut s, "sys-net").virt = Some(Virt::Pvh);
        assert_eq!(codes(&s), ["pci-needs-hvm", "no-boot-source"]);

        let mut s = base();
        qube(&mut s, "work").kernel = None;
        assert_eq!(codes(&s), ["no-boot-source"]);

        let mut s = base();
        qube(&mut s, "work").iso = Some("/iso/x.iso".into());
        qube(&mut s, "work").virt = Some(Virt::Pvh);
        assert_eq!(codes(&s), ["iso-needs-hvm"]);
    }

    #[test]
    fn defaults_pick_hvm_only_where_it_is_needed() {
        let s = base();
        assert_eq!(s.qube("sys-net").unwrap().effective_virt(), Virt::Hvm);
        assert_eq!(s.qube("work").unwrap().effective_virt(), Virt::Pvh);
        assert_eq!(s.qube("dom0").unwrap().effective_virt(), Virt::Pvh);
    }

    #[test]
    fn strings_that_could_break_out_of_a_config_are_refused() {
        for bad in [
            "relative/disk.img",
            "/var/lib/rbx/a.img,backendtype=phy",
            "/var/lib/rbx/a\".img",
            "/var/lib/rbx/a b.img",
            "/var/lib/rbx/../../etc/shadow",
            "/var/lib/rbx/a\n.img",
            "",
        ] {
            let mut s = base();
            qube(&mut s, "sys-net").disks[0].path = bad.into();
            assert_eq!(codes(&s), ["unsafe-path"], "{bad:?}");
        }
        for bad in ["quiet\" extra = \"1", "a\nb", "back\\slash", "caf\u{e9}"] {
            let mut s = base();
            qube(&mut s, "work").cmdline = Some(bad.into());
            assert_eq!(codes(&s), ["unsafe-cmdline"], "{bad:?}");
        }
        for bad in [
            "Work",
            "work qube",
            "1work",
            "",
            "a/b",
            "work\"",
            &"x".repeat(32),
        ] {
            let mut s = base();
            qube(&mut s, "work").name = bad.into();
            assert!(codes(&s).contains(&"bad-name"), "{bad:?}");
        }
    }

    #[test]
    fn windows_need_a_label_and_a_real_gui_qube() {
        let mut s = base();
        qube(&mut s, "work").label = None;
        assert_eq!(codes(&s), ["missing-label"]);

        let mut s = base();
        qube(&mut s, "work").guivm = Some("sys-net".into());
        assert_eq!(codes(&s), ["guivm-not-gui"]);

        let mut s = base();
        s.system.guivm = Some("ghost".into());
        assert_eq!(codes(&s), ["unknown-guivm"]);

        let mut s = base();
        qube(&mut s, "sys-firewall").guivm = Some("sys-gui".into());
        assert_eq!(codes(&s), ["guivm-not-applicable"]);

        // Without any GUI qube an app qube is headless and needs no label.
        let mut s = base();
        s.system.guivm = None;
        qube(&mut s, "work").label = None;
        assert_eq!(codes(&s), [] as [&str; 0]);
    }

    #[test]
    fn service_qubes_without_their_device_only_warn() {
        let mut s = base();
        for name in ["sys-net", "sys-usb", "sys-gui"] {
            qube(&mut s, name).pci.clear();
            qube(&mut s, name).virt = Some(Virt::Hvm);
        }
        let report = check(&s);
        assert!(!report.has_errors());
        let warned: Vec<_> = report.warnings().map(|i| i.code).collect();
        assert_eq!(
            warned,
            [
                "net-without-nic",
                "usb-without-controller",
                "gui-without-gpu"
            ]
        );
        let (_, warnings) = ValidSystem::new(&s).unwrap();
        assert_eq!(warnings.issues.len(), 3);
    }

    #[test]
    fn resources_have_floors() {
        let mut s = base();
        qube(&mut s, "work").memory_mb = 8;
        qube(&mut s, "work").vcpus = 0;
        assert_eq!(codes(&s), ["memory-too-small", "no-vcpus"]);
    }

    #[test]
    fn unknown_fields_and_bad_addresses_fail_at_parse_time() {
        let typo = BASE.replace("netvm = \"sys-firewall\"", "netvn = \"sys-firewall\"");
        assert!(SystemSpec::parse(&typo).is_err());
        let bad_bdf = BASE.replace("0000:02:00.0", "0000:02:00.9");
        assert!(SystemSpec::parse(&bad_bdf)
            .unwrap_err()
            .to_string()
            .contains("invalid PCI address"));
    }
}
