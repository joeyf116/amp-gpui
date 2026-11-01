"""Renders the app's gear art, logo, and backdrop from scratch.

Nothing here is traced from a photo or a real product. Each surface is a height
field lit from the top left, which is what gives tolex, paint, and chrome their
depth. Knobs, lamps, and names are left off the gear because the app draws those
live, so they always show real values.

Run from the repo root, then commit what changes:

    python tools/gear-art/render.py

Needs numpy and Pillow. Output is fixed by the seed, so a rerun only changes the
files whose drawing changed.
"""

from pathlib import Path

import numpy as np
from PIL import Image, ImageDraw, ImageFilter

ROOT = Path(__file__).resolve().parents[2]
GEAR = ROOT / "crates" / "amp-app" / "assets" / "gear"
DOCS = ROOT / "docs" / "assets"

SEED = 1729
LIGHT = np.array([-0.46, -0.62, 0.64])
LIGHT = LIGHT / np.linalg.norm(LIGHT)
HALF = LIGHT + np.array([0.0, 0.0, 1.0])
HALF = HALF / np.linalg.norm(HALF)

PEDAL = (560, 900)
HEAD = (1500, 700)
CAB = (900, 860)
# What the app shows in the chain strip, relative to the full render.
THUMB = 0.25
MOSS = (0.5, 0.84, 0.63)

rng = np.random.default_rng(SEED)


def grid(width, height):
    return np.meshgrid(
        np.arange(width, dtype=np.float32), np.arange(height, dtype=np.float32)
    )


def smooth(edge0, edge1, x):
    t = np.clip((x - edge0) / (edge1 - edge0), 0.0, 1.0)
    return t * t * (3.0 - 2.0 * t)


def noise(width, height, scale, stretch=(1.0, 1.0)):
    """Smooth random field from -1 to 1, with features about `scale` pixels wide."""
    cells_x = max(2, int(width / (scale * stretch[0])) + 2)
    cells_y = max(2, int(height / (scale * stretch[1])) + 2)
    coarse = rng.random((cells_y, cells_x), dtype=np.float32)
    image = Image.fromarray(coarse, mode="F").resize((width, height), Image.BICUBIC)
    field = np.asarray(image, dtype=np.float32)
    return (field - field.mean()) / (field.std() + 1e-6) * 0.5


def fractal(width, height, scale, octaves=3):
    field = np.zeros((height, width), dtype=np.float32)
    amplitude = 1.0
    for _ in range(octaves):
        field += noise(width, height, max(scale, 1.5)) * amplitude
        scale /= 2.0
        amplitude /= 2.0
    return field


def rounded_box(x, y, left, top, right, bottom, radius):
    """Signed distance to a rounded rectangle, negative inside."""
    cx, cy = (left + right) / 2, (top + bottom) / 2
    qx = np.abs(x - cx) - (right - left) / 2 + radius
    qy = np.abs(y - cy) - (bottom - top) / 2 + radius
    outside = np.hypot(np.maximum(qx, 0), np.maximum(qy, 0))
    return outside + np.minimum(np.maximum(qx, qy), 0) - radius


def coverage(distance):
    return np.clip(0.5 - distance, 0.0, 1.0)


def blur(mask, radius):
    image = Image.fromarray((np.clip(mask, 0, 1) * 255).astype(np.uint8))
    return np.asarray(image.filter(ImageFilter.GaussianBlur(radius)), np.float32) / 255


def normals(height):
    dy, dx = np.gradient(height)
    nx, ny, nz = -dx, -dy, np.ones_like(height)
    length = np.sqrt(nx * nx + ny * ny + nz * nz)
    return nx / length, ny / length, nz / length


def lit(albedo, height, gloss=0.25, shine=30.0, ambient=0.34):
    """Painted or dielectric surface: diffuse plus a tight highlight."""
    nx, ny, nz = normals(height)
    diffuse = np.maximum(nx * LIGHT[0] + ny * LIGHT[1] + nz * LIGHT[2], 0.0)
    specular = np.maximum(nx * HALF[0] + ny * HALF[1] + nz * HALF[2], 0.0) ** shine
    return albedo * (ambient + diffuse[..., None] * 0.9) + specular[..., None] * gloss


def metal(height, tint=(1.0, 1.0, 1.0), contrast=1.0):
    """Polished metal: reflects a soft room with a bright ceiling and dark floor."""
    nx, ny, nz = normals(height)
    # Mirror the view around the normal, and read the room in that direction.
    ry = 2.0 * nz * ny
    rx = 2.0 * nz * nx
    room = 0.5 - 0.72 * ry - 0.18 * rx
    banded = 0.5 + 0.5 * np.tanh((room - 0.46) * 5.0 * contrast)
    value = 0.12 + banded * 0.72 + smooth(0.92, 1.1, room) * 0.3
    return value[..., None] * np.array(tint, dtype=np.float32)


