//! Embed the Windows resources: the icon and the version information.
//!
//! # Why this exists
//!
//! Without it the executable has no icon and no metadata, and Windows has
//! nothing to show. Task Manager lists a nameless `lumen.exe` beside a blank
//! rectangle, the Start Menu entry is a generic document, and the notification
//! area has nothing to draw. None of that is cosmetic: a background process
//! nobody can identify is a background process people kill.
//!
//! # Why a failure here is a warning and not an error
//!
//! Embedding needs a resource compiler -- `rc.exe` from the Windows SDK, or
//! `llvm-rc`. It is present on any machine with the MSVC build tools, which is
//! already required to build at all, but "already required" is not "always
//! there": a container image, a cross-compile, a toolchain installed without
//! the SDK component.
//!
//! A missing icon must not stop the launcher from being built. So this reports
//! what went wrong and carries on, and the binary works exactly as before with
//! the system's default icon.

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=../../assets/icon/lumen.ico");

    #[cfg(target_os = "windows")]
    embed_windows_resources();
}

#[cfg(target_os = "windows")]
fn embed_windows_resources() {
    let mut resource = winresource::WindowsResource::new();

    resource
        .set_icon("../../assets/icon/lumen.ico")
        // What Task Manager shows in its Name column, and what the Details
        // tab shows as the description. "Lumen" alone would be ambiguous
        // beside anything else called that; this says what it is.
        .set("FileDescription", "Lumen launcher")
        .set("ProductName", "Lumen")
        .set("OriginalFilename", "lumen.exe")
        .set("CompanyName", "Lumen contributors")
        // Section 2: the licence is part of the artefact, not only of the
        // repository it was built from.
        .set("LegalCopyright", "MIT OR Apache-2.0")
        .set("Comments", "A native command palette in Rust and GPUI");

    if let Err(error) = resource.compile() {
        println!(
            "cargo:warning=could not embed the icon and version information \
             ({error}); the binary will build and run, but Windows will have \
             no icon or name to show for it"
        );
    }
}
