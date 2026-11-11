//! `rbx`: the rustBOX qube manager.
//!
//! Today it checks a system description against the isolation policy and
//! renders it for Xen's `xl`. It is the seed of the dom0 toolstack: the same
//! checked model will drive domain creation directly once the native Xen
//! backend exists.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use rbx_core::pci::{self, DeviceClass};
use rbx_core::{plan, xl, SystemSpec, ValidSystem};

const USAGE: &str = "\
rbx - rustBOX qube manager

usage:
  rbx check <system.toml>                    check the isolation policy
  rbx plan <system.toml>                     print the qube start order
  rbx render <system.toml> <qube>            print one qube's xl config
  rbx render <system.toml> --all --out <dir> write <dir>/<qube>.cfg for every qube
  rbx xen-cmdline <system.toml>              print the Xen command line for dom0
  rbx devices [--sysfs <dir>]                list host PCI devices and who may own them
";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let result = match args.as_slice() {
        ["check", file] => check(Path::new(file)),
        ["plan", file] => with_system(Path::new(file), |system| {
            for q in plan::start_order(system) {
                println!("{}", q.name);
            }
            Ok(())
        }),
        ["render", file, "--all", "--out", dir] | ["render", file, "--out", dir, "--all"] => {
            with_system(Path::new(file), |system| render_all(system, Path::new(dir)))
        }
        ["render", file, qube] if !qube.starts_with('-') => {
            with_system(Path::new(file), |system| {
                let cfg = xl::render(system, qube).ok_or_else(|| {
                    format!("no qube named `{qube}` to render (dom0 has no domain config)")
                })?;
                print!("{cfg}");
                Ok(())
            })
        }
        ["xen-cmdline", file] => with_system(Path::new(file), |system| {
            println!("{}", xl::xen_cmdline(system));
            Ok(())
        }),
        ["devices"] => devices(Path::new("/sys")),
        ["devices", "--sysfs", dir] => devices(Path::new(dir)),
        ["help" | "--help" | "-h"] => {
            print!("{USAGE}");
            Ok(())
        }
        _ => {
            eprint!("{USAGE}");
            return ExitCode::from(2);
        }
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            if !message.is_empty() {
                eprintln!("rbx: {message}");
            }
            ExitCode::FAILURE
        }
    }
}

fn load(file: &Path) -> Result<SystemSpec, String> {
    let text = std::fs::read_to_string(file).map_err(|e| format!("{}: {e}", file.display()))?;
    SystemSpec::parse(&text).map_err(|e| format!("{}: {e}", file.display()))
}

/// Prints every issue; fails if any is an error.
fn check(file: &Path) -> Result<(), String> {
    let spec = load(file)?;
    let report = rbx_core::check(&spec);
    for issue in &report.issues {
        eprintln!("{issue}");
    }
    let (errors, warnings) = (report.errors().count(), report.warnings().count());
    if errors > 0 {
        return Err(format!(
            "{}: {errors} error(s), {warnings} warning(s)",
            file.display()
        ));
    }
    println!(
        "{}: ok ({} qubes, {warnings} warning(s))",
        file.display(),
        spec.qubes.len()
    );
    Ok(())
}

/// Loads and validates `file`, then runs `f`. Policy errors are printed and
/// nothing is produced; warnings are printed and do not stop anything.
fn with_system(
    file: &Path,
    f: impl FnOnce(&ValidSystem) -> Result<(), String>,
) -> Result<(), String> {
    let spec = load(file)?;
    match ValidSystem::new(&spec) {
        Ok((system, warnings)) => {
            for issue in &warnings.issues {
                eprintln!("{issue}");
            }
            f(&system)
        }
        Err(report) => {
            for issue in &report.issues {
                eprintln!("{issue}");
            }
            Err(format!(
                "{}: refusing to continue with {} policy error(s)",
                file.display(),
                report.errors().count()
            ))
        }
    }
}

fn render_all(system: &ValidSystem, dir: &Path) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    for q in plan::start_order(system) {
        let cfg = xl::render(system, &q.name).expect("start order only lists renderable qubes");
        let path: PathBuf = dir.join(format!("{}.cfg", q.name));
        std::fs::write(&path, cfg).map_err(|e| format!("{}: {e}", path.display()))?;
        println!("{}", path.display());
    }
    Ok(())
}

fn devices(sysfs: &Path) -> Result<(), String> {
    let devices = pci::scan_sysfs(sysfs).map_err(|e| format!("{}: {e}", sysfs.display()))?;
    println!(
        "{:<14}{:<11}{:<10}{:<7}may be owned by",
        "ADDRESS", "ID", "CLASS", "KIND"
    );
    for d in devices {
        let class = d.class();
        let owner = match class {
            DeviceClass::Other => "- (standalone only, if you really mean it)",
            other => other.allowed_roles(),
        };
        println!(
            "{:<14}{:04x}:{:04x}  0x{:06x}  {:<7}{owner}",
            d.address.to_string(),
            d.vendor,
            d.device,
            d.class_code,
            class.as_str()
        );
    }
    Ok(())
}
