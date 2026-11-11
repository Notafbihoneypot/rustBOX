//! The shipped reference topology must always satisfy the policy.

use rbx_core::spec::{Role, Virt};
use rbx_core::{plan, xl, SystemSpec, ValidSystem};

const REFERENCE: &str = include_str!("../../../config/rustbox.toml");

#[test]
fn reference_system_passes_the_policy_without_warnings() {
    let spec = SystemSpec::parse(REFERENCE).unwrap();
    let (_, warnings) =
        ValidSystem::new(&spec).unwrap_or_else(|report| panic!("{:#?}", report.issues));
    assert_eq!(warnings.issues, vec![]);
}

#[test]
fn reference_system_has_the_advertised_shape() {
    let spec = SystemSpec::parse(REFERENCE).unwrap();
    let (system, _) = ValidSystem::new(&spec).unwrap();

    // Hardware lives in service qubes, never in dom0.
    assert!(system.dom0().pci.is_empty());
    for (name, role) in [
        ("sys-net", Role::Net),
        ("sys-usb", Role::Usb),
        ("sys-gui", Role::Gui),
    ] {
        let q = spec.qube(name).unwrap();
        assert_eq!(q.role, role);
        assert_eq!(q.effective_virt(), Virt::Hvm, "{name} owns PCI devices");
    }
    // App qubes are PVH and reach the network only through the firewall.
    assert_eq!(spec.qube("work").unwrap().effective_virt(), Virt::Pvh);
    assert_eq!(
        spec.qube("work").unwrap().netvm.as_deref(),
        Some("sys-firewall")
    );
    assert_eq!(spec.qube("vault").unwrap().netvm, None);

    let order: Vec<&str> = plan::start_order(&system)
        .iter()
        .map(|q| q.name.as_str())
        .collect();
    assert_eq!(
        order,
        [
            "sys-net",
            "sys-firewall",
            "sys-usb",
            "sys-gui",
            "work",
            "vault",
            "byo",
            "gpu-hvm"
        ]
    );

    // Every qube renders, and the BYO-ISO qube boots its installer.
    for name in &order {
        assert!(xl::render(&system, name).is_some(), "{name}");
    }
    let byo = xl::render(&system, "byo").unwrap();
    assert!(byo.contains("devtype=cdrom,target=/var/lib/rbx/iso/install.iso"));
    assert!(byo.contains("backend=sys-firewall"));
}