class Canvas:
    def __init__(self, width, height):
        self.width, self.height = width, height
        self.rgb = np.zeros((height, width, 3), dtype=np.float32)
        self.alpha = np.zeros((height, width), dtype=np.float32)
        self.x, self.y = grid(width, height)

    def over(self, rgb, mask):
        """Lays `rgb`, a color or an image, over the canvas through `mask`."""
        rgb = np.asarray(rgb, dtype=np.float32)
        mask = np.clip(mask, 0.0, 1.0)
        out = mask + self.alpha * (1 - mask)
        safe = np.maximum(out, 1e-5)
        kept = self.alpha * (1 - mask)
        self.rgb = (rgb * mask[..., None] + self.rgb * kept[..., None]) / safe[..., None]
        self.alpha = out

    def shade(self, mask, amount):
        """Darkens what is already there, for contact shadows."""
        self.rgb = self.rgb * (1 - (np.clip(mask, 0, 1) * amount)[..., None])

    def drop_shadow(self, mask, offset, radius, strength=0.6):
        moved = np.roll(np.roll(mask, offset[1], axis=0), offset[0], axis=1)
        self.over((0.0, 0.0, 0.0), blur(moved, radius) * strength)

    def image(self):
        out = np.dstack([np.clip(self.rgb, 0, 1), np.clip(self.alpha, 0, 1)])
        return Image.fromarray((out * 255 + 0.5).astype(np.uint8), mode="RGBA")


def tolex(width, height):
    """Pebbled vinyl covering."""
    pebble = np.abs(noise(width, height, 7.0)) + 0.5 * np.abs(noise(width, height, 3.5))
    return pebble * 1.4


def brushed(width, height):
    return noise(width, height, 2.0, (40.0, 0.6)) * 0.22 + noise(width, height, 1.2, (60, 0.5)) * 0.1


def hammertone(width, height):
    return smooth(-0.7, 0.9, fractal(width, height, 13.0, 2)) * 1.5


def wrinkle(width, height):
    ridges = 1.0 - np.abs(fractal(width, height, 9.0, 2))
    return smooth(0.3, 1.0, ridges) * 1.5


def satin(width, height):
    return fractal(width, height, 3.0, 2) * 0.08


def weave(canvas, pitch, x0=0.0):
    """Basket-woven grille cloth: pairs of threads passing over and under."""
    u = (canvas.x - x0) / pitch
    v = canvas.y / pitch
    cell = (np.floor(u).astype(int) + np.floor(v).astype(int)) % 2 == 0
    across = np.sin(np.pi * np.mod(v * 2, 1)) ** 2
    along = np.sin(np.pi * np.mod(u * 2, 1)) ** 2
    return np.where(cell, across, along) * 1.1


def round_part(canvas, cx, cy, radius, profile):
    """Height of a part turned on a lathe. `profile` takes radius from 0 to 1."""
    r = np.hypot(canvas.x - cx, canvas.y - cy) / radius
    return profile(np.clip(r, 0, 1.0)) * radius, coverage((r - 1.0) * radius)


def screw(canvas, cx, cy, radius, tint=(0.72, 0.72, 0.7)):
    height, mask = round_part(
        canvas, cx, cy, radius, lambda r: 0.32 * np.sqrt(np.maximum(1.0 - r**2.4, 0.0))
    )
    slot_a = coverage(rounded_box(canvas.x, canvas.y, cx - radius * 0.62, cy - radius * 0.13, cx + radius * 0.62, cy + radius * 0.13, 1))
    slot_b = coverage(rounded_box(canvas.x, canvas.y, cx - radius * 0.13, cy - radius * 0.62, cx + radius * 0.13, cy + radius * 0.62, 1))
    height = height - np.maximum(slot_a, slot_b) * radius * 0.34
    canvas.drop_shadow(mask, (1, 2), 2.0, 0.55)
    canvas.over(metal(height, tint), mask)


def hex_distance(canvas, cx, cy, radius):
    x, y = np.abs(canvas.x - cx), np.abs(canvas.y - cy)
    return np.maximum(y * 0.866 + x * 0.5, x) - radius * 0.866


def footswitch(canvas, cx, cy, radius):
    x, y = canvas.x - cx, canvas.y - cy
    r = np.hypot(x, y)
    # Threaded collar and hex nut, then the button.
    nut = coverage(hex_distance(canvas, cx, cy, radius * 1.2))
    canvas.drop_shadow(nut, (5, 8), 8.0, 0.65)
    nut_height = smooth(0.0, 1.0, -hex_distance(canvas, cx, cy, radius * 1.2) / 7.0) * 7.0
    canvas.over(metal(nut_height, (0.8, 0.8, 0.8)), nut)
    collar, mask = round_part(canvas, cx, cy, radius * 1.04, lambda s: 0.1 * (s < 0.97) + 0.06 * np.sin(s * 40.0))
    canvas.over(metal(collar, (0.82, 0.82, 0.8)), mask)
    canvas.shade(coverage(r - radius * 0.86) * (r > radius * 0.7), 0.3)
    button, mask = round_part(canvas, cx, cy, radius * 0.8, lambda s: 0.52 * np.sqrt(np.maximum(1.0 - s**3.2, 0.0)))
    canvas.drop_shadow(mask, (2, 3), 3.0, 0.5)
    canvas.over(metal(button, (0.95, 0.95, 0.93), 0.8), mask)


def led_socket(canvas, cx, cy, radius):
    ring, mask = round_part(canvas, cx, cy, radius, lambda s: 0.35 * np.sqrt(np.maximum(1 - (2 * s - 1.2) ** 2, 0)))
    canvas.drop_shadow(mask, (1, 2), 2.0, 0.5)
    canvas.over(metal(ring, (0.76, 0.76, 0.74)), mask)
    hole = coverage(np.hypot(canvas.x - cx, canvas.y - cy) - radius * 0.56)
    canvas.over((0.03, 0.03, 0.03), hole)


