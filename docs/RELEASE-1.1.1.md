# NetTime Next 1.1.1

A fix release for how quickly the clock settles after a restart or after the
clock is set.

## Clock correction

- **The learned clock drift is kept.** Every PC's clock crystal runs slightly
  fast or slow. NetTime Next measures this drift while it tracks the time. In
  1.1.0 that estimate was thrown away whenever the clock was set or correction
  paused, so the clock drifted freely while measurements rebuilt and then took
  about an hour to relearn. On a typical PC that showed as an offset of around
  10 ms after a step.
- **It is saved and reloaded.** Once learned (about 17 minutes of tracking), the
  drift is saved to `data\clock-drift.json` at most once an hour, and loaded
  when the service starts if it is under 30 days old and within the rate limit.
- **It runs straight away.** After a start or a step, the known drift rate is
  applied at once while fresh measurements rebuild, bounded by the 15-minute
  holdover and the existing watchdog. If sources never agree, correction falls
  back to neutral as before.

In simulation, a clock running 25 ppm fast now stays within 1 ms through a step
and the rebuild, against about 6.4 ms of added error in 1.1.0.

## Validation

- 82 unit tests, including five new tests for keeping, saving, loading and
  rejecting saved drift values, and the step simulation above.
- Not yet done: the physical clock-response and install/upgrade tests in a
  disposable Windows VM, which are waiting on the dedicated runner.

A newly upgraded PC has no saved drift yet. It saves one after its first ~17
minutes of normal tracking; restarts and steps after that settle within minutes.

The installer is unsigned. Windows may show an unknown-publisher warning. Verify
the SHA-256 against `SHA256SUMS.txt`.
