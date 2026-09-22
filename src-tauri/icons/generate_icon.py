#!/usr/bin/env python3
"""Generate a macOS-style CXMail icon bundle."""

from pathlib import Path
import subprocess

import numpy as np
from PIL import Image, ImageChops, ImageDraw, ImageFilter

SIZE = 1024


def rgba_image(rgb_array, alpha=255):
    """Create an RGBA image from an RGB array."""
    alpha_channel = np.full((*rgb_array.shape[:2], 1), alpha, dtype=np.uint8)
    return Image.fromarray(np.concatenate((rgb_array, alpha_channel), axis=2))


def superellipse_mask(size, exponent=5.4, inset=18):
    """Generate an Apple-style continuous-corner mask with oversampling."""
    render_size = size * 2
    radius = (render_size / 2) - (inset * 2)

    axis = np.arange(render_size, dtype=np.float32)
    centered = (axis - (render_size / 2)) / radius
    x = np.abs(centered)[None, :]
    y = np.abs(centered)[:, None]
    values = (x ** exponent) + (y ** exponent)

    feather = np.clip((1.01 - values) / 0.01, 0.0, 1.0)
    mask = Image.fromarray((feather * 255).astype(np.uint8))
    return mask.resize((size, size), Image.LANCZOS)


def diagonal_gradient(size, start, end):
    """Create a soft diagonal gradient."""
    x = np.linspace(0.0, 1.0, size, dtype=np.float32)
    y = np.linspace(0.0, 1.0, size, dtype=np.float32)
    xx, yy = np.meshgrid(x, y)
    blend = np.clip((xx * 0.52) + (yy * 0.84), 0.0, 1.0)[..., None]

    start_rgb = np.array(start, dtype=np.float32)
    end_rgb = np.array(end, dtype=np.float32)
    rgb = (start_rgb * (1.0 - blend)) + (end_rgb * blend)
    return rgba_image(np.clip(rgb, 0, 255).astype(np.uint8))


def radial_overlay(size, center, radius, color, alpha, power=1.9):
    """Create a radial light or shade layer."""
    x = np.linspace(0.0, 1.0, size, dtype=np.float32)
    y = np.linspace(0.0, 1.0, size, dtype=np.float32)
    xx, yy = np.meshgrid(x, y)

    distance = np.sqrt((xx - center[0]) ** 2 + (yy - center[1]) ** 2)
    falloff = np.clip(1.0 - (distance / radius), 0.0, 1.0) ** power

    rgb = np.zeros((size, size, 3), dtype=np.uint8)
    rgb[..., 0] = color[0]
    rgb[..., 1] = color[1]
    rgb[..., 2] = color[2]

    layer = rgba_image(rgb)
    layer.putalpha(Image.fromarray((falloff * alpha).astype(np.uint8)))
    return layer


def vertical_alpha_layer(size, color, top_alpha, bottom_alpha, height=1.0):
    """Create a vertical alpha ramp."""
    y = np.linspace(0.0, 1.0, size, dtype=np.float32)
    ramp = np.clip(1.0 - (y / max(height, 0.001)), 0.0, 1.0)
    alpha = (bottom_alpha + ((top_alpha - bottom_alpha) * ramp)).astype(np.uint8)

    rgb = np.zeros((size, size, 3), dtype=np.uint8)
    rgb[..., 0] = color[0]
    rgb[..., 1] = color[1]
    rgb[..., 2] = color[2]

    layer = rgba_image(rgb)
    layer.putalpha(Image.fromarray(np.repeat(alpha[:, None], size, axis=1)))
    return layer


def masked_alpha_composite(base, overlay, mask):
    """Composite a layer after clipping it to the icon mask."""
    clipped = overlay.copy()
    clipped.putalpha(ImageChops.multiply(clipped.split()[3], mask))
    return Image.alpha_composite(base, clipped)


