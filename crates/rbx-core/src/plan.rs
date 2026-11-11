//! Start order: a qube starts after the qubes it depends on.

use crate::policy::ValidSystem;
use crate::spec::{QubeSpec, Role};

/// Every qube except dom0, ordered so that each one comes after its netvm
/// and its GUI qube. Among qubes with no ordering constraint the order of
/// the system description is kept.
pub fn start_order<'a>(system: &ValidSystem<'a>) -> Vec<&'a QubeSpec> {
    let spec = system.spec();
    let mut order: Vec<&QubeSpec> = Vec::new();
    for q in spec.qubes.iter().filter(|q| q.role != Role::Dom0) {
        visit(system, q, &mut order);
    }
    order
}

fn visit<'a>(system: &ValidSystem<'a>, q: &'a QubeSpec, order: &mut Vec<&'a QubeSpec>) {
    if order.iter().any(|o| o.name == q.name) {
        return;
    }
    let spec = system.spec();
    // Validation guarantees both references resolve and that the netvm
    // chain is acyclic; GUI qubes have no dependencies of their own.
    let deps = [q.netvm.as_deref(), spec.guivm_of(q)];
    for dep in deps
        .into_iter()
        .flatten()
        .filter_map(|name| spec.qube(name))
    {
        visit(system, dep, order);
    }
    order.push(q);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spec::SystemSpec;

    #[test]
    fn dependencies_start_first_even_when_declared_last() {
        let spec = SystemSpec::parse(
            r#"
            [system]
            name = "t"
            guivm = "sys-gui"

            [[qube]]
            name = "work"
            role = "app"
            label = "blue"
            memory_mb = 512
            netvm = "sys-firewall"
            kernel = "/k"

            [[qube]]
            name = "vault"
            role = "app"
            label = "black"
            memory_mb = 512
            kernel = "/k"

            [[qube]]
            name = "sys-firewall"
            role = "firewall"
            memory_mb = 256
            netvm = "sys-net"
            kernel = "/k"

            [[qube]]
            name = "dom0"
            role = "dom0"
            memory_mb = 1024

            [[qube]]
            name = "sys-gui"
            role = "gui"
            memory_mb = 1024
            pci = [{ bdf = "00:02.0", class = "gpu" }]
            kernel = "/k"

            [[qube]]
            name = "sys-net"
            role = "net"
            memory_mb = 256
            pci = [{ bdf = "02:00.0", class = "nic" }]
            kernel = "/k"
            "#,
        )
        .unwrap();
        let (system, _) = ValidSystem::new(&spec).unwrap();
        let names: Vec<&str> = start_order(&system)
            .iter()
            .map(|q| q.name.as_str())
            .collect();
        assert_eq!(
            names,
            ["sys-net", "sys-firewall", "sys-gui", "work", "vault"]
        );
    }
}
