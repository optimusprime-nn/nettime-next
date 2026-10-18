"""NetTime Next observer: python app.py, --once, --headless, or --export PATH."""
from __future__ import annotations

import argparse
import datetime as dt
import json
import os
import queue
import threading
import time
import sys
from pathlib import Path
from types import SimpleNamespace

from monitor import BASE, InstanceLock, Monitor, load_config
from storage import Store
from runtime_paths import VERSION, machine_root
from selection import Peer


def view_state(cfg):
    """A desktop service view has no collector and performs no startup SQL."""
    return SimpleNamespace(peers=[Peer(s["host"], s["port"]) for s in cfg["servers"]])


def utc(value):
    return dt.datetime.fromtimestamp(value, dt.timezone.utc).isoformat(timespec="seconds") if value is not None else "—"


def number(value, signed=False):
    if value is None:
        return "—"
    return f"{value:+.3f}" if signed else f"{value:.3f}"


def ranking_evidence(candidate, latest):
    if not candidate:
        return "No eligible ranking sample. Latest measurements remain in the history."
    parts = candidate.get("distance_components_ms")
    text = (f"Ranking sample: {utc(candidate.get('ranking_sample_utc'))} · age {candidate['sample_age_seconds']:.1f}s · "
            f"offset {number(candidate['offset_ms'], True)} ms · RTT {number(candidate['delay_ms'])} ms.\n"
            f"Latest sample: {utc(latest.get('utc'))} · offset {number(latest.get('offset_ms'), True)} ms · "
            f"RTT {number(latest.get('delay_ms'))} ms.")
    if parts:
        text += (f"\nRanking estimate {candidate['distance_ms']:.3f} ms = "
                 f"network RTT/2 {parts['network_half_rtt_ms']:.3f} + upstream delay/2 {parts['upstream_half_delay_ms']:.3f} + "
                 f"root dispersion {parts['root_dispersion_ms']:.3f} + age allowance {parts['age_allowance_ms']:.3f} + "
                 f"observer guard {parts['observer_precision_guard_ms']:.3f} + client-relative variation {parts['client_offset_variation_ms']:.3f}.")
    else:
        text += "\nLegacy record: component breakdown was not logged by the earlier version."
    return text


