use std::env;
use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=SWIFT_RUNTIME_PATH");
    println!("cargo:rustc-link-arg=-Wl,-rpath,/usr/lib/swift");

    if let Err(err) = copy_swift_runtime() {
        println!("cargo:warning=failed to bundle Swift runtime libraries: {err}");
    }
}

fn copy_swift_runtime() -> Result<(), Box<dyn Error>> {
    let runtime_dirs = find_swift_runtime_dirs()
        .ok_or_else(|| "unable to locate Swift runtime libraries".to_string())?;

    // OUT_DIR = target/<profile>/build/<crate>/out
    let out_dir = PathBuf::from(env::var("OUT_DIR")?);
    let profile_dir = out_dir
        .ancestors()
        .nth(3)
        .ok_or_else(|| "failed to resolve cargo profile directory".to_string())?
        .to_path_buf();
    let deps_dir = profile_dir.join("deps");
    let targets = [profile_dir, deps_dir];

    let mut copied_any = false;

    for runtime_dir in runtime_dirs {
        // println!(
        //     "cargo:warning=Bundling Swift runtime from {}",
        //     runtime_dir.display()
        // );

        for entry in fs::read_dir(&runtime_dir)? {
            let entry = entry?;
            let path = entry.path();
            if !is_swift_runtime(&path) {
                continue;
            }

            for target in &targets {
                if !target.exists() {
                    fs::create_dir_all(target)?;
                }
                let destination = target.join(
                    path.file_name()
                        .ok_or_else(|| "missing file name".to_string())?,
                );
                fs::copy(&path, &destination)?;
                copied_any = true;
                eprintln!("Copied {} -> {}", path.display(), destination.display());
            }
        }
    }

    if !copied_any {
        return Err("no Swift runtime dylibs were copied".into());
    }

    Ok(())
}

fn is_swift_runtime(path: &Path) -> bool {
    if path.extension().and_then(|ext| ext.to_str()) != Some("dylib") {
        return false;
    }
    matches!(path.file_name().and_then(|n| n.to_str()), Some(name) if name.starts_with("libswift"))
}

fn find_swift_runtime_dirs() -> Option<Vec<PathBuf>> {
    if let Ok(custom) = env::var("SWIFT_RUNTIME_PATH") {
        let path = PathBuf::from(custom);
        if path.exists() {
            eprintln!(
                "Using Swift runtime from SWIFT_RUNTIME_PATH: {}",
                path.display()
            );
            return Some(vec![path]);
        }
    }

    let mut results = Vec::new();

    if let Some(developer_dir) = developer_dir() {
        let toolchain_root = developer_dir
            .join("Toolchains")
            .join("XcodeDefault.xctoolchain")
            .join("usr/lib");
        eprintln!("Checking Xcode toolchain at {}", toolchain_root.display());
        for path in find_swift_subdirs(&toolchain_root) {
            push_unique(&mut results, path);
        }

        let clt_root = developer_dir.join("usr/lib");
        eprintln!("Checking developer usr/lib at {}", clt_root.display());
        for path in find_swift_subdirs(&clt_root) {
            push_unique(&mut results, path);
        }
    }

    let system_root = PathBuf::from("/usr/lib");
    eprintln!("Checking system Swift runtime at {}", system_root.display());
    for path in find_swift_subdirs(&system_root) {
        push_unique(&mut results, path);
    }

    if results.is_empty() {
        None
    } else {
        Some(results)
    }
}

fn find_swift_subdirs(root: &Path) -> Vec<PathBuf> {
    if !root.exists() {
        return Vec::new();
    }

    let mut results = Vec::new();

    if let Some(path) = try_swift_dir(root.join("swift").join("macosx")) {
        results.push(path);
    }

    let mut entries = match fs::read_dir(root) {
        Ok(entries) => entries.filter_map(|e| e.ok()).collect::<Vec<_>>(),
        Err(_) => return results,
    };
    entries.sort_by_key(|e| std::cmp::Reverse(e.file_name()));

    for entry in entries {
        let file_name = entry.file_name();
        let name = match file_name.to_str() {
            Some(name) => name,
            None => continue,
        };
        if !name.starts_with("swift") {
            continue;
        }
        if let Some(path) = try_swift_dir(entry.path().join("macosx")) {
            eprintln!("Found Swift runtime directory at {}", path.display());
            results.push(path);
        }
    }

    results
}

fn push_unique(vec: &mut Vec<PathBuf>, path: PathBuf) {
    if !vec.iter().any(|existing| existing == &path) {
        vec.push(path);
    }
}

fn try_swift_dir(path: PathBuf) -> Option<PathBuf> {
    if path.exists() && contains_runtime(&path) {
        Some(path)
    } else {
        None
    }
}

fn contains_runtime(dir: &Path) -> bool {
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            if is_swift_runtime(&entry.path()) {
                return true;
            }
        }
    }
    false
}

fn developer_dir() -> Option<PathBuf> {
    let output = Command::new("xcode-select").args(["-p"]).output().ok()?;
    if !output.status.success() {
        return None;
    }
    let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if path.is_empty() {
        return None;
    }
    Some(PathBuf::from(path))
}
