import os
import unittest
import xml.etree.ElementTree as ET
from pathlib import Path

from ui_dpi import DpiLayout, diagnostics, enable_dpi_awareness


class ManifestTests(unittest.TestCase):
    def test_desktop_manifest_requests_native_dpi_without_elevation(self):
        manifest = ET.parse(Path(__file__).parent / "desktop.manifest")
        self.assertEqual(manifest.find(".//{http://schemas.microsoft.com/SMI/2016/WindowsSettings}dpiAwareness").text, "PerMonitorV2")
        self.assertEqual(manifest.find(".//{http://schemas.microsoft.com/SMI/2005/WindowsSettings}dpiAware").text, "true/pm")
        level = manifest.find(".//{urn:schemas-microsoft-com:asm.v3}requestedExecutionLevel")
        self.assertEqual(level.get("level"), "asInvoker")


@unittest.skipUnless(os.name == "nt", "Windows DPI and Tk rendering")
class DpiLayoutTests(unittest.TestCase):
    def setUp(self):
        import tkinter as tk
        enable_dpi_awareness()
        self.root = tk.Tk()
        self.root.withdraw()

    def tearDown(self):
        self.root.destroy()

    def test_pixel_metrics_scale_once_at_standard_display_scales(self):
        from tkinter import ttk
        for dpi in (96, 120, 144, 192):
            with self.subTest(dpi=dpi):
                panel = ttk.Frame(self.root, padding=20)
                panel.pack()
                label = ttk.Label(panel, text="Source details", wraplength=400)
                label.pack()
                table = ttk.Treeview(panel, columns=("source",), show="headings")
                table.column("source", width=100, minwidth=80)
                table.pack()
                layout = DpiLayout(self.root, dpi=dpi)
                layout.apply()
                layout.apply()
                self.assertEqual(table.column("source", "width"), round(100 * dpi / 96))
                self.assertEqual(int(label.cget("wraplength")), round(400 * dpi / 96))
                self.assertLess(abs(self.root.winfo_fpixels("1i") - dpi), 2)
                panel.destroy()

    def test_monitor_change_updates_existing_font_and_table(self):
        from tkinter import font, ttk
        layout = DpiLayout(self.root, dpi=96)
        text_font = font.Font(self.root, family="Segoe UI", size=10)
        table = ttk.Treeview(self.root, columns=("source",))
        table.column("source", width=100, minwidth=80)
        table.pack()
        layout.apply()
        normal = text_font.metrics("linespace")
        layout.apply(192)
        self.assertEqual(table.column("source", "width"), 200)
        self.assertGreaterEqual(text_font.metrics("linespace"), normal * 1.8)
        self.assertGreaterEqual(int(ttk.Style(self.root).lookup("Treeview", "rowheight")), text_font.metrics("linespace"))

    def test_windows_and_tk_dpi_agree(self):
        result = diagnostics()
        self.assertTrue(result["per_monitor_v2"])
        self.assertLess(abs(result["tk_pixels_per_inch"] - result["window_dpi"]), 2)


if __name__ == "__main__":
    unittest.main(verbosity=2)
