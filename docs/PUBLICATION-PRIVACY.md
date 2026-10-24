# Publication privacy

Release packaging remaps the local home and checkout paths to anonymous build
paths and rejects binaries containing a Windows user profile path. Fresh
installations use the public NTS endpoints in `native/settings.default.json`.
Historical examples use reserved documentation addresses.

Local settings, databases, logs, packet captures, build output and the `work/`
directory must remain outside tracked source and public release attachments.
Commit authors should use a GitHub privacy email rather than a personal mailbox.

The publication cleanup rewrote historical commits and tags. Existing clones
should be replaced with a fresh clone; merging an older clone can reintroduce
the original metadata. Older preview downloads were withdrawn; the current
release replaces their role as the supported download.

History rewriting cannot remove copies already downloaded, forked or cached by
third parties. GitHub may retain old commit objects until its retention and
garbage-collection processes remove them.
