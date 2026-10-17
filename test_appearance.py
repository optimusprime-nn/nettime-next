import tempfile
import unittest
from pathlib import Path

from appearance import Appearance, PALETTES, load_theme, save_theme


class AppearanceTests(unittest.TestCase):
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
