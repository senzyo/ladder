fn main() {
    println!("cargo:rerun-if-changed=assets/ladder.ico");
    println!("cargo:rerun-if-changed=assets/ladder.manifest");
    // 只给桌面程序嵌入提升清单，普通权限的开发终端也能运行测试。
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
