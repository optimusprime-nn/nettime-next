"""Match Windows non-client chrome to the desktop's selected appearance."""
import ctypes
import os
from ctypes import wintypes


def colorref(color):
    red, green, blue = (int(color[i:i + 2], 16) for i in (1, 3, 5))
    return red | green << 8 | blue << 16


def apply_title_bar(window, dark, background, foreground):
    if os.name != "nt":
        return
    # Tk's public ID is the client HWND; DWM needs its enclosing top-level HWND.
    # Creating the wrapper does not display a withdrawn settings/tray window.
    window.update_idletasks()
    user32 = ctypes.WinDLL("user32", use_last_error=True)
    user32.GetAncestor.argtypes = [wintypes.HWND, wintypes.UINT]
    user32.GetAncestor.restype = wintypes.HWND
    hwnd = user32.GetAncestor(window.winfo_id(), 2)
    if not hwnd:
        return
    try:
        dwm = ctypes.WinDLL("dwmapi")
        setter = dwm.DwmSetWindowAttribute
        setter.argtypes = [wintypes.HWND, wintypes.DWORD, ctypes.c_void_p, wintypes.DWORD]
        setter.restype = ctypes.c_long
        enabled = wintypes.BOOL(dark)
        if setter(hwnd, 20, ctypes.byref(enabled), ctypes.sizeof(enabled)) < 0:
            # Earlier Windows 10 versions used attribute 19.
            setter(hwnd, 19, ctypes.byref(enabled), ctypes.sizeof(enabled))
        # Windows 11 supports explicit caption/text colors. Older Windows safely
        # reject these attributes while retaining the immersive light/dark mode.
        for attribute, color in ((35, background), (36, foreground)):
            value = wintypes.DWORD(colorref(color))
            setter(hwnd, attribute, ctypes.byref(value), ctypes.sizeof(value))
    except (OSError, AttributeError):
        # Appearance must never prevent startup on older Windows installations.
        return
