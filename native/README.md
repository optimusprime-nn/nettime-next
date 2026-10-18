# NetTime Next — Rust migration

This is an additive, native Rust implementation. The installed 0.3.1 service
retains clock ownership while this port is validated. No Python interpreter or
Python subprocess is used by the native binaries.

Implemented: NTPv3/v4 header validation and era handling; NTS-KE with TLS 1.3,
certificate verification and directional exporters; AES-SIV NTP authentication,
cookie renewal and key-exchange backoff; concurrent polling of all configured
sources; persistent RATE/DENY/RSTR restrictions; eight-stage clock filtering,
source-group voting, intersection, clustering and weighted combination;
SQLite evidence with minimum seven-day retention; dark native desktop with
stratum/security/history/evidence and notification-area icon; separate SCM
service host and installable preview.

The native desktop reads the current service history by default. Settings are
edited as an observer draft: they do not silently change the active clock
controller. Native foreground/service collection uses a separate database and
rejects slew mode. The existing database schema and JSON configuration are
retained, but native decision policy identifiers distinguish ported decisions.

Still to port and validate before replacing the active service: Windows clock
adjustment backend, durable takeover/rollback, active settings administration,
the full comparison/export UI, complete automated parity and installed service
lifecycle checks. Full RFC 5905 conformance is not claimed by either preview.
NTS control-response handling needs the same remaining conformance work tracked
in GitHub issue 1. Do not use this preview as a second clock writer.

Build on Windows with the MSVC Rust toolchain:

```
python package.py --iscc C:\path\to\ISCC.exe
```

`NetTimeNextNativeService.exe --help` lists diagnostic and service commands.
Foreground collection requires explicit `--config FILE --data-dir FOLDER`
with `--once` or `--headless`. `--export FOLDER --database FILE` exports JSON.
The optional service is named `NetTimeNextNativePreview`, runs as LocalService,
uses `ProgramData\NetTimeNextNativePreview`, and never changes `NetTimeNext`,
`NetTimeSvc`, `W32Time` or clock privileges. It is registered for manual start.

The installer is a migration preview, not a replacement for the active release.
