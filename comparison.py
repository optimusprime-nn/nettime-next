"""Windowed, snapshot-consistent server and selection evidence reports."""
from collections import Counter, defaultdict
import csv
import json
import math
from pathlib import Path
import statistics
import time


def percentile(values, fraction):
    if not values:
        return None
    values = sorted(values)
    index = (len(values) - 1) * fraction
    lo, hi = math.floor(index), math.ceil(index)
    return values[lo] + (values[hi] - values[lo]) * (index - lo)


def build_report(store, days=7, now=None):
    now = time.time() if now is None else now
    start = now - days * 86400
    with store.connect() as db:
        db.execute("BEGIN")
        samples = db.execute("SELECT utc,host,port,status,stratum,offset_ms,delay_ms,payload FROM samples WHERE utc>=? AND utc<=? ORDER BY utc,id", (start, now)).fetchall()
        decisions = db.execute("SELECT utc,payload FROM decisions WHERE utc>=? AND utc<=? ORDER BY utc,id", (start, now)).fetchall()
        carry = db.execute("SELECT utc,payload FROM decisions WHERE utc<? ORDER BY utc DESC,id DESC LIMIT 1", (start,)).fetchone()
    grouped, reasons, spans, wins = defaultdict(list), defaultdict(Counter), Counter(), Counter()
    timeline = []
    series = ([carry] if carry else []) + decisions
    for i, (stamp, encoded) in enumerate(series):
        payload = json.loads(encoded)
        selected = payload.get("selected")
        # Evidence expires: do not count service downtime as selected uptime.
        intervals = [s.get("interval_seconds", 64) for s in payload.get("servers", []) if s.get('peer') == selected]
        expires = stamp + 3 * max(intervals, default=64)
        end = min(now, series[i + 1][0] if i + 1 < len(series) else now, expires)
        duration = max(0, end - max(start, stamp))
        if selected:
            spans[selected] += duration
        previous = json.loads(series[i - 1][1]).get("selected") if i else None
        if stamp >= start:
            if selected != previous:
                if selected:
                    wins[selected] += 1
                timeline.append(dict(utc=stamp, selected=selected, previous=previous,
                                     reason=payload.get("explanation", "")))
            for peer, reason in payload.get("reasons", {}).items():
                reasons[peer][reason.split(';')[0].split(' (')[0]] += 1
    for row in samples:
        grouped[f"{row[1]}:{row[2]}"].append(row)
    servers = []
    for peer, rows in sorted(grouped.items()):
        valid = [r for r in rows if r[3] == 'valid' and r[5] is not None and r[6] is not None]
        offsets, delays = [r[5] for r in valid], [r[6] for r in valid]
        strata = [r[4] for r in valid if r[4] is not None]
        authenticated = sum(json.loads(r[7]).get('authentication_status') == 'authenticated' for r in valid)
        gaps = [b[0] - a[0] for a, b in zip(rows, rows[1:])]
        servers.append(dict(peer=peer, attempts=len(rows), valid=len(valid), valid_percent=100 * len(valid) / len(rows),
            first_utc=rows[0][0], last_utc=rows[-1][0], observed_hours=(rows[-1][0] - rows[0][0]) / 3600,
            median_offset_ms=statistics.median(offsets) if offsets else None,
            p95_abs_offset_ms=percentile([abs(v) for v in offsets], .95),
            median_rtt_ms=statistics.median(delays) if delays else None,
            p95_rtt_ms=percentile(delays, .95),
            offset_stddev_ms=statistics.pstdev(offsets) if offsets else None,
            min_stratum=min(strata) if strata else None, max_stratum=max(strata) if strata else None,
            stratum_changes=sum(a != b for a, b in zip(strata, strata[1:])),
            authenticated_valid=authenticated, authenticated_percent=100 * authenticated / len(valid) if valid else None,
            selected_hours=spans[peer] / 3600, selection_entries=wins[peer],
            longest_attempt_gap_seconds=max(gaps, default=0),
            failures=dict(Counter(r[3] for r in rows if r[3] != 'valid')),
            selection_reasons=dict(reasons[peer])))
    return dict(start_utc=start, end_utc=now, requested_days=days, servers=servers, selection_changes=timeline,
                notes=["Valid % is successful recorded attempts; it is not continuous server uptime.",
                       "Observed coverage is first-to-last attempt, including gaps; missing time is not backfilled.",
                       "Offset variation includes local PC drift and corrections; it does not measure physical PPS jitter.",
                       "Selection durations expire after three poll intervals; service downtime is not credited."])


def export_report(report, folder):
    folder = Path(folder)
    folder.mkdir(parents=True, exist_ok=True)
    (folder / "comparison.json").write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    for key, filename in (("servers", "comparison.csv"), ("selection_changes", "selection-changes.csv")):
        records = report[key]
        fields = list(records[0]) if records else (["peer", "attempts", "valid"] if key == "servers" else ["utc", "selected", "previous", "reason"])
        with (folder / filename).open('w', newline='', encoding='utf-8-sig') as file:
            writer = csv.DictWriter(file, fieldnames=fields)
            writer.writeheader()
            for row in records:
                writer.writerow({k: json.dumps(v) if isinstance(v, dict) else v for k, v in row.items()})


