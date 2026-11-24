//! Build script: renders the application icon and embeds it in the executable
//! together with the version information and the application manifest.

use std::error::Error;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::{env, fs};

#[path = "src/brand/ico.rs"]
mod ico;
#[path = "src/brand/mark.rs"]
mod mark;
#[path = "src/brand/png.rs"]
mod png;

/// Pixel sizes stored in the icon: the shell's small and large sizes at
/// 100-200 % scaling, plus the 256-pixel size used by large thumbnail views.
const ICON_SIZES: [u32; 8] = [16, 20, 24, 32, 40, 48, 64, 256];

const PRODUCT_NAME: &str = "RS-CPUZ";
const INTERNAL_NAME: &str = "rs-cpuz";
const COPYRIGHT: &str = "Copyright (C) the rs-cpuz contributors";

const MANIFEST: &str = "assets/rs-cpuz.manifest";
const INPUTS: [&str; 5] = ["build.rs", MANIFEST, "src/brand/ico.rs", "src/brand/mark.rs", "src/brand/png.rs"];

fn main() -> Result<(), Box<dyn Error>> {
    for input in INPUTS {
        println!("cargo:rerun-if-changed={input}");
    }
    if env::var_os("CARGO_CFG_WINDOWS").is_none() {
        return Ok(());
    }
    // The DLLs the executable imports are looked up in the system directory only, never
    // next to it: a DLL dropped beside a program that runs elevated must not be loaded.
    // GNU ld has no such switch; the program restricts its own later loads at startup.
    if env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") {
        println!("cargo:rustc-link-arg-bins=/DEPENDENTLOADFLAG:0x800");
    }

    // The resource script names its files without a directory: the resource
    // compiler looks them up in `OUT_DIR`, where all of them are written.
    let out_dir = PathBuf::from(env::var_os("OUT_DIR").ok_or("OUT_DIR is not set")?);
    write(&out_dir.join("rs-cpuz.ico"), ico::file(&ICON_SIZES, mark::ACCENT_RGB))?;
    // The manifest is written anew rather than copied: a copy would inherit a
    // read-only attribute, and the next build could not replace it.
    let manifest = fs::read(MANIFEST).map_err(|error| format!("cannot read {MANIFEST}: {error}"))?;
    write(&out_dir.join("rs-cpuz.manifest"), manifest)?;
    let script = out_dir.join("rs-cpuz.rc");
    write(&script, resource_script(&env::var("CARGO_PKG_VERSION")?, version_numbers()))?;

    if windres_breaks_in(&out_dir) {
        return compile_with_windres(&script, &out_dir);
    }
    Ok(embed_resource::compile(&script, embed_resource::NONE).manifest_required()?)
}

/// Writes a generated file, naming it should that fail.
fn write(path: &Path, contents: impl AsRef<[u8]>) -> Result<(), Box<dyn Error>> {
    fs::write(path, contents).map_err(|error| format!("cannot write {}: {error}", path.display()).into())
}

/// GNU `windres` on Windows hands its include directories to the preprocessor
/// unquoted, so the `--include-dir OUT_DIR` that `embed-resource` always adds
/// cannot work below a path that contains a space. Like `embed-resource`, this
/// goes by the toolchain the build script itself was compiled with.
fn windres_breaks_in(out_dir: &Path) -> bool {
    cfg!(all(windows, not(target_env = "msvc"))) && out_dir.to_string_lossy().contains(' ')
}

/// Compiles `script` the way `embed-resource` does and links the result into
/// the executable, but passes no include directory: `windres` finds the files
/// that a script names next to the script by itself.
fn compile_with_windres(script: &Path, out_dir: &Path) -> Result<(), Box<dyn Error>> {
    // The crate builds for x86 and x86-64 only.
    let format = if env::var("CARGO_CFG_TARGET_ARCH")? == "x86_64" { "pe-x86-64" } else { "pe-i386" };
    let archive = out_dir.join("librs-cpuz.a");
    let status = Command::new("windres")
        .arg("--input")
        .arg(script)
        .arg("--output")
        .arg(&archive)
        .args(["--output-format=coff", "--target", format, "-c", "65001"])
        .status()
        .map_err(|error| format!("cannot run windres: {error}"))?;
    if !status.success() {
        eprintln!("note: GNU windres is known to fail below a path that contains a space;");
        eprintln!("      if that is the cause, set CARGO_TARGET_DIR to a path without spaces");
        return Err(format!("windres could not compile {}: {status}", script.display()).into());
    }
    println!("cargo:rustc-link-arg-bins={}", archive.display());
    Ok(())
}

/// Major, minor and patch number of the package, as the 16-bit fields of a `VERSIONINFO` resource.
fn version_numbers() -> [u16; 3] {
    ["MAJOR", "MINOR", "PATCH"].map(|part| {
        env::var(format!("CARGO_PKG_VERSION_{part}")).ok().and_then(|value| value.parse().ok()).unwrap_or(0)
    })
}

/// The resource script: icon, manifest (resource type 24) and version information (US English, Unicode).
fn resource_script(version: &str, [major, minor, patch]: [u16; 3]) -> String {
    format!(
        r#"LANGUAGE 0x09, 0x01

1 ICON "rs-cpuz.ico"
1 24 "rs-cpuz.manifest"

1 VERSIONINFO
FILEVERSION {major},{minor},{patch},0
PRODUCTVERSION {major},{minor},{patch},0
FILEFLAGSMASK 0x3FL
FILEFLAGS 0x0L
FILEOS 0x40004L
FILETYPE 0x1L
FILESUBTYPE 0x0L
BEGIN
    BLOCK "StringFileInfo"
    BEGIN
        BLOCK "040904B0"
        BEGIN
            VALUE "FileDescription", "{PRODUCT_NAME}"
            VALUE "FileVersion", "{version}"
            VALUE "InternalName", "{INTERNAL_NAME}"
            VALUE "LegalCopyright", "{COPYRIGHT}"
            VALUE "OriginalFilename", "{INTERNAL_NAME}.exe"
            VALUE "ProductName", "{PRODUCT_NAME}"
            VALUE "ProductVersion", "{version}"
        END
    END
    BLOCK "VarFileInfo"
    BEGIN
        VALUE "Translation", 0x0409, 1200
    END
END
"#
    )
}
