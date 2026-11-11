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

options:
  --release             use the release profile
  --xen <path>          uncompressed Xen multiboot image for test-xen
                        (default: $RBX_XEN, see scripts/fetch-xen.sh)
  --timeout <seconds>   boot test timeout (default 60, test-xen 180)
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

fn test_xen(kernel: &Path, opts: &Opts) -> Result<(), String> {
    let xen = opts
        .xen
        .as_deref()
        .ok_or("no Xen image: pass --xen <path> or set RBX_XEN (scripts/fetch-xen.sh gets one)")?;
    if !xen.is_file() {
        return Err(format!("Xen image {} does not exist", xen.display()));
    }
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
    // TCG can emulate AMD SVM, which Xen needs for a PVH dom0. Nested paging
    // is switched off because QEMU's TCG NPT emulation does not hold up
    // under Xen; Xen then uses shadow paging. A PVH dom0 also requires an
    // IOMMU, hence the emulated AMD-Vi on q35.
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
    cmd.arg("-kernel").arg(xen);
    cmd.args([
        "-append",
        "dom0=pvh dom0_mem=256M console=com1 com1=115200,8n1 loglvl=all guest_loglvl=all noreboot",
    ]);
    // Multiboot module 0 is the dom0 kernel; text after the path is its
    // command line.
    cmd.arg("-initrd")
        .arg(format!("{kernel} rbx.test=xen-dom0"));

    let log = boot_and_wait(cmd, Duration::from_secs(opts.timeout.unwrap_or(180)))?;
    expect(
        &log,
        &[
            "Building a PVH Dom0",
            "hypervisor: Xen ",
            "role: dom0",
            "cmdline: rbx.test=xen-dom0",
            "memory map (XENMEM_memory_map",
            "traps: IDT live",
            "frames: allocator ok",
        ],
    )?;
    println!("test-xen: ok");
    Ok(())
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
