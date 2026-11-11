//! End-to-end checks of the `rbx` binary.

use std::path::PathBuf;
use std::process::{Command, Output};

fn rbx(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_rbx"))
        .args(args)
        .output()
        .unwrap()
}

fn reference() -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../config/rustbox.toml");
    path.to_str().unwrap().to_string()
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rbx-cli-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn check_accepts_the_reference_system() {
    let out = rbx(&["check", &reference()]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(String::from_utf8_lossy(&out.stdout).contains("ok (9 qubes, 0 warning(s))"));
}

#[test]
fn a_policy_violation_fails_check_and_blocks_rendering() {
    let dir = scratch("violation");
    // Give dom0 a network connection.
    let text = std::fs::read_to_string(reference()).unwrap();
    let bad = text.replacen("role = \"dom0\"", "role = \"dom0\"\nnetvm = \"sys-net\"", 1);
    assert_ne!(text, bad);
    let file = dir.join("bad.toml");
    std::fs::write(&file, bad).unwrap();
    let file = file.to_str().unwrap();

    let out = rbx(&["check", file]);
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).contains("error[dom0-networked]: dom0:"));

    let out = rbx(&["render", file, "work"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(
        out.stdout.is_empty(),
        "nothing may be rendered from an invalid system"
    );

    let out = rbx(&[
        "render",
        file,
        "--all",
        "--out",
        dir.join("cfg").to_str().unwrap(),
    ]);
    assert_eq!(out.status.code(), Some(1));
    assert!(!dir.join("cfg").exists());
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn render_all_writes_one_config_per_qube() {
    let dir = scratch("render");
    let out = rbx(&[
        "render",
        &reference(),
        "--all",
        "--out",
        dir.to_str().unwrap(),
    ]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let mut files: Vec<String> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    files.sort();
    assert_eq!(
        files,
        [
            "byo.cfg",
            "gpu-hvm.cfg",
            "sys-firewall.cfg",
            "sys-gui.cfg",
            "sys-net.cfg",
            "sys-usb.cfg",
            "vault.cfg",
            "work.cfg"
        ]
    );
    let gui = std::fs::read_to_string(dir.join("sys-gui.cfg")).unwrap();
    assert!(gui.contains("\"0000:00:02.0\""));
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn plan_and_xen_cmdline() {
    let out = rbx(&["plan", &reference()]);
    assert!(out.status.success());
    assert!(String::from_utf8_lossy(&out.stdout).starts_with("sys-net\nsys-firewall\n"));

    let out = rbx(&["xen-cmdline", &reference()]);
    assert!(out.status.success());
    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
        "dom0=pvh dom0_mem=1024M,max:1024M dom0_max_vcpus=2 iommu=required smt=off ucode=scan\n"
    );
}

#[test]
fn bad_usage_and_missing_things_fail_cleanly() {
    assert_eq!(rbx(&[]).status.code(), Some(2));
    assert_eq!(rbx(&["frobnicate"]).status.code(), Some(2));
    assert_eq!(
        rbx(&["check", "/nonexistent/system.toml"]).status.code(),
        Some(1)
    );
    assert_eq!(
        rbx(&["render", &reference(), "dom0"]).status.code(),
        Some(1)
    );
    assert_eq!(
        rbx(&["render", &reference(), "no-such-qube"]).status.code(),
        Some(1)
    );
}
