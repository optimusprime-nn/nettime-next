# 0.3.1 clock-direction regression

The first deployment verified that Windows accepted an adjustment setting, but
that was insufficient evidence that the clock was being corrected in the intended
direction. The longer run revealed a reversed precise-frequency conversion.

From 04:03–04:47 UTC on 2026-10-18, 26 LAN measurement intervals wholly within
constant adjustment commands showed a regression slope of **+1.00151 offset ppm
per requested ppm**, with approximately **−28.331 ppm** free-running offset slope.
The intended wall-speed actuator should instead produce a negative offset slope
per positive requested wall-speed correction. No local clock discontinuities were
recorded in this interval. The controller ultimately exceeded its 128 ms offset
limit and suspended corrections; free-running drift then accumulated.

The user authorized rollback. The original NetTime and Windows Time service
states were restored on 2026-10-18; NetTime Next remains an observer. History and
the rolled-back ownership journal were preserved. The Rust migration observer
does not call any clock-adjustment API.

In 0.3.2, positive `rate_ppm` consistently means a faster wall clock. The precise
Windows API takes a frequency divisor, so its value is nominal frequency divided
by `(1 + rate_ppm / 1e6)`. Readback uses the reciprocal mapping. Integer rounding
moves towards neutral so the requested rate bound is never exceeded.

Validation now includes a disposable Windows runner test which stops the clock
services, applies zero and both signs of a bounded correction, and measures real
wall-clock progress against QPC. Readback alone is no longer accepted as actuator
direction/gain evidence. The test restores neutral adjustment in `finally` and
the ownership fixture restores the saved service states. It cannot run on an
ordinary user machine.

Passing this test verifies actuator direction/gain. It does not establish full
NTP conformance, long-term loop convergence, or seven-day reliability. Those
remain separate requirements before promoting the native replacement.
