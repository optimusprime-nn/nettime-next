"""Per-user desktop appearance; independent of service configuration."""
import json
import os
from pathlib import Path
import tkinter as tk
from tkinter import ttk

PALETTES = {
    "Dark": dict(bg="#121923", field="#1b2533", text="#e6edf7", muted="#a9b9cc",
                 border="#39495e", grid="#2c394b", accent="#80b4ff", selected="#284867",
                 good="#183e32", good_text="#b2eed0", error="#ffabab", warning="#f4c584"),
    "Light": dict(bg="#f4f6fa", field="#ffffff", text="#17243b", muted="#52627b",
                  border="#a9b5c8", grid="#e8ecf3", accent="#2864c5", selected="#c9def6",
                  good="#e2f5ec", good_text="#145b3b", error="#a33232", warning="#925424"),
}


def preference_path():
    return Path(os.environ.get("LOCALAPPDATA", Path.home() / "AppData/Local")) / "NetTimeNext/ui.json"


def load_theme(path):
    try:
        value = json.loads(Path(path).read_text(encoding="utf-8")).get("theme")
        return value if value in PALETTES else "Dark"
    except (OSError, ValueError, AttributeError, TypeError):
        return "Dark"


def save_theme(path, theme):
    if theme not in PALETTES:
        raise ValueError("Unknown appearance")
    path = Path(path)
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_suffix(".tmp")
    temporary.write_text(json.dumps({"theme": theme}) + "\n", encoding="utf-8")
    temporary.replace(path)


class Appearance:
    def __init__(self, root, path=None):
        self.root = root
        root._nettime_appearance = self
        self.path = Path(path) if path is not None else preference_path()
        self.theme = load_theme(self.path)
        self.palette = PALETTES[self.theme]

    def change(self, theme):
        save_theme(self.path, theme)
        self.theme = theme
        self.palette = PALETTES[theme]
        self.apply()

    def apply(self, window=None):
        p = self.palette
        style = ttk.Style(self.root)
        style.configure(".", background=p["bg"], foreground=p["text"],
                        bordercolor=p["border"], lightcolor=p["border"], darkcolor=p["border"],
                        troughcolor=p["bg"], selectbackground=p["selected"], selectforeground=p["text"])
        for name in ("TFrame", "TLabel", "Title.TLabel", "TNotebook"):
            style.configure(name, background=p["bg"], foreground=p["text"])
        style.configure("Warning.TLabel", foreground=p["warning"])
        for name in ("TButton", "TNotebook.Tab", "Treeview.Heading", "TScrollbar"):
            style.configure(name, background=p["field"], foreground=p["text"], arrowcolor=p["text"])
            style.map(name, background=[("active", p["selected"]), ("selected", p["selected"])],
                      foreground=[("disabled", p["muted"])])
        for name in ("TEntry", "TCombobox", "TSpinbox"):
            style.configure(name, fieldbackground=p["field"], background=p["field"],
                            foreground=p["text"], insertcolor=p["text"], arrowcolor=p["text"])
            style.map(name, fieldbackground=[("readonly", p["field"]), ("disabled", p["bg"])],
                      foreground=[("readonly", p["text"]), ("disabled", p["muted"])],
                      selectbackground=[("readonly", p["selected"])])
        style.configure("Treeview", background=p["field"], fieldbackground=p["field"], foreground=p["text"])
        style.map("Treeview", background=[("selected", p["selected"])], foreground=[("selected", p["text"])])
        for option, value in (("background", p["field"]), ("foreground", p["text"]),
                              ("selectBackground", p["selected"]), ("selectForeground", p["text"])):
            self.root.option_add("*TCombobox*Listbox." + option, value)

        def visit(widget):
            if isinstance(widget, (tk.Tk, tk.Toplevel, tk.Frame)):
                widget.configure(background=p["bg"])
            if isinstance(widget, tk.Text):
                widget.configure(background=p["field"], foreground=p["text"], insertbackground=p["text"],
                                 selectbackground=p["selected"], selectforeground=p["text"])
            if isinstance(widget, tk.Scrollbar) and not isinstance(widget, ttk.Scrollbar):
                widget.configure(background=p["field"], troughcolor=p["bg"], activebackground=p["selected"])
            if isinstance(widget, tk.Canvas):
                widget.configure(background=p["field"])
            if isinstance(widget, ttk.Treeview):
                widget.tag_configure("selected", background=p["good"], foreground=p["good_text"])
                widget.tag_configure("failed", foreground=p["error"])
            for child in widget.winfo_children():
                visit(child)
        visit(window or self.root)
