"""Generate the committed multi-resolution clock/network icon, using stdlib only."""
from pathlib import Path
import math
import struct


def distance(x, y, a, b):
    dx, dy = b[0] - a[0], b[1] - a[1]
    t = max(0, min(1, ((x-a[0])*dx + (y-a[1])*dy) / (dx*dx + dy*dy)))
    return math.hypot(x-a[0]-t*dx, y-a[1]-t*dy)


def color(x, y):
    if abs(x-.5) > .47 or abs(y-.5) > .47:
        return (0, 0, 0, 0)
    dx, dy = max(abs(x-.5)-.35, 0), max(abs(y-.5)-.35, 0)
    if math.hypot(dx, dy) > .12:
        return (0, 0, 0, 0)
    c = (20, 34, 53, 255)
    if distance(x, y, (.25, .83), (.5, .68)) < .025 or distance(x, y, (.75, .83), (.5, .68)) < .025:
        c = (91, 195, 230, 255)
    r = math.hypot(x-.5, y-.43)
    if r < .31:
        c = (97, 175, 255, 255) if r > .265 else (29, 51, 80, 255)
    if distance(x, y, (.5, .43), (.5, .255)) < .024 or distance(x, y, (.5, .43), (.65, .49)) < .024:
        c = (245, 250, 255, 255)
    if math.hypot(x-.25, y-.83) < .065 or math.hypot(x-.75, y-.83) < .065:
        c = (119, 225, 205, 255)
    return c


def image(size):
    pixels = []
    for y in range(size):
        for x in range(size):
            samples = [color((x+(i+.5)/4)/size, (y+(j+.5)/4)/size) for j in range(4) for i in range(4)]
            alpha = sum(c[3] for c in samples) / 16
            channels = [round(sum(c[k]*c[3] for c in samples) / (16*alpha)) if alpha else 0 for k in range(3)]
            pixels.append((*channels, round(alpha)))
    return pixels


def main():
    sizes = (16, 20, 24, 32, 40, 48, 64, 256)
    blocks = []
    for size in sizes:
        pixels = image(size)
        bitmap = b''.join(bytes((b, g, r, a)) for y in reversed(range(size)) for r, g, b, a in pixels[y*size:(y+1)*size])
        mask = bytes(((size + 31) // 32) * 4 * size)
        header = struct.pack('<IiiHHIIiiII', 40, size, size*2, 1, 32, 0, len(bitmap)+len(mask), 0, 0, 0, 0)
        blocks.append(header + bitmap + mask)
    offset = 6 + 16 * len(sizes)
    entries = []
    for size, block in zip(sizes, blocks):
        entries.append(struct.pack('<BBBBHHII', size % 256, size % 256, 0, 0, 1, 32, len(block), offset))
        offset += len(block)
    root = Path(__file__).resolve().parents[1]
    (root / 'nettime.ico').write_bytes(struct.pack('<HHH', 0, 1, len(sizes)) + b''.join(entries) + b''.join(blocks))


if __name__ == '__main__':
    main()
