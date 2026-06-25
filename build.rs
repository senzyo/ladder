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
        let icons_dest = profile_dir.join("icons");
        let _ = std::fs::create_dir_all(&icons_dest);
        let _ = std::fs::copy("icons/ladder.ico", icons_dest.join("ladder.ico"));
    }
}
