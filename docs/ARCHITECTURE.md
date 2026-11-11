# rustBOX architecture

## Goal

A desktop OS where a compromise of any one component (a browser, a USB
stack, a GPU driver, a network card's firmware) stays inside one qube, and
where the code that could break that containment is as small as possible and
written in Rust.

```
+--------+ +---------+ +--------------+ +---------+ +---------+ +-------+ +----------+
|  dom0  | | sys-net | | sys-firewall | | sys-usb | | sys-gui | | work  | | byo HVM  |
|        | |         | |              | |         | | Wayland | | vault | | gpu-hvm  |
| rustBOX| | NICs    | |              | | xHCI    | | GPU     | | ...   | | (own GPU)|
| kernel | |         | |              | |         | |         | |       | |          |
+--------+ +---------+ +--------------+ +---------+ +---------+ +-------+ +----------+
+------------------------------------------------------------------------------------+
|                        Xen  (CPU, memory, IOMMU, interrupts)                       |
+------------------------------------------------------------------------------------+
```

## The layers

### Xen

rustBOX uses upstream Xen unchanged. Xen already is, in effect, a
microkernel: it handles CPUs, memory, interrupts and the IOMMU and leaves
every device to a domain. Rewriting it is not a goal. What rustBOX replaces
is everything Xen normally trusts above it.

### dom0: a Rust microkernel

On a conventional Xen system dom0 is a full Linux with every driver, the
toolstack, and often the desktop. All of it can take over every VM.

rustBOX's dom0 is a PVH domain running the kernel in `kernel/`. The design
that makes this tractable: **dom0 owns no hardware**, so it needs no hardware
drivers. It needs to speak only Xen's own interfaces (hypercalls, event
channels, grant tables, xenstore). Its job is to build qubes, connect them
according to policy, and supervise them.

Planned structure, Redox-style, with the kernel providing only address
spaces, threads, capability-checked IPC and interrupt/event delivery:

- **xenstore server**: the configuration bus qubes use to find their devices
- **domain builder**: loads a kernel image into a new domain and starts it
- **qube manager**: applies the system description (the model in `rbx-core`)
- **policy/RPC broker**: the only way one qube can ask another for anything

None of these exist as kernel-hosted services yet; see the roadmap.

### Service qubes

Each class of hardware is passed through (IOMMU-protected PCI passthrough)
to one qube whose only job is to drive it:

- **`sys-net`** owns the NICs. **`sys-firewall`** sits between it and
  everything else, so a compromised network driver sees only filtered
  traffic it already carried.
- **`sys-usb`** owns the USB controllers. USB devices are exposed to other
  qubes one at a time, by policy.
- **`sys-gui`** owns the primary GPU and runs the Wayland compositor. It has
  no network. Application qubes do not get GPU access; they hand window
  contents to `sys-gui`, which draws them with a border in the qube's label
  colour. That border is the user's trustworthy cue for where a window
  comes from, which is why the policy refuses any windowed qube without a
  label.

Service qubes need real device drivers, so they will run Linux for the
foreseeable future. "Rust-based" describes the trusted core (dom0 kernel and
toolstack), not every guest.

### Application qubes and HVMs

- **App qubes** are PVH: hardware-virtualised, no emulated devices, the
  smallest attack surface Xen offers.
- **Standalone HVMs** run unmodified systems from an ISO you supply. Their
  emulated hardware (QEMU) runs in a per-VM stub domain, not in dom0.
- An HVM may be given a **second GPU** by passthrough (`gpu-hvm` in the
  reference system). It then drives its own display and bypasses `sys-gui`.

## The system description and the isolation policy

A system is one TOML file; [`config/rustbox.toml`](../config/rustbox.toml) is
the reference. `rbx check` enforces the rules below, and nothing can be
rendered from a description that breaks one (`rbx_core::ValidSystem` is the
only input rendering accepts).

| Rule | Code |
|---|---|
| Exactly one dom0, named `dom0` | `no-dom0`, `multiple-dom0`, `dom0-name` |
| dom0 has no network | `dom0-networked` |
| dom0 owns no PCI device | `dom0-owns-device` |
| A PCI function has exactly one owner | `device-shared` |
| NICs only in net qubes, USB controllers only in usb qubes, GPUs and audio only in gui or standalone qubes | `device-role-mismatch` |
| usb and gui qubes have no network | `isolated-role-networked` |
| A netvm exists, is a net or firewall qube, and the chain ends at a net qube | `unknown-netvm`, `netvm-not-a-provider`, `netvm-cycle`, `net-qube-has-netvm` |
| PCI passthrough and ISO boot use HVM; every qube has something to boot | `pci-needs-hvm`, `iso-needs-hvm`, `no-boot-source` |
| Every windowed qube has a label and a real gui qube | `missing-label`, `unknown-guivm`, `guivm-not-gui` |
| Names, paths and command lines use restricted alphabets, so they cannot inject into a generated config | `bad-name`, `unsafe-path`, `unsafe-cmdline` |

