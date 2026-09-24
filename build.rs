//! Builds the window for the browser and embeds it, so `moon serve` hands it out at `/moon`
//! from the executable alone - see `src/server/web_page.rs`. Every build of `moon` does this:
//! `cargo run`, `cargo install`, the release script.
//!
//! The window is this same crate compiled to wasm32, by a second cargo with a target directory
//! of its own - the one running this script holds the lock on the usual one. That build runs
//! this script too, and is told apart by its target.

use std::{
    env, fs,
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

use flate2::{Compression, write::GzEncoder};


const WASM_TARGET: &str = "wasm32-unknown-unknown";

/// What the page is made of besides the module and its JavaScript, by where it is published
/// under `/moon/` and where it is in the repo.
const PAGE_FILES: &[(&str, &str)] = &[
    ("index.html", "web/index.html"),
    (GHOSTTY_ENV_SHIM, "web/ghostty_env.js"),
];

/// What Ghostty's VT engine imports from its host - `env.log` - is taken from this page file
/// instead: wasm-bindgen writes the import as a bare `env` module, which no browser can load.
const GHOSTTY_ENV_SHIM: &str = "ghostty_env.js";
const GHOSTTY_ENV_IMPORT: &str = "from \"env\"";

/// For each profile of the build embedding it: the browser build's cargo profile, and the
/// folder cargo writes that profile to. A debug `moon` gets a debug page, which compiles
/// faster, and that is what `cargo run` is for.
const WASM_PROFILES: &[(&str, &str, &str)] = &[
    ("debug", "dev", "debug"),
    ("release", "release", "release"),
];

/// What the browser build is compiled from. Anything else changing leaves the page as it was.
const WATCHED: &[&str] = &["src", "web", "crates", "Cargo.toml", "Cargo.lock", ".cargo"];

/// What cargo hands this script that would steer the second cargo away from its own settings -
/// the native build's flags above all, which would replace the wasm ones in .cargo/config.toml.
const INHERITED_ENV: &[&str] = &[
    "CARGO_ENCODED_RUSTFLAGS",
    "RUSTFLAGS",
    "CARGO_TARGET_DIR",
    "CARGO_BUILD_TARGET",
];

fn main() {
    let out = PathBuf::from(env::var("OUT_DIR").expect("cargo sets OUT_DIR"));
    let bundle = out.join("web_bundle.rs");

    if env::var("TARGET").expect("cargo sets TARGET") == WASM_TARGET {
        // The browser build itself, which has no server to embed a page in.
        fs::write(&bundle, "&[]\n").expect("could not write web_bundle.rs");
        return;
    }
    for watched in WATCHED {
        println!("cargo:rerun-if-changed={watched}");
    }

    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("cargo sets it"));
    let profile = env::var("PROFILE").expect("cargo sets PROFILE");
    let (_, wasm_profile, wasm_folder) = WASM_PROFILES
        .iter()
        .find(|(native, _, _)| *native == profile)
        .unwrap_or_else(|| panic!("no browser build profile for the {profile} profile"));

    let wasm = compile_wasm(&manifest_dir, &target_root(&out), wasm_profile, wasm_folder);

    let dist = out.join("web-dist");
    if dist.exists() {
        fs::remove_dir_all(&dist).expect("could not clear the last browser build");
    }
    wasm_bindgen_cli_support::Bindgen::new()
        .input_path(&wasm)
        .web(true)
        .expect("wasm-bindgen refused the web target")
        .typescript(false)
        // The page's `init()` finds the module beside the script, as the CLI's output does.
        .omit_default_module_path(false)
        .out_name("moonreview")
        .generate(&dist)
        .unwrap_or_else(|error| panic!("wasm-bindgen could not bind {}: {error:#}", wasm.display()));
    point_env_import_at_shim(&dist.join("moonreview.js"));
    for (published, source) in PAGE_FILES {
        let published = dist.join(published);
        fs::create_dir_all(published.parent().expect("a published file is in a folder"))
            .expect("could not make a folder of the browser build");
        fs::copy(manifest_dir.join(source), &published)
            .unwrap_or_else(|error| panic!("could not copy {source}: {error}"));
    }

    let mut files = Vec::new();
    collect(&dist, &dist, &mut files);
    files.sort();
    let gzipped = out.join("web-gzipped");
    let entries: String = files
        .iter()
        .map(|(published, source)| {
            let packed = gzip_into(&gzipped, published, Path::new(source));
            format!("    ({published:?}, include_bytes!({:?})),\n", packed.display())
        })
        .collect();
    fs::write(&bundle, format!("&[\n{entries}]\n")).expect("could not write web_bundle.rs");
}

/// Write `source` gzipped under `folder`, and answer with where. Every file of the page is
/// embedded gzipped and sent that way - see `src/server/web_page.rs` - which takes the module
/// from 17 MB to under 7, in the executable and down the wire alike.
fn gzip_into(folder: &Path, published: &str, source: &Path) -> PathBuf {
    let packed = folder.join(format!("{published}.gz"));
    fs::create_dir_all(packed.parent().expect("a gzipped file is in a folder"))
        .expect("could not make a folder for the gzipped page");
    let contents = fs::read(source)
        .unwrap_or_else(|error| panic!("could not read {}: {error}", source.display()));
    let mut encoder = GzEncoder::new(Vec::new(), Compression::best());
    encoder
        .write_all(&contents)
        .and_then(|()| encoder.finish())
        .and_then(|gzipped| fs::write(&packed, gzipped))
        .unwrap_or_else(|error| panic!("could not gzip {published}: {error}"));
    packed
}

