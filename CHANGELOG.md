# Change history

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
