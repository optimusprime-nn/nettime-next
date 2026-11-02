---
layout: default
title: "History and exports"
---
# History and exports

## What is recorded

The service keeps a SQLite database at
`C:\ProgramData\NetTimeNextNativePreview\data\history.sqlite3` with:

- **samples**: every measurement attempt for every source, valid or not,
- **decisions**: the selection result and clock action for each cycle,
  including the reason,
- **events**: service starts and stops, settings changes, takeover and
  rollback, maintenance, and correction state changes.

The desktop app opens it read-only. Only the service writes to it.

## Retention and compression

- History is on by default and kept for **7 days**. You can set 1–3650 days.
- Records older than **24 hours** are compressed inside the database. Graphs
  and reports read compressed records the same way as recent ones.
- Cleanup runs after the service starts, after settings change, and then
  hourly. It deletes records past the retention limit and reclaims disk
  space.
- The clock ownership journal is stored separately and is **never** purged
  by retention.

With **Record history and event logs** turned off, only the current status
and the minimum state needed for safety are kept. Clock control carries on
as normal. Existing history still expires at the retention limit.

**Detailed diagnostic logging** adds packet headers and full decision
snapshots to every record. Use it while investigating a problem, then turn
it off again.

## Viewing history

- **Overview**: the current state, the combined offset, each source's
  latest result, and why each source is or isn't selected.
- **Source history**: a graph of one source's offsets over a chosen period
  (for example the last 24 hours, 7 days, or all retained history).
- **Source comparison**: per-source statistics over a period, such as valid
  responses, NTS-authenticated responses, median offset and round trip,
  failures, stratum changes and the longest gap. Also a timeline of selection
  changes.

## Exporting

From the app, **Export reports** writes to `Documents\NetTime Next Reports`:

- **Comparison** as CSV and JSON summaries for the chosen period.

From the command line (see [Command-line reference](Command-line-reference)):

| Command | Output |
| --- | --- |
| `--export FOLDER --database FILE` | Full history: `samples.json.gz`, `decisions.json.gz`, `events.json.gz` |
| `--compare-export FOLDER --days 7 --database FILE` | Comparison report for the last N days |

Exported files are yours to manage. The service never deletes them.

## Disk space

The database grows with the number of sources, the check interval and the
retention period. Detailed diagnostic logging makes it several times larger.
Compression after 24 hours keeps older records compact.
