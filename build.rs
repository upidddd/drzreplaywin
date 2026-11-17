// Windows: the exe's icon and version resource (name in Task Manager, the file's properties)
fn main() {
    let target_windows = std::env::var("CARGO_CFG_TARGET_OS").map(|t| t == "windows").unwrap_or(false);
    // only when building on Windows: a cross check on another system has no resource compiler
    #[cfg(windows)]
    if target_windows {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/icons/drzreplaywin.ico");
        res.set("CompanyName", "DRZ");
        res.set("FileDescription", "DrzReplayWIN");
        res.set("ProductName", "DrzReplayWIN");
        res.set("InternalName", "DrzReplayWIN");
        res.set("OriginalFilename", "DrzReplayWIN.exe");
        res.set("LegalCopyright", "Copyright (c) 2026 DRZ, MIT License");
        if let Err(e) = res.compile() {
            println!("cargo:warning=could not embed the icon: {e}");
        }
    }
    let _ = target_windows;
    println!("cargo:rerun-if-changed=assets/icons/drzreplaywin.ico");
    println!("cargo:rerun-if-changed=build.rs");
}
