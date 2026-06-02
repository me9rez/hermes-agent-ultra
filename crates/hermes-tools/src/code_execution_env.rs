//! Child environment scrubbing for `execute_code` (Python `code_execution_tool._scrub_child_env` parity).

use std::collections::BTreeMap;

use hermes_core::tz_for_child_env;

pub const SANDBOX_ALLOWED_TOOLS: &[&str] = &[
    "web_search",
    "web_extract",
    "read_file",
    "write_file",
    "search_files",
    "patch",
    "terminal",
];

const SAFE_ENV_PREFIXES: &[&str] = &[
    "PATH", "HOME", "USER", "LANG", "LC_", "TERM", "TMPDIR", "TMP", "TEMP", "SHELL", "LOGNAME",
    "XDG_", "PYTHONPATH", "VIRTUAL_ENV", "CONDA", "HERMES_",
];

const SECRET_SUBSTRINGS: &[&str] = &[
    "KEY", "TOKEN", "SECRET", "PASSWORD", "CREDENTIAL", "PASSWD", "AUTH",
];

const WINDOWS_ESSENTIAL_ENV_VARS: &[&str] = &[
    "SYSTEMROOT",
    "SYSTEMDRIVE",
    "WINDIR",
    "COMSPEC",
    "PATHEXT",
    "OS",
    "PROCESSOR_ARCHITECTURE",
    "NUMBER_OF_PROCESSORS",
    "PUBLIC",
    "ALLUSERSPROFILE",
    "PROGRAMDATA",
    "PROGRAMFILES",
    "PROGRAMFILES(X86)",
    "PROGRAMW6432",
    "APPDATA",
    "LOCALAPPDATA",
    "USERPROFILE",
    "USERDOMAIN",
    "USERNAME",
    "HOMEDRIVE",
    "HOMEPATH",
    "COMPUTERNAME",
];

fn windows_essential_names_upper() -> Vec<String> {
    WINDOWS_ESSENTIAL_ENV_VARS
        .iter()
        .map(|s| s.to_ascii_uppercase())
        .collect()
}

/// Produce scrubbed child-process environment (Python `_scrub_child_env`).
pub fn scrub_child_env(
    source_env: &BTreeMap<String, String>,
    is_passthrough: impl Fn(&str) -> bool,
    is_windows: bool,
) -> BTreeMap<String, String> {
    let win_essentials = windows_essential_names_upper();
    let mut scrubbed = BTreeMap::new();
    for (k, v) in source_env {
        if is_passthrough(k) {
            scrubbed.insert(k.clone(), v.clone());
            continue;
        }
        let ku = k.to_ascii_uppercase();
        if SECRET_SUBSTRINGS.iter().any(|s| ku.contains(s)) {
            continue;
        }
        if SAFE_ENV_PREFIXES.iter().any(|p| k.starts_with(p)) {
            scrubbed.insert(k.clone(), v.clone());
            continue;
        }
        if is_windows && win_essentials.iter().any(|e| e == &ku) {
            scrubbed.insert(k.clone(), v.clone());
        }
    }
    scrubbed
}

/// Scrub secrets then apply Hermes child timezone (`TZ` from configured IANA zone).
pub fn prepare_child_env(
    source_env: &BTreeMap<String, String>,
    is_passthrough: impl Fn(&str) -> bool,
    is_windows: bool,
) -> BTreeMap<String, String> {
    let mut scrubbed = scrub_child_env(source_env, is_passthrough, is_windows);
    scrubbed.remove("HERMES_TIMEZONE");
    if let Some(tz) = tz_for_child_env() {
        scrubbed.insert("TZ".to_string(), tz);
    }
    scrubbed
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn blocks_secret_substrings() {
        let src = env(&[
            ("PATH", "/bin"),
            ("OPENAI_API_KEY", "sk-secret"),
            ("MY_PASSWORD", "x"),
        ]);
        let out = scrub_child_env(&src, |_| false, false);
        assert!(out.contains_key("PATH"));
        assert!(!out.contains_key("OPENAI_API_KEY"));
        assert!(!out.contains_key("MY_PASSWORD"));
    }

    #[test]
    fn injects_tz_without_leaking_hermes_timezone() {
        hermes_core::init_global_clock(Some("Asia/Kolkata"));
        let src = env(&[("HERMES_TIMEZONE", "Asia/Kolkata"), ("PATH", "/bin")]);
        let out = prepare_child_env(&src, |_| false, false);
        assert_eq!(out.get("TZ").map(String::as_str), Some("Asia/Kolkata"));
        assert!(!out.contains_key("HERMES_TIMEZONE"));
        assert_eq!(out.get("PATH").map(String::as_str), Some("/bin"));
    }

    #[test]
    fn passthrough_overrides_secret_block() {
        let src = env(&[("TENOR_API_KEY", "x"), ("PATH", "/bin")]);
        let out = scrub_child_env(&src, |k| k == "TENOR_API_KEY", false);
        assert_eq!(out.get("TENOR_API_KEY").map(String::as_str), Some("x"));
    }

    #[test]
    fn windows_essentials_allowed_on_windows() {
        let src = env(&[
            ("SYSTEMROOT", r"C:\Windows"),
            ("RANDOM_UNKNOWN_VAR", "nope"),
        ]);
        let out = scrub_child_env(&src, |_| false, true);
        assert_eq!(out.get("SYSTEMROOT").map(String::as_str), Some(r"C:\Windows"));
        assert!(!out.contains_key("RANDOM_UNKNOWN_VAR"));
    }

    #[test]
    fn windows_essentials_not_allowed_on_posix() {
        let src = env(&[("SYSTEMROOT", r"C:\Windows"), ("PATH", "/bin")]);
        let out = scrub_child_env(&src, |_| false, false);
        assert!(!out.contains_key("SYSTEMROOT"));
        assert!(out.contains_key("PATH"));
    }
}
