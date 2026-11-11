//! `cargo xtask`: builds the kernel and boots it under QEMU.
//!
//! Std-only on purpose: it has to work on a fresh checkout with nothing but
//! a Rust toolchain and QEMU.

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

const USAGE: &str = "\
usage: cargo xtask <command> [options]

commands:
  build                 build the kernel (x86_64-unknown-none)
  run                   boot the kernel with QEMU's PVH loader, serial on stdio
  test-boot             boot with QEMU's PVH loader and check the boot log
  test-xen              boot Xen under QEMU with the kernel as PVH dom0 and
                        check the boot log
  iso                   build out/rustbox.iso: GRUB -> Xen -> kernel as dom0,
                        bootable in any VM (needs grub-mkrescue, xorriso, mtools)
  test-iso              build the ISO, boot it under QEMU, check the boot log
  run-iso               build the ISO and boot it in a QEMU window

options:
  --release             use the release profile
  --xen <path>          Xen multiboot image (default: $RBX_XEN, see
                        scripts/fetch-xen.sh); test-xen needs it uncompressed
  --timeout <seconds>   boot test timeout (default 60; test-xen, test-iso 240)
";

/// Must match `BOOT_OK` in kernel/src/main.rs.
const BOOT_OK: &str = "rustbox: boot ok";
const FAILURE_MARKERS: &[&str] = &["KERNEL PANIC", "FATAL TRAP", "Panic on CPU"];

struct Opts {
    release: bool,
    xen: Option<PathBuf>,
    timeout: Option<u64>,
}

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let Some(cmd) = args.next() else {
        eprint!("{USAGE}");
        return ExitCode::from(2);
    };
    let mut opts = Opts {
        release: false,
        xen: std::env::var_os("RBX_XEN").map(PathBuf::from),
        timeout: None,
    };
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--release" => opts.release = true,
            "--xen" => opts.xen = args.next().map(PathBuf::from),
            "--timeout" => opts.timeout = args.next().and_then(|s| s.parse().ok()),
            other => {
                eprintln!("xtask: unknown option `{other}`\n\n{USAGE}");
                return ExitCode::from(2);
            }
        }
    }

    let result = match cmd.as_str() {
        "build" => build_kernel(opts.release).map(|k| println!("{}", k.display())),
        "run" => build_kernel(opts.release).and_then(|k| run_interactive(&k)),
        "test-boot" => build_kernel(opts.release).and_then(|k| test_boot(&k, &opts)),
        "test-xen" => build_kernel(opts.release).and_then(|k| test_xen(&k, &opts)),
        "iso" => build_kernel(opts.release)
            .and_then(|k| build_iso(&k, &opts))
            .map(|iso| println!("{}", iso.display())),
        "test-iso" => build_kernel(opts.release)
            .and_then(|k| build_iso(&k, &opts))
            .and_then(|iso| test_iso(&iso, &opts)),
        "run-iso" => build_kernel(opts.release)
            .and_then(|k| build_iso(&k, &opts))
            .and_then(|iso| run_iso(&iso)),
        "help" | "--help" | "-h" => {
            print!("{USAGE}");
            Ok(())
        }
        other => Err(format!("unknown command `{other}`\n\n{USAGE}")),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("xtask: {e}");
            ExitCode::FAILURE
        }
    }
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask lives in the repo root")
        .to_path_buf()
}

/// Builds the kernel and returns the path of the ELF image.
fn build_kernel(release: bool) -> Result<PathBuf, String> {
    // Run from kernel/ so its .cargo/config.toml (target, rustflags) applies.
    let kernel_dir = repo_root().join("kernel");
    let mut cmd = Command::new(std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into()));
    cmd.current_dir(&kernel_dir).arg("build");
    // Flags meant for the host build of xtask must not leak into the
    // freestanding kernel build.
    cmd.env_remove("RUSTFLAGS")
        .env_remove("CARGO_ENCODED_RUSTFLAGS");
    if release {
        cmd.arg("--release");
    }
    let status = cmd.status().map_err(|e| format!("cannot run cargo: {e}"))?;
    if !status.success() {
        return Err("kernel build failed".into());
    }
    let profile = if release { "release" } else { "debug" };
    Ok(kernel_dir
        .join("target/x86_64-unknown-none")
        .join(profile)
        .join("rustbox-kernel"))
}

fn qemu() -> Command {
    let mut cmd =
        Command::new(std::env::var_os("QEMU").unwrap_or_else(|| "qemu-system-x86_64".into()));
    cmd.args(["-display", "none", "-no-reboot", "-serial", "stdio"]);
    cmd
}

