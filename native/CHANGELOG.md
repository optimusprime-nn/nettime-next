## 0.4.1-preview

- Ports the bounded clock controller and corrected Windows precise-frequency backend to Rust, with independent ownership/holdover checks and controller convergence simulations.
- Adds administrator-authorized clock takeover, a flushed ownership journal, exact configuration/service restoration, dedicated virtual identity, limited clock privilege and retryable rollback.
- Replaces settings drafts with validated native service administration and explicit start/stop/import/takeover/restore controls.
- Adds source comparison, windowed statistics, selection reasons/timeline, recorded-offset plots and CSV/JSON/full-evidence exports.
- Defaults the native Windows service to automatic observation, protects active ownership during upgrades and restores it before uninstall.
- Adds failure-injection, comparison-window and UTC tests plus installed Windows administration, watchdog and measured physical clock-response validation.

## 0.4.0-dev

- Native observation service, NTP/NTS polling and mitigation, retained SQLite evidence, desktop/tray, draft settings, static-runtime installer and parity checks.
