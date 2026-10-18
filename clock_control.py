"""Bounded slew-only clock controller, with an injectable Windows backend.

No clock API runs merely by importing/constructing the preview planner. Only
the service can construct an active controller after administrative takeover.
Large offsets, leap announcements and lost consensus suspend correction.
"""
import ctypes
import ctypes.wintypes as wt
import math
import time


def precise_api():
    # These APIs are exported by KernelBase/API sets, not kernel32.dll.
    # Restrict lookup to system libraries; merely resolving exports is read-only.
    for name in ('api-ms-win-core-sysinfo-l1-2-3.dll', 'kernelbase.dll'):
        try:
            library = ctypes.WinDLL(name, use_last_error=True, winmode=0x800)
            getattr(library, 'GetSystemTimeAdjustmentPrecise')
            getattr(library, 'SetSystemTimeAdjustmentPrecise')
            return library
        except (AttributeError, OSError):
            continue
    raise RuntimeError('Windows precise clock-adjustment API is unavailable')


class SlewPlanner:
    def __init__(self, options):
        self.options = options
        self.last_sample = None
        self.last_offset = None
        self.frequency_ppm = 0
        self.rate_ppm = 0
        self.updates = 0
        self.last_good = None

    def reset(self):
        self.last_sample = self.last_offset = self.last_good = None
        self.frequency_ppm = self.rate_ppm = 0
        self.updates = 0

    def plan(self, assessment, now):
        result = dict(action='suspend', rate_ppm=0.0, reason=assessment.get('explanation', 'No consensus'))
        offset = assessment.get('offset_ms')
        if not assessment.get('ready') or offset is None or not math.isfinite(offset):
            self.reset()
            return result
        if abs(offset) > self.options['max_offset_ms']:
            self.reset()
            result['reason'] = 'Offset exceeds automatic slew limit; manual review required; no clock step'
            return result
        stamp = assessment['sample_mono']
        if now - stamp > self.options['holdover_seconds'] or stamp > now + 1:
            self.reset()
            result['reason'] = 'Filtered sample expired; correction suspended'
            return result
        if self.last_sample is not None and stamp <= self.last_sample:
            return dict(action='hold', rate_ppm=self.rate_ppm, reason='No newer system-peer filter sample; not reused')
        if self.last_sample is not None:
            elapsed = stamp - self.last_sample
            if elapsed > self.options['holdover_seconds']:
                self.reset()
            elif elapsed > 0:
                # Offset slope removes our applied rate; the residual estimates
                # the free-running oscillator error. Low-pass the estimate.
                estimate = (offset - self.last_offset) * 1000 / elapsed + self.rate_ppm
                bound = self.options['max_slew_ppm']
                self.frequency_ppm += .125 * (max(-bound, min(bound, estimate)) - self.frequency_ppm)
        self.last_sample, self.last_offset, self.last_good = stamp, offset, now
        self.updates += 1
        if self.updates < 3:
            self.rate_ppm = 0
            return dict(action='warmup', rate_ppm=0.0, reason=f'Clock filter settling: {self.updates}/3 new system-peer samples')
        bound = self.options['max_slew_ppm']
        self.rate_ppm = max(-bound, min(bound, self.frequency_ppm + offset * 1000 / 1024))
        return dict(action='slew', rate_ppm=self.rate_ppm, frequency_ppm=self.frequency_ppm,
                    offset_ms=offset, reason='Bounded gradual correction; no clock step')


def adjustment_for_rate(increment, ppm):
    """Map positive *wall-clock speed* to the precise API's frequency divisor.

    The precise API is not the legacy per-tick increment API. Increasing its
    frequency value slows wall time. Round towards neutral so quantization
    cannot exceed the requested absolute rate.
    """
    if increment <= 0 or not math.isfinite(ppm) or abs(ppm) > 500:
        raise ValueError('Invalid clock frequency or rate outside 500 ppm bound')
    frequency = increment / (1 + ppm / 1000000)
    return math.ceil(frequency) if ppm >= 0 else math.floor(frequency)


def rate_for_adjustment(increment, adjustment):
    if increment <= 0 or adjustment <= 0:
        raise ValueError('Windows reported an invalid clock frequency')
    return (increment / adjustment - 1) * 1000000


