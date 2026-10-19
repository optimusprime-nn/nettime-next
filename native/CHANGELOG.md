## 0.4.2-preview

- Redesigns the desktop with sidebar navigation, source and clock summary cards, a sortable live source table, and a dedicated settings page.
- Adds source-history ranges, UTC chart labels, readable source-selection and clock-action explanations, and collapsed technical evidence.
- Groups settings into Time sources, Collection & history, Clock control, Windows service, and Appearance; keeps unsaved edits separate from the selected history source.
- Adds persisted per-user dark/light appearance and interface scaling, with native Windows UI fonts and matching title-bar themes.
- Reworks source reports around a multi-source plot, compact quality table, detailed source statistics, failure/reason lists and a selection timeline; keeps CSV/JSON and full-evidence exports.
- Preserves collector protocol, logging, clock limits, ownership and rollback behavior; upgrades leave the observer in its existing mode.

## 0.4.1-preview

- Ports the bounded clock controller and corrected Windows precise-frequency backend to Rust, with independent ownership/holdover checks and controller convergence simulations.
- Adds administrator-authorized clock takeover, a flushed ownership journal, exact configuration/service restoration, dedicated virtual identity, limited clock privilege and retryable rollback.
- Replaces settings drafts with validated native service administration and explicit start/stop/import/takeover/restore controls.
- Adds source comparison, windowed statistics, selection reasons/timeline, recorded-offset plots and CSV/JSON/full-evidence exports.
- Defaults the native Windows service to automatic observation, protects active ownership during upgrades and restores it before uninstall.
- Uses Direct3D with the Windows software-rendering adapter so server/remote sessions do not depend on an OpenGL driver; desktop smoke tests wait for actual process exit.
- Adds failure-injection, comparison-window and UTC tests plus installed Windows administration, watchdog and measured physical clock-response validation.

## 0.4.0-dev

- Native observation service, NTP/NTS polling and mitigation, retained SQLite evidence, desktop/tray, draft settings, static-runtime installer and parity checks.
