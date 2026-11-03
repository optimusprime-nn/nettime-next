"""Native DPI rendering and pixel-metric scaling for the Tk desktop only."""
import ctypes
import os
import weakref


def enable_dpi_awareness():
    """Call before creating Tk or any dialogs; the frozen app also has a manifest."""
    if os.name != "nt":
        return
    user = ctypes.WinDLL("user32", use_last_error=True)
    if hasattr(user, "SetProcessDpiAwarenessContext"):
        user.SetProcessDpiAwarenessContext.argtypes = [ctypes.c_void_p]
        user.SetProcessDpiAwarenessContext.restype = ctypes.c_int
        if user.SetProcessDpiAwarenessContext(ctypes.c_void_p(-4)):
            return
        # An existing process manifest can have fixed process awareness already.
        # Setting the GUI thread context also covers source-hosted Python runs.
        user.SetThreadDpiAwarenessContext.argtypes = [ctypes.c_void_p]
        user.SetThreadDpiAwarenessContext.restype = ctypes.c_void_p
        if user.SetThreadDpiAwarenessContext(ctypes.c_void_p(-4)):
            return
    try:
        ctypes.WinDLL("shcore").SetProcessDpiAwareness(2)
    except (AttributeError, OSError):
        user.SetProcessDPIAware()


def window_dpi(window):
    if os.name == "nt":
        user = ctypes.WinDLL("user32", use_last_error=True)
        if hasattr(user, "GetDpiForWindow"):
            user.GetDpiForWindow.argtypes = [ctypes.c_void_p]
            user.GetDpiForWindow.restype = ctypes.c_uint
            dpi = user.GetDpiForWindow(window.winfo_id())
            if dpi:
                return dpi
    return round(window.winfo_fpixels("1i"))


class DpiLayout:
    def __init__(self, root, dpi=None):
        self.root = root
        self.dpi = dpi or window_dpi(root)
        self.scale = self.dpi / 96
        self.widgets = weakref.WeakKeyDictionary()
        root._nettime_dpi = self
        root.tk.call("tk", "scaling", self.dpi / 72)

    def px(self, value):
        return max(1, round(value * self.scale))

    def size_window(self, window, width, height, min_width, min_height):
        available_w = max(320, window.winfo_screenwidth() - self.px(48))
        available_h = max(240, window.winfo_screenheight() - self.px(90))
        window.geometry(f"{min(self.px(width), available_w)}x{min(self.px(height), available_h)}")
        window.minsize(min(self.px(min_width), available_w), min(self.px(min_height), available_h))

    def apply(self, dpi=None):
        from tkinter import font, ttk
        self.dpi = dpi or self.dpi
        self.scale = self.dpi / 96
        self.root.tk.call("tk", "scaling", self.dpi / 72)
        for name in font.names(self.root):
            current = font.nametofont(name, root=self.root)
            current.configure(size=current.cget("size"))
        style = ttk.Style(self.root)
        style.configure("Treeview", rowheight=max(self.px(33), font.Font(self.root, font=("Segoe UI", 10)).metrics("linespace") + self.px(10)))
        style.configure("Treeview.Heading", padding=self.px(7))
        style.configure("TButton", padding=(self.px(12), self.px(7)))
        pending = [self.root]
        while pending:
            widget = pending.pop()
            if not widget.winfo_exists():
                continue
            pending.extend(widget.winfo_children())
            if widget not in self.widgets:
                metrics = {}
                if widget.winfo_class() in ("TFrame", "TLabel"):
                    padding = widget.cget("padding")
                    if padding:
                        metrics["padding"] = tuple(widget.winfo_pixels(str(v)) for v in self.root.tk.splitlist(padding))
                if widget.winfo_class() == "TLabel" and widget.cget("wraplength"):
                    wrap = widget.winfo_pixels(str(widget.cget("wraplength")))
                    if wrap > 0:
                        metrics["wraplength"] = wrap
                if widget.winfo_class() == "Canvas":
                    metrics["height"] = widget.winfo_pixels(str(widget.cget("height")))
                if widget.winfo_class() == "Treeview":
                    metrics["columns"] = {key: (widget.column(key, "width"), widget.column(key, "minwidth")) for key in widget["columns"]}
                self.widgets[widget] = metrics
            metrics = self.widgets[widget]
            for key, value in metrics.items():
                if key == "columns":
                    for column, (width, minimum) in value.items():
                        widget.column(column, width=self.px(width), minwidth=self.px(minimum))
                elif key == "padding":
                    widget.configure(padding=tuple(round(v * self.scale) for v in value))
                else:
                    widget.configure(**{key: self.px(value)})

    def watch(self):
        if not self.root.winfo_exists():
            return
        focused = self.root.grab_current() or self.root
        dpi = window_dpi(focused.winfo_toplevel())
        if dpi != self.dpi:
            self.apply(dpi)
        self.root.after(500, self.watch)


def diagnostics():
    import tkinter as tk
    enable_dpi_awareness()
    root = tk.Tk()
    root.withdraw()
    try:
        layout = DpiLayout(root)
        result = dict(window_dpi=layout.dpi, tk_pixels_per_inch=root.winfo_fpixels("1i"),
                      tk_scaling=float(root.tk.call("tk", "scaling")), tk_version=root.tk.call("info", "patchlevel"))
        if os.name == "nt":
            user = ctypes.WinDLL("user32")
            user.GetThreadDpiAwarenessContext.restype = ctypes.c_void_p
            user.AreDpiAwarenessContextsEqual.argtypes = [ctypes.c_void_p, ctypes.c_void_p]
            result["per_monitor_v2"] = bool(user.AreDpiAwarenessContextsEqual(user.GetThreadDpiAwarenessContext(), ctypes.c_void_p(-4)))
        return result
    finally:
        root.destroy()