class WindowsClock:
    def __init__(self):
        import win32api
        import win32con
        import win32security
        token = win32security.OpenProcessToken(win32api.GetCurrentProcess(), win32con.TOKEN_ADJUST_PRIVILEGES | win32con.TOKEN_QUERY)
        try:
            privilege = win32security.LookupPrivilegeValue(None, 'SeSystemtimePrivilege')
            win32security.AdjustTokenPrivileges(token, False, [(privilege, win32con.SE_PRIVILEGE_ENABLED)])
            if win32api.GetLastError() == 1300:
                raise PermissionError('Service token does not have permission to adjust system time')
        finally:
            token.Close()
        self.kernel = precise_api()
        self.kernel.GetSystemTimeAdjustmentPrecise.argtypes = [ctypes.POINTER(ctypes.c_ulonglong), ctypes.POINTER(ctypes.c_ulonglong), ctypes.POINTER(wt.BOOL)]
        self.kernel.GetSystemTimeAdjustmentPrecise.restype = wt.BOOL
        self.kernel.SetSystemTimeAdjustmentPrecise.argtypes = [ctypes.c_ulonglong, wt.BOOL]
        self.kernel.SetSystemTimeAdjustmentPrecise.restype = wt.BOOL
        adjustment, increment, disabled = ctypes.c_ulonglong(), ctypes.c_ulonglong(), wt.BOOL()
        if not self.kernel.GetSystemTimeAdjustmentPrecise(ctypes.byref(adjustment), ctypes.byref(increment), ctypes.byref(disabled)):
            raise ctypes.WinError(ctypes.get_last_error())
        self.increment = increment.value
        if not self.increment:
            raise RuntimeError('Windows reported a zero clock increment')

    def rate(self, ppm):
        if not math.isfinite(ppm) or abs(ppm) > 500:
            raise ValueError('Clock rate exceeds 500 ppm safety bound')
        adjustment = adjustment_for_rate(self.increment, ppm)
        if not self.kernel.SetSystemTimeAdjustmentPrecise(adjustment, False):
            raise ctypes.WinError(ctypes.get_last_error())
        actual, increment, disabled = ctypes.c_ulonglong(), ctypes.c_ulonglong(), wt.BOOL()
        if not self.kernel.GetSystemTimeAdjustmentPrecise(ctypes.byref(actual), ctypes.byref(increment), ctypes.byref(disabled)):
            raise ctypes.WinError(ctypes.get_last_error())
        if disabled.value or not increment.value:
            raise RuntimeError('Windows did not enable the requested clock adjustment')
        # Readback is the OS-reported setting, not an external accuracy test.
        return rate_for_adjustment(increment.value, actual.value)

    def restore(self):
        if not self.kernel.SetSystemTimeAdjustmentPrecise(0, True):
            raise ctypes.WinError(ctypes.get_last_error())


class Controller:
    def __init__(self, options, store, backend, guard):
        self.options, self.store, self.backend, self.guard = options, store, backend, guard
        self.planner = SlewPlanner(options)
        self.last_plan = None
        self.suspended = False
        self.next_guard = 0
        # Remove any adjustment left behind by a previous crashed instance.
        self.backend.restore()
        self.store.event('clock_controller_started', {'mode': 'slew', 'limits': options})

    def update(self, assessment, now):
        if not self.guard():
            self.close('Another clock controller is running or takeover authorization was withdrawn')
            raise RuntimeError('Clock ownership conflict; correction stopped')
        plan = self.planner.plan(assessment, now)
        if plan['action'] in ('suspend', 'warmup'):
            self.backend.restore()
            self.suspended = True
        elif plan['action'] == 'slew':
            plan['windows_reported_rate_ppm'] = self.backend.rate(plan['rate_ppm'])
            self.suspended = False
        if plan != self.last_plan:
            self.store.event('clock_correction', dict(plan, assessment=assessment))
            self.last_plan = plan
        return plan

    def watchdog(self, now):
        if now >= self.next_guard:
            self.next_guard = now + 2
            if not self.guard():
                self.close('Clock ownership conflict detected by watchdog')
                raise RuntimeError('Clock ownership conflict; correction stopped')
        if self.planner.last_good is not None and now - self.planner.last_good > self.options['holdover_seconds'] and not self.suspended:
            self.backend.restore()
            self.planner.reset()
            self.suspended = True
            self.store.event('clock_holdover_expired', {'action': 'Restored Windows clock rate; no fresh accepted system-peer sample'})

    def close(self, reason='Service stopping'):
        self.backend.restore()
        self.suspended = True
        self.planner.reset()
        self.store.event('clock_controller_stopped', {'reason': reason})