def side_jack(canvas, cx, cy, length, thickness):
    """A jack barrel seen end-on past the side of the pedal."""
    box = rounded_box(canvas.x, canvas.y, cx - length, cy - thickness, cx + length, cy + thickness, 4)
    rolled = np.sqrt(np.maximum(1 - ((canvas.y - cy) / thickness) ** 2, 0)) * thickness
    canvas.over(metal(rolled, (0.42, 0.42, 0.41)), coverage(box))


def ink(canvas, draw_graphic):
    """Screen printing, drawn at double size so the lines stay clean."""
    image = Image.new("L", (canvas.width * 2, canvas.height * 2), 0)
    draw_graphic(ImageDraw.Draw(image), canvas.width * 2, canvas.height * 2)
    small = image.resize((canvas.width, canvas.height), Image.LANCZOS)
    return np.asarray(small, np.float32) / 255


def curve(draw, points, width, fill=255):
    draw.line(points, fill=fill, width=width, joint="curve")


def graphic_gate(draw, w, h):
    top, bottom = h * 0.44, h * 0.59
    for index in range(13):
        x = w * (0.17 + index * 0.055)
        open_part = 0.0 if 4 <= index <= 8 else 1.0
        draw.line([(x, top), (x, top + (bottom - top) * (0.34 if open_part else 1.0))], fill=255, width=7)
        if open_part:
            draw.line([(x, bottom - (bottom - top) * 0.34), (x, bottom)], fill=255, width=7)


def graphic_compressor(draw, w, h):
    mid = h * 0.515
    points_hi, points_lo = [], []
    for step in range(121):
        t = step / 120
        x = w * (0.14 + 0.72 * t)
        envelope = 0.075 * (1.0 if t < 0.34 else 0.42) * h
        wave = np.sin(t * 56.0)
        points_hi.append((x, mid + envelope * wave))
        points_lo.append((x, mid - envelope * 0.55 * wave))
    curve(draw, points_hi, 5)
    curve(draw, points_lo, 3)


def graphic_boost(draw, w, h):
    for index in range(4):
        y = h * (0.585 - index * 0.046)
        half = w * 0.2
        draw.line([(w / 2 - half, y + h * 0.03), (w / 2, y - h * 0.012), (w / 2 + half, y + h * 0.03)], fill=255, width=12 - index * 2, joint="curve")


def graphic_overdrive(draw, w, h):
    for ring in range(4):
        points = []
        for step in range(90):
            t = step / 89
            x = w * (0.14 + 0.72 * t)
            squash = np.tanh(np.sin(t * 15.7 + ring * 0.2) * (1.0 + ring * 1.4))
            points.append((x, h * 0.515 + squash * h * (0.03 + ring * 0.013)))
        curve(draw, points, 5)


def graphic_distortion(draw, w, h):
    state = np.random.default_rng(23)
    points = [(w * 0.13, h * 0.515)]
    for index in range(1, 22):
        x = w * (0.13 + index * 0.035)
        sign = 1 if index % 2 else -1
        points.append((x, h * 0.515 + sign * h * (0.025 + state.random() * 0.075)))
    draw.line(points, fill=255, width=7, joint="curve")


def graphic_fuzz(draw, w, h):
    state = np.random.default_rng(7)
    for _ in range(760):
        angle = state.random() * 6.283
        reach = state.random() ** 0.6
        x = w / 2 + np.cos(angle) * reach * w * 0.34
        y = h * 0.515 + np.sin(angle) * reach * h * 0.086
        size = 1.5 + state.random() * 4.5
        draw.ellipse([x - size, y - size, x + size, y + size], fill=255)


def graphic_modulation(draw, w, h):
    for shift in (0.0, 1.05, 2.1):
        points = []
        for step in range(121):
            t = step / 120
            points.append((w * (0.14 + 0.72 * t), h * 0.515 + np.sin(t * 15.0 + shift) * h * 0.062))
        curve(draw, points, 5)


def graphic_delay(draw, w, h):
    for index in range(7):
        x = w * (0.2 + index * 0.1)
        tall = h * 0.082 * 0.76**index
        fill = int(255 * 0.9**index)
        draw.line([(x, h * 0.515 - tall), (x, h * 0.515 + tall)], fill=fill, width=11)


def graphic_reverb(draw, w, h):
    for index in range(6):
        reach = w * (0.05 + index * 0.066)
        box = [w / 2 - reach, h * 0.515 - reach * 0.34, w / 2 + reach, h * 0.515 + reach * 0.34]
        draw.ellipse(box, outline=int(255 * (1 - index * 0.12)), width=5)


def graphic_capture(draw, w, h):
    columns = [(0.2, 3), (0.4, 4), (0.6, 4), (0.8, 2)]
    nodes = [
        [(w * x, h * (0.515 + (row - (count - 1) / 2) * 0.032)) for row in range(count)]
        for x, count in columns
    ]
    for left, right in zip(nodes, nodes[1:]):
        for a in left:
            for b in right:
                draw.line([a, b], fill=120, width=2)
    for column in nodes:
        for x, y in column:
            draw.ellipse([x - 9, y - 9, x + 9, y + 9], fill=255)