def add_dashboard(notebook, store, cfg):
    import tkinter as tk
    from tkinter import ttk
    from tkinter.scrolledtext import ScrolledText
    from app import number, utc
    panel = ttk.Frame(notebook, padding=12)
    notebook.add(panel, text="Source comparison")
    toolbar = ttk.Frame(panel)
    toolbar.pack(fill='x')
    ttk.Label(toolbar, text="Compare recorded evidence over").pack(side='left')
    period = tk.StringVar(value="7 days")
    picker = ttk.Combobox(toolbar, textvariable=period, values=("24 hours", "7 days", "All retained"), state='readonly', width=14)
    picker.pack(side='left', padx=8)
    coverage = tk.StringVar()
    ttk.Label(panel, textvariable=coverage, wraplength=1150).pack(anchor='w', pady=8)
    frame = ttk.Frame(panel)
    frame.pack(fill='x')
    columns = ('peer', 'valid', 'hours', 'offset', 'p95', 'rtt', 'variation', 'stratum', 'changes', 'nts', 'selected')
    table = ttk.Treeview(frame, columns=columns, show='headings', height=min(8, max(4, len(cfg['servers']))))
    for key, title, width in zip(columns,
        ('Server', 'Valid attempts %', 'Coverage h', 'Median offset ms', 'P95 |offset| ms', 'Median RTT ms', 'PC offset SD ms', 'Stratum range', 'Stratum changes', 'NTS valid %', 'Selected h'),
        (200, 120, 100, 130, 130, 120, 130, 110, 120, 100, 100)):
        table.heading(key, text=title)
        table.column(key, width=width, minwidth=width, stretch=key == 'peer')
    table.pack(fill='x')
    scrollbar = ttk.Scrollbar(frame, orient='horizontal', command=table.xview)
    scrollbar.pack(fill='x')
    table.configure(xscrollcommand=scrollbar.set)
    detail = ScrolledText(panel, height=8, font=('Consolas', 10), wrap='word', relief='flat')
    detail.pack(fill='both', expand=True, pady=(10, 0))
    cached = {}

    def show_detail(event=None):
        selected = table.selection()
        row = next((s for s in cached.get('servers', []) if selected and s['peer'] == selected[0]), None)
        lines = list(cached.get('notes', []))
        if row:
            lines += ['', row['peer'], f"Valid {row['valid']} / {row['attempts']} attempts; failures: {row['failures'] or 'none'}",
                      f"Longest gap between attempts: {row['longest_attempt_gap_seconds']:.0f}s; selection entries: {row['selection_entries']}",
                      'Selection/rejection evidence:']
            lines += [f"  {count} decisions: {reason}" for reason, count in row['selection_reasons'].items()]
        lines += ['', 'Selection changes (latest 100):']
        lines += [f"{utc(c['utc'])}  {c['previous'] or 'none'} → {c['selected'] or 'none'}\n  {c['reason']}" for c in cached.get('selection_changes', [])[-100:][::-1]]
        detail.configure(state='normal')
        detail.delete('1.0', 'end')
        detail.insert('end', '\n'.join(lines))
        detail.configure(state='disabled')

    def refresh(event=None):
        nonlocal cached
        days = 1 if period.get() == '24 hours' else cfg['retention_days'] if period.get() == 'All retained' else 7
        try:
            cached = build_report(store, days)
            selected = table.selection()
            table.delete(*table.get_children())
            for row in cached['servers']:
                strata = f"{row['min_stratum']}–{row['max_stratum']}" if row['min_stratum'] is not None else '—'
                table.insert('', 'end', iid=row['peer'], values=(row['peer'], number(row['valid_percent']), number(row['observed_hours']),
                    number(row['median_offset_ms'], True), number(row['p95_abs_offset_ms']), number(row['median_rtt_ms']),
                    number(row['offset_stddev_ms']), strata, row['stratum_changes'], number(row['authenticated_percent']), number(row['selected_hours'])))
            children = table.get_children()
            if children:
                table.selection_set(selected[0] if selected and selected[0] in children else children[0])
            coverage.set(f"{len(cached['servers'])} sources · {len(cached['selection_changes'])} selection changes · a full week accumulates over time; gaps remain visible")
            show_detail()
        except Exception as exc:
            coverage.set(f"Comparison unavailable: {exc}")
    ttk.Button(toolbar, text='Refresh comparison', command=refresh).pack(side='left')
    picker.bind('<<ComboboxSelected>>', refresh)
    table.bind('<<TreeviewSelect>>', show_detail)
    refresh()
    return refresh
