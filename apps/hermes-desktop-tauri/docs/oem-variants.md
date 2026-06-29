# OEM variants

Terra desktop supports white-label builds via `variants/<id>/variant.json`.

## Apply a variant

```bash
npm run variant:apply -- default
npm run variant:apply -- <oem-id>
```

This updates `src/branding.ts`, copies icons, and emits `src-tauri/tauri.conf.<oem>.json`.

## Build

```bash
npm run variant:apply -- <oem-id>
npm run tauri:build
```

CI matrix: `.github/workflows/build-variants.yml`.
