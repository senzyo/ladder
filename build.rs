fn main() {
    build_dns_recovery();
    println!("cargo:rerun-if-changed=assets/ladder.ico");
    println!("cargo:rerun-if-changed=assets/ladder.manifest");
    // 只给桌面程序嵌入提升清单, 普通权限的开发终端也能运行测试。
    let manifest = std::path::absolute("assets/ladder.manifest").unwrap();
    println!("cargo:rustc-link-arg-bin=ladder=/MANIFEST:EMBED");
    println!("cargo:rustc-link-arg-bin=ladder=/MANIFESTUAC:NO");
    println!(
        "cargo:rustc-link-arg-bin=ladder=/MANIFESTINPUT:{}",
        manifest.display()
    );
    winresource::WindowsResource::new()
        .set_icon("assets/ladder.ico")
        .set("ProductName", "Ladder")
        .set("FileDescription", "Ladder")
        .compile()
        .expect("无法生成 Ladder 的 Windows 图标资源");
}

fn build_dns_recovery() {
    use std::{env, fs, path::PathBuf, process::Command};

    for path in [
        "dns-recovery/Cargo.toml",
        "dns-recovery/Cargo.lock",
        "dns-recovery/src/main.rs",
        "src/dns.rs",
        "src/logging.rs",
        "src/core.rs",
        "src/i18n.rs",
        "src/i18n",
    ] {
        println!("cargo:rerun-if-changed={path}");
    }
    let target = env::var("TARGET").unwrap();
    let out = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    let target_dir = out.join("dns-recovery-target");
    // 独立输出目录避免嵌套 Cargo 争用主项目锁；恢复程序始终按体积优化。
    let status = Command::new(env::var_os("CARGO").unwrap())
        .args([
            "build",
            "--manifest-path",
            "dns-recovery/Cargo.toml",
            "--release",
            "--locked",
            "--target",
            &target,
            "--target-dir",
        ])
        .arg(&target_dir)
        // 主项目的 Clippy wrapper 不能代替这里的二进制构建。
        .env("RUSTC_WORKSPACE_WRAPPER", "")
        .env_remove("CLIPPY_ARGS")
        .status()
        .expect("无法构建 DNS 恢复程序");
    assert!(status.success(), "构建 DNS 恢复程序失败");
    let executable = target_dir
        .join(&target)
        .join("release/ladder-dns-recovery.exe");
    let bytes = fs::read(&executable).expect("无法读取 DNS 恢复程序");
    assert!(bytes.len() < 2 * 1024 * 1024, "DNS 恢复程序超过 2 MiB");
    let pe = u32::from_le_bytes(bytes[0x3c..0x40].try_into().unwrap()) as usize;
    let machine = u16::from_le_bytes(bytes[pe + 4..pe + 6].try_into().unwrap());
    let expected = if target.starts_with("aarch64-") {
        0xaa64
    } else if target.starts_with("x86_64-") {
        0x8664
    } else {
        panic!("不支持的 DNS 恢复程序架构: {target}");
    };
    assert_eq!(machine, expected, "DNS 恢复程序架构不匹配");
    fs::copy(executable, out.join("ladder-dns-recovery.exe")).expect("无法嵌入 DNS 恢复程序");
}
