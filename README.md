# NetTime Next — Windows service preview 0.2.1

This first working build monitors every configured time source while the existing
NetTime service continues controlling the PC clock. It cannot change the clock.
It is **not yet a complete RFC 5905 synchronization engine**, and **NTS is not
implemented**. Its source recommendation is explicitly experimental.

## Install the Windows build

Run `NetTimeNext-0.2.1-Setup-x64.exe` and approve Windows elevation. Python does
not need to be installed on the destination computer. The installer adds the
automatically starting **NetTime Next Observer** Windows service and a desktop
app accessible from the Start menu. The original NetTime service is not changed.

The service collects while the window is closed and users are logged out.
The desktop reads its history without starting another collector. Configuration
and history are stored in `ProgramData\NetTimeNext`; upgrades and uninstall
preserve them. The service runs as LocalService with access restricted to its
own writable data directory. Saving service settings requests administrator
approval and restarts the observer service. This preview installer is unsigned.

See [Windows service deployment and build details](docs/WINDOWS-SERVICE.md).

Upgrading from 0.2.0 fixes the read-only database startup error. Install over the
existing version; the installer stops/restarts its own service and migrates the
database without clearing history or changing your configured servers.

## Run from source

Double-click **Launch NetTime Next.cmd**. Python 3.11 with Tk is already present
on this machine; no additional packages are required. Keep the application open
to accumulate history in standalone source mode. Closing a **standalone** window
stops its collector; closing the **installed service** window does not. The
development launcher uses this machine's Python path with `python` as a fallback.
On another development machine, copy `settings.example.json` to `settings.json`
and run `python app.py`. Live settings/logs are excluded from Git.

The local development settings match the supplied screenshots:

- `192.0.2.54:123`
- `192.0.2.62:123`
- `time-a-b.nist.gov:123`
- `time-b-b.nist.gov:123`

Every server is checked at a default 64-second interval, concurrently. Server
RATE responses slow its polling, and DENY/RSTR stop its association. Restrictions
persist across restarts. There is no first-success shortcut and no automatic
removal of a failed source. DNS answers rotate between checks; each measurement
records the address actually contacted.

## Settings

The redesigned settings window separates **Servers**, **Security & NTS**,
**Polling**, **Clock correction**, and **History & logs**. Source mode saves for
restart; the installed app applies settings through its service helper. Editing a server requires **Replace selected**;
adding requires **Add server**, followed by **Save for restart**.

- Plain NTP is explicitly labelled **unauthenticated**, in both UI and logs.
- **NTS required** can be reserved per server, but in this build it blocks that
  entry from sending requests. It does not emulate authentication or fall back.
- The NTS page describes the missing RFC 8915 implementation, including TLS key
  establishment and authenticated NTP packets. Server support is not assumed.
- Clock correction is informational and disabled in this observer, including
  when it runs as a Windows service.
- Retention defaults to seven days and cannot be set below seven.

## Measurements and evidence

The table shows stratum, offset, round-trip delay, filtered jitter, reachability,
reference ID, leap indicator, security and the next scheduled check. Select a
server for its history graph and current selection/rejection reason. Full
decision evidence is available in the second tab.

**Positive offset means the server is ahead of this PC.** These are measurements
relative to a clock still disciplined by the current NetTime service. They do
not establish the PC's natural oscillator drift. A reference ID is the server's
advertised identifier, not necessarily a usable upstream hostname. LI values 1/2
announce leap events; LI 3 reports an unsynchronized server.

`ProgramData\NetTimeNext\data\history.sqlite3` in service mode, or local
`data/history.sqlite3` in source mode, stores every attempt (including failures), parsed reply
metadata and raw base-header request/reply bytes; timestamps T1–T4; resolved
endpoint; stratum; security mode; offset and delay; root delay and dispersion;
server control messages; diagnostic candidate metrics; all source reasons;
selection changes; configuration and clock-discontinuity events. Each decision
links to its measurement cycle and identifies the policy version. Jitter is a
filter statistic in the decision record, not a packet field.

Seven days of history **accumulate over time**; a new installation does not
already contain a week of data. Downtime produces gaps. Retention cleanup runs
hourly while collecting; the database can contain slightly more than seven days.
Clock discontinuities delay cleanup to avoid prematurely losing evidence.
Exports are not automatically deleted by rolling retention. SQLite pages may
remain allocated after older rows are removed.

**Export evidence** saves `samples.csv`, `decisions.csv` and `events.csv` from one
database snapshot. Decision/event payloads preserve structured JSON inside CSV.
All persisted timestamps are Unix UTC; graph labels show ISO UTC. No credentials
or NTS keys/cookies are logged. Data remains local.

## Diagnostic recommendation, not clock control

The observer filters up to eight recent valid samples using minimum round-trip
delay, estimates jitter and uncertainty, rejects stale/overly uncertain sources,
and looks for a unique majority of overlapping offset uncertainty intervals. It
then recommends the lowest estimated distance among agreeing sources. Stratum
is recorded and used only as a later tie-break, not as proof of accuracy.

A single candidate is labelled uncorroborated. Conflicting sources can produce
no recommendation. Aliases resolving to the same endpoint do not receive extra
votes. Distinct endpoints do **not** prove independent clocks; the two NIST
entries and the local devices may share upstream references. The complete RFC
selection, cluster, combine and clock-discipline pipeline remains a separate
implementation and validation task. This diagnostic policy must not be passed
off as that implementation.

The observer checks packet size, version, mode, matching originate timestamp,
stratum, leap status and timestamp consistency. Connected UDP limits replies to
the requested endpoint, and client sockets use ephemeral ports. It handles the
2036 era rollover assuming the local date is within 68 years of the real date.
High-resolution steady timing separates network elapsed time from local clock
steps; affected cycles are discarded. This does not detect every tiny clock
change or distinguish which program changed the clock.

Extensions and MACs are deliberately rejected as unsupported rather than
silently ignored. Replies are unauthenticated; header checks do not provide
cryptographic server authentication. See **Security & NTS**.

## Command-line operation

From this folder:

```text
python app.py --once
python app.py --headless
python app.py --export path-to-export-folder
python -m unittest discover -s . -p "test_*.py" -v
```

`python app.py --service-view` connects a source checkout to the installed
service; this requires pywin32. Frozen desktop builds use service view by default.
`--standalone` explicitly requests an independent development collector.

`--once` honors persisted polling restrictions and can report that no checks are
due. `--headless` collects continuously until stopped; it is not an installed
background service. The installer provides the managed Windows service instead.
Use `--data-dir` for an alternative standalone database directory.
One collector may use a data directory at a time.

## Standards and next release gates

- [RFC 5905 — NTPv4](https://www.rfc-editor.org/rfc/rfc5905.html)
- [RFC 8633 — operational best practices](https://www.rfc-editor.org/rfc/rfc8633.html)
- [RFC 9109 — client port randomization](https://www.rfc-editor.org/rfc/rfc9109.html)
- [RFC 8915 — NTS](https://www.rfc-editor.org/rfc/rfc8915.html)

See `PRODUCT-REQUIREMENTS.md` for the agreed direction and acceptance gates.

## Source history and builds

Source is tracked in the private [nettime-next GitHub repository](https://github.com/Optimusprimeums/nettime-next).
The Windows workflow runs tests, builds the installer and checks service
installation, collection, stop/start, upgrade and uninstall. Build artifacts
include a SHA-256 checksum and the source commit. NTS, full synchronization
conformance and signed releases remain separately tracked issues.
