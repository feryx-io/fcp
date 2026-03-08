use std::process::Command;

fn main() {
    let ui_dir = std::path::Path::new("ui");

    // Only build frontend if ui/ directory exists
    if !ui_dir.exists() {
        println!("cargo:warning=ui/ directory not found, skipping frontend build");
        // Create an empty dist directory so include_dir doesn't fail
        std::fs::create_dir_all("dist").ok();
        std::fs::write("dist/index.html", "<!-- placeholder -->").ok();
        return;
    }

    // Gate npm execution behind FCP_BUILD_UI env var.
    // Set FCP_BUILD_UI=1 to build the frontend during `cargo build`.
    // In CI, prefer building the UI separately and committing dist/.
    println!("cargo:rerun-if-env-changed=FCP_BUILD_UI");
    let build_ui = std::env::var("FCP_BUILD_UI").unwrap_or_default();
    if build_ui != "1" && build_ui != "true" {
        // If dist/ already exists with a real index.html, use it as-is
        let dist_index = std::path::Path::new("dist/index.html");
        if dist_index.exists() {
            let content = std::fs::read_to_string(dist_index).unwrap_or_default();
            if !content.contains("placeholder") {
                println!("cargo:warning=Using pre-built dist/. Set FCP_BUILD_UI=1 to rebuild.");
                return;
            }
        }
        // No pre-built dist — create placeholder so compile doesn't fail
        println!("cargo:warning=Skipping UI build (set FCP_BUILD_UI=1 to enable). Using placeholder.");
        std::fs::create_dir_all("dist").ok();
        std::fs::write("dist/index.html", "<!-- Admin UI not built. Set FCP_BUILD_UI=1 and rebuild. -->").ok();
        return;
    }

    // Check if node_modules exists, run npm install if not
    let node_modules = ui_dir.join("node_modules");
    if !node_modules.exists() {
        println!("cargo:warning=Running npm install...");
        let status = Command::new("npm")
            .arg("install")
            .current_dir(ui_dir)
            .status()
            .expect("Failed to run npm install. Is Node.js installed?");

        if !status.success() {
            panic!("npm install failed with status: {}", status);
        }
    }

    // Run npm run build
    println!("cargo:warning=Building frontend with Vite...");
    let status = Command::new("npm")
        .args(["run", "build"])
        .current_dir(ui_dir)
        .status()
        .expect("Failed to run npm run build. Is Node.js installed?");

    if !status.success() {
        panic!("npm run build failed with status: {}", status);
    }

    // Tell Cargo to rerun if frontend source changes
    println!("cargo:rerun-if-changed=ui/src");
    println!("cargo:rerun-if-changed=ui/index.html");
    println!("cargo:rerun-if-changed=ui/package.json");
    println!("cargo:rerun-if-changed=ui/vite.config.ts");
    println!("cargo:rerun-if-changed=ui/tsconfig.json");
}
