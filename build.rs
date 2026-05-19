use std::{env, fs, path::Path, process::Command};

fn main() {
    println!("cargo:rerun-if-changed=web/src");
    println!("cargo:rerun-if-changed=web/build.mjs");
    println!("cargo:rerun-if-changed=package.json");
    println!("cargo:rerun-if-changed=package-lock.json");
    println!("cargo:rerun-if-changed=tsconfig.json");

    let manifest_dir = env::var("CARGO_MANIFEST_DIR").expect("missing manifest dir");
    let package_json_path = Path::new(&manifest_dir).join("package.json");
    let package_json = fs::read_to_string(&package_json_path).expect("failed to read package.json");
    let package: serde_json::Value =
        serde_json::from_str(&package_json).expect("failed to parse package.json");
    let version = package
        .get("version")
        .and_then(serde_json::Value::as_str)
        .expect("package.json version must be a string");
    println!("cargo:rustc-env=MOONREVIEW_VERSION={version}");

    let dist_file = Path::new(&manifest_dir).join("web/dist/app.js");

    let install_status = Command::new("npm")
        .arg("install")
        .current_dir(&manifest_dir)
        .status()
        .expect("failed to run npm install");
    assert!(install_status.success(), "npm install failed");

    let build_status = Command::new("npm")
        .args(["run", "build"])
        .current_dir(&manifest_dir)
        .status()
        .expect("failed to run npm run build");
    assert!(build_status.success(), "npm run build failed");

    assert!(
        dist_file.exists(),
        "expected bundled frontend at web/dist/app.js"
    );
}