PEDALS = {
    "gate": ((0.27, 0.3, 0.31), wrinkle, graphic_gate, (0.86, 0.87, 0.84), 0.16),
    "compressor": ((0.07, 0.19, 0.3), hammertone, graphic_compressor, (0.87, 0.83, 0.71), 0.3),
    "boost": ((0.7, 0.65, 0.53), satin, graphic_boost, (0.1, 0.1, 0.09), 0.2),
    "overdrive": ((0.45, 0.2, 0.09), hammertone, graphic_overdrive, (0.93, 0.83, 0.65), 0.35),
    "distortion": ((0.27, 0.035, 0.045), wrinkle, graphic_distortion, (0.9, 0.84, 0.74), 0.16),
    "fuzz": ((0.25, 0.07, 0.2), hammertone, graphic_fuzz, (0.86, 0.68, 0.26), 0.3),
    "modulation": ((0.22, 0.18, 0.36), satin, graphic_modulation, (0.84, 0.86, 0.92), 0.26),
    "delay": ((0.05, 0.19, 0.14), satin, graphic_delay, (0.8, 0.9, 0.82), 0.26),
    "reverb": ((0.2, 0.27, 0.36), hammertone, graphic_reverb, (0.88, 0.91, 0.93), 0.28),
    "capture": ((0.56, 0.57, 0.56), brushed, graphic_capture, (0.07, 0.09, 0.08), 0.3),
}


def render_pedal(name):
    paint, finish, graphic, ink_color, gloss = PEDALS[name]
    width, height = PEDAL
    canvas = Canvas(width, height)
    left, top, right, bottom = 56, 22, width - 56, height - 34
    box = rounded_box(canvas.x, canvas.y, left, top, right, bottom, 34)

    side_jack(canvas, left - 8, height * 0.25, 14, 30)
    side_jack(canvas, right + 8, height * 0.25, 14, 30)
    canvas.drop_shadow(coverage(box), (10, 16), 16.0, 0.72)

    # A die-cast shell: flat top, with a wide rounded shoulder down to its edge.
    inward = np.clip(-box, 0, 34) / 34
    shoulder = np.sqrt(np.maximum(1 - (1 - inward) ** 2, 0)) * 18
    surface = shoulder + finish(width, height) * smooth(0.2, 0.9, inward)
    printed = ink(canvas, graphic)
    albedo = np.array(paint, dtype=np.float32) * np.ones((height, width, 3), dtype=np.float32)
    albedo = albedo * (1 + 0.1 * noise(width, height, 110.0))[..., None]
    if finish is brushed:
        albedo = albedo * (1 + brushed(width, height) * 0.45)[..., None]
    albedo = albedo * (1 - printed[..., None]) + np.array(ink_color, np.float32) * printed[..., None]
    body = lit(albedo, surface, gloss, 24.0)
    # Ambient falloff toward the base of the shell.
    body = body * (1.08 - 0.3 * smooth(0.2, 1.0, canvas.y / height))[..., None]
    canvas.over(body, coverage(box))

    # Thin raised lip at the very edge catches the light.
    lip = coverage(np.abs(box + 2.5) - 1.3)
    canvas.over((1.0, 1.0, 1.0), lip * 0.1 * smooth(0.9, 0.0, canvas.y / height))

    for x, y in ((left + 24, top + 24), (right - 24, top + 24), (left + 24, bottom - 24), (right - 24, bottom - 24)):
        screw(canvas, x, y, 10)
    led_socket(canvas, width / 2, height * 0.645, 19)
    footswitch(canvas, width / 2, height * 0.8, 62)
    return canvas


def tolex_box(canvas, left, top, right, bottom, radius, color):
    box = rounded_box(canvas.x, canvas.y, left, top, right, bottom, radius)
    inward = np.clip(-box, 0, 18) / 18
    rounded = np.sqrt(np.maximum(1 - (1 - inward) ** 2, 0)) * 12
    surface = rounded + tolex(canvas.width, canvas.height)
    albedo = np.array(color, np.float32) * np.ones((canvas.height, canvas.width, 3), np.float32)
    albedo = albedo * (1 + 0.06 * noise(canvas.width, canvas.height, 70.0))[..., None]
    canvas.over(lit(albedo, surface, 0.14, 18.0, 0.42), coverage(box))
    return box


def piping(canvas, left, top, right, bottom, radius, color, wide=7):
    edge = rounded_box(canvas.x, canvas.y, left, top, right, bottom, radius)
    rolled = np.sqrt(np.maximum(1 - (edge / wide) ** 2, 0)) * wide * 0.8
    canvas.over(lit(np.array(color, np.float32), rolled, 0.35, 34.0), coverage(np.abs(edge) - wide))


def corner(canvas, cx, cy, size, sign_x, sign_y):
    """A stamped steel corner protector."""
    x = (canvas.x - cx) * sign_x
    y = (canvas.y - cy) * sign_y
    shape = rounded_box(x, y, 0, 0, size, size, 14)
    arc = np.hypot(x - size, y - size) - size * 0.46
    mask_distance = np.maximum(shape, -arc)
    crown = np.hypot(x - size * 0.2, y - size * 0.2)
    surface = smooth(size * 1.05, 0.0, crown) * size * 0.16 + smooth(0.0, 5.0, -mask_distance) * 4
    mask = coverage(mask_distance)
    canvas.drop_shadow(mask, (3, 4), 4.0, 0.55)
    canvas.over(metal(surface, (0.46, 0.47, 0.46), 0.9), mask)


