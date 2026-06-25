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
        resource
            .compile()
            .expect("failed to compile Windows resources");

        let out_dir = std::env::var("OUT_DIR").unwrap();
        let profile_dir = std::path::Path::new(&out_dir)
            .parent().and_then(|p| p.parent()).and_then(|p| p.parent())
            .expect("failed to resolve target profile dir");

        copy_dir(std::path::Path::new("icons"), &profile_dir.join("icons"));
        copy_dir(std::path::Path::new("configs"), &profile_dir.join("configs"));
    }
}

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
