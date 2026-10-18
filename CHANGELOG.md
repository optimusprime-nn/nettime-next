# Change history

## 0.3.0 — Source comparison, authenticated NTS and synchronization assessment

- Compare 24-hour, seven-day or retained evidence: valid attempts, coverage/gaps, median/P95 offsets and RTT, PC offset variation, stratum changes, authentication and selection duration/reasons. Export comparison JSON/CSV and selection changes.
- Implement NTS-KE with TLS 1.3, CA and hostname validation, ALPN, exporter-derived AES-SIV keys, negotiated NTP endpoints, identifiers, cookie renewal, daily session renewal and restart-persistent retry backoff. No plain-NTP fallback; no keys/cookies in logs.
- Add a separate eight-stage NTP mitigation assessment with aged dispersion, correctness-interval selection, clustering and weighted combination. Log all candidates, reasons, weights and cluster removals. Source groups prevent shared upstreams receiving extra votes; unassigned groups are monitored but excluded from synchronization.
- Implement bounded slew planning, drift estimation, warmup, stale-sample/conflict watchdog and a durable takeover/rollback transaction using a dedicated virtual service account. **Takeover is release-gated off**, pending the conformance/endurance matrix. No clock steps or local NTP serving.
- Add adversarial NTS/TLS, synthetic drift, comparison and transaction tests, plus a disposable Windows ownership/privilege/rollback integration fixture. Existing settings and history are retained.

## 0.2.4 — Dark desktop appearance

- Default dark appearance for the main window, settings, server tables, chart and decision evidence.
- Light/Dark selector remembers a per-user choice independently of service configuration, without elevation or service restart.
- Retains native high-DPI rendering from 0.2.3.

## 0.2.3 — Native high-DPI desktop rendering

- Embed Per-Monitor V2 DPI awareness in the desktop executable so Windows does
  not enlarge an unaware bitmap of its text and controls.
- Enable equivalent awareness for source runs before creating Tk windows.
- Match Tk point scaling to window DPI; scale rows, columns, padding, chart
  margins and settings-window geometry. Refresh metrics when window DPI changes.
- Reserve footer space before the expanding history panel so action controls
  remain accessible at increased display scaling.
- Check 100%, 125%, 150% and 200% layout metrics; verify the packaged manifest
  and the installed app's actual Windows/Tk DPI state in CI.

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
