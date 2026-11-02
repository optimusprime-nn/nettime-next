---
layout: default
title: "Time sources and NTS"
---
# Time sources and NTS

## Default sources

New installations use three public NTS sources from three separate
operators:

| Host | Operator | Source group |
| --- | --- | --- |
| `time.cloudflare.com` | [Cloudflare](https://developers.cloudflare.com/time-services/ntp/usage/) | `cloudflare` |
| `nts.netnod.se` | [Netnod](https://www.netnod.se/nts/network-time-security) | `netnod` |
| `time.dfm.dk` | [DFM](https://dfm.dk/ntp/) | `dfm` |

All three are NTS-required. Change them under **Settings > Time sources** to
suit your location or network. An upgrade never replaces sources you've
already configured.

## Adding a source

Each source has:

- **Server name**: a host name or IP address.
- **NTP port**: normally 123.
- **Security**: *NTS required* or *Plain NTP · unauthenticated*.
- **NTS-KE port**: the NTS key-exchange port, normally 4460. It is only used
  when NTS is required.
- **Independent source group**: see below.

You can configure 1–32 sources. Use **Move up** and **Move down** to change
the order they are listed in. The order is only for display; it doesn't
make a source preferred.

## Source groups

A source group says which endpoints share an upstream clock. **Each group
gets one vote** when deciding whether sources agree, whatever the number of
endpoints in it.

- Put endpoints from the same operator or the same upstream in the **same
  group**. For example, two servers fed by one GPS receiver belong in one
  group.
- Give genuinely independent sources **different groups**.
- A source with **no group** is still measured, but it never votes.

Group names are your assertion. NetTime Next can't verify that two sources
are physically independent, so an honest grouping matters more than adding
many endpoints. Clock control needs at least three groups to agree; this
minimum is 3 by default and can be set from 3 to 32.

When a group has several qualified sources, it votes through the one with
the smallest root distance (its estimated error bound).

## NTS (Network Time Security)

NTS, defined in [RFC 8915](https://www.rfc-editor.org/rfc/rfc8915.html),
authenticates NTP replies so they can't be forged or altered in transit:

1. A TLS 1.3 key exchange runs on the NTS-KE port. The server's certificate
   is checked against the system's trusted roots and the host name.
2. Each NTP request and reply then carries authenticated extension fields
   (AES-SIV), using keys from that exchange.
3. The server hands out encrypted cookies so later requests don't need a new
   TLS handshake.

How NetTime Next handles NTS:

- A source marked **NTS required** never falls back to plain NTP. If NTS
  fails, the source simply produces no valid sample.
- A failed key exchange is retried with increasing backoff.
- Session keys and cookies are kept in memory only. They are never written
  to disk or to logs.
- Certificate checks need the PC's date to be roughly right to begin with.
  If the clock is wildly wrong (days or years), NTS sources will fail until
  the date is corrected.

Turn on **Require NTS-authenticated sources for clock control** under
Settings > Clock control to stop plain-NTP sources from voting at all. They are still
measured and displayed.

## What authentication does and doesn't prove

Authentication proves the reply came from the server you named and wasn't
altered. It doesn't prove that server's clock is right. That's why
NetTime Next also requires agreement between independent groups before it
corrects anything.

## Network requirements

- UDP 123 outbound to each source, and replies back.
- TCP 4460 outbound (or the configured NTS-KE port) for NTS sources.
- DNS resolution for host names. NetTime Next uses the first address the
  name resolves to.
