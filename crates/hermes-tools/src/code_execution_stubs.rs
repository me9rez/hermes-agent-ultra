//! `hermes_tools.py` stub generator for execute_code PTC (Python `generate_hermes_tools_module` parity).

use std::collections::BTreeSet;

use crate::code_execution_env::SANDBOX_ALLOWED_TOOLS;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RpcTransport {
    /// Unix domain socket or loopback TCP via `HERMES_RPC_SOCKET` (Python `transport="uds"`).
    Uds,
    /// Same generated module as [`RpcTransport::Uds`] (Windows PTC uses `tcp://` in env).
    Tcp,
}

struct ToolStubDef {
    name: &'static str,
    signature: &'static str,
    doc: &'static str,
    args_expr: &'static str,
}

const TOOL_STUBS: &[ToolStubDef] = &[
    ToolStubDef {
        name: "web_search",
        signature: "query: str, limit: int = 5",
        doc: "\"\"\"Search the web. Returns dict with data.web list of {url, title, description}.\"\"\"",
        args_expr: r#"{"query": query, "limit": limit}"#,
    },
    ToolStubDef {
        name: "web_extract",
        signature: "urls: list",
        doc: "\"\"\"Extract content from URLs. Returns dict with results list of {url, title, content, error}.\"\"\"",
        args_expr: r#"{"urls": urls}"#,
    },
    ToolStubDef {
        name: "read_file",
        signature: "path: str, offset: int = 1, limit: int = 500",
        doc: "\"\"\"Read a file (1-indexed lines). Returns dict with \"content\" and \"total_lines\".\"\"\"",
        args_expr: r#"{"path": path, "offset": offset, "limit": limit}"#,
    },
    ToolStubDef {
        name: "write_file",
        signature: "path: str, content: str",
        doc: "\"\"\"Write content to a file (always overwrites). Returns dict with status.\"\"\"",
        args_expr: r#"{"path": path, "content": content}"#,
    },
    ToolStubDef {
        name: "search_files",
        signature: r#"pattern: str, target: str = "content", path: str = ".", file_glob: str = None, limit: int = 50, offset: int = 0, output_mode: str = "content", context: int = 0"#,
        doc: "\"\"\"Search file contents (target=\"content\") or find files by name (target=\"files\"). Returns dict with \"matches\".\"\"\"",
        args_expr: r#"{"pattern": pattern, "target": target, "path": path, "file_glob": file_glob, "limit": limit, "offset": offset, "output_mode": output_mode, "context": context}"#,
    },
    ToolStubDef {
        name: "patch",
        signature: r#"path: str = None, old_string: str = None, new_string: str = None, replace_all: bool = False, mode: str = "replace", patch: str = None"#,
        doc: "\"\"\"Targeted find-and-replace (mode=\"replace\") or V4A multi-file patches (mode=\"patch\"). Returns dict with status.\"\"\"",
        args_expr: r#"{"path": path, "old_string": old_string, "new_string": new_string, "replace_all": replace_all, "mode": mode, "patch": patch}"#,
    },
    ToolStubDef {
        name: "terminal",
        signature: "command: str, timeout: int = None, workdir: str = None",
        doc: "\"\"\"Run a shell command (foreground only). Returns dict with \"output\" and \"exit_code\".\"\"\"",
        args_expr: r#"{"command": command, "timeout": timeout, "workdir": workdir}"#,
    },
];

/// Python `_UDS_TRANSPORT_HEADER` (helpers + `_connect` + `_call`).
const UDS_TRANSPORT_HEADER: &str = r##""""Auto-generated Hermes tools RPC stubs."""
import json, os, socket, shlex, threading, time

_sock = None
# The RPC server handles a single client connection serially and has no
# request-id in the protocol, so concurrent _call() invocations from multiple
# threads (e.g. ThreadPoolExecutor) would race on the shared socket and get
# each other's responses. Serialize the entire send+recv round-trip.
_call_lock = threading.Lock()

# ---------------------------------------------------------------------------
# Convenience helpers (avoid common scripting pitfalls)
# ---------------------------------------------------------------------------

def json_parse(text: str):
    """Parse JSON tolerant of control characters (strict=False).
    Use this instead of json.loads() when parsing output from terminal()
    or web_extract() that may contain raw tabs/newlines in strings."""
    return json.loads(text, strict=False)


