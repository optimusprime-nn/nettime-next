"""Reject Windows release binaries that expose local account/build paths."""
from pathlib import Path
import re


def verify(path):
    path = Path(path)
    data = path.read_bytes()
    patterns = (
        r"[A-Za-z]:[\\/]+Users[\\/]+[^\\/\x00\r\n]+[\\/]",
        re.escape(str(Path(__file__).resolve().parent.parent)),
        re.escape(str(Path.home())),
    )
    for encoding in ("latin1", "utf-16-le"):
        text = data.decode(encoding, errors="ignore")
        if any(re.search(pattern, text, re.IGNORECASE) for pattern in patterns):
            # Do not repeat the personal path in public build logs.
            raise SystemExit(f"Release blocked: {path.name} embeds a local account/build path")
    print(f"Release privacy check passed: {path.name}")


if __name__ == "__main__":
    import sys
    for argument in sys.argv[1:]:
        verify(argument)
