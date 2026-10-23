# NetTime Next — native Rust preview 0.4.4

Native Rust desktop and Windows service, with no Python runtime or Python
subprocess. The MSVC runtime is linked statically and packaging checks the
actual executable import tables. This remains an unsigned preview; full RFC
5905 conformance and seven-day clock-control endurance are still outstanding.
The desktop uses native Direct3D with a built-in Windows software adapter
fallback, including server or remote sessions without an OpenGL driver.
`--software-rendering` explicitly exercises that fallback in launch tests.

The desktop has an Overview dashboard, Source comparison, and grouped Settings.
It displays clock mode, source agreement, all configured endpoints and retained
history with UTC labels. Technical evidence is available in a collapsed panel.
Dark/light appearance and interface size are saved for the current Windows user;
`--compact` opens a smaller window, with summary cards adapting to its width.
Servers can be moved up and down in Settings; Save and apply preserves that order,
and Overview follows it by default through its Configured order option.
Settings drafts survive page changes. Save and apply is the explicit service configuration action.
Minimise keeps the desktop on the taskbar. X hides it in the system tray. Left-click its clock icon
to reopen it; right-click for Open, Settings and Exit desktop app. Exit closes
only the desktop. If the tray cannot be created, X closes the desktop normally
and minimising leaves it accessible on the taskbar.
To also show the desktop's tray icon at Windows sign-in, run
`native/Configure-TrayStartup.ps1` from the source checkout. It creates a
current-user Startup shortcut configured to launch the installed desktop
minimised; the Windows service remains a separate automatic boot service.

## Clock ownership

The service measures every configured NTP/NTS source, defaults to seven days of
SQLite evidence, and starts in observation mode. Installing it
does not take the clock away from the original NetTime or Windows Time services.

Settings includes **Take over clock** and **Restore previous clocks**. Both
request Windows administrator approval. Takeover requires current independent
source agreement within the configured slew limit. Before changing anything,
it flushes an ownership journal containing the previous configuration bytes,
service startup/running states, account and existing clock privilege. It stops
and disables competing controllers (`NetTimeSvc`, `W32Time` and `NetTimeNext`),
then starts the native service under its own virtual account with the time
privilege confined to that account. The desktop never adjusts the clock.

The controller uses the corrected reciprocal Windows precise-frequency
conversion. Corrections are gradual and bounded, with fresh-filter warmup,
stale-sample checks, an independent ownership/holdover watchdog, leap and
consensus suspension, and neutral-rate restoration on shutdown. There are no
clock steps. Windows rate readback is logged alongside each correction; it is
not itself evidence of actual convergence. A disposable CI test measures real
wall-clock response against QPC for both correction signs. Failure injection
tests verify each takeover mutation and retryable rollback.

Restore previous clocks reinstates exact saved configuration bytes and prior
service states and removes only the clock right added by the transaction.
Incomplete restoration retains the journal for retry. Upgrades pause and resume
only the native collector and preserve an existing active ownership transaction.
Uninstall restores ownership before removing
the native service; history and settings are retained.

## Settings and comparison

Settings edits the native Windows service configuration: source address,
NTP port, NTS policy and key-exchange port, independent source group, polling,
timeout, retention, source-distance and clock limits. **Save and apply** validates
the request before stopping a running service and restores the previous
configuration on failure. Saving preserves existing clock ownership; saving
without active ownership stays
in observation mode. Only the separate takeover action grants ownership. While
controlling, changed sources must pass an independent response/authentication
check before the collector stops. Failed checks leave the live collector running;
failed restarts restore the exact previous settings and restart the collector.
Interrupted updates are recovered by the next administrator action. Fresh filters
are required after restart.

The desktop shows only this service history. Source comparison supports one hour, 24 hours, seven days and all retained history, including
valid-attempt percentage, coverage, median and P95 offset/delay, offset variation,
stratum range/changes, authentication coverage, selected duration, failures,
selection/rejection reasons and a selection-change timeline. Offset plots show
recorded measurements without bridging gaps. CSV/JSON comparison exports and
compressed full-history JSON exports run in the background and report their folder.
Reports use one read snapshot, exclude future records, and expire selection
duration after three poll intervals rather than crediting service downtime.

## Install and commands

Run `NetTimeNext-Rust-0.4.4-Preview-Setup-x64.exe`. The default service task
registers and starts `NetTimeNextNativePreview` automatically as LocalService,
using `ProgramData\NetTimeNextNativePreview`. The existing service name is kept
for preview upgrade compatibility. Native evidence stays in its own database;
the desktop no longer offers a previous-service history chooser.

`NetTimeNextNativeService.exe --help` lists commands. Administrative commands:

```
--apply-config FILE
--import-settings
--takeover
--rollback
--start-service
--stop-service
--health
```

`--compare-export FOLDER --days 7 --database FILE` exports a comparison.
`--export FOLDER --database FILE` exports full evidence. Foreground collection
requires `--config FILE --data-dir FOLDER` with `--once` or `--headless`, rejects
clock-control mode and refuses either installed service database.

Build with the MSVC Rust toolchain and Inno Setup:

```
python package.py --iscc C:\path\to\ISCC.exe
```

The Python command is build tooling only. CI tests the installed service,
administration, virtual identity and privilege, independent watchdog, physical
actuator response, rollback, desktop screens, reports, uninstall and retained
history on a disposable Windows runner. Synthetic lifecycle evidence is confined
to that runner; no valid NTP sample fixtures can drive its controller.

NTP/NTS transport, filtering and mitigation parity tests remain in place. NTS
control-response handling still needs the remaining conformance work tracked in
GitHub issue 1. This port does not claim completion of that wider protocol work.

## History storage policy

Settings > Collection & history includes a logging switch, optional detailed
diagnostics and retention from 1 to 3650 days (default 7). Turning logging off
stops new sample, decision and event history; live status and minimal
peer/ownership safety state continue independently. Existing history still expires.
Normal decisions reference measured sources without duplicating every server
snapshot, survivors reference candidate names, and repeated hold events are
suppressed. Diagnostic mode includes packet headers and full decision snapshots.

A background worker runs on startup/settings restart and hourly. It purges
records strictly older than the configured retention cutoff, compresses retained
JSON payloads older than 24 hours with gzip, and reclaims freed database pages
when worthwhile. Compression is stored inside SQLite; numerical indexes stay
queryable. Graphs, comparisons and exports transparently decode older payloads.
Short transactions share the database with collection; compaction has a five
second execution limit and interrupted compaction rolls back for a later pass.
The desktop warns and asks for confirmation before a retention reduction.
Compressed records follow the same purge cutoff as recent records.

Full history exports are samples.json.gz, decisions.json.gz and events.json.gz.
Existing exported files are not automatically modified or deleted. CLI maintenance:
`--maintain-history --database FILE --days DAYS` (administrator rights required
for a live installed database). Manual retention changes permanently remove older
records; export first when needed. The original ownership journal is never pruned.
