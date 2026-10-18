import tempfile
import unittest
from pathlib import Path

from appearance import Appearance, PALETTES, load_theme, save_theme


class AppearanceTests(unittest.TestCase):
    @unittest.skipUnless(__import__('os').name == 'nt', 'Windows window chrome')
    def test_native_title_bars_follow_theme_and_restore(self):
        import ctypes
        from ctypes import wintypes
        import tkinter as tk
        from window_chrome import colorref
        user32 = ctypes.WinDLL('user32')
        user32.GetAncestor.argtypes = [wintypes.HWND, wintypes.UINT]
        user32.GetAncestor.restype = wintypes.HWND
        getter = ctypes.WinDLL('dwmapi').DwmGetWindowAttribute
        getter.argtypes = [wintypes.HWND, wintypes.DWORD, ctypes.c_void_p, wintypes.DWORD]
        getter.restype = ctypes.c_long
        with tempfile.TemporaryDirectory() as folder:
            root = tk.Tk()
            root.withdraw()
            try:
                app = Appearance(root, Path(folder) / 'ui.json')
                settings = tk.Toplevel(root)
                settings.withdraw()
                for theme in ('Dark', 'Light', 'Dark'):
                    app.change(theme)
                    # Applying a theme must leave tray/hidden settings hidden.
                    self.assertEqual(root.state(), 'withdrawn')
                    self.assertEqual(settings.state(), 'withdrawn')
                    for window in (root, settings):
                        window.deiconify()
                        window.update()
                        hwnd = user32.GetAncestor(window.winfo_id(), 2)
                        value = wintypes.DWORD()
                        hr = getter(hwnd, 20, ctypes.byref(value), ctypes.sizeof(value))
                        if hr < 0:
                            hr = getter(hwnd, 19, ctypes.byref(value), ctypes.sizeof(value))
                        if hr < 0:
                            self.skipTest('Immersive title bars unsupported by this Windows version')
                        self.assertEqual(value.value, int(theme == 'Dark'))
                        if getter(hwnd, 35, ctypes.byref(value), ctypes.sizeof(value)) >= 0:
                            self.assertEqual(value.value, colorref(PALETTES[theme]['bg']))
                        if getter(hwnd, 36, ctypes.byref(value), ctypes.sizeof(value)) >= 0:
                            self.assertEqual(value.value, colorref(PALETTES[theme]['text']))
                        window.withdraw()
            finally:
                root.destroy()

    def tearDown(self):
        # Release Tk cycles on the UI thread before later network tests start.
        import gc
        gc.collect()

    def test_missing_corrupt_and_unknown_preferences_default_dark(self):
        with tempfile.TemporaryDirectory() as folder:
            path = Path(folder) / "ui.json"
            self.assertEqual(load_theme(path), "Dark")
            for text in ('bad json', '[]', '{"theme":"unknown"}'):
                path.write_text(text)
                self.assertEqual(load_theme(path), "Dark")

    def test_preference_round_trip_and_invalid_value_preserves_file(self):
        with tempfile.TemporaryDirectory() as folder:
            path = Path(folder) / "nested/ui.json"
            save_theme(path, "Light")
            self.assertEqual(load_theme(path), "Light")
            with self.assertRaises(ValueError):
                save_theme(path, "unknown")
            self.assertEqual(load_theme(path), "Light")
            self.assertFalse(path.with_suffix('.tmp').exists())

    def test_switch_updates_existing_and_new_settings_widgets(self):
        import tkinter as tk
        from tkinter import ttk
        with tempfile.TemporaryDirectory() as folder:
            root = tk.Tk()
            root.withdraw()
            try:
                ttk.Style(root).theme_use("clam")
                app = Appearance(root, Path(folder) / "ui.json")
                from tkinter.scrolledtext import ScrolledText
                ttk.Scrollbar(root)
                ScrolledText(root)
                text = tk.Text(root)
                chart = tk.Canvas(root)
                tree = ttk.Treeview(root)
                settings = tk.Toplevel(root)
                settings.withdraw()
                entry = ttk.Entry(settings)
                for theme in ("Dark", "Light", "Dark"):
                    app.change(theme)
                    p = PALETTES[theme]
                    self.assertEqual(text.cget("background"), p["field"])
                    self.assertEqual(chart.cget("background"), p["field"])
                    self.assertEqual(settings.cget("background"), p["bg"])
                    self.assertEqual(tree.tag_configure("failed")["foreground"], p["error"])
                    self.assertEqual(ttk.Style(root).lookup("TEntry", "fieldbackground"), p["field"])
                    self.assertEqual(load_theme(app.path), theme)
            finally:
                root.destroy()
