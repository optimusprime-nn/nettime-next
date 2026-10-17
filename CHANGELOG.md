# Change history

## 0.2.2 — Auditable ranking and timestamp correction

- Distinguish latest measurements from historical ranking samples in the UI.
- Show ranking age/estimate and log all score components and sample timestamps.
- Label offset variation as client-relative and reference IDs as advertised.
- Preserve physical local T1 instead of replacing it with a randomized request
  timestamp; limit randomization to sub-60-nanosecond fraction bits.
- Document the exact TS1 / GPS-disciplined Raspberry Pi decision in
  `docs/SELECTION-AUDIT.md`. No hardware preference or ranking-weight change.

## 0.2.1 — Read-only database startup fix

- Fix desktop startup failure when users have read-only access to the service's
  history folder and SQLite WAL/SHM files are absent.
- Migrate existing WAL history to rollback journaling during service upgrade,
  preserving settings, measurements and selection evidence.
- Build service-view state without instantiating a collector or running startup
  queries. Database read errors remain recoverable in the status display.
- Show one friendly startup error instead of a second unhandled-exception dialog.
- Add restricted-token database and installed desktop regression checks.

## 0.2.0 — Windows service preview

- Installable x64 Windows build with bundled runtimes and uninstaller.
- Automatic delayed-start observer service, independent of the original NetTime.
- Service recovery, stop/shutdown handling and separate lifecycle/error log.
- LocalService account, service SID and restricted ProgramData data/config ACLs.
- Read-only desktop service view: collection continues after closing the app.
- Elevated, validated configuration application with rollback on startup failure.
- Configuration and evidence preserved across upgrades and uninstall.
- Persistent reach bitmap and previous recommendation across collector restarts.
- Pinned build dependencies, Windows CI, installer lifecycle tests and checksums.
- NTS and full synchronization remain explicitly pending.

## 0.1.0 — Initial source observer

- All-source NTP monitoring, stratum/security table and redesigned settings.
- Seven-day SQLite evidence, CSV exports, offset graph and selection reasons.
- Base-header validation, ephemeral UDP ports and server access/rate controls.
- Experimental source recommendation; no clock-setting functionality.