def masked_shadow(size, shape_drawer, mask, fill, blur, offset=(0, 0)):
    """Draw a blurred shadow that stays inside the icon body."""
    shadow = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    shadow_draw = ImageDraw.Draw(shadow)
    shape_drawer(shadow_draw, offset)
    shadow = shadow.filter(ImageFilter.GaussianBlur(radius=blur))
    shadow.putalpha(ImageChops.multiply(shadow.split()[3], mask))

    tint = Image.new("RGBA", (size, size), fill)
    tint.putalpha(shadow.split()[3])
    return tint


def create_icon():
    print("  Rendering icon with baked squircle mask...")

    # Build on opaque canvas — squircle mask applied at the end
    icon = Image.new("RGBA", (SIZE, SIZE), (0, 0, 0, 255))

    # Diagonal gradient background
    for y in range(SIZE):
        for x in range(SIZE):
            t = (x + y) / (2 * SIZE)
            r = int(52 - t * 22)
            g = int(52 - t * 22)
            b = int(57 - t * 22)
            icon.putpixel((x, y), (r, g, b, 255))

    draw = ImageDraw.Draw(icon)

    # Envelope dimensions — centered with good padding
    pad_x = int(SIZE * 0.195)
    env_left = pad_x
    env_right = SIZE - pad_x
    env_width = env_right - env_left
    env_height = int(env_width * 0.62)
    env_top = (SIZE - env_height) // 2
    env_bottom = env_top + env_height
    center_x = SIZE // 2

    # Envelope shadow
    shadow = Image.new("RGBA", (SIZE, SIZE), (0, 0, 0, 0))
    shadow_draw = ImageDraw.Draw(shadow)
    shadow_draw.rounded_rectangle(
        [env_left + 4, env_top + 6, env_right + 4, env_bottom + 6],
        radius=18,
        fill=(0, 0, 0, 70),
    )
    shadow = shadow.filter(ImageFilter.GaussianBlur(radius=12))
    icon = Image.alpha_composite(icon, shadow)
    draw = ImageDraw.Draw(icon)

    # Envelope body
    draw.rounded_rectangle(
        [env_left, env_top, env_right, env_bottom],
        radius=14,
        fill=(250, 250, 250, 255),
    )

    # Subtle body gradient (top lighter, bottom slightly darker)
    for y in range(env_top + 14, env_bottom - 14):
        t = (y - env_top) / (env_bottom - env_top)
        overlay_alpha = int(t * 15)
        for x in range(env_left + 14, env_right - 14):
            px = icon.getpixel((x, y))
            icon.putpixel(
                (x, y),
                (
                    max(0, px[0] - overlay_alpha),
                    max(0, px[1] - overlay_alpha),
                    max(0, px[2] - overlay_alpha),
                    px[3],
                ),
            )

    draw = ImageDraw.Draw(icon)

    # Flap (top V-shape)
    flap_peak_y = env_top + int(env_height * 0.45)
    draw.polygon(
        [(env_left, env_top + 1), (env_right, env_top + 1), (center_x, flap_peak_y)],
        fill=(238, 238, 240, 255),
    )

    # Flap fold lines
    line_color = (80, 80, 85, 255)
    draw.line([(env_left + 1, env_top + 8), (center_x, flap_peak_y)], fill=line_color, width=7)
    draw.line([(env_right - 1, env_top + 8), (center_x, flap_peak_y)], fill=line_color, width=7)

    # Bottom V-lines
    bottom_peak_y = env_bottom - int(env_height * 0.40)
    bottom_line_color = (100, 100, 105, 255)
    draw.line([(env_left + 1, env_bottom - 8), (center_x, bottom_peak_y)], fill=bottom_line_color, width=6)
    draw.line([(env_right - 1, env_bottom - 8), (center_x, bottom_peak_y)], fill=bottom_line_color, width=6)

    # Envelope border
    draw.rounded_rectangle(
        [env_left, env_top, env_right, env_bottom],
        radius=14,
        outline=(40, 40, 45, 255),
        width=7,
    )

    # Subtle top highlight (gloss)
    highlight = Image.new("RGBA", (SIZE, SIZE), (0, 0, 0, 0))
    for y in range(SIZE // 3):
        alpha = int(18 * (1 - y / (SIZE // 3)))
        for x in range(SIZE):
            highlight.putpixel((x, y), (255, 255, 255, alpha))
    icon = Image.alpha_composite(icon, highlight)

    # Apply squircle mask — Tauri uses raw PNGs for the dock icon,
    # so we bake the Apple-style continuous corners into the image.
    mask = superellipse_mask(SIZE, inset=100)
    icon.putalpha(mask)

    # Drop shadow behind the masked icon for depth
    shadow = Image.new("RGBA", (SIZE, SIZE), (0, 0, 0, 0))
    shadow.putalpha(ImageChops.multiply(
        mask,
        Image.new("L", (SIZE, SIZE), 45),
    ))
    shadow = shadow.filter(ImageFilter.GaussianBlur(radius=18))
    # Shift shadow down slightly
    shifted_shadow = Image.new("RGBA", (SIZE, SIZE), (0, 0, 0, 0))
    shifted_shadow.paste(shadow, (0, 8))

    # Composite: shadow behind, masked icon on top
    final = Image.alpha_composite(shifted_shadow, icon)

    return final


def generate_iconset(img, icons_dir):
    """Generate all sizes needed for macOS .iconset"""
    iconset_dir = icons_dir / "cxmail.iconset"
    iconset_dir.mkdir(exist_ok=True)

    sizes = [
        ("icon_16x16.png", 16),
        ("icon_16x16@2x.png", 32),
        ("icon_32x32.png", 32),
        ("icon_32x32@2x.png", 64),
        ("icon_128x128.png", 128),
        ("icon_128x128@2x.png", 256),
        ("icon_256x256.png", 256),
        ("icon_256x256@2x.png", 512),
        ("icon_512x512.png", 512),
        ("icon_512x512@2x.png", 1024),
    ]

    for filename, size in sizes:
        resized = img.resize((size, size), Image.LANCZOS)
        resized.save(iconset_dir / filename)
        print(f"  {filename} ({size}x{size})")

    return iconset_dir


def generate_tauri_icons(img, icons_dir):
    """Generate the PNG sizes Tauri expects"""
    tauri_sizes = {
        "32x32.png": 32,
        "128x128.png": 128,
        "128x128@2x.png": 256,
        "icon.png": 512,
        "icon_source.png": 1024,
        "Square30x30Logo.png": 30,
        "Square44x44Logo.png": 44,
        "Square71x71Logo.png": 71,
        "Square89x89Logo.png": 89,
        "Square107x107Logo.png": 107,
        "Square142x142Logo.png": 142,
        "Square150x150Logo.png": 150,
        "Square284x284Logo.png": 284,
        "Square310x310Logo.png": 310,
        "StoreLogo.png": 50,
    }

    for filename, size in tauri_sizes.items():
        resized = img.resize((size, size), Image.LANCZOS)
        resized.save(icons_dir / filename)
        print(f"  {filename} ({size}x{size})")


def generate_ico(img, icons_dir):
    """Generate Windows .ico file"""
    ico_sizes = [16, 24, 32, 48, 64, 128, 256]
    ico_images = []
    for size in ico_sizes:
        resized = img.resize((size, size), Image.LANCZOS)
        ico_images.append(resized)
    ico_images[0].save(
        icons_dir / "icon.ico",
        format="ICO",
        sizes=[(s, s) for s in ico_sizes],
        append_images=ico_images[1:]
    )
    print("  icon.ico")


if __name__ == "__main__":
    icons_dir = Path(__file__).resolve().parent

    print("Generating 1024x1024 master icon...")
    icon = create_icon()

    print("\nGenerating macOS .iconset...")
    iconset_dir = generate_iconset(icon, icons_dir)

    print("\nGenerating Tauri icon PNGs...")
    generate_tauri_icons(icon, icons_dir)

    print("\nGenerating .ico...")
    generate_ico(icon, icons_dir)

    print(f"\nConverting to .icns via iconutil...")
    subprocess.run(
        ["iconutil", "-c", "icns", str(iconset_dir), "-o", str(icons_dir / "icon.icns")],
        check=True,
    )

    print("\nDone! Rebuild the app to see the new icon.")
