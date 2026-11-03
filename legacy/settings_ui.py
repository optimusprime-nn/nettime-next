"""Settings are explicit about operational controls versus future capabilities."""
import copy
import json
from pathlib import Path

from monitor import validate_config


def show_settings(parent, cfg, config_path, store, on_save, save_handler=None):
    import tkinter as tk
    from tkinter import messagebox, ttk

    window = tk.Toplevel(parent)
    window.title("NetTime Next · Settings")
    dpi_layout = getattr(parent, "_nettime_dpi", None)
    if dpi_layout:
        dpi_layout.size_window(window, 1000, 730, 900, 650)
    else:
        window.geometry("1000x730")
        window.minsize(900, 650)
    window.transient(parent)
    window.grab_set()
    draft = copy.deepcopy(cfg)
    body = ttk.Frame(window, padding=20)
    body.pack(fill="both", expand=True)
    ttk.Label(body, text="Settings", font=("Segoe UI", 20, "bold")).pack(anchor="w")
    ttk.Label(body, text="Saving restarts the observer service and needs administrator approval." if save_handler else
              "Applies after reopening the observer. NetTime remains responsible for the PC clock.").pack(anchor="w", pady=(4, 14))
    notebook = ttk.Notebook(body)
    notebook.pack(fill="both", expand=True)
    pages = {}
    for name in ("Servers", "Security & NTS", "Polling", "Clock correction", "History & logs"):
        pages[name] = ttk.Frame(notebook, padding=18)
        notebook.add(pages[name], text=name)

    servers = pages["Servers"]
    ttk.Label(servers, text="Every configured server is checked, including sources that are not recommended.").pack(anchor="w", pady=(0, 10))
    listing = ttk.Treeview(servers, columns=("host", "port", "security"), show="headings", height=6)
    for name, text, width in (("host", "Hostname / IP address", 350), ("port", "NTP port", 90), ("security", "Security policy", 310)):
        listing.heading(name, text=text)
        listing.column(name, width=width)
    listing.pack(fill="x")
    edit = ttk.Frame(servers)
    edit.pack(fill="x", pady=12)
    host, port, mode = tk.StringVar(), tk.StringVar(value="123"), tk.StringVar(value="Plain NTP · unauthenticated")
    ttk.Label(edit, text="Hostname / IP").grid(row=0, column=0, sticky="w")
    ttk.Label(edit, text="NTP port").grid(row=0, column=1, sticky="w", padx=8)
    ttk.Label(edit, text="Security policy").grid(row=0, column=2, sticky="w")
    ttk.Entry(edit, textvariable=host, width=32).grid(row=1, column=0, sticky="ew")
    ttk.Entry(edit, textvariable=port, width=8).grid(row=1, column=1, padx=8)
    ttk.Combobox(edit, textvariable=mode, state="readonly", width=38,
                 values=("Plain NTP · unauthenticated", "NTS required · authenticated")).grid(row=1, column=2, sticky="ew")
    ttk.Label(servers, text="NTS verifies the server certificate and authenticates time replies. Failures never fall back to plain NTP.\nFor NTS, the authenticated key exchange chooses the actual NTP endpoint and port.",
              wraplength=860, style="Warning.TLabel").pack(anchor="w")


    group = tk.StringVar()

    ke_port = tk.StringVar(value="4460")

    ttk.Label(edit, text="Independent source group").grid(row=2, column=0, sticky="w", pady=(8, 0))

    ttk.Entry(edit, textvariable=group, width=32).grid(row=3, column=0, sticky="ew")

    ttk.Label(edit, text="NTS-KE TCP port").grid(row=2, column=1, columnspan=2, sticky="w", padx=8, pady=(8, 0))

    ttk.Entry(edit, textvariable=ke_port, width=8).grid(row=3, column=1, padx=8)

    ttk.Label(edit, text="Shared upstream/provider = same group", wraplength=320).grid(row=3, column=2, sticky="w")

    def refresh():
        listing.delete(*listing.get_children())
        for i, s in enumerate(draft["servers"]):
            listing.insert("", "end", iid=str(i), values=(s["host"], s["port"],
                ("NTS required" if s["security"] == "nts_required" else "Plain NTP") + " | " + (s.get("source_group") or "group unassigned")))

    def select(event=None):
        if listing.selection():
            s = draft["servers"][int(listing.selection()[0])]
            host.set(s["host"])
            port.set(str(s["port"]))

            group.set(s.get("source_group", ""))

            ke_port.set(str(s.get("nts_ke_port", 4460)))
            mode.set("NTS required · authenticated" if s["security"] == "nts_required" else "Plain NTP · unauthenticated")

    def put(replace=False):
        try:
            value = dict(host=host.get().strip(), port=int(port.get()),
                         security="nts_required" if mode.get().startswith("NTS") else "plain_ntp",

                         source_group=group.get().strip(), nts_ke_port=int(ke_port.get()))
            attempt = copy.deepcopy(draft)
            if replace:
                if not listing.selection():
                    raise ValueError("Select the server to replace")
                attempt["servers"][int(listing.selection()[0])] = value
            else:
                attempt["servers"].append(value)
            validate_config(attempt)
            draft["servers"] = attempt["servers"]
            refresh()
        except (ValueError, TypeError) as exc:
            messagebox.showerror("Check server settings", str(exc), parent=window)

    def remove():
        if listing.selection():
            if len(draft["servers"]) == 1:
                messagebox.showerror("Keep one server", "At least one server is required.", parent=window)
                return
            draft["servers"].pop(int(listing.selection()[0]))
            refresh()

    buttons = ttk.Frame(servers)
    buttons.pack(fill="x", pady=12)
    ttk.Button(buttons, text="Add server", command=put).pack(side="left")
    ttk.Button(buttons, text="Replace selected", command=lambda: put(True)).pack(side="left", padx=8)
    ttk.Button(buttons, text="Remove selected", command=remove).pack(side="left")

    def add_nts_example():

        host.set("time.cloudflare.com")

        port.set("123")

        ke_port.set("4460")

        group.set("cloudflare")

        mode.set("NTS required · authenticated")

    ttk.Button(buttons, text="Fill NTS example", command=add_nts_example).pack(side="left", padx=8)
    listing.bind("<<TreeviewSelect>>", select)
    refresh()

    security = pages["Security & NTS"]
    for heading, text in (
        ("NTS authentication", "TLS 1.3 and certificate/hostname verification protect key establishment. AES-SIV authenticates NTP replies; request identifiers prevent replay. Cookies are replenished and keys renewed. No plain-NTP fallback is allowed for an NTS entry."),

        ("Trust and connectivity", "Uses the bundled Mozilla CA roots. Certificates must be valid at the PC's current date. NTS-KE normally uses TCP 4460; the authenticated negotiation selects the NTP endpoint. Certificate and authentication errors remain in history."),

        ("Secrets and logs", "Session keys and reusable cookies stay in memory and are never exported. Packet evidence contains only the NTP header for NTS. A service restart performs fresh key establishment. Retry backoff survives restarts."),

        ("Independence and accuracy", "Authentication does not establish clock accuracy or independent upstreams. Assign a group to each known independent source: for example TS1, Raspberry Pi and NIST. Two NIST endpoints share one group. Unassigned groups remain monitored but cannot steer the clock.")):
        ttk.Label(security, text=heading, font=("Segoe UI", 11, "bold")).pack(anchor="w", pady=(0, 5))
        ttk.Label(security, text=text, wraplength=860).pack(anchor="w", pady=(0, 15))

    fields = {}

    def field(page, name, label, note):
        frame = ttk.Frame(page)
        frame.pack(fill="x", pady=(0, 15))
        ttk.Label(frame, text=label, font=("Segoe UI", 10, "bold")).pack(anchor="w")
        value = tk.StringVar(value=str(cfg[name]))
        fields[name] = value
        ttk.Entry(frame, textvariable=value, width=15).pack(anchor="w", pady=5)
        ttk.Label(frame, text=note, wraplength=850).pack(anchor="w")

    polling = pages["Polling"]
    field(polling, "poll_seconds", "Check interval · seconds", "Default 64 seconds per server. Every entry is checked; no first-success shortcut. Range: 64–131072 seconds.")
    field(polling, "timeout_seconds", "Response timeout · seconds", "Default 3 seconds. Failures remain visible and do not remove a server from monitoring.")
    field(polling, "max_distance_ms", "Diagnostic uncertainty limit · milliseconds", "Rejects sources whose estimated uncertainty exceeds this limit. Synchronization assessment separately filters, intersects, clusters and combines independent source groups.")
    ttk.Label(polling, text="Server RATE responses increase that server's interval. DENY/RSTR stop its association.\nRestrictions persist across restarts; a refresh cannot bypass them.", wraplength=860).pack(anchor="w")

    clock = pages["Clock correction"]
    from takeover import readiness, read_journal, TAKEOVER_RELEASE_ENABLED

    from runtime_paths import machine_root

    _, last_decision = store.latest_decision()

    ready, ready_reason = readiness(cfg, last_decision)

    active = cfg['clock']['mode'] == 'slew'

    ttk.Label(clock, text="Gradual clock correction active" if active else "Measurement only · takeover is off", font=("Segoe UI", 13, "bold")).pack(anchor="w")

    ttk.Label(clock, text=ready_reason, wraplength=850).pack(anchor="w", pady=10)
    if not TAKEOVER_RELEASE_ENABLED:
        ttk.Label(clock, text="Takeover is locked in this preview until NTP conformance and seven-day reliability checks are complete. The assessment engine runs and logs its evidence in measurement mode.",
                  style='Warning.TLabel', wraplength=850).pack(anchor='w', pady=(0, 10))

    ttk.Label(clock, text="Preview synchronization engine: requires at least three independent source groups, four fresh filter samples and three new system-peer updates before correction. Leap announcements or large errors suspend correction. No automatic clock steps.", wraplength=850).pack(anchor="w", pady=(0, 10))

    limits = ttk.Frame(clock)

    limits.pack(fill="x")

    clock_fields = {}

    for row, (key, label) in enumerate((("min_sources", "Minimum source groups"), ("max_offset_ms", "Maximum automatic offset · ms"), ("max_slew_ppm", "Maximum correction rate · ppm"), ("holdover_seconds", "Maximum sample/holdover age · seconds"))):

        ttk.Label(limits, text=label).grid(row=row, column=0, sticky="w", pady=4)

        value = tk.StringVar(value=str(cfg['clock'][key]))

        clock_fields[key] = value

        ttk.Entry(limits, textvariable=value, width=12).grid(row=row, column=1, padx=15, pady=4)

    require_nts = tk.BooleanVar(value=cfg['clock']['require_nts'])

    ttk.Checkbutton(clock, text="Only NTS-authenticated sources may steer the clock", variable=require_nts).pack(anchor="w", pady=10)

    ttk.Label(clock, text="Takeover stops and disables NetTimeSvc and Windows Time, saves their startup/running states and gives this service its own clock permission. Restore returns the saved settings and service states. Other clock software must be stopped separately.", wraplength=850).pack(anchor="w", pady=8)

    ownership = ttk.Frame(clock)

    ownership.pack(fill="x")


    def ownership_change(command):

        from service_client import elevated_command

        question = ("Stop the original NetTime and Windows Time services and let NetTime Next gradually adjust your clock? Saved service states can be restored. This is a preview synchronization engine.\n\nSave any edited settings before takeover." if command == 'takeover' else

                    "Stop NetTime Next clock correction and restore the previous clock services and settings?")

        if not messagebox.askyesno("Clock ownership", question, parent=window):

            return

        try:

            elevated_command(command)

            on_save()

            window.destroy()

        except Exception as exc:

            messagebox.showerror("Clock ownership change failed", str(exc), parent=window)

    ttk.Button(ownership, text="Take over clock control…", command=lambda: ownership_change('takeover'),

               state='normal' if TAKEOVER_RELEASE_ENABLED and save_handler and ready and not active else 'disabled').pack(side='left')

    journal = read_journal(machine_root()) if save_handler else None

    ttk.Button(ownership, text="Restore previous controller…", command=lambda: ownership_change('rollback'),

               state='normal' if journal and journal.get('phase') != 'rolled_back' else 'disabled').pack(side='left', padx=8)

    history = pages["History & logs"]
    field(history, "retention_days", "Rolling retention · days", "Default and minimum: 7 days. Measured history accumulates while the observer runs; downtime is not backfilled.")
    ttk.Label(history, text="Always captured", font=("Segoe UI", 11, "bold")).pack(anchor="w", pady=(4, 7))
    ttk.Label(history, text="Every attempt and reply; stratum and its changes; leap status; reference ID; offset; delay; root delay/dispersion; filter jitter; failures; source recommendations; all acceptance/rejection reasons; security status; clock discontinuities; configuration and monitoring events.", wraplength=850).pack(anchor="w")
    ttk.Label(history, text="Evidence is saved to a local SQLite database. CSV export includes samples, decisions and events.\nDetailed evidence logging is standard; there is no 'Normal' level that hides selection reasons.", wraplength=850).pack(anchor="w", pady=16)
    ttk.Label(history, text=f"Data location: {store.path}", wraplength=850).pack(anchor="w")

    footer = ttk.Frame(body)
    footer.pack(fill="x", pady=(14, 0))

    def save():
        try:
            for key, value in fields.items():
                draft[key] = float(value.get())

            for key, value in clock_fields.items():

                draft["clock"][key] = float(value.get())

            draft["clock"]["require_nts"] = require_nts.get()
            validate_config(draft)
            if save_handler:
                save_handler(draft)
                on_save()
                window.destroy()
                return
            path = Path(config_path)
            temp = path.with_suffix(".json.tmp")
            temp.write_text(json.dumps(draft, indent=2) + "\n", encoding="utf-8")
            temp.replace(path)
            store.event("settings_saved_for_restart", {"old": cfg, "new": draft})
            on_save()
            window.destroy()
        except (ValueError, OSError) as exc:
            messagebox.showerror("Settings could not be saved", str(exc), parent=window)

    ttk.Label(footer, text="Changing settings does not activate takeover.").pack(side="left")
    ttk.Button(footer, text="Save & restart service…" if save_handler else "Save for restart", command=save).pack(side="right", padx=(8, 0))
    ttk.Button(footer, text="Cancel", command=window.destroy).pack(side="right")
    if dpi_layout:
        dpi_layout.apply()
    appearance = getattr(parent, "_nettime_appearance", None)
    if appearance:
        appearance.apply(window)
    return window
