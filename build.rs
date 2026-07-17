//! 构建脚本。
//!
//! 1. 根据构建模式 (debug/release) 嵌入不同的 Windows 清单:
//!    - debug   → `asInvoker` (不提权, 方便开发调试)
//!    - release → `requireAdministrator` (TUN 接口和 DNS 清理需要管理员权限)
//! 2. 将 `icons/` 和 `configs/` 目录复制到输出目录, 使可执行文件运行时能
//!    通过相对路径找到图标和配置文件。

fn main() {
    if cfg!(target_os = "windows") {
        let manifest = if cfg!(debug_assertions) {
            r#"
<assembly xmlns="urn:schemas-microsoft-com:asm.v1" manifestVersion="1.0">
  <trustInfo xmlns="urn:schemas-microsoft-com:asm.v3">
    <security>
      <requestedPrivileges>
        <requestedExecutionLevel level="asInvoker" uiAccess="false" />
      </requestedPrivileges>
    </security>
  </trustInfo>
</assembly>
"#
        } else {
            r#"
<assembly xmlns="urn:schemas-microsoft-com:asm.v1" manifestVersion="1.0">
  <trustInfo xmlns="urn:schemas-microsoft-com:asm.v3">
    <security>
      <requestedPrivileges>
        <requestedExecutionLevel level="requireAdministrator" uiAccess="false" />
      </requestedPrivileges>
    </security>
  </trustInfo>
</assembly>
"#
        };

        let mut resource = winresource::WindowsResource::new();
        resource.set_manifest(manifest);
        resource.set_icon("icons/ladder.ico");
        resource.compile().expect("failed to compile Windows resources");

        // OUT_DIR 形如 target/debug/build/<crate>/out, 向上三级到达 profile 目录
        let out_dir = std::env::var("OUT_DIR").unwrap();
        let profile_dir = std::path::Path::new(&out_dir)
            .parent()
            .and_then(|p| p.parent())
            .and_then(|p| p.parent())
            .expect("failed to resolve target profile dir");

        // 将图标、配置、settings.json 复制到输出目录, 使可执行文件能通过相对路径访问
        copy_dir(std::path::Path::new("icons"), &profile_dir.join("icons"));
        copy_dir(std::path::Path::new("configs"), &profile_dir.join("configs"));
        let _ = std::fs::copy("settings.json", profile_dir.join("settings.json"));
        let _ = std::fs::copy("README.md", profile_dir.join("README.md"));
        let _ = std::fs::copy("LICENSE", profile_dir.join("LICENSE"));
    }
}

/// 递归复制目录, 目标目录不存在时自动创建。
fn copy_dir(src: &std::path::Path, dest: &std::path::Path) {
    let _ = std::fs::create_dir_all(dest);
    let Ok(entries) = std::fs::read_dir(src) else { return };
    for entry in entries.flatten() {
        let src_path = entry.path();
        let dest_path = dest.join(entry.file_name());
        if src_path.is_dir() {
            copy_dir(&src_path, &dest_path);
        } else {
            let _ = std::fs::copy(&src_path, &dest_path);
        }
    }
}