def shell_quote(s: str) -> str:
    """Shell-escape a string for safe interpolation into commands.
    Use this when inserting dynamic content into terminal() commands:
        terminal(f"echo {shell_quote(user_input)}")
    """
    return shlex.quote(s)


def retry(fn, max_attempts=3, delay=2):
    """Retry a function up to max_attempts times with exponential backoff.
    Use for transient failures (network errors, API rate limits):
        result = retry(lambda: terminal("gh issue list ..."))
    """
    last_err = None
    for attempt in range(max_attempts):
        try:
            return fn()
        except Exception as e:
            last_err = e
            if attempt < max_attempts - 1:
                time.sleep(delay * (2 ** attempt))
    raise last_err


def _connect():
    """Connect to the parent's RPC server via the transport it picked.

    HERMES_RPC_SOCKET can be either:
      - a filesystem path (POSIX Unix domain socket — the default on
        Linux and macOS)
      - a string of the form ``tcp://127.0.0.1:<port>`` (Windows, where
        AF_UNIX is unreliable — the parent falls back to loopback TCP)
    """
    global _sock
    if _sock is None:
        endpoint = os.environ["HERMES_RPC_SOCKET"]
        if endpoint.startswith("tcp://"):
            # tcp://host:port  (host is always 127.0.0.1 in practice — we
            # only bind loopback server-side)
            _host_port = endpoint[len("tcp://"):]
            _host, _, _port = _host_port.rpartition(":")
            _sock = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
            _sock.connect((_host or "127.0.0.1", int(_port)))
        else:
            _sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
            _sock.connect(endpoint)
        _sock.settimeout(300)
    return _sock

def _call(tool_name, args):
    """Send a tool call to the parent process and return the parsed result."""
    request = json.dumps({"tool": tool_name, "args": args}) + "\n"
    with _call_lock:
        conn = _connect()
        conn.sendall(request.encode())
        buf = b""
        while True:
            chunk = conn.recv(65536)
            if not chunk:
                raise RuntimeError("Agent process disconnected")
            buf += chunk
            if buf.endswith(b"\n"):
                break
    raw = buf.decode().strip()
    result = json.loads(raw)
    if isinstance(result, str):
        try:
            return json.loads(result)
        except (json.JSONDecodeError, TypeError):
            return result
    return result

"##;

/// Tools in both [`SANDBOX_ALLOWED_TOOLS`] and `enabled_tools`, sorted.
pub fn resolve_sandbox_tools(enabled_tools: &[String]) -> Vec<String> {
    let allowed: BTreeSet<&str> = SANDBOX_ALLOWED_TOOLS.iter().copied().collect();
    let enabled: BTreeSet<&str> = enabled_tools.iter().map(|s| s.as_str()).collect();
    allowed
        .intersection(&enabled)
        .map(|s| (*s).to_string())
        .collect()
}

fn stub_function(stub: &ToolStubDef) -> String {
    format!(
        "def {}({}):\n    {}\n    return _call('{name}', {args})\n",
        stub.name,
        stub.signature,
        stub.doc,
        name = stub.name,
        args = stub.args_expr
    )
}

/// Build `hermes_tools.py` source (Python `generate_hermes_tools_module`).
pub fn generate_hermes_tools_module(enabled_tools: &[String], transport: RpcTransport) -> String {
    let tools = resolve_sandbox_tools(enabled_tools);
    let header = match transport {
        RpcTransport::Uds | RpcTransport::Tcp => UDS_TRANSPORT_HEADER,
    };
    let stubs: Vec<String> = tools
        .iter()
        .filter_map(|name| {
            TOOL_STUBS
                .iter()
                .find(|s| s.name == name)
                .map(stub_function)
        })
        .collect();
    if stubs.is_empty() {
        header.to_string()
    } else {
        format!("{header}{}", stubs.join("\n"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_python_web_search_only() {
        let rust = generate_hermes_tools_module(&["web_search".into()], RpcTransport::Uds);
        let expected = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../hermes-parity-tests/fixtures/code_execution_stubs/web_search_only.expected.txt"
        ));
        assert_eq!(rust, expected);
    }

    #[test]
    fn intersects_sandbox_allowlist() {
        let src = generate_hermes_tools_module(
            &["web_search".into(), "execute_code".into()],
            RpcTransport::Uds,
        );
        assert!(src.contains("def web_search("));
        assert!(!src.contains("def execute_code("));
    }
}
