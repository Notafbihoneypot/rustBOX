# rustBOX

A security-by-compartmentalisation operating system: **Xen** underneath, a
**Rust microkernel as dom0**, hardware pushed out into single-purpose service
qubes, and everything you actually do running in isolated qubes on top.

Think Qubes OS's architecture with Redox's implementation philosophy:

| | |
|---|---|
| Hypervisor | Xen (unmodified upstream) |
| dom0 | rustBOX microkernel + Rust toolstack. Owns no devices, has no network, runs no applications |
| `sys-net` | owns the network cards |
| `sys-usb` | owns the USB controllers |
| `sys-gui` | owns the primary GPU (IOMMU passthrough) and runs the Wayland compositor |
| app qubes | PVH guests, windows forwarded to `sys-gui`, network only through `sys-firewall` |
| HVMs | bring your own ISO; optionally a second GPU passed straight through |

## Status

This is the foundation, not a distro you can install yet. What exists and is
tested today:

- **`kernel/`** boots through the Xen PVH protocol as a real **Xen dom0**
  (and as a plain PVH guest under QEMU): 32-bit entry stub to long mode, Xen
  detection and hypercall page, console via hypercall or UART, exception
  handling, physical memory map and a frame allocator. It then halts: there
  is no scheduler, userspace or IPC yet.
- **`rbx`** (`tools/rbx`, `crates/rbx-core`) parses a system description
  ([`config/rustbox.toml`](config/rustbox.toml)), enforces the isolation
  policy on it, and renders each qube to a Xen `xl` domain config. It does
  not create domains itself yet.

Not started: the Wayland GUI path, the service qube images, native domain
building from the Rust dom0, an installer. See [docs/ROADMAP.md](docs/ROADMAP.md).

## Try it

Needs a Rust toolchain (`rust-toolchain.toml` pins the target) and
`qemu-system-x86_64`.

```sh
cargo test                        # host-side unit and CLI tests
cargo xtask test-boot             # boot the kernel with QEMU's PVH loader
cargo xtask run                   # same, interactive serial console

# Boot Xen under QEMU with the rustBOX kernel as PVH dom0.
# fetch-xen.sh downloads (does not install) the Debian/Ubuntu Xen package.
cargo xtask test-xen --xen "$(scripts/fetch-xen.sh)"
```

The Xen run ends like this:

```
(XEN) *** Building a PVH Dom0 ***
rustBOX kernel 0.1.0
hypervisor: Xen 4.17
role: dom0 (initial control domain)
traps: IDT live, breakpoint round-trip ok
memory map (XENMEM_memory_map, 11 entries):
  ...
frames: allocator ok (first frame 0x100000)
rustbox: boot ok
```

Working with a system description:

```sh
cargo run -p rbx -- check config/rustbox.toml        # isolation policy
cargo run -p rbx -- plan config/rustbox.toml         # start order
cargo run -p rbx -- render config/rustbox.toml byo   # xl config of one qube
cargo run -p rbx -- xen-cmdline config/rustbox.toml  # Xen options for dom0
cargo run -p rbx -- devices                          # host PCI devices by role
```

## Layout

```
kernel/            the microkernel (x86_64-unknown-none, separate Cargo workspace)
crates/xen-abi/    Xen public ABI: hypercall numbers, PVH start_info, CPUID leaves
crates/kmem/       physical frame bookkeeping (no_std, tested on the host)
crates/rbx-core/   qube model, isolation policy, xl rendering
tools/rbx/         the qube manager CLI
xtask/             build and boot-test driver
config/            reference system topology
docs/              architecture and roadmap
```

## Documentation

- [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md): the design, the isolation
  policy, and how this relates to Xen, Qubes OS, Redox and krata.
- [docs/ROADMAP.md](docs/ROADMAP.md): milestones and known limitations.