fn run_interactive(kernel: &Path) -> Result<(), String> {
    let status = qemu()
        .args(["-m", "256M", "-kernel"])
        .arg(kernel)
        .status()
        .map_err(|e| format!("cannot run qemu: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("qemu exited with {status}"))
    }
}

fn test_boot(kernel: &Path, opts: &Opts) -> Result<(), String> {
    let mut cmd = qemu();
    cmd.args(["-m", "256M", "-append", "rbx.test=pvh-direct", "-kernel"])
        .arg(kernel);
    let log = boot_and_wait(cmd, Duration::from_secs(opts.timeout.unwrap_or(60)))?;
    expect(
        &log,
        &[
            "hypervisor: none detected",
            "cmdline: rbx.test=pvh-direct",
            "traps: IDT live",
            "frames: allocator ok",
        ],
    )?;
    println!("test-boot: ok");
    Ok(())
}

/// Xen options shared by every way we boot it: a PVH dom0, everything
/// logged to the serial port, and no reboot when dom0 stops so the log stays
/// readable.
const XEN_CMDLINE: &str =
    "dom0=pvh dom0_mem=256M com1=115200,8n1 loglvl=all guest_loglvl=all noreboot";

const XEN_BOOT_LOG: &[&str] = &[
    "Building a PVH Dom0",
    "hypervisor: Xen ",
    "role: dom0",
    "memory map (XENMEM_memory_map",
    "traps: IDT live",
    "frames: allocator ok",
];

fn xen_image(opts: &Opts) -> Result<&Path, String> {
    let xen = opts
        .xen
        .as_deref()
        .ok_or("no Xen image: pass --xen <path> or set RBX_XEN (scripts/fetch-xen.sh gets one)")?;
    if !xen.is_file() {
        return Err(format!("Xen image {} does not exist", xen.display()));
    }
    Ok(xen)
}

/// The virtual machine Xen needs when QEMU emulates the CPU in software.
///
/// TCG can emulate AMD SVM, which Xen needs for a PVH dom0. Nested paging is
/// switched off because QEMU's TCG NPT emulation does not hold up under Xen;
/// Xen then uses shadow paging. A PVH dom0 also requires an IOMMU, hence the
/// emulated AMD-Vi on q35.
fn xen_machine(cmd: &mut Command) {
    cmd.args([
        "-machine",
        "q35",
        "-cpu",
        "max,-npt",
        "-m",
        "1G",
        "-device",
        "amd-iommu",
    ]);
}

fn test_xen(kernel: &Path, opts: &Opts) -> Result<(), String> {
    let xen = xen_image(opts)?;
    if xen.extension().is_some_and(|e| e == "gz") {
        return Err(format!(
            "{} is compressed; QEMU's multiboot loader needs it gunzipped",
            xen.display()
        ));
    }
    let kernel = kernel.to_str().ok_or("kernel path is not UTF-8")?;
    if kernel.contains([' ', ',']) {
        return Err("kernel path must not contain spaces or commas (QEMU module syntax)".into());
    }

    let mut cmd = qemu();
    xen_machine(&mut cmd);
    cmd.arg("-kernel").arg(xen);
    cmd.arg("-append")
        .arg(format!("{XEN_CMDLINE} console=com1"));
    // Multiboot module 0 is the dom0 kernel; text after the path is its
    // command line.
    cmd.arg("-initrd")
        .arg(format!("{kernel} rbx.test=xen-dom0"));

    let log = boot_and_wait(cmd, Duration::from_secs(opts.timeout.unwrap_or(240)))?;
    expect(&log, XEN_BOOT_LOG)?;
    expect(&log, &["cmdline: rbx.test=xen-dom0"])?;
    println!("test-xen: ok");
    Ok(())
}

/// Builds `out/rustbox.iso`: GRUB loads Xen, which starts the kernel as
/// dom0. Unlike `test-xen` this goes through a real boot loader, so the
/// image also boots outside QEMU.
fn build_iso(kernel: &Path, opts: &Opts) -> Result<PathBuf, String> {
    let xen = xen_image(opts)?;
    let out = repo_root().join("out");
    let staging = out.join("iso-root");
    let boot = staging.join("boot");
    let io = |what: &str, e: std::io::Error| format!("{what}: {e}");

    // Start from an empty tree so nothing stale ends up in the image.
    if staging.exists() {
        std::fs::remove_dir_all(&staging).map_err(|e| io("clearing out/iso-root", e))?;
    }
    std::fs::create_dir_all(boot.join("grub")).map_err(|e| io("creating out/iso-root", e))?;
    // GRUB decompresses gzip transparently, so either form of Xen works.
    std::fs::copy(xen, boot.join("xen")).map_err(|e| io("copying Xen", e))?;
    std::fs::copy(kernel, boot.join("rustbox-kernel")).map_err(|e| io("copying the kernel", e))?;
    std::fs::write(boot.join("grub/grub.cfg"), grub_cfg())
        .map_err(|e| io("writing grub.cfg", e))?;

    let iso = out.join("rustbox.iso");
    let output = Command::new("grub-mkrescue")
        .arg("-o")
        .arg(&iso)
        .arg(&staging)
        .output()
        .map_err(|e| format!("cannot run grub-mkrescue: {e} (install grub, xorriso and mtools)"))?;
    // grub-mkrescue can exit 0 without writing anything when xorriso or
    // mtools is missing, so check for the image itself.
    if !output.status.success() || !iso.is_file() {
        return Err(format!(
            "grub-mkrescue failed (are xorriso and mtools installed?):\n{}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    Ok(iso)
}

/// Xen logs to the screen and to COM1. GRUB hands Xen a framebuffer;
/// `vga=current,keep` makes Xen draw its console there and keep doing so
/// after dom0 starts. That matters because the kernel has no display driver:
/// its messages reach the screen through Xen's console. (Asking Xen for VGA
/// text mode instead leaves the screen blank when booted from GRUB.)
fn grub_cfg() -> String {
    format!(
        "\
set timeout=3
set default=0

if serial --unit=0 --speed=115200; then
    terminal_input console serial
    terminal_output console serial
fi

menuentry \"rustBOX (Xen + rustBOX kernel as PVH dom0)\" {{
    multiboot2 /boot/xen {XEN_CMDLINE} console=vga,com1 vga=current,keep
    module2 /boot/rustbox-kernel rbx.boot=iso
}}
"
    )
}

fn test_iso(iso: &Path, opts: &Opts) -> Result<(), String> {
    let mut cmd = qemu();
    xen_machine(&mut cmd);
    cmd.arg("-cdrom").arg(iso);
    let log = boot_and_wait(cmd, Duration::from_secs(opts.timeout.unwrap_or(240)))?;
    expect(&log, XEN_BOOT_LOG)?;
    expect(&log, &["cmdline: rbx.boot=iso"])?;
    println!("test-iso: ok ({})", iso.display());
    Ok(())
}

/// Boots the ISO in a QEMU window (Xen's console is on the emulated screen
/// and mirrored to this terminal). Close the window to stop.
fn run_iso(iso: &Path) -> Result<(), String> {
    let mut cmd =
        Command::new(std::env::var_os("QEMU").unwrap_or_else(|| "qemu-system-x86_64".into()));
    cmd.args(["-no-reboot", "-serial", "stdio"]);
    xen_machine(&mut cmd);
    cmd.arg("-cdrom").arg(iso);
    let status = cmd.status().map_err(|e| format!("cannot run qemu: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("qemu exited with {status}"))
    }
}

/// Runs QEMU, echoing its serial output, until the kernel reports success,
/// something reports failure, QEMU exits, or the timeout expires. Returns
/// the captured log on success.
fn boot_and_wait(mut cmd: Command, timeout: Duration) -> Result<String, String> {
    let mut child = cmd
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .map_err(|e| format!("cannot run qemu: {e} (is qemu-system-x86_64 installed?)"))?;
    let stdout = child.stdout.take().expect("stdout was piped");
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut reader = BufReader::new(stdout);
        let mut raw = Vec::new();
        // Serial output is not guaranteed to be UTF-8.
        while matches!(reader.read_until(b'\n', &mut raw), Ok(n) if n > 0) {
            let line = String::from_utf8_lossy(&raw).trim_end().to_string();
            raw.clear();
            if tx.send(line).is_err() {
                break;
            }
        }
    });

    let deadline = Instant::now() + timeout;
    let mut log = String::new();
    let outcome = loop {
        let left = deadline.saturating_duration_since(Instant::now());
        match rx.recv_timeout(left) {
            Ok(line) => {
                println!("{line}");
                log.push_str(&line);
                log.push('\n');
                if line.contains(BOOT_OK) {
                    break Ok(());
                }
                if let Some(marker) = FAILURE_MARKERS.iter().find(|m| line.contains(**m)) {
                    break Err(format!("boot failed: saw `{marker}`"));
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                break Err(format!("no `{BOOT_OK}` within {}s", timeout.as_secs()));
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                break Err(format!("qemu exited before `{BOOT_OK}`"));
            }
        }
    };
    let _ = child.kill();
    let _ = child.wait();
    outcome.map(|()| log)
}

fn expect(log: &str, needles: &[&str]) -> Result<(), String> {
    match needles.iter().find(|n| !log.contains(**n)) {
        Some(missing) => Err(format!("boot log is missing `{missing}`")),
        None => Ok(()),
    }
}
