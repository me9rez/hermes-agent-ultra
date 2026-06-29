use std::path::PathBuf;

pub fn resolve_hermes_http_bin() -> Option<PathBuf> {
    if let Ok(bin) = std::env::var("HERMES_HTTP_BIN") {
        let path = PathBuf::from(bin.trim());
        if path.exists() {
            return Some(path);
        }
    }

    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            for name in candidate_names() {
                let candidate = dir.join(name);
                if candidate.exists() {
                    return Some(candidate);
                }
            }
        }
    }

    if let Ok(cwd) = std::env::current_dir() {
        for rel in [
            "target/debug/hermes-http",
            "target/release/hermes-http",
            "../target/debug/hermes-http",
            "../target/release/hermes-http",
        ] {
            let candidate = cwd
                .join(rel)
                .with_extension(if cfg!(windows) { "exe" } else { "" });
            if candidate.exists() {
                return Some(candidate);
            }
            let candidate_no_ext = cwd.join(rel);
            if candidate_no_ext.exists() {
                return Some(candidate_no_ext);
            }
        }
    }

    find_on_path("hermes-http")
}

fn candidate_names() -> [&'static str; 2] {
    if cfg!(windows) {
        ["hermes-http.exe", "hermes-http"]
    } else {
        ["hermes-http", "hermes-http.exe"]
    }
}

fn find_on_path(name: &str) -> Option<PathBuf> {
    let path_var = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path_var) {
        let candidate = dir.join(name);
        if candidate.exists() {
            return Some(candidate);
        }
        if cfg!(windows) {
            let with_exe = dir.join(format!("{name}.exe"));
            if with_exe.exists() {
                return Some(with_exe);
            }
        }
    }
    None
}
