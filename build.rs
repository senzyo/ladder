fn main() {
    println!("cargo:rerun-if-changed=assets/ladder.ico");
    winresource::WindowsResource::new()
        .set_icon("assets/ladder.ico")
        .set("ProductName", "Ladder")
        .set("FileDescription", "Ladder")
        .compile()
        .expect("无法生成 Ladder 的 Windows 图标资源");
}
