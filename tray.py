"""Windows notification-area icon; Win32 callbacks never touch Tk directly."""
import os
import queue
import threading

from runtime_paths import RESOURCE_DIR


class TrayIcon:
    def __init__(self):
        self.events = queue.Queue()
        self.available = False
        self.ready = threading.Event()
        self.hwnd = None
        self.tooltip = 'NetTime Next'
        self.thread = threading.Thread(target=self.run, name='Desktop-tray', daemon=True)
        self.thread.start()
        self.ready.wait(2)

    def run(self):
        import win32api
        import win32con
        import win32gui
        message = win32con.WM_USER + 20
        taskbar_created = win32gui.RegisterWindowMessage('TaskbarCreated')
        icon = None
        added = False

        def add():
            nonlocal added
            win32gui.Shell_NotifyIcon(win32gui.NIM_ADD,
                (self.hwnd, 0, win32gui.NIF_ICON | win32gui.NIF_MESSAGE | win32gui.NIF_TIP,
                 message, icon, self.tooltip[:127]))
            added = True
            self.available = True

        def handle(hwnd, msg, wparam, lparam):
            if msg == message:
                if lparam == win32con.WM_LBUTTONDBLCLK:
                    self.events.put('show')
                elif lparam in (win32con.WM_RBUTTONUP, win32con.WM_CONTEXTMENU):
                    menu = win32gui.CreatePopupMenu()
                    try:
                        for number, text in ((1, 'Open NetTime Next'), (2, 'Settings'), (3, 'Exit desktop app')):
                            win32gui.AppendMenu(menu, win32con.MF_STRING, number, text)
                        win32gui.SetForegroundWindow(hwnd)
                        x, y = win32gui.GetCursorPos()
                        selected = win32gui.TrackPopupMenu(menu, win32con.TPM_RETURNCMD | win32con.TPM_NONOTIFY,
                                                          x, y, 0, hwnd, None)
                        win32gui.PostMessage(hwnd, win32con.WM_NULL, 0, 0)
                        action = {1: 'show', 2: 'settings', 3: 'exit'}.get(selected)
                        if action:
                            self.events.put(action)
                    finally:
                        win32gui.DestroyMenu(menu)
            elif msg == taskbar_created:
                try:
                    add()
                except Exception:
                    self.available = False
            elif msg == win32con.WM_USER + 21:
                if added:
                    win32gui.Shell_NotifyIcon(win32gui.NIM_MODIFY,
                        (hwnd, 0, win32gui.NIF_TIP, message, icon, self.tooltip[:127]))
            elif msg == win32con.WM_CLOSE:
                win32gui.DestroyWindow(hwnd)
            elif msg == win32con.WM_DESTROY:
                self.available = False
                win32gui.PostQuitMessage(0)
            else:
                return win32gui.DefWindowProc(hwnd, msg, wparam, lparam)
            return 0

        name = f'NetTimeNext.Tray.{os.getpid()}.{id(self)}'
        try:
            instance = win32api.GetModuleHandle(None)
            wc = win32gui.WNDCLASS()
            wc.hInstance, wc.lpszClassName, wc.lpfnWndProc = instance, name, handle
            win32gui.RegisterClass(wc)
            self.hwnd = win32gui.CreateWindow(name, 'NetTime Next tray', 0, 0, 0, 0, 0, 0, 0, instance, None)
            icon = win32gui.LoadImage(0, str(RESOURCE_DIR / 'nettime.ico'), win32con.IMAGE_ICON,
                                     win32api.GetSystemMetrics(win32con.SM_CXSMICON),
                                     win32api.GetSystemMetrics(win32con.SM_CYSMICON), win32con.LR_LOADFROMFILE)
            add()
            self.ready.set()
            win32gui.PumpMessages()
        except Exception as exc:
            self.available = False
            self.events.put(('error', str(exc)))
        finally:
            self.ready.set()
            if added and self.hwnd:
                try:
                    win32gui.Shell_NotifyIcon(win32gui.NIM_DELETE, (self.hwnd, 0))
                except Exception:
                    pass
            if icon:
                win32gui.DestroyIcon(icon)
            try:
                win32gui.UnregisterClass(name, win32api.GetModuleHandle(None))
            except Exception:
                pass

    def update(self, text):
        self.tooltip = text
        if self.available and self.hwnd:
            import win32gui
            win32gui.PostMessage(self.hwnd, 0x0400 + 21, 0, 0)

    def close(self):
        if self.hwnd:
            import win32gui
            try:
                win32gui.PostMessage(self.hwnd, 0x0010, 0, 0)
            except Exception:
                pass
        self.thread.join(timeout=2)


class TrayController:
    def __init__(self, root, show_settings, exit_app, status, backend=None):
        self.root, self.show_settings, self.exit_app, self.status = root, show_settings, exit_app, status
        self.backend = backend or TrayIcon()
        self.hidden = False
        self.closed = False
        root.bind('<Unmap>', self.minimized, add='+')
        root.bind('<Destroy>', self.destroyed, add='+')
        root.after(100, self.poll)

    def minimized(self, event):
        if event.widget is self.root and self.root.state() == 'iconic' and self.backend.available:
            self.root.after_idle(self.hide)

    def hide(self):
        if not self.closed and self.backend.available:
            self.hidden = True
            self.root.withdraw()

    def show(self):
        self.hidden = False
        self.root.deiconify()
        self.root.state('normal')
        self.root.lift()
        self.root.focus_force()

    def poll(self):
        if self.closed:
            return
        try:
            while True:
                action = self.backend.events.get_nowait()
                if action == 'show':
                    self.show()
                elif action == 'settings':
                    self.show()
                    self.show_settings()
                elif action == 'exit':
                    self.exit_app()
                    return
                elif isinstance(action, tuple) and action[0] == 'error':
                    self.status.set('Tray unavailable; window remains accessible: ' + action[1])
                    if self.hidden:
                        self.show()
        except queue.Empty:
            pass
        if self.hidden and not self.backend.available:
            self.show()
        if not self.closed:
            self.root.after(100, self.poll)

    def destroyed(self, event):
        if event.widget is self.root:
            self.closed = True
            self.backend.close()
