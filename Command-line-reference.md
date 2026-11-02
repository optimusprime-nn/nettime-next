---
layout: default
title: "Command-line reference"
---
# Command-line reference

Two programs are installed in the program folder (normally
`C:\Program Files\NetTime Next`):

- `NetTimeNextNative.exe`: the desktop app.
- `NetTimeNextNativeService.exe`: the service and administrator helper.

Both accept `--help` and `--version`.

## Desktop

| Command | What it does |
| --- | --- |
| `NetTimeNextNative.exe` | Opens the desktop app. |
| `NetTimeNextNative.exe --tray` | Starts hidden in the notification area. This is used by the sign-in shortcut. |

## Status and diagnostics

These don't need administrator rights.

| Command | What it does |
| --- | --- |
| `--health` | Prints the service state, clock ownership and current measurements. |
| `--status-service` | Prints the Windows service state. |
| `--export FOLDER --database FILE` | Exports full history as gzipped JSON. |
| `--compare-export FOLDER --days DAYS --database FILE` | Exports a comparison report for the last DAYS days. |

The installed database is
`C:\ProgramData\NetTimeNextNativePreview\data\history.sqlite3`.

Example:

```powershell
$exe = "C:\Program Files\NetTime Next\NetTimeNextNativeService.exe"
& $exe --health
& $exe --compare-export "$env:USERPROFILE\Desktop\ntn" --days 7 `
    --database C:\ProgramData\NetTimeNextNativePreview\data\history.sqlite3
```

## Administrator commands

These need an elevated prompt. The app runs them for you, so they're mainly
useful for scripting and recovery. Each one writes its outcome to
`C:\ProgramData\NetTimeNextNativePreview\admin-result.json`.

| Command | What it does |
| --- | --- |
| `--takeover` | Takes over the clock (see [Taking over the clock](Taking-over-the-clock)). |
| `--rollback` | Restores the previous clock services and returns to measurement mode. |
| `--apply-config FILE` | Validates and applies a settings file, keeping ownership. |
| `--import-settings` | Imports the sources and settings of the older NetTime Next 0.x service (`C:\ProgramData\NetTimeNext\settings.json`). Only allowed in measurement mode. |
| `--install-service` | Registers the Windows service. |
| `--start-service` / `--stop-service` | Starts or stops the service. |
| `--remove-service` | Removes the service, restoring clock ownership first if needed. |
| `--maintain-history --database FILE --days DAYS` | Compresses history and **permanently deletes** records older than DAYS. |

`--prepare-upgrade` and `--finish-upgrade` are used by the installer. Don't
run them by hand.

## Stand-alone observation

To measure without the service, for example to try a source list:

```powershell
& $exe --once --config .\settings.json --data-dir .\trial-data
```

`--once` runs one measurement cycle and `--headless` keeps running. Both
need an explicit `--config` and a separate `--data-dir`. Neither ever
changes the clock; only the installed, authorised service can do that.
