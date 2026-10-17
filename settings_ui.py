"""Settings are explicit about operational controls versus future capabilities."""
import copy
import json
from pathlib import Path

from monitor import validate_config


def show_settings(parent, cfg, config_path, store, on_save):
    import tkinter as tk
    from tkinter import messagebox, ttk

    window = tk.Toplevel(parent)
    window.title("NetTime Next · Settings")
    window.geometry("1000x730")
    window.minsize(900, 650)
    window.transient(parent)
    window.grab_set()
    draft = copy.deepcopy(cfg)
    body = ttk.Frame(window, padding=20)
    body.pack(fill="both", expand=True)
    ttk.Label(body, text="Settings", font=("Segoe UI", 20, "bold")).pack(anchor="w")
    ttk.Label(body, text="Applies after reopening the observer. NetTime remains responsible for the PC clock.").pack(anchor="w", pady=(4, 14))
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
                 values=("Plain NTP · unauthenticated", "NTS required · blocked until supported")).grid(row=1, column=2, sticky="ew")
    ttk.Label(servers, text="NTS required is a fail-closed reservation in this observer: it sends no packets to that entry.\nNTS key exchange and authenticated packets are not implemented yet.",
              wraplength=860, foreground="#925424").pack(anchor="w")

    def refresh():
        listing.delete(*listing.get_children())
        for i, s in enumerate(draft["servers"]):
            listing.insert("", "end", iid=str(i), values=(s["host"], s["port"],
                "NTS required · blocked" if s["security"] == "nts_required" else "Plain NTP · unauthenticated"))

    def select(event=None):
        if listing.selection():
            s = draft["servers"][int(listing.selection()[0])]
            host.set(s["host"])
            port.set(str(s["port"]))
            mode.set("NTS required · blocked until supported" if s["security"] == "nts_required" else "Plain NTP · unauthenticated")

    def put(replace=False):
        try:
            value = dict(host=host.get().strip(), port=int(port.get()),
                         security="nts_required" if mode.get().startswith("NTS") else "plain_ntp")
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
    listing.bind("<<TreeviewSelect>>", select)
    refresh()

    security = pages["Security & NTS"]
    for heading, text in (
        ("Current observer: unauthenticated NTP", "A valid NTP reply is not proof of server identity. The main table and each logged sample identify unauthenticated measurements."),
        ("NTS required: fail closed", "Choose this per server on the Servers tab. Until NTS is implemented, that entry is marked security-blocked. No plaintext fallback occurs."),
        ("NTS implementation requirement · RFC 8915", "NTS-KE over TLS, trusted certificates with hostname validation, negotiated time endpoint, authenticated NTP packets, replay protection, cookie renewal and key rotation. The default NTS-KE port is TCP 4460; the negotiated NTP endpoint may differ."),
        ("Security history", "Record authentication outcome, key-exchange and certificate failures, policy changes and any explicitly configured fallback. Never write session keys or reusable NTS cookies to logs."),
        ("Source identity and accuracy", "NTS authenticates a source. It does not establish that its clock is accurate or independent of other sources. Stratum, delay, jitter and agreement remain separate checks.")):
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
    field(polling, "max_distance_ms", "Diagnostic uncertainty limit · milliseconds", "Used by the experimental recommendation policy. Full RFC 5905 filtering, clustering and clock discipline are still pending.")
    ttk.Label(polling, text="Server RATE responses increase that server's interval. DENY/RSTR stop its association.\nRestrictions persist across restarts; a refresh cannot bypass them.", wraplength=860).pack(anchor="w")

    clock = pages["Clock correction"]
    ttk.Label(clock, text="Measurement only", font=("Segoe UI", 13, "bold")).pack(anchor="w")
    ttk.Label(clock, text="The current NetTime service continues controlling your clock. This observer has no clock-setting capability, installs no service and opens no NTP server port.", wraplength=850).pack(anchor="w", pady=12)
    ttk.Label(clock, text="For the synchronization release", font=("Segoe UI", 11, "bold")).pack(anchor="w", pady=(12, 8))
    ttk.Label(clock, text="• Explicit takeover from the existing clock controller\n• Gradual correction for small errors; defined handling of large steps\n• Maximum correction and holdover limits with units and clear actions\n• Full NTP mitigation and discipline validation before enabling correction\n• Separate local-server settings, disabled by default", wraplength=850).pack(anchor="w")

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
            validate_config(draft)
            path = Path(config_path)
            temp = path.with_suffix(".json.tmp")
            temp.write_text(json.dumps(draft, indent=2) + "\n", encoding="utf-8")
            temp.replace(path)
            store.event("settings_saved_for_restart", {"old": cfg, "new": draft})
            on_save()
            window.destroy()
        except (ValueError, OSError) as exc:
            messagebox.showerror("Settings could not be saved", str(exc), parent=window)

    ttk.Label(footer, text="No clock-control settings are applied to the running NetTime service.").pack(side="left")
    ttk.Button(footer, text="Save for restart", command=save).pack(side="right", padx=(8, 0))
    ttk.Button(footer, text="Cancel", command=window.destroy).pack(side="right")
    return window
