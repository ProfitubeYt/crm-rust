//! What a plain `cargo build` cannot do by itself: embed the Telegram web page,
//! put WebView2Loader.dll next to the executable (it is linked dynamically on
//! the gnu target and webview2-com-sys ships it in its build output), and give
//! the executable its icon and version for Explorer and the taskbar.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::{env, fs};

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    let Ok(out) = env::var("OUT_DIR") else { return };
    let out = PathBuf::from(out);
    embed_telegram_page(&out);
    embed_hosted_pages(&out);
    copy_webview_loader(&out);
    embed_resources(&out);
}

/// The pages of the workspaces that are web pages, taken from where each is
/// kept: Time's interface from its server, the administration panel from the
/// backend that also serves it. Each file gets the name the page's own links
/// ask for. A file that is not there leaves its table short, and the
/// workspace says so instead of failing to compile.
fn embed_hosted_pages(out: &Path) {
    let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap_or_default());
    let repository = manifest.join("../../..");
    let time = repository.join("time/crates/server/assets");
    let admin = repository.join("backend/admin_panel");
    let tables = [
        ("TIME_ASSETS", vec![("index.html", time.join("index.html")), ("assets/app.css", time.join("app.css")), ("assets/app.js", time.join("app.js"))]),
        ("ADMIN_ASSETS", vec![("admin/index.html", admin.join("index.html")), ("admin/admin.css", admin.join("admin.css")), ("admin/admin.js", admin.join("admin.js"))]),
    ];
    let mut code = String::new();
    for (table, files) in tables {
        let _ = writeln!(code, "pub static {table}: &[(&str, &[u8])] = &[");
        for (name, file) in files {
            println!("cargo:rerun-if-changed={}", file.display());
            if file.is_file() {
                let _ = writeln!(code, "    ({name:?}, include_bytes!({:?})),", file.to_string_lossy());
            } else {
                println!("cargo:warning={} is missing, its workspace page is not embedded", file.display());
            }
        }
        code.push_str("];\n");
    }
    let _ = fs::write(out.join("hosted_assets.rs"), code);
}

/// Without windres the program still builds and runs, just with a blank icon in
/// Explorer, so that is a warning and not a failure. Only a Windows executable
/// has resources; a build for it on another system has the tool under the
/// name of its target.
fn embed_resources(out: &Path) {
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap_or_default());
    let icon = manifest.join("assets").join("app.ico");
    println!("cargo:rerun-if-changed={}", icon.display());
    let version = env::var("CARGO_PKG_VERSION").unwrap_or_default();
    let numbers = version.replace('.', ",") + ",0";
    let script = format!(
        r#"1 ICON "{icon}"
1 VERSIONINFO
FILEVERSION {numbers}
PRODUCTVERSION {numbers}
FILEOS 0x40004
FILETYPE 0x1
BEGIN
  BLOCK "StringFileInfo"
  BEGIN
    BLOCK "040904B0"
    BEGIN
      VALUE "CompanyName", "TBS inform"
      VALUE "FileDescription", "TBS inform CRM"
      VALUE "FileVersion", "{version}"
      VALUE "InternalName", "tbsparcer"
      VALUE "OriginalFilename", "tbsparcer.exe"
      VALUE "ProductName", "TBS inform CRM"
      VALUE "ProductVersion", "{version}"
    END
  END
  BLOCK "VarFileInfo"
  BEGIN
    VALUE "Translation", 0x409, 1200
  END
END
"#,
        icon = icon.to_string_lossy().replace('\\', "/"),
    );
    let source = out.join("app.rc");
    let object = out.join("app-resources.o");
    if fs::write(&source, script).is_err() {
        return;
    }
    let built = ["windres", "x86_64-w64-mingw32-windres"].into_iter().any(|tool| {
        let made = Command::new(tool).arg("--input").arg(&source).arg("--output").arg(&object).args(["--output-format=coff"]).status();
        made.is_ok_and(|status| status.success())
    });
    if built {
        println!("cargo:rustc-link-arg-bins={}", object.display());
    } else {
        println!("cargo:warning=windres failed, the executable has no icon");
    }
}

/// `npm run build:telegram` fills this folder; a build without it gets an empty
/// table and the section says so instead of failing to compile.
fn embed_telegram_page(out: &Path) {
    let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap_or_default());
    let root = manifest.join("../../../frontend/dist-telegram");
    println!("cargo:rerun-if-changed={}", root.display());
    let mut files = Vec::new();
    collect(&root, &mut files);
    files.sort();
    let mut table = String::from("pub static ASSETS: &[(&str, &[u8])] = &[\n");
    for file in &files {
        let Ok(relative) = file.strip_prefix(&root) else { continue };
        let name = relative.to_string_lossy().replace('\\', "/");
        let _ = writeln!(table, "    ({name:?}, include_bytes!({:?})),", file.to_string_lossy());
    }
    table.push_str("];\n");
    let _ = fs::write(out.join("telegram_assets.rs"), table);
}

fn collect(dir: &Path, files: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect(&path, files);
        } else {
            files.push(path);
        }
    }
}

fn copy_webview_loader(out: &Path) {
    // OUT_DIR is <target>/<profile>/build/tbs-app-<hash>/out
    let Some(profile_dir) = out.ancestors().nth(3).map(PathBuf::from) else { return };
    let Ok(entries) = fs::read_dir(profile_dir.join("build")) else { return };
    for entry in entries.flatten() {
        if !entry.file_name().to_string_lossy().starts_with("webview2-com-sys-") {
            continue;
        }
        let dll = entry.path().join("out").join("x64").join("WebView2Loader.dll");
        if dll.exists() {
            let _ = fs::copy(&dll, profile_dir.join("WebView2Loader.dll"));
            let _ = fs::copy(&dll, profile_dir.join("deps").join("WebView2Loader.dll"));
            return;
        }
    }
}