def gui(cfg, store, smoke=False, config_path=BASE / "settings.json", service_view=False):
    from ui_dpi import DpiLayout, enable_dpi_awareness
    enable_dpi_awareness()
    import tkinter as tk
    from tkinter import filedialog, messagebox, ttk
    from tkinter.scrolledtext import ScrolledText

    root = tk.Tk()
    dpi_layout = DpiLayout(root)
    root.title(f"NetTime Next {VERSION} · {'Windows service' if service_view else 'Standalone'} observer")
    dpi_layout.size_window(root, 1430, 830, 1060, 680)
    root.configure(bg="#f4f6fa")
    style = ttk.Style(root)
    style.theme_use("clam")
    style.configure("TFrame", background="#f4f6fa")
    style.configure("TLabel", background="#f4f6fa", foreground="#17243b", font=("Segoe UI", 10))
    style.configure("Title.TLabel", font=("Segoe UI", 22, "bold"))
    style.configure("Treeview", rowheight=33, font=("Segoe UI", 10), background="white", fieldbackground="white")
    style.configure("Treeview.Heading", font=("Segoe UI", 10, "bold"), padding=7)
    style.configure("TButton", padding=(12, 7), font=("Segoe UI", 10))

    from appearance import Appearance
    appearance = Appearance(root)

    outer = ttk.Frame(root, padding=20)
    outer.pack(fill="both", expand=True)
    header = ttk.Frame(outer)
    header.pack(fill="x")
    ttk.Label(header, text="NetTime Next", style="Title.TLabel").pack(side="left")
    theme_choice = tk.StringVar(value=appearance.theme)
    theme_picker = ttk.Combobox(header, textvariable=theme_choice, values=("Dark", "Light"), state="readonly", width=8)
    theme_picker.pack(side="right")
    ttk.Label(header, text="Appearance ").pack(side="right")

    def change_theme(event=None):
        try:
            appearance.change(theme_choice.get())
            draw_history()
        except OSError as exc:
            theme_choice.set(appearance.theme)
            messagebox.showerror("Appearance could not be saved", str(exc), parent=root)

    theme_picker.bind("<<ComboboxSelected>>", change_theme)
    mode_description = tk.StringVar(value="MEASUREMENT ONLY | Clock takeover is off | Seven-day rolling evidence")
    ttk.Label(outer, textvariable=mode_description).pack(anchor="w", pady=(4, 12))
    sync_status = tk.StringVar(value="Synchronization assessment: waiting for fresh evidence")
    ttk.Label(outer, textvariable=sync_status, wraplength=1240).pack(anchor="w", pady=(0, 8))
    summary = tk.StringVar(value="Waiting for measurements from every configured server…")
    ttk.Label(outer, textvariable=summary, wraplength=1240, font=("Segoe UI", 11, "bold")).pack(anchor="w")
    explanation = tk.StringVar(value="Recommendations are experimental; no system clock changes are made.")
    ttk.Label(outer, textvariable=explanation, wraplength=1240).pack(anchor="w", pady=(4, 14))

    columns = ("host", "status", "stratum", "offset", "delay", "jitter", "estimate", "age", "reach", "refid", "leap", "security", "next")
    table_frame = ttk.Frame(outer)
    table_frame.pack(fill="x")
    table = ttk.Treeview(table_frame, columns=columns, show="headings", height=max(5, min(10, len(cfg["servers"]))))
    for key, title, width in zip(columns,
            ("Server", "Measurement", "Stratum", "Latest offset ms", "Latest RTT ms", "Client variation ms", "Rank estimate ms", "Rank age s", "Reach", "Advertised refID", "LI", "Security", "Next check"),
            (190, 105, 70, 120, 110, 135, 125, 85, 60, 120, 40, 140, 90)):
        table.heading(key, text=title)
        table.column(key, width=width, minwidth=width, stretch=key in ("host", "refid"))
    table.pack(fill="x")
    scroll = ttk.Scrollbar(table_frame, orient="horizontal", command=table.xview)
    scroll.pack(fill="x")
    table.configure(xscrollcommand=scroll.set)
    ttk.Label(outer, text="Latest values and ranking samples can differ. Client variation includes PC drift/corrections; it is not physical PPS jitter.", wraplength=1250).pack(anchor="w", pady=(5, 0))
    table.tag_configure("selected", background="#e2f5ec", foreground="#145b3b")
    table.tag_configure("failed", foreground="#a33232")
    for server in cfg["servers"]:
        key = f"{server['host']}:{server['port']}"
        table.insert("", "end", iid=key, values=(server["host"], "Waiting", *(["—"] * 9)))

    tabs = ttk.Notebook(outer)
    tabs.pack(fill="both", expand=True, pady=(14, 8))
    history_panel = ttk.Frame(tabs, padding=12)
    details_panel = ttk.Frame(tabs, padding=12)
    tabs.add(history_panel, text="Server history & selection reason")
    tabs.add(details_panel, text="Full decision evidence")
    reason = tk.StringVar(value="Select a server to inspect its history and selection reason.")
    ttk.Label(history_panel, textvariable=reason, wraplength=1200).pack(anchor="w")
    evidence = tk.StringVar()
    ttk.Label(history_panel, textvariable=evidence, wraplength=1200).pack(anchor="w", pady=(6, 0))
    statistics = tk.StringVar()
    ttk.Label(history_panel, textvariable=statistics, wraplength=1200).pack(anchor="w", pady=7)
    canvas = tk.Canvas(history_panel, bg="white", highlightthickness=0, height=190)
    canvas.pack(fill="both", expand=True)
    ttk.Label(history_panel, text="Offset = server minus this PC (positive means PC is behind). History labels use UTC. Gaps break the line.").pack(anchor="w", pady=(7, 0))

    from comparison import add_dashboard

    refresh_comparison = add_dashboard(tabs, store, cfg)
    details = ScrolledText(details_panel, font=("Consolas", 10), wrap="word", relief="flat")
    details.pack(fill="both", expand=True)

    footer = ttk.Frame(outer)
    footer.pack(fill="x")
    status = tk.StringVar(value=f"Checking all sources every {cfg['poll_seconds']:g}s; server restrictions may extend this.")
    ttk.Label(footer, textvariable=status).pack(side="left")
    messages = queue.Queue()
    monitor = view_state(cfg) if service_view else Monitor(cfg, store, messages)
    worker = None
    last = {}
    closing = False
    last_decision_id = 0
    next_service_read = 0
    service_state = "unknown"

    def draw_history(event=None):
        selected = table.selection()
        if not selected:
            return
        key = selected[0]
        peer = next(p for p in monitor.peers if p.key == key)
        server = next((s for s in last.get("servers", []) if s["peer"] == key), None)
        candidate = next((c for c in last.get("candidates", []) if c["peer"] == key), None)
        evidence.set(ranking_evidence(candidate, (server or {}).get("latest") or {}))
        reason.set((server["reason"] if server else peer.disabled or "Waiting for measurement") +
                   "  |  Reach is an octal bitmap of the last eight attempted checks.")
        try:
            stats = next((s for s in store.stats() if s["host"] == peer.host and s["port"] == peer.port), None)
            rows = store.history(peer.host, peer.port)
        except Exception as exc:
            status.set(f"History read failed: {exc}")
            return
        if stats:
            coverage = max(0, stats["last_utc"] - stats["first_utc"]) / 3600
            statistics.set(f"Retained history: {coverage:.1f} hours  ·  Valid {stats['valid']}/{stats['attempts']} attempts  ·  "
                f"Mean offset {number(stats['mean_offset_ms'], True)} ms  ·  "
                f"Range {number(stats['min_offset_ms'], True)} to {number(stats['max_offset_ms'], True)} ms  ·  "
                f"Stratum {stats['min_stratum']}–{stats['max_stratum']}")
        else:
            statistics.set("No measurements stored yet. Seven days of history accumulate while the observer runs.")
        canvas.delete("all")
        colors = appearance.palette
        px = dpi_layout.px
        width, height = max(canvas.winfo_width(), px(300)), max(canvas.winfo_height(), px(70))
        valid = [r for r in rows if r[4] == "valid" and r[1] is not None]
        if not valid:
            canvas.create_text(width / 2, height / 2, text="No valid time measurements yet", fill=colors["muted"])
            return
        xmin, xmax = rows[0][0], max(rows[-1][0], rows[0][0] + 1)
        ymin = min(r[1] for r in valid)
        ymax = max(r[1] for r in valid)
        margin = max((ymax - ymin) * 0.15, 0.05)
        ymin, ymax = ymin - margin, ymax + margin
        left, right, top, bottom = px(80), width - px(25), px(25), height - px(40)
        canvas.create_line(left, top, left, bottom, right, bottom, fill=colors["border"])
        for fraction in (0, .5, 1):
            value = ymin + (ymax - ymin) * fraction
            y = bottom - (bottom - top) * fraction
            canvas.create_line(left, y, right, y, fill=colors["grid"])
            canvas.create_text(left - px(8), y, anchor="e", text=f"{value:+.3f}", fill=colors["muted"])
        canvas.create_text(left, height - px(16), anchor="w", text=utc(xmin), fill=colors["muted"])
        canvas.create_text(right, height - px(16), anchor="e", text=utc(xmax), fill=colors["muted"])
        previous = None
        for stamp, offset, delay, stratum, sample_status in rows:
            if sample_status != "valid" or offset is None:
                previous = None
                continue
            x = left + (stamp - xmin) / (xmax - xmin) * (right - left)
            y = bottom - (offset - ymin) / (ymax - ymin) * (bottom - top)
            if previous and 0 < stamp - previous[2] <= 3 * cfg["poll_seconds"]:
                canvas.create_line(previous[0], previous[1], x, y, fill=colors["accent"], width=px(2))
            canvas.create_oval(x - px(2), y - px(2), x + px(2), y + px(2), fill=colors["accent"], outline="")
            previous = x, y, stamp

    def export():
        folder = filedialog.askdirectory(title="Choose a folder for measurement, decision and event CSV files")
        if folder:
            try:
                store.export(folder)
                from comparison import build_report, export_report

                export_report(build_report(store), folder)

                messagebox.showinfo("Export complete", "Saved raw evidence and seven-day comparison reports\n" + folder)
            except Exception as exc:
                messagebox.showerror("Export failed", str(exc))

    def stop_monitor():
        if service_view:
            from service_client import elevated_command
            try:
                elevated_command("stop" if service_state == "running" else "start")
                status.set("Service state changed; refreshing…")
            except Exception as exc:
                messagebox.showerror("Service change failed", str(exc))
            return
        monitor.stop.set()
        status.set("Stopping after pending measurements finish…")
        stop_button.configure(state="disabled")

    def close():
        nonlocal closing
        if service_view:
            root.destroy()
            return
        closing = True
        stop_monitor()

    def settings():
        from settings_ui import show_settings
        if service_view:
            from service_client import save_service_config
            def saved():
                nonlocal cfg, monitor, last, last_decision_id, next_service_read
                cfg = load_config(config_path)
                monitor = view_state(cfg)
                table.delete(*table.get_children())
                for server in cfg["servers"]:
                    key = f"{server['host']}:{server['port']}"
                    table.insert("", "end", iid=key, values=(server["host"], "Waiting", *(["—"] * 9)))
                last, last_decision_id, next_service_read = {}, 0, 0
                table.selection_set(table.get_children()[0])
                status.set("Service settings applied; waiting for fresh evidence.")
            show_settings(root, load_config(config_path), config_path, store, saved, save_service_config)
        else:
            show_settings(root, cfg, config_path, store, lambda: status.set("Settings saved. Close and reopen to apply; current monitor still uses previous settings."))

    ttk.Button(footer, text="Settings…", command=settings).pack(side="right", padx=(8, 0))
    ttk.Button(footer, text="Export evidence…", command=export).pack(side="right", padx=(8, 0))
    ttk.Button(footer, text="Open data folder", command=lambda: os.startfile(str(store.path.parent))).pack(side="right", padx=(8, 0))
    stop_button = ttk.Button(footer, text="Stop service…" if service_view else "Stop monitoring", command=stop_monitor)
    stop_button.pack(side="right")
    table.bind("<<TreeviewSelect>>", draw_history)
    canvas.bind("<Configure>", draw_history)
    root.protocol("WM_DELETE_WINDOW", close)

    def tick():
        nonlocal last, last_decision_id, next_service_read, service_state
        if service_view and time.perf_counter() >= next_service_read:
            try:
                from service_control import query_status
                service_state = query_status()
                stop_button.configure(text="Stop service…" if service_state == "running" else "Start service…")
                current_id, payload = store.latest_decision()
                if payload is not None and current_id != last_decision_id:
                    last_decision_id = current_id
                    messages.put(("cycle", payload))
                elif last:
                    status.set(f"Service: {service_state}  ·  Last evidence {utc(last['utc'])}  ·  Closing this window keeps collection running")
                else:
                    status.set(f"Service: {service_state}  ·  Waiting for the first measurement cycle")
            except Exception as exc:
                status.set(f"Service data unavailable: {exc}")
            next_service_read = time.perf_counter() + 2
        try:
            while True:
                kind, payload = messages.get_nowait()
                if kind == "cycle":
                    last = payload

                    mode_description.set("GRADUAL CLOCK CORRECTION · NetTime Next owns the clock" if payload.get('clock_mode') == 'slew' else

                                         "MEASUREMENT ONLY · Clock takeover is off · Windows service retains evidence while this window is closed")

                    sync = payload.get('synchronization', {})

                    action = payload.get('clock_action', {})

                    sync_status.set("Synchronization: " + sync.get('explanation', 'Available after upgrading the service') +

                                    " · " + action.get('reason', 'Clock control off'))
                    summary.set(f"Recommended source: {payload['selected'] or 'none'}  ·  {payload['confidence']}")
                    explanation.set(payload["explanation"])
                    details.configure(state="normal")
                    details.delete("1.0", "end")
                    details.insert("1.0", json.dumps(payload, indent=2))
                    details.configure(state="disabled")
                    for server in payload["servers"]:
                        if not table.exists(server["peer"]):
                            continue
                        s = server["latest"] or {}
                        c = next((c for c in payload["candidates"] if c["peer"] == server["peer"]), {})
                        tag = "selected" if payload["selected"] == server["peer"] else "failed" if s.get("status") != "valid" else ""
                        table.item(server["peer"], tags=(tag,), values=(server["peer"], s.get("status", "Waiting"),
                            s.get("stratum") if s.get("stratum") is not None else "—",
                            number(s.get("offset_ms"), True), number(s.get("delay_ms")), number(c.get("jitter_ms")),
                            number(c.get("distance_ms")), number(c.get("sample_age_seconds")),
                            server["reach_octal"], s.get("reference_id", "—"),
                            s.get("leap") if s.get("leap") is not None else "—",
                            "NTS authenticated" if s.get("authentication_status") == "authenticated" else "NTS failed" if s.get("security_mode") == "nts_required" else "Unauthenticated", ""))
                    status.set(f"Last cycle {utc(payload['utc'])}  ·  {len(payload['servers'])} configured sources  ·  History saved")
                    draw_history()

                    refresh_comparison()
                elif kind == "error":
                    status.set(payload)
                    messagebox.showerror("Monitoring stopped", payload)
                elif kind == "stopped":
                    stop_button.configure(state="disabled")
                    if not closing:
                        status.set("Monitoring stopped. Evidence is saved; reopen the app to resume.")
        except queue.Empty:
            pass
        for peer in monitor.peers:
            if service_view:
                snapshot = next((s for s in last.get("servers", []) if s["peer"] == peer.key), None)
                next_utc = snapshot["next_check_utc"] if snapshot else None
                table.set(peer.key, "next", "Stopped" if service_state != "running" else "Blocked" if snapshot and snapshot["disabled"] else
                          f"{max(0, next_utc - time.time()):.0f}s" if next_utc else "Waiting")
                continue
            if peer.disabled and peer.latest is None:
                table.set(peer.key, "status", "Access refused")
            table.set(peer.key, "next", "Stopped" if monitor.stop.is_set() else "Blocked" if peer.disabled else
                      f"{max(0, peer.next_due - time.perf_counter()):.0f}s")
        if closing and (worker is None or not worker.is_alive()):
            root.destroy()
            return
        root.after(500, tick)

    table.selection_set(table.get_children()[0])
    # Reserve footer space before the expanding notebook; it must not disappear
    # when high-DPI fonts and explanatory text need more vertical space.
    footer.pack_configure(side="bottom", before=tabs)
    dpi_layout.apply()
    appearance.apply()
    root.after(500, dpi_layout.watch)
    if smoke:
        root.after(300, root.destroy)
    elif service_view:
        root.after(100, tick)
    else:
        worker = threading.Thread(target=monitor.run, name="ntp-observer", daemon=True)
        worker.start()
        root.after(100, tick)
    root.mainloop()


