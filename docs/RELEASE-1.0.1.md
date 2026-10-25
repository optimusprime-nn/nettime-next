# NetTime Next 1.0.1

Fixes clock correction remaining suspended after an outage lets the offset grow beyond 128 ms. With enough agreeing sources, the service confirms three new filter samples then recovers gradually at the configured rate limit.

The normal tracking threshold remains configurable up to 128 ms. A separate automatic recovery limit defaults to 10 seconds and is configurable up to 60 seconds. Offsets beyond that bound, lost agreement, stale evidence and clock ownership conflicts still stop correction. Existing installations retain settings and ownership. The clock is never stepped.

The overview and evidence identify recovery, confirmation and suspension with the measured offset. Regression simulations cover both offset/drift directions and an outage during recovery.

Hosted Windows lifecycle validation remains paused pending the dedicated remote runner. The installer is unsigned.
