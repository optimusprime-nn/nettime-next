# Windows service preview 0.3.0

The installer bundles Python and all runtime dependencies. A separate Python
installation is not required on the destination computer. Windows 10/11 x64 is
the current build target; ARM64 native builds are not provided.

## Installed layout

- `Program Files\NetTime Next\App`: desktop app and its private runtime.
- `Program Files\NetTime Next\Service`: service host and administrative helper.
- `ProgramData\NetTimeNext\settings.json`: machine configuration.
- `ProgramData\NetTimeNext\data\history.sqlite3`: rolling measurement evidence.
- `ProgramData\NetTimeNext\data\service.log`: service lifecycle/error log, rotated
  daily with seven backups. Detailed selection evidence is in SQLite.

`NetTimeNext` is installed with automatic delayed startup. It runs as LocalService
with a service SID. Administrators and SYSTEM own configuration; ordinary users
can read it and history. The service SID has write permission on its data
directory. The desktop opens SQLite read-only, does not take the collector lock,
and does not start a second collector. Closing it leaves collection running.

Starting/stopping the service or applying settings uses an administrative helper
with Windows elevation. Saving settings validates the file, stops the service,
atomically replaces configuration, records the change and restarts a previously
running service. On failure it restores the previous configuration. Application
settings are never interpreted as executable code. The app refreshes its server
list after applying changes.

The service is configured to restart after failures with backoff. Startup errors
and unexpected monitor failures propagate to the Service Control Manager.
Stop/shutdown requests stop the collector and preserve committed evidence. DNS
lookup uses the Windows resolver, so stop can wait for an in-flight lookup.

## Coexistence

This preview remains **measurement only** behind a clock-takeover release gate.
It does not stop the original NetTime/W32Time services or bind UDP 123. NTS
authentication is now implemented and uses ephemeral UDP ports after TLS key
establishment. Existing clock controllers continue operating. Source comparison
and synchronization assessments are recorded without adjusting the clock.
See [the validation matrix](SYNCHRONIZATION-PREVIEW.md) for the implemented but
release-gated controller and ownership/rollback transaction.

## Upgrade and uninstall

The installer stops its own service before replacing files, preserves existing
machine configuration and history, then registers/starts the new build. It refuses
to replace a service whose executable is different from the expected target.
Uninstall stops/removes the service and installed executables; configuration and
evidence remain in ProgramData. This preserves diagnostic data deliberately.

## Build and verification

Install the exact dependencies in `requirements-build.txt` into a Python 3.11 x64
virtual environment, obtain the signed Inno Setup compiler with
`scripts/Get-InnoSetup.ps1`, then run:

```text
python build.py --iscc path-to-ISCC.exe
```

The build runs tests, creates the app/service executables, includes runtime
license notices and produces `dist/NetTimeNext-0.3.0-Setup-x64.exe`, a checksum and
a manifest identifying the source commit. Builds are traceable to source and
pinned dependencies; byte-for-byte reproducible binaries are not claimed.

The Windows GitHub workflow builds the installer and tests fresh install,
collection, service stop/start and uninstall with an offline NTS-blocked fixture.
`scripts/Test-InstalledBuild.ps1` requires administrator rights and refuses to
overwrite an existing observer installation or settings. Its optional
`-KeepInstalled` is intended for an explicitly requested development deployment.

CI also verifies bundled TLS/AES-SIV/certificate dependencies and, only on
disposable GitHub-hosted runners, real Windows takeover/virtual-account privilege
and rollback without usable clock samples. The frozen helper keeps takeover
locked. Ordinary installer tests never change another clock service.

This is an unsigned preview. Full NTP conformance and seven-day reliability
validation remain tracked work; NTS interoperability alone does not establish
those properties.
