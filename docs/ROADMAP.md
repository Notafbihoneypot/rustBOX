# Roadmap

## M0: foundation (done)

- [x] Kernel boots via Xen PVH as dom0 and under QEMU's PVH loader
- [x] Xen detection, hypercall page, `console_io`, `xen_version`,
      `XENMEM_memory_map`
- [x] UART console fallback
- [x] IDT and exception handling
- [x] Memory map parsing and a physical frame allocator
- [x] `cargo xtask test-boot` / `test-xen` boot tests
- [x] Bootable ISO (GRUB, Xen, kernel as dom0) and `cargo xtask test-iso`
- [x] System description, isolation policy, `xl` rendering, start order
- [x] `rbx` CLI and a reference topology

## M1: a real kernel core

- [ ] Own GDT and TSS, with an interrupt stack for double faults
- [ ] Proper page tables: higher-half kernel, W^X, NX, guard pages (the boot
      stub's identity map is one writable+executable 4 GiB mapping)
- [ ] Kernel heap
- [ ] Time and interrupts on Xen: event channels, the shared info page,
      the PV clock
- [ ] PV console ring, so the kernel also has a console as an unprivileged
      domU on release builds of Xen
- [ ] SMP

## M2: the microkernel

- [ ] Address spaces and threads, a scheduler
- [ ] Capability-based IPC and the syscall ABI
- [ ] ELF loader; start an `init` server from a boot module
- [ ] Userspace runtime crate for servers

## M3: controlling Xen from Rust

- [ ] Privileged hypercalls (domctl, memory, event channel and grant table
      operations) behind a safe API
- [ ] xenstore server
- [ ] PVH domain builder: start a guest from dom0 without `xl`
- [ ] Decide what to reuse from krata (API fit, licence)

## M4: bootstrap distro (in parallel with M1 to M3)

- [ ] Image build: Xen + minimal Linux dom0 + `rbx`
- [ ] `rbx` applies a system description (create, start, stop qubes via `xl`)
- [ ] Service qube images: `sys-net`, `sys-firewall`, `sys-usb`
- [ ] BYO-ISO HVM end to end on real hardware
- [ ] GPU passthrough to a standalone HVM on real hardware
- [ ] Device discovery writes the PCI section of the system description

## M5: GUI

- [ ] `sys-gui` image with a Wayland compositor on the passed-through GPU
- [ ] Window forwarding from app qubes over grant-shared buffers
- [ ] Label-coloured decorations drawn by the compositor
- [ ] Input routing from `sys-usb` to `sys-gui`
- [ ] Inter-qube RPC with policy: clipboard, file copy, device attach

## M6: the swap

- [ ] rustBOX kernel replaces Linux as dom0 on real hardware
- [ ] Storage qube; service qubes started from boot modules
- [ ] Installer

## Known limitations of what is in the tree

- The kernel halts after its self-checks. It has no scheduler, no userspace
  and no IPC: it is not yet a microkernel in the functional sense.
- Only memory below 4 GiB is used, and it is mapped writable and executable.
- As a domU on a release build of Xen the kernel has no console (see M1).
- The Xen boot test runs under QEMU's software emulation with shadow paging
  and an emulated IOMMU. Nothing has been run on real hardware.
- `rbx render` output has been checked against the documented `xl.cfg`
  syntax by tests on the generated text, not by starting domains from it.
- GPU passthrough is modelled in the system description and rendered; it
  has not been exercised.
