# Dependency review for 1.0.0

Checked 2026-10-22. The 340 registry packages in the release Cargo.lock were
queried against the OSV advisory service by package name and exact version.

The release updates webpki-roots to 1.0.10 and rustls-webpki to 0.103.16 for
the emSign certificate distrust fixes. Packaging rejects earlier versions.
The repository's older Python build requirements also update certifi to 2026.10.19;
the Windows release does not bundle or run Python.

References:

- https://rustsec.org/advisories/RUSTSEC-2026-0381
- https://rustsec.org/advisories/RUSTSEC-2026-0382
- https://github.com/advisories/GHSA-6q48-6c7w-9gw2

After those updates the only returned Rust advisory was RUSTSEC-2026-0192 for
ttf-parser 0.25.1, an informational unmaintained-package notice, with no patched
version. It is a transitive dependency of the desktop font renderer. This remains
a maintenance follow-up; no vulnerability advisory was returned for the patched
lockfile in this check. This scan is not an independent security audit.

- https://rustsec.org/advisories/RUSTSEC-2026-0192