def handle(canvas, cx, cy, wide, tall):
    box = rounded_box(canvas.x, canvas.y, cx - wide, cy - tall, cx + wide, cy + tall, tall)
    surface = np.sqrt(np.maximum(1 - ((canvas.y - cy) / tall) ** 2, 0)) * tall * 0.8
    surface += noise(canvas.width, canvas.height, 3.0) * 0.5
    stitched = (np.abs(np.abs(canvas.y - cy) - tall * 0.66) < 1.5) & (np.mod(canvas.x, 14) < 8)
    albedo = np.ones((canvas.height, canvas.width, 3), np.float32) * np.array((0.07, 0.065, 0.06), np.float32)
    albedo[stitched] = (0.28, 0.27, 0.24)
    canvas.over(lit(albedo, surface, 0.22, 16.0), coverage(box))


def jewel(canvas, cx, cy, radius):
    x, y = canvas.x - cx, canvas.y - cy
    r = np.hypot(x, y) / radius
    theta = np.arctan2(y, x)
    facets = np.abs(np.mod(theta * 12 / np.pi + 0.5, 1) - 0.5) * (r * 0.5)
    surface = np.sqrt(np.maximum(1 - r * r, 0)) * radius * 0.7 + facets * 7
    bezel, mask = round_part(canvas, cx, cy, radius * 1.42, lambda s: 0.3 * np.sqrt(np.maximum(1 - (2 * s - 1.3) ** 2, 0)))
    canvas.drop_shadow(mask, (2, 3), 3.0, 0.6)
    canvas.over(metal(bezel, (0.8, 0.8, 0.76)), mask)
    albedo = np.ones((canvas.height, canvas.width, 3), np.float32) * np.array((0.17, 0.05, 0.02), np.float32)
    canvas.over(lit(albedo, surface, 0.8, 44.0), coverage((r - 1.0) * radius))


def toggle(canvas, cx, cy):
    nut, mask = round_part(canvas, cx, cy, 20, lambda s: 0.22 * (s < 0.9))
    canvas.drop_shadow(mask, (2, 3), 3.0, 0.55)
    canvas.over(metal(nut, (0.82, 0.82, 0.8)), mask)
    box = rounded_box(canvas.x, canvas.y, cx - 7, cy - 36, cx + 7, cy + 3, 7)
    surface = np.sqrt(np.maximum(1 - ((canvas.x - cx) / 7.5) ** 2, 0)) * 7
    canvas.drop_shadow(coverage(box), (3, 3), 3.0, 0.5)
    canvas.over(metal(surface, (0.95, 0.95, 0.93)), coverage(box))


def quarter_jack(canvas, cx, cy):
    nut, mask = round_part(canvas, cx, cy, 25, lambda s: 0.25 * (s < 0.92))
    canvas.drop_shadow(mask, (2, 3), 3.0, 0.55)
    canvas.over(metal(nut, (0.8, 0.8, 0.78)), mask)
    canvas.over((0.02, 0.02, 0.02), coverage(np.hypot(canvas.x - cx, canvas.y - cy) - 13))


def tube_vent(canvas, left, top, right, bottom):
    """Louvered vent over a row of glowing power tubes."""
    wide, tall = right - left, bottom - top
    opening = rounded_box(canvas.x, canvas.y, left, top, right, bottom, 8)
    glass = np.zeros((canvas.height, canvas.width), np.float32)
    for index in range(4):
        cx = left + wide * (0.2 + index * 0.2)
        ball = np.hypot((canvas.x - cx) / 38, (canvas.y - (top + tall * 0.6)) / (tall * 0.42))
        glass = np.maximum(glass, smooth(1.1, 0.0, ball))
    heat = blur(glass, 26) * 1.5
    light = np.dstack([heat, heat * 0.46, heat * 0.11]) + 0.012
    canvas.over(light, coverage(opening))
    canvas.shade(coverage(opening) * smooth(16, 0, canvas.y - top), 0.8)
    slat = np.mod(canvas.y - top, 16) / 16
    shutter = (slat < 0.56) * coverage(opening)
    surface = smooth(0.0, 0.6, slat) * 3.5
    albedo = np.ones((canvas.height, canvas.width, 3), np.float32) * np.array((0.08, 0.085, 0.08), np.float32)
    canvas.over(lit(albedo, surface + noise(canvas.width, canvas.height, 4.0) * 0.12, 0.3, 24.0), shutter)


def control_panel(canvas, left, top, right, bottom, tint):
    panel = rounded_box(canvas.x, canvas.y, left, top, right, bottom, 8)
    surface = brushed(canvas.width, canvas.height) * 1.6 + smooth(0, 3, -panel) * 2.4
    albedo = np.ones((canvas.height, canvas.width, 3), np.float32) * np.array(tint, np.float32)
    albedo = albedo * (1 + brushed(canvas.width, canvas.height) * 0.5)[..., None]
    canvas.drop_shadow(coverage(panel), (2, 4), 5.0, 0.6)
    canvas.over(lit(albedo, surface, 0.22, 16.0, 0.5), coverage(panel))
    for x in (left + 18, right - 18):
        for y in (top + 18, bottom - 18):
            screw(canvas, x, y, 8)


