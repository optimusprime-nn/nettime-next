---
layout: default
title: "How correction works"
---
# How correction works

Each check interval (64 seconds by default), NetTime Next runs this pipeline.
Every decision it records says which step stopped a source, or why a source
was chosen.

## 1. Measure

Each source is sent one NTP request (NTPv4,
[RFC 5905](https://www.rfc-editor.org/rfc/rfc5905.html)). The reply gives:

- **offset**: how far the source's time is from this PC's time,
- **round trip**: the network delay,
- **stratum**, **root delay** and **root dispersion**: the server's own
  distance from a reference clock.

A reply is rejected if it doesn't match the request, isn't synchronised,
fails NTS authentication, or has impossible timestamps. A "kiss-o'-death"
reply asking the client to slow down doubles that source's interval. One
that denies access stops polling that source.

**Reach** is an 8-bit record of the last eight attempts; 255 means all eight
succeeded.

## 2. Filter

Each source keeps its last eight valid samples. The filter:

- picks the sample with the **lowest round trip**, since it's the least
  disturbed by network queuing,
- computes **jitter**, the spread of the recent offsets,
- computes **root distance**, an error bound made of half the round trip,
  the server's root delay and dispersion, the filter's own dispersion
  (which grows as samples age), and jitter.

An empty filter slot counts as a very large dispersion. This is why a
source needs about four fresh samples before its root distance falls under
the 1000 ms limit, and why things take a few minutes to settle after a
restart.

A source is not used if:

- its root distance is over **Maximum source distance** (1000 ms by
  default),
- it has fewer than four fresh samples,
- its latest response is too old,
- it has no source group.

## 3. Select

The qualified sources, one per group, go through NTP's selection
algorithm:

- **Intersection.** Each source is an interval: its offset plus or minus
  its root distance. The algorithm finds the largest set of sources whose
  intervals overlap. Sources outside it are rejected as *falsetickers*.
- **Clustering.** The survivors with the most jitter relative to the rest
  are trimmed until the group is tight.
- **Combine.** The remaining offsets are averaged, weighted by distance, to
  give the **combined offset**. The best survivor is the **system peer**.

If fewer than the minimum number of groups (3 by default) survive, there is
no agreement and correction suspends.

The **Selection explanations** panel shows the outcome of every step for
every source.

## 4. Correct

Correction changes the clock's **rate**. It never sets the time directly.
Windows lets the service make the clock tick slightly faster or slower,
measured in parts per million (ppm). 1 ppm is 1 microsecond per second, or
86.4 ms per day.

The rate it applies has two parts:

- **frequency**: an estimate of how fast or slow this PC's crystal runs on
  its own. It's learnt gradually from how the offset changes between
  samples. A typical PC is somewhere within ±100 ppm.
- **phase**: a term proportional to the current offset, which removes about
  1/1024 of the offset each second. A 10 ms offset gives a bit under 10 ppm
  of extra correction.

The total is clamped to **Maximum correction rate** (250 ppm by default). At
that rate the clock gains or loses at most 0.25 ms per second, so a 10 ms
error takes at least 40 seconds to remove.

### Rules that keep correction safe

- **One correction per new sample.** A sample is never reused. Between new
  samples the rate is held.
- **Warm-up.** After the service starts, correction waits for three fresh
  samples from the system peer.
- **Normal tracking limit** (128 ms by default). Within this, correction is
  normal.
- **Large offsets.** Above the tracking limit, the service waits for three
  new samples that agree, then recovers at the maximum correction rate. This
  continues up to the **Automatic recovery limit** (10 seconds by default).
- **Beyond the recovery limit** the service suspends correction and asks for
  manual review. It never steps the clock.
- **Holdover.** If no fresh agreeing sample arrives within **Maximum
  holdover** (900 seconds by default), the service suspends correction and
  puts the clock back to its neutral rate.
- **Leap seconds.** An announced leap second suspends correction; it isn't
  applied.
- **Ownership.** If another clock controller starts, correction stops.

## Typical behaviour

After a restart there are several minutes of settling, then the offset is
brought down over tens of minutes. Meanwhile the frequency estimate
converges on the PC's real drift. On a stable network with nearby stratum-1
sources, a settled system usually stays within about a millisecond of the
agreed time.
