fn main() {
    println!("cargo:rerun-if-changed=macafond.manifest");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows")
        && std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc")
    {
        let root =
            std::env::var("CARGO_MANIFEST_DIR").expect("Cargo supplies the manifest directory");
        println!("cargo:rustc-link-arg=/MANIFEST:EMBED");
        println!("cargo:rustc-link-arg=/MANIFESTINPUT:{root}/macafond.manifest");
        resources(&root);
    }
}

/// The icon and file description Task Manager and Windows volume settings show for the
/// daemon instead of a bare `macafond.exe`.
fn resources(root: &str) {
    let icon = std::path::Path::new(root).join("../macafon-ui/src-tauri/icons/icon.ico");
    println!("cargo:rerun-if-changed={}", icon.display());
    let version = std::env::var("CARGO_PKG_VERSION").expect("Cargo supplies the version");
    let numbers: Vec<u16> = version
        .split(['.', '-', '+'])
        .take(3)
        .map(|part| part.parse().unwrap_or(0))
        .collect();
    let [major, minor, patch] = [0, 1, 2].map(|i| numbers.get(i).copied().unwrap_or(0));
    let icon = icon.display().to_string().replace('\\', "\\\\");
    let rc = format!(
        r#"#pragma code_page(65001)
1 ICON "{icon}"
1 VERSIONINFO
FILEVERSION {major},{minor},{patch},0
PRODUCTVERSION {major},{minor},{patch},0
FILEOS 0x40004
FILETYPE 0x1
BEGIN
  BLOCK "StringFileInfo"
  BEGIN
    BLOCK "040904b0"
    BEGIN
      VALUE "CompanyName", "Macafon"
      VALUE "FileDescription", "Macafon"
      VALUE "FileVersion", "{version}"
      VALUE "InternalName", "macafond"
      VALUE "OriginalFilename", "macafond.exe"
      VALUE "ProductName", "Macafon"
      VALUE "ProductVersion", "{version}"
    END
  END
  BLOCK "VarFileInfo"
  BEGIN
    VALUE "Translation", 0x409, 1200
  END
END
"#
    );
    let out = std::path::PathBuf::from(std::env::var("OUT_DIR").expect("Cargo supplies OUT_DIR"));
    let path = out.join("macafond.rc");
    std::fs::write(&path, rc).expect("write the resource script");
    embed_resource::compile(&path, embed_resource::NONE)
        .manifest_optional()
        .expect("compile the Windows resources");
}
