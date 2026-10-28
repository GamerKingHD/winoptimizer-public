# WinOptimizer Free

WinOptimizer is a Windows system utility: disk cleanup, startup management,
system tweaks, hosts editing, repair diagnostics, process management, and
read-only disk analysis.

This repository contains **WinOptimizer Free**, the open-source community
edition. It is a complete, standalone application and builds without any
private dependency, credential, or network resource.

This tree is **generated** from the private WinOptimizer repository by its
public exporter. Do not develop here: changes are overwritten on the next
release. Report issues against the released version instead.

## Build from a clean clone

Requirements:

- Windows 10/11 x64
- Node.js `>=20 <21` and npm `>=10 <12` (see `.nvmrc`)
- The Rust toolchain pinned by `rust-toolchain.toml` (rustup installs it
  automatically)

```powershell
npm ci
cmd /c build.bat --release
```

The result is `dist\winoptimizer.exe`. Individual steps:

```powershell
npm run build:frontend                 # Vite -> .build/frontend
cmd /c build.bat --release             # Rust release -> dist\winoptimizer.exe
npm run audit:public                   # source boundary audit
npm run verify:public                  # artifact leakage verification
```

## Tests

```powershell
npm run typecheck
npm test                               # vitest
```

## Layout

- `src/` — Vue 3 + TypeScript frontend
- `src-tauri/` — Rust application (Tauri v2)
- `scripts/` — boundary audit and artifact verification
- `third_party/` — vendored third-party tweak catalogue (MIT)

## Licensing

Licensed under the Apache License, Version 2.0. See `LICENSE` and `NOTICE`.
Third-party components are attributed in `NOTICE`.
