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
        resource.set_icon("icon/ladder.ico");
        resource
            .compile()
            .expect("failed to compile Windows resources");
    }
}
