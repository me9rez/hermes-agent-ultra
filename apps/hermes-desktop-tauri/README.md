# Terra Desktop (Tauri)

Desktop app for the Terra task-centric platform. Backend is **hermes-http** (Rust), not Python CLI.

## Dev workflow

```bash
# 1. Build backend
cargo build -p hermes-http

# 2. Run desktop (from apps/hermes-desktop-tauri)
npm install
npm run tauri:dev
```

### Backend binary resolution

The Tauri shell probes `hermes-http` in this order:

1. `HERMES_HTTP_BIN` environment variable
2. Next to the desktop executable
3. `target/debug/hermes-http` (repo root)
4. `PATH`

Default HTTP port: **8787** (override via `HERMES_HTTP_ADDR` when spawning).

### Terra UI

- Task home: `#/terra`
- Settings (billing, watchlist, schedules): `#/terra/settings`

### Verify

```bash
npm run verify:terra
```

### WebSocket

- Task event stream: `GET /api/tasks/{id}/stream`
- Multiplexed streams: `GET /api/ws?mode=tasks`

## Service install (optional)

Windows:

```powershell
./installers/windows/install-service.ps1 -BinaryPath "..\..\..\target\debug\hermes-http.exe"
```

macOS: see `installers/macos/app.terra.http.plist` and `postinstall.sh`.
