---
layout: default
title: "Building from source"
---
# Building from source

The supported application is the Rust code in `native/`. The Python files at
the repository root are the earlier implementation, kept as a reference.
They are not what the installer ships.

## Requirements

- Windows 10/11 x64.
- Rust with the MSVC toolchain (`x86_64-pc-windows-msvc`) and the Windows
  SDK, via Visual Studio Build Tools.
- Python 3.11 or later. This is for the packaging script only; the installed
  programs don't use Python.
- [Inno Setup 6](https://jrsoftware.org/isinfo.php) to build the installer.
  `scripts/Get-InnoSetup.ps1` downloads and verifies it.

## Development checks

From the repository root:

```powershell
cargo fmt --manifest-path native/Cargo.toml --check
cargo test --manifest-path native/Cargo.toml --locked
cargo clippy --manifest-path native/Cargo.toml --locked --all-targets -- -D warnings
```

Tests use local fixtures and simulated clocks. They never change the PC's
clock or services.

## Building the installer

```powershell
cd native
python package.py --iscc "C:\Path\To\Inno Setup 6\ISCC.exe"
```

`package.py`:

1. checks dependency versions that carry certificate-distrust fixes,
2. runs formatting checks and the tests,
3. builds both programs in release mode with a static C runtime,
4. checks the programs don't need an external C/C++ runtime DLL,
5. checks no local user or checkout paths are embedded in the binaries,
6. collects third-party licence notices,
7. builds the installer and writes `dist/SHA256SUMS.txt` and
   `dist/build-manifest.json`.

The manifest records the source commit and whether the working tree had
uncommitted changes. Commit your changes before a release build so it
reports a clean revision.

Builds can be traced to their source and locked dependencies. Byte-for-byte
reproducibility isn't claimed.

## Testing an installed build

Installer, service lifecycle and real clock-response tests change Windows
services and the system clock. Run them only on a disposable Windows VM,
never on a machine whose clock matters.