def main():
    parser = argparse.ArgumentParser(description="Read-only all-source NTP observer")
    parser.add_argument("--config", type=Path, default=BASE / "settings.json")
    parser.add_argument("--data-dir", type=Path, default=BASE / "data")
    parser.add_argument("--once", action="store_true", help="One due measurement cycle; no clock changes")
    parser.add_argument("--headless", action="store_true", help="Continuous foreground collector without GUI")
    parser.add_argument("--export", type=Path, metavar="FOLDER")

    parser.add_argument("--report", type=Path, metavar="FOLDER", help="Export seven-day server comparison and selection timeline")
    parser.add_argument("--smoke-ui", action="store_true", help=argparse.SUPPRESS)
    parser.add_argument("--service-view", action="store_true", help="Read the installed Windows service without starting another collector")
    parser.add_argument("--standalone", action="store_true", help="Run an independent development collector")
    parser.add_argument("--version", action="version", version=VERSION)
    parser.add_argument("--dpi-report", type=Path, help=argparse.SUPPRESS)
    parser.add_argument("--nts-report", type=Path, help=argparse.SUPPRESS)
    args = parser.parse_args()
    if args.nts_report:
        from nts import diagnostics
        args.nts_report.write_text(json.dumps(diagnostics(), indent=2), encoding='utf-8')
        return
    if args.dpi_report:
        from ui_dpi import diagnostics
        args.dpi_report.write_text(json.dumps(diagnostics(), indent=2), encoding="utf-8")
        return
    service_view = args.service_view or (getattr(sys, "frozen", False) and not args.standalone and not args.once and not args.headless)
    if service_view:
        args.config = machine_root() / "settings.json"
        args.data_dir = machine_root() / "data"
    cfg = load_config(args.config)
    store = Store(args.data_dir / "history.sqlite3", cfg["retention_days"], read_only=service_view)

    if args.report:

        from comparison import build_report, export_report

        export_report(build_report(store), args.report)

        print(args.report)

        return
    if args.export:
        print(store.export(args.export))
        return
    if service_view:
        gui(cfg, store, args.smoke_ui, args.config, service_view=True)
        return
    with InstanceLock(args.data_dir / "monitor.lock"):
        if args.once:
            monitor = Monitor(cfg, store)
            decision = monitor.collect()
            print(json.dumps(decision or {"status": "No checks due; persisted polling restrictions honored"}, indent=2))
        elif args.headless:
            monitor = Monitor(cfg, store)
            try:
                monitor.run()
            except KeyboardInterrupt:
                monitor.stop.set()
        else:
            gui(cfg, store, args.smoke_ui, args.config)


def entrypoint():
    try:
        main()
    except Exception as exc:
        import sys
        if not any(a in sys.argv for a in ("--once", "--headless", "--export", "--smoke-ui")):
            from tkinter import messagebox
            messagebox.showerror("NetTime Next", str(exc))
            return 1
        raise
    return 0


if __name__ == "__main__":
    sys.exit(entrypoint())