def badge_plate(canvas, cx, cy, wide, tall):
    plate = rounded_box(canvas.x, canvas.y, cx - wide, cy - tall, cx + wide, cy + tall, 6)
    surface = smooth(0, 3, -plate) * 2.0 + brushed(canvas.width, canvas.height) * 0.5
    albedo = np.ones((canvas.height, canvas.width, 3), np.float32) * np.array((0.035, 0.04, 0.038), np.float32)
    canvas.drop_shadow(coverage(plate), (2, 3), 3.0, 0.7)
    canvas.over(lit(albedo, surface, 0.3, 30.0), coverage(plate))
    piping(canvas, cx - wide + 5, cy - tall + 5, cx + wide - 5, cy + tall - 5, 3, (0.45, 0.37, 0.22), 1.2)


def render_head(color, panel, trim):
    width, height = HEAD
    canvas = Canvas(width, height)
    left, top, right, bottom = 34, 64, width - 34, height - 36
    box = rounded_box(canvas.x, canvas.y, left, top, right, bottom, 20)
    canvas.drop_shadow(coverage(box), (12, 18), 20.0, 0.72)
    handle(canvas, width / 2, top - 22, 170, 24)
    tolex_box(canvas, left, top, right, bottom, 20, color)

    inner = (left + 44, top + 34, right - 44, bottom - 34)
    split = top + (bottom - top) * 0.47
    tube_vent(canvas, inner[0] + 16, inner[1] + 12, inner[2] - 16, split - 14)
    piping(canvas, inner[0], inner[1], inner[2], split, 10, trim)
    control_panel(canvas, inner[0], split + 16, inner[2], inner[3], panel)
    badge_plate(canvas, width / 2, inner[1] + (split - inner[1]) / 2, 188, 44)

    panel_mid = (split + 16 + inner[3]) / 2
    quarter_jack(canvas, inner[0] + 78, panel_mid)
    toggle(canvas, inner[2] - 196, panel_mid + 12)
    toggle(canvas, inner[2] - 128, panel_mid + 12)
    jewel(canvas, inner[2] - 62, panel_mid, 19)

    for sign_x, x in ((1, left), (-1, right)):
        for sign_y, y in ((1, top), (-1, bottom)):
            corner(canvas, x, y, 60, sign_x, sign_y)
    return canvas


def speaker(canvas, cx, cy, radius, cloth_over):
    r = np.hypot(canvas.x - cx, canvas.y - cy) / radius
    theta = np.arctan2(canvas.y - cy, canvas.x - cx)
    if cloth_over:
        # Cloth sags a little over each cone, so the speakers read through it.
        canvas.shade(smooth(1.04, 0.84, r), 0.3)
        canvas.shade(smooth(0.5, 0.1, r), 0.14)
        return
    frame = smooth(1.0, 0.98, r) * smooth(0.86, 0.9, r)
    ribs = 0.8 + 0.2 * (np.mod(theta * 8 / np.pi, 1) < 0.1)
    surface = smooth(1.0, 0.9, r) * 5
    albedo = np.ones((canvas.height, canvas.width, 3), np.float32) * 0.13
    canvas.over(lit(albedo, surface + noise(canvas.width, canvas.height, 2.0) * 0.15, 0.3, 20.0, 0.5) * ribs[..., None], frame)
    # The paper cone falls toward the voice coil, with faint concentric ribs.
    depth = (r - 0.3) * radius * 0.42 + np.sin(r * 50.0) * 0.5 * smooth(0.82, 0.6, r)
    fibers = noise(canvas.width, canvas.height, 2.5) * 0.2
    albedo = np.ones((canvas.height, canvas.width, 3), np.float32) * np.array((0.085, 0.085, 0.08), np.float32)
    canvas.over(lit(albedo, depth + fibers, 0.1, 12.0, 0.55), smooth(0.9, 0.88, r) * smooth(0.27, 0.3, r))
    # Surround and dust cap.
    roll = np.sqrt(np.maximum(1 - ((r - 0.87) / 0.035) ** 2, 0)) * 6
    canvas.over(lit(albedo * 0.8, roll, 0.2, 20.0), smooth(0.92, 0.9, r) * smooth(0.82, 0.84, r))
    cap = np.sqrt(np.maximum(1 - (r / 0.3) ** 2, 0)) * radius * 0.2
    canvas.over(metal(cap, (0.2, 0.2, 0.2), 0.7), smooth(0.3, 0.29, r))
    for index in range(8):
        angle = index * np.pi / 4 + np.pi / 8
        screw(canvas, cx + np.cos(angle) * radius * 0.94, cy + np.sin(angle) * radius * 0.94, 5, (0.5, 0.5, 0.48))


