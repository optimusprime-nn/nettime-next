# NetTime Next — Windows service preview 0.3.1

NetTime Next checks every configured NTP/NTS source and retains at least seven
days of measurements and selection evidence. The service collects while the
desktop is closed or users are logged out. Installation starts in measurement mode. Explicit administrator-approved takeover
can enable bounded gradual correction. Full conformance and endurance validation
remain outstanding; this is a slew-only preview, not a complete RFC implementation.

## Install and upgrade

Run `NetTimeNext-0.3.1-Setup-x64.exe`. Python is bundled. Windows 10/11 x64 is the
target; installation needs administrator approval. Install over the existing
version to preserve configured servers, settings and history. Installation does
not stop the original NetTime or Windows Time services. This preview is unsigned.

The automatic delayed-start `NetTimeNext` Windows service runs as LocalService
with a service-specific writable data directory. The desktop reads evidence
without starting another collector. Machine configuration and evidence live in
`ProgramData\NetTimeNext`; ordinary users can read them but cannot write them.
Saving service settings requests elevation and restarts this application's service.
Dark mode and native high-DPI rendering remain standard. The Light/Dark selector
saves a per-user preference without restarting the service.

## Source comparison

Open **Source comparison** for 24-hour, seven-day or all-retained summaries:

- Successful recorded attempts, first-to-last coverage and longest attempt gaps.
- Median offset, P95 absolute offset, median/P95 RTT and PC offset standard deviation.
- Stratum range/changes, authenticated-valid percentage, selected duration and reasons.
- Selection changes with the recorded reason for each change.

Valid-attempt percentage is not continuous uptime. Coverage includes gaps;
missing time is not backfilled. Selected durations expire after three selected-source
poll intervals so stale evidence does not credit service downtime. Offset variation
includes PC drift and corrections; it is not physical PPS jitter. A full week
accumulates over time. Retention defaults to seven days and cannot be set lower.

**Export evidence** saves raw `samples.csv`, `decisions.csv`, `events.csv` from
one snapshot, plus `comparison.json`, `comparison.csv` and `selection-changes.csv`
from a separate comparison snapshot. Structured JSON inside CSV preserves full
decision/event evidence. Timestamps and graph labels use UTC. Exports are not
removed by rolling retention. Cleanup runs hourly and pauses after clock jumps.

## NTP and NTS sources

Every entry is checked at a default 64-second interval. RATE responses slow
polling; DENY/RSTR stop associations. Restrictions survive restarts. Failed sources
stay visible. Requests use ephemeral UDP ports; no local NTP server is installed.

In **Settings → Servers**, enter a host, port and security policy, use **Add server**
or **Replace selected**, then **Save & restart service**.

- Plain NTP is unauthenticated; unsupported extensions/MACs fail closed.
- NTS required uses TLS 1.3, certificate/hostname validation, ALPN, exporter-derived
  AES-SIV keys and authenticated NTP. Failures never fall back to plain NTP.
- NTS-KE normally uses TCP 4460. Its authenticated negotiation chooses the actual
  NTP endpoint/port; the configured NTP port identifies the configured entry.
- **Fill NTS example** fills `time.cloudflare.com`, NTS required and group
  `cloudflare`; review and add it before saving. Existing server lists are not
  changed automatically. Netnod publishes `nts.netnod.se` as another NTS endpoint.
- Trust uses bundled Mozilla CA roots. The PC date must permit certificate
  validation; validation is never disabled. Private CA configuration is not provided.
- Cookies are replenished and sessions renewed daily. Secrets stay in memory;
  restart performs fresh key establishment. Retry backoff survives restarts.
  NTS packet evidence contains only the 48-byte header; no keys or reusable cookies
  are logged/exported. Certificate fingerprints, auth outcomes, renewal/reuse status
  and session age are recorded.

## Synchronization assessment and release gate

The existing diagnostic recommendation remains visible for continuity. It uses
minimum RTT and interval-overlap consensus; it is not the clock controller. Latest
measurements and ranking samples remain distinct, with score components. See the
[TS1/Raspberry Pi audit](docs/SELECTION-AUDIT.md).

A separate synchronization assessment now logs eight-stage filtering, aged
dispersion, correctness-interval intersection, clustering and inverse-distance
combination. Candidates, exclusions, survivor weights and cluster removals are
in **Full decision evidence**. This assessment does not change the clock.

Assign an **independent source group** to participate. Shared upstream/provider
means the same group: both NIST endpoints should share `nist`. Distinct IPs,
GPS/PPS reference IDs and stratum 1 do not prove independence. Unassigned sources
remain monitored but do not vote. At least three groups and four fresh filter
samples are required. A separate policy can require NTS for all steering sources.

The implemented slew planner estimates oscillator error, requires three new
system-peer filter samples, caps rate at 250 ppm by default (hard maximum 500),
and suspends above 128 ms offset, on stale evidence, lost consensus, leap notices
or competing clock services. It never steps time. The ownership transaction saves
NetTimeSvc/W32Time startup/running states and this application's configuration,
uses a dedicated virtual service account and grants only that identity clock
permission. Rollback restores saved settings/service states and retains history.

**Takeover requires an explicit action and administrator approval** in Settings
or the service helper. It requires fresh agreement from three configured source
groups and an offset within the 128 ms limit. Installation and settings saves do
not activate correction. Restore previous controller returns saved service states
and configuration. The seven-day campaign and full RFC conformance remain open.
No clock steps are supported; leaps and large errors suspend correction.

## Run and build from source

Use Python 3.11 x64 with Tk on Windows and install pinned dependencies:

```text
python -m pip install -r requirements-build.txt
python app.py --service-view
python app.py --once
python app.py --headless
python app.py --report path-to-report-folder
python app.py --export path-to-raw-export-folder
python -m unittest discover -s . -p "test_*.py" -v
```

Copy `settings.example.json` to `settings.json` for standalone collection. Source
mode never activates correction. Frozen desktop builds use service view by default;
`--standalone` requests a development collector. One collector may use a data
directory at a time. Live settings, databases and secrets are excluded from Git.

See [Windows deployment/build details](docs/WINDOWS-SERVICE.md), the
[validation matrix](docs/SYNCHRONIZATION-PREVIEW.md) and the private
[GitHub repository](https://github.com/Optimusprimeums/nettime-next).
