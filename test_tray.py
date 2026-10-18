import queue
import unittest
from types import SimpleNamespace
from tray import TrayController

class Root:
    def __init__(self): self.mode='iconic'; self.pending=[]
    def bind(self,*a,**kw): pass
    def after(self,*a): pass
    def after_idle(self,fn): self.pending.append(fn)
    def state(self,value=None):
        if value: self.mode=value
        return self.mode
    def withdraw(self): self.mode='withdrawn'
    def deiconify(self): self.mode='normal'
    def lift(self): pass
    def focus_force(self): pass

class TrayTests(unittest.TestCase):
    def make(self, available=True):
        root=Root()
        backend=SimpleNamespace(available=available, events=queue.Queue(), close=lambda:None)
        controller=TrayController(root,lambda:None,lambda:None,SimpleNamespace(set=lambda s:None),backend)
        return root,backend,controller
    def test_minimize_only_hides_with_working_tray_and_failure_restores_window(self):
        root,backend,controller=self.make()
        controller.minimized(SimpleNamespace(widget=root))
        root.pending.pop()()
        self.assertEqual(root.mode,'withdrawn')
        backend.available=False
        controller.poll()
        self.assertEqual(root.mode,'normal')
    def test_failed_tray_keeps_taskbar_window(self):
        root,backend,controller=self.make(False)
        controller.minimized(SimpleNamespace(widget=root))
        self.assertEqual(root.pending,[])
        self.assertEqual(root.mode,'iconic')
    def test_restore_event_is_handled_on_gui_thread(self):
        root,backend,controller=self.make()
        backend.events.put('show')
        controller.poll()
        self.assertEqual(root.mode,'normal')