def render_cab(exposed):
    width, height = CAB
    canvas = Canvas(width, height)
    left, top, right, bottom = 30, 24, width - 30, height - 42
    box = rounded_box(canvas.x, canvas.y, left, top, right, bottom, 20)
    canvas.drop_shadow(coverage(box), (12, 20), 20.0, 0.75)
    tolex_box(canvas, left, top, right, bottom, 20, (0.065, 0.07, 0.066))

    baffle = (left + 40, top + 40, right - 40, bottom - 40)
    distance = rounded_box(canvas.x, canvas.y, *baffle, 10)
    if exposed:
        albedo = np.ones((height, width, 3), np.float32) * 0.035
        canvas.over(lit(albedo, noise(width, height, 2.0) * 0.5, 0.05, 10.0, 0.6), coverage(distance))
    else:
        cloth = weave(canvas, 13.0)
        albedo = np.ones((height, width, 3), np.float32) * np.array((0.135, 0.142, 0.138), np.float32)
        albedo = albedo * (0.8 + 0.42 * (cloth / 1.1) * (1 + 0.18 * noise(width, height, 3.0)))[..., None]
        canvas.over(lit(albedo, cloth * 1.6, 0.05, 10.0, 0.62), coverage(distance))
    centers = [(0.27, 0.27), (0.73, 0.27), (0.27, 0.73), (0.73, 0.73)]
    wide, tall = baffle[2] - baffle[0], baffle[3] - baffle[1]
    radius = min(wide, tall) * 0.245
    for u, v in centers:
        speaker(canvas, baffle[0] + wide * u, baffle[1] + tall * v, radius, not exposed)
    canvas.shade(smooth(16, 0, -distance) * coverage(distance), 0.5)
    canvas.shade(smooth(0.0, 1.0, (canvas.y - baffle[1]) / tall), 0.32)
    piping(canvas, *baffle, 10, (0.2, 0.22, 0.2), 4.5)

    for sign_x, x in ((1, left), (-1, right)):
        for sign_y, y in ((1, top), (-1, bottom)):
            corner(canvas, x, y, 68, sign_x, sign_y)
    if not exposed:
        badge_plate(canvas, width - 186, height - 128, 96, 26)
    return canvas


def render_knob_amp():
    size = 300
    canvas = Canvas(size, size)
    cx, cy = size / 2 - 2, size / 2 - 3
    radius = size * 0.45
    x, y = canvas.x - cx, canvas.y - cy
    r = np.hypot(x, y) / radius
    theta = np.arctan2(y, x)
    canvas.drop_shadow(smooth(1.02, 1.0, r), (10, 13), 12.0, 0.75)
    # Skirt, a fluted grip, and a spun metal cap.
    flutes = 0.5 + 0.5 * np.cos(theta * 24)
    grip = smooth(0.92, 0.84, r) * (18 + 14 * smooth(0.84, 0.58, r) + flutes * 6.0 * smooth(0.5, 0.56, r))
    skirt = smooth(1.0, 0.92, r) * 12
    surface = np.maximum(grip, skirt) + (r < 0.49) * 6
    albedo = np.ones((size, size, 3), np.float32) * 0.045
    canvas.over(lit(albedo, surface, 0.4, 26.0, 0.5), smooth(1.0, 0.985, r))
    spun = 0.5 + 0.5 * np.cos(2 * (theta + 0.9))
    value = 0.34 + 0.56 * spun**3 + noise(size, size, 1.4) * 0.02
    canvas.over(value[..., None] * np.array((0.86, 0.86, 0.84), np.float32), smooth(0.49, 0.475, r))
    canvas.over((0.0, 0.0, 0.0), smooth(0.03, 0.0, np.abs(r - 0.49)) * 0.5)
    return canvas


def render_knob_pedal():
    size = 220
    canvas = Canvas(size, size)
    cx, cy = size / 2 - 2, size / 2 - 3
    radius = size * 0.44
    x, y = canvas.x - cx, canvas.y - cy
    r = np.hypot(x, y) / radius
    theta = np.arctan2(y, x)
    canvas.drop_shadow(smooth(1.02, 1.0, r), (7, 9), 8.0, 0.75)
    knurl = 0.5 + 0.5 * np.cos(theta * 52)
    side = smooth(1.0, 0.8, r) * (14 + knurl * 2.4 * smooth(0.66, 0.8, r))
    dome = smooth(0.72, 0.62, r) * (4 + (1 - r**2) * 5)
    albedo = np.ones((size, size, 3), np.float32) * 0.05
    canvas.over(lit(albedo, side + dome, 0.42, 30.0, 0.5), smooth(1.0, 0.98, r))
    return canvas


def save(canvas, name):
    image = canvas.image()
    image.save(GEAR / f"{name}.webp", quality=88, method=6)
    small = image.resize(
        (round(image.width * THUMB), round(image.height * THUMB)), Image.LANCZOS
    )
    small.save(GEAR / f"{name}_s.webp", quality=92, method=6)


