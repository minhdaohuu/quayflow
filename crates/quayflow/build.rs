//! Embeds the Quayflow application icon and version information in the Windows executables.

fn main() {
    println!("cargo:rerun-if-changed=../../assets/brand/quayflow.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let mut res = winresource::WindowsResource::new();
    res.set_icon("../../assets/brand/quayflow.ico")
        .set("ProductName", "Quayflow")
        .set("FileDescription", "Quayflow account switching for Claude Code")
        .set("LegalCopyright", "Copyright (c) 2026 Minh Dao Huu. MIT License.");
    if let Err(e) = res.compile() {
        panic!("failed to embed the Windows resources: {e}");
    }
}