/// Point the module's `env` import at [`GHOSTTY_ENV_SHIM`]. Exactly one such import is expected:
/// none means Ghostty stopped importing from its host, and more means something else started
/// to, and either way the shim wants a look before the page is built on it.
fn point_env_import_at_shim(script: &Path) {
    let generated = fs::read_to_string(script)
        .unwrap_or_else(|error| panic!("could not read {}: {error}", script.display()));
    let found = generated.matches(GHOSTTY_ENV_IMPORT).count();
    assert_eq!(
        found, 1,
        "expected wasm-bindgen's output to import `env` once, for Ghostty's log; it does {found} times"
    );
    let pointed = generated.replace(GHOSTTY_ENV_IMPORT, &format!("from \"./{GHOSTTY_ENV_SHIM}\""));
    fs::write(script, pointed)
        .unwrap_or_else(|error| panic!("could not write {}: {error}", script.display()));
}

/// Compile the library to a wasm module, and answer with where it is.
fn compile_wasm(
    manifest_dir: &Path,
    target_root: &Path,
    wasm_profile: &str,
    wasm_folder: &str,
) -> PathBuf {
    add_wasm_target();

    let target_dir = target_root.join("web-build");
    let cargo = env::var("CARGO").expect("cargo sets CARGO");
    let mut build = Command::new(cargo);
    build
        .current_dir(manifest_dir)
        // A cdylib for this build alone: as a crate type in Cargo.toml, every native build
        // would link one as well.
        .args(["rustc", "--lib", "--crate-type", "cdylib", "--target", WASM_TARGET])
        .args(["--profile", wasm_profile])
        // Debug info would be most of the module, and all of it downloaded by every page load.
        .arg("--config")
        .arg(format!("profile.{wasm_profile}.debug=false"))
        // Built again after every change to the sources, so a small change should cost a small
        // rebuild - which release builds only get when told to.
        .arg("--config")
        .arg(format!("profile.{wasm_profile}.incremental=true"))
        .arg("--target-dir")
        .arg(&target_dir)
        // Anything on stdout would be read as instructions to cargo from this script.
        .stdout(Stdio::null());
    for inherited in INHERITED_ENV {
        build.env_remove(inherited);
    }
    let status = build.status().expect("could not start cargo for the browser build");
    assert!(status.success(), "the browser build failed - its errors are above");

    target_dir
        .join(WASM_TARGET)
        .join(wasm_folder)
        .join("moonreview.wasm")
}

/// Install the wasm32 standard library when the toolchain has none, so a first build needs
/// nothing set up by hand.
fn add_wasm_target() {
    let rustc = env::var("RUSTC").expect("cargo sets RUSTC");
    let sysroot = Command::new(&rustc)
        .args(["--print", "sysroot"])
        .output()
        .expect("could not ask rustc for its sysroot");
    let sysroot = PathBuf::from(String::from_utf8_lossy(&sysroot.stdout).trim());
    if sysroot.join("lib/rustlib").join(WASM_TARGET).is_dir() {
        return;
    }
    println!("cargo:warning=adding the {WASM_TARGET} target with rustup, for the browser build");
    let mut add = Command::new("rustup");
    add.args(["target", "add", WASM_TARGET]);
    // The toolchain this build runs on, not rustup's default.
    if let Ok(toolchain) = env::var("RUSTUP_TOOLCHAIN") {
        add.args(["--toolchain", &toolchain]);
    }
    let status = add
        .stdout(Stdio::null())
        .status()
        .expect("the browser build needs the wasm32 target, and rustup could not be run to add it");
    assert!(status.success(), "rustup could not add {WASM_TARGET}");
}

/// The target directory this build is in. `out` is `<target>/<profile>/build/<package>/out`,
/// with the target triple between the first two when the build named one. The browser build
/// goes beside the native builds there, so `cargo clean` takes it too.
fn target_root(out: &Path) -> PathBuf {
    let profile_dir = out
        .ancestors()
        .nth(3)
        .unwrap_or_else(|| panic!("{} is not where cargo puts OUT_DIR", out.display()));
    let above = profile_dir.parent().expect("a profile folder is in a target directory");
    let target = env::var("TARGET").expect("cargo sets TARGET");
    if above.file_name().is_some_and(|name| name == target.as_str()) {
        above.parent().expect("a triple folder is in a target directory").to_path_buf()
    } else {
        above.to_path_buf()
    }
}

/// Every file under `dir`, as its path from `root` with `/` between the parts, and where it is.
fn collect(root: &Path, dir: &Path, files: &mut Vec<(String, String)>) {
    for entry in fs::read_dir(dir).expect("could not read the browser build") {
        let path = entry.expect("could not read the browser build").path();
        if path.is_dir() {
            collect(root, &path, files);
            continue;
        }
        let published = path
            .strip_prefix(root)
            .expect("a file under the browser build is under it")
            .components()
            .map(|part| part.as_os_str().to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .join("/");
        files.push((published, path.to_string_lossy().into_owned()));
    }
}