Warnings (allowed, but reported): a net qube without a NIC, a usb qube
without a controller, a gui qube without a GPU, a firewall without an uplink.

`rbx xen-cmdline` produces the hypervisor options that go with the
description: `dom0=pvh`, a fixed dom0 size, `iommu=required` (refuse to boot
without DMA protection rather than run passthrough unprotected) and
`smt=off`.

## How the kernel boots

Xen's PVH domain builder (and QEMU's `-kernel` loader, which follows the
same protocol) finds the `XEN_ELFNOTE_PHYS32_ENTRY` note in the ELF image and
jumps to it in 32-bit protected mode with paging off and `ebx` pointing at
the `hvm_start_info` structure.

1. `kernel/src/boot.rs`: identity-map the first 4 GiB, enable long mode,
   call `kmain`.
2. `xen.rs`: find Xen's CPUID leaves, install the hypercall page through the
   MSR Xen advertises. From here the console is `HYPERVISOR_console_io`
   (dom0) or the UART (no hypervisor).
3. `traps.rs`: load an IDT for the 32 CPU exceptions and prove the path with
   a breakpoint round-trip.
4. `mm.rs`: read the memory map (from `start_info`, or `XENMEM_memory_map`
   when the builder supplies none, as Xen's dom0 builder does), reserve the
   kernel image and all boot structures, and self-test the frame allocator.

`cargo xtask test-xen` runs exactly this against a real Xen binary under
QEMU. QEMU's software emulation provides AMD SVM, which is enough for Xen to
build a PVH dom0; nested paging is disabled there (Xen falls back to shadow
paging) and an emulated AMD-Vi IOMMU is attached because Xen requires one
for a PVH dom0.

## Bootstrap path versus target

Running the reference topology on real hardware needs things the Rust dom0
cannot do yet. So there are two tracks:

| | Bootstrap (to get a usable system) | Target |
|---|---|---|
| dom0 kernel | minimal Linux | rustBOX microkernel |
| Domain creation | `xl`, fed by `rbx render` | native Rust domain builder |
| Policy source | `rbx-core` | `rbx-core` (same model) |

The system description and its policy are shared, so work on the topology,
the service qubes and the GUI path carries over unchanged when dom0 is
swapped.

## Open design questions

- **Storage.** Something must own the disk controller. The intended answer
  is a storage qube, with the service qube images handed to dom0 as boot
  modules so dom0 never needs a disk driver; this is not designed in detail.
- **PCI passthrough from a PVH dom0.** Xen has historically relied on
  Linux's `pciback` in a PV dom0. Support for assigning devices from a PVH
  dom0 was still being developed upstream in the patch series we found
  (2024); which Xen release is the minimum for the target design has to be
  established before the dom0 swap.
- **Built-in input devices.** A laptop's PS/2 keyboard is not a PCI device
  and cannot simply be passed to `sys-gui`.
- **The cross-qube window protocol.** Wayland buffers shared through Xen
  grant tables is the plan; the protocol itself is not specified.
- **Grow this kernel or port Redox's.** The boot and Xen layers here would
  be needed either way.

## Related projects

- [Xen Project](https://xenproject.org/): the hypervisor.
- [Qubes OS](https://www.qubes-os.org/): the architecture rustBOX follows:
  service qubes, GUI domain, labels, disaggregated dom0.
- [Redox OS](https://www.redox-os.org/): a Rust microkernel OS with drivers
  and services in userspace; the model for dom0's internals.
- [krata](https://github.com/edera-dev/krata) (Edera): a Xen control plane
  written in Rust, with crates for privcmd hypercalls, xenstore, event
  channels and grant tables. The closest existing code to the native
  toolstack on the roadmap. It targets a Linux dom0, and at least part of it
  is GPL-2.0-or-later, which matters for reuse.
- [Smithay](https://github.com/Smithay/smithay): Rust building blocks for
  Wayland compositors; the candidate base for `sys-gui`.
