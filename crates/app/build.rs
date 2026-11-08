use std::path::{PathBuf, MAIN_SEPARATOR};

const APP_NAME: &str = "Lan Kvm";
const AUTHOR: &str = "Gabriel Griffo";

fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let icon = manifest.join("../../branding/lan-kvm.ico");
    println!("cargo:rerun-if-changed={}", icon.display());
    let rc = PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("lan-kvm.rc");
    let path = icon.display().to_string().replace(MAIN_SEPARATOR, "/");
    let version = std::env::var("CARGO_PKG_VERSION").unwrap();
    let number = |key: &str| std::env::var(key).unwrap_or_else(|_| "0".to_string());
    let numeric = format!(
        "{},{},{},0",
        number("CARGO_PKG_VERSION_MAJOR"),
        number("CARGO_PKG_VERSION_MINOR"),
        number("CARGO_PKG_VERSION_PATCH")
    );
    let resource = format!(
        r#"1 ICON "{path}"
1 VERSIONINFO
FILEVERSION {numeric}
PRODUCTVERSION {numeric}
FILEOS 0x40004
FILETYPE 0x1
BEGIN
  BLOCK "StringFileInfo"
  BEGIN
    BLOCK "041604B0"
    BEGIN
      VALUE "CompanyName", "{AUTHOR}"
      VALUE "FileDescription", "{APP_NAME}"
      VALUE "FileVersion", "{version}"
      VALUE "InternalName", "{APP_NAME}"
      VALUE "LegalCopyright", "Copyright (C) {AUTHOR}"
      VALUE "OriginalFilename", "{APP_NAME}.exe"
      VALUE "ProductName", "{APP_NAME}"
      VALUE "ProductVersion", "{version}"
    END
  END
  BLOCK "VarFileInfo"
  BEGIN
    VALUE "Translation", 0x416, 1200
  END
END
"#
    );
    std::fs::write(&rc, resource).unwrap();
    embed_resource::compile(&rc, embed_resource::NONE)
        .manifest_optional()
        .unwrap();
}