def render_stage():
    """Pines fading into mist, with a low pool of light where the gear stands."""
    width, height = 1920, 1200
    x, y = grid(width, height)
    fog = smooth(0.0, 0.62, y / height) * smooth(1.05, 0.6, y / height)
    rgb = (0.018 + fog[..., None] * 0.9) * np.array((0.098, 0.106, 0.102), np.float32)
    state = np.random.default_rng(SEED)
    layers = (
        (0.2, 34, (0.073, 0.079, 0.076)),
        (0.45, 20, (0.045, 0.049, 0.047)),
        (0.8, 11, (0.021, 0.023, 0.022)),
    )
    for layer, (depth, count, tone) in enumerate(layers):
        drawing = Image.new("L", (width, height), 0)
        draw = ImageDraw.Draw(drawing)
        for _ in range(count):
            trunk = state.random() * width
            scale = 40 + depth * 170
            ground = height * (0.68 + depth * 0.2)
            peak = height * (0.06 + state.random() * 0.2 + (1 - depth) * 0.16) - scale
            draw.rectangle([trunk - scale * 0.06, peak, trunk + scale * 0.06, height], fill=255)
            tiers = 16 + int(state.random() * 8)
            for tier in range(tiers):
                t = tier / tiers
                span = scale * (0.14 + t * 0.95) * (0.8 + state.random() * 0.4)
                level = peak + (ground - peak) * t
                drop = scale * (0.24 + t * 0.2)
                draw.polygon([(trunk - span, level + drop), (trunk, level - drop), (trunk + span, level + drop)], fill=255)
        mask = np.asarray(drawing.filter(ImageFilter.GaussianBlur(7 - layer * 2)), np.float32) / 255
        rgb = rgb * (1 - mask[..., None]) + np.array(tone, np.float32) * mask[..., None]
        mist = smooth(-0.4, 0.8, fractal(width, height, 520.0, 4))
        band = smooth(0.3, 0.72, y / height) * smooth(1.1, 0.7, y / height)
        rgb = rgb + (mist * band * (0.042 - layer * 0.009))[..., None] * np.array((0.84, 0.88, 0.86), np.float32)
    pool = np.exp(-(((x - width / 2) / (width * 0.34)) ** 2 + ((y - height * 0.8) / (height * 0.22)) ** 2))
    rgb = rgb + pool[..., None] * np.array((0.044, 0.048, 0.046), np.float32)
    vignette = 1 - 0.5 * np.clip(np.hypot((x - width / 2) / (width * 0.62), (y - height * 0.55) / (height * 0.7)), 0, 1) ** 2.4
    rgb = rgb * vignette[..., None]
    # Dither, so the long dark gradients don't band.
    rgb = rgb + (np.random.default_rng(SEED).random(rgb.shape, dtype=np.float32) - 0.5) / 255
    Image.fromarray((np.clip(rgb, 0, 1) * 255 + 0.5).astype(np.uint8)).save(GEAR / "stage.webp", quality=88, method=6)


def ring_points(radius, wobble, seed, skip):
    """A growth ring: a slightly uneven circle, open where the waveform crosses."""
    state = np.random.default_rng(seed)
    phases = state.random(3) * 6.283
    runs, run = [], []
    for step in range(361):
        angle = np.radians(step)
        if min(abs(step - 90), abs(step - 270)) < skip:
            if run:
                runs.append(run)
            run = []
            continue
        reach = radius * (
            1
            + wobble * np.sin(angle * 3 + phases[0])
            + wobble * 0.6 * np.sin(angle * 5 + phases[1])
            + wobble * 0.3 * np.sin(angle * 7 + phases[2])
        )
        run.append((50 + reach * np.cos(angle), 50 + reach * np.sin(angle)))
    if run:
        runs.append(run)
    return runs


LOGO_RINGS = [(44.0, 0.018, 3, 14), (33.0, 0.024, 5, 19), (22.0, 0.03, 8, 29), (11.0, 0.04, 13, 90)]
LOGO_WAVE = [(50, 2), (50, 22), (43, 34), (58, 46), (39, 56), (62, 66), (46, 76), (50, 84), (50, 98)]


def render_logo():
    """Tree rings cut by a waveform: growth and signal. The SVG and PNG match."""
    color = "#7fd6a0"
    paths = []
    for index, (radius, wobble, seed, skip) in enumerate(LOGO_RINGS):
        for run in ring_points(radius, wobble, seed, skip):
            data = " ".join(f"{x:.1f},{y:.1f}" for x, y in run[::3])
            width = 3.4 - index * 0.25
            paths.append(f'<polyline points="{data}" fill="none" stroke="{color}" stroke-width="{width:.2f}" stroke-linecap="round" stroke-linejoin="round"/>')
    wave = " ".join(f"{x},{y}" for x, y in LOGO_WAVE)
    paths.append(f'<polyline points="{wave}" fill="none" stroke="#e7f2ea" stroke-width="4.2" stroke-linecap="round" stroke-linejoin="round"/>')
    svg = '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100">' + "".join(paths) + "</svg>\n"
    DOCS.mkdir(parents=True, exist_ok=True)
    (DOCS / "logo.svg").write_text(svg)

    scale = 12
    image = Image.new("RGBA", (100 * scale, 100 * scale), (0, 0, 0, 0))
    draw = ImageDraw.Draw(image)
    for index, (radius, wobble, seed, skip) in enumerate(LOGO_RINGS):
        for run in ring_points(radius, wobble, seed, skip):
            points = [(x * scale, y * scale) for x, y in run]
            draw.line(points, fill=(127, 214, 160, 255), width=round((3.4 - index * 0.25) * scale), joint="curve")
    draw.line([(x * scale, y * scale) for x, y in LOGO_WAVE], fill=(231, 242, 234, 255), width=round(4.2 * scale), joint="curve")
    image.resize((192, 192), Image.LANCZOS).save(GEAR / "logo.png")


def main():
    GEAR.mkdir(parents=True, exist_ok=True)
    for name in PEDALS:
        print("pedal", name)
        save(render_pedal(name), f"pedal_{name}")
    print("heads and cabinets")
    save(render_head((0.038, 0.042, 0.04), (0.11, 0.115, 0.11), (0.45, 0.37, 0.22)), "head_understory")
    save(render_head((0.07, 0.075, 0.072), (0.17, 0.175, 0.172), (0.17, 0.18, 0.175)), "head_capture")
    save(render_cab(False), "cab_closed")
    save(render_cab(True), "cab_open")
    render_knob_amp().image().resize((150, 150), Image.LANCZOS).save(GEAR / "knob_amp.png")
    render_knob_pedal().image().resize((110, 110), Image.LANCZOS).save(GEAR / "knob_pedal.png")
    print("backdrop and logo")
    render_stage()
    render_logo()


if __name__ == "__main__":
    main()
