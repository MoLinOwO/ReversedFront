use std::env;
use std::fs;
use std::path::Path;

fn main() {
    let manifest_dir = env::var("CARGO_MANIFEST_DIR").unwrap();
    let root_path = Path::new(&manifest_dir).parent().unwrap();
    let web_path = root_path.join("web");
    let resources_path = Path::new(&manifest_dir).join("resources");

    println!("cargo:rerun-if-changed=../web");

    // Clean resources dir if it exists
    if resources_path.exists() {
        // Ignore errors if removal fails (resource busy etc)
        let _ = fs::remove_dir_all(&resources_path);
    }
    fs::create_dir_all(&resources_path).expect("Failed to create resources dir");

    // Copy specific directories and files from web/ to src-tauri/resources/
    let dirs_to_copy = vec!["tiles", "dexopt", "static"];
    let files_to_copy = vec![
        "index.html",
        "desktop_bridge.js",
        "manifest.json",
        "transporter.html",
    ];

    // Copy directories
    for dir_name in dirs_to_copy {
        let src = web_path.join(dir_name);
        let dst = resources_path.join(dir_name);
        if src.exists() {
            copy_dir_recursive(&src, &dst).expect(&format!("Failed to copy {}", dir_name));
        }
    }

    // Copy individual files
    for file_name in files_to_copy {
        let src = web_path.join(file_name);
        let dst = resources_path.join(file_name);
        if src.exists() {
            fs::copy(&src, &dst).expect(&format!("Failed to copy {}", file_name));
        }
    }

    // Copy mod directory structure selectively (exclude node_modules and source code)
    let mod_dst = resources_path.join("mod");
    fs::create_dir_all(&mod_dst).expect("Failed to create mod dir");

    // Copy mod/js - only compiled bundles and essential files
    let mod_js_src = web_path.join("mod").join("js");
    let mod_js_dst = mod_dst.join("js");
    fs::create_dir_all(&mod_js_dst).expect("Failed to create mod/js dir");

    // Copy every compiled bundle. Webpack may emit a vendor chunk in addition
    // to main.bundle.js; omitting it makes the packaged app fail only when a
    // particular Mod feature is opened.
    if let Ok(entries) = fs::read_dir(&mod_js_src) {
        for entry in entries.flatten() {
            let src = entry.path();
            let file_name = entry.file_name();
            let file_name_str = file_name.to_string_lossy();
            if src.is_file()
                && (file_name_str.ends_with(".bundle.js")
                    || file_name_str.ends_with(".LICENSE.txt"))
            {
                let dst = mod_js_dst.join(file_name);
                fs::copy(&src, &dst).ok(); // Ignore errors for optional files
            }
        }
    }

    // Copy mod/data directory completely
    let mod_data_src = web_path.join("mod").join("data");
    let mod_data_dst = mod_dst.join("data");
    if mod_data_src.exists() {
        copy_dir_recursive(&mod_data_src, &mod_data_dst).expect("Failed to copy mod/data");
    }

    tauri_build::build()
}

fn copy_dir_recursive(src: &Path, dst: &Path) -> std::io::Result<()> {
    if !dst.exists() {
        fs::create_dir_all(dst)?;
    }
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        let file_name = entry.file_name();
        let file_name_str = file_name.to_string_lossy();

        // Skip unwanted files and directories
        if file_name_str.ends_with(".pfx")
            || file_name_str == "config.json"
            || file_name_str == "node_modules"
            || file_name_str == "package.json"
            || file_name_str == "package-lock.json"
            || file_name_str == "webpack.config.cjs"
        {
            continue;
        }

        let dst_path = dst.join(file_name);
        if file_type.is_dir() {
            copy_dir_recursive(&entry.path(), &dst_path)?;
        } else {
            fs::copy(entry.path(), dst_path)?;
        }
    }
    Ok(())
}
