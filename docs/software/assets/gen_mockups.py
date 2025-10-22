#!/usr/bin/env python3
"""
Pixel-accurate mock generator for UPS120 160x50 RGB565-like UI.

Outputs (scaled 4x, nearest neighbor):
  - boot.png
  - dashboard-discharge.png
  - dashboard-charge.png
  - dashboard-standby.png

Requires: Pillow (pip install pillow)

Design rules implemented per docs/software/ui-spec.md:
  - Screen: 160x50, safe margins L/R=4px, T/B=1px
  - 8x8 mono grid, integer pixel placement
  - Element colors: V=ORANGE, A=RED, W=GREEN
  - Celsius rendered as 'C' with a crisp 2x2 degree dot at y-1
  - 4-row dashboard layout with mode-specific third row

NOTE: This is a host-side renderer to verify layout; not firmware code.
"""
from PIL import Image, ImageDraw, ImageFont

W, H = 160, 50
LM, RM, TM, BM = 4, 4, 1, 1  # margins

# RGB palette approximations of RGB565 values from the spec
BLACK   = (0, 0, 0)
WHITE   = (255, 255, 255)
GRAY    = (132, 132, 132)  # 0x8410 approx
GREEN   = (0, 255, 0)
YELLOW  = (255, 255, 0)
ORANGE  = (253, 165, 0)    # 0xFD20 approx
RED     = (255, 0, 0)
CYAN    = (0, 255, 255)

PALETTE = {
    'BLACK': BLACK,
    'WHITE': WHITE,
    'GRAY': GRAY,
    'GREEN': GREEN,
    'YELLOW': YELLOW,
    'ORANGE': ORANGE,
    'RED': RED,
    'CYAN': CYAN,
}


def new_canvas():
    img = Image.new('RGB', (W, H), BLACK)
    draw = ImageDraw.Draw(img)
    return img, draw


def save_scaled(img: Image.Image, path: str, scale: int = 4) -> None:
    out = img.resize((img.width * scale, img.height * scale), resample=Image.NEAREST)
    out.save(path)


def font_bitmap():
    # Pillow's built-in bitmap font; crisp, no anti-alias.
    return ImageFont.load_default()


def text_size(draw: ImageDraw.ImageDraw, text: str, font) -> tuple[int, int]:
    # Consistent text size across Pillow versions
    try:
        # Pillow >=10
        w = int(font.getlength(text))
        h = font.getbbox(text)[3]
        return w, h
    except Exception:
        return draw.textsize(text, font=font)


def draw_text(draw: ImageDraw.ImageDraw, x: int, y: int, text: str, color, font) -> int:
    draw.text((x, y), text, fill=color, font=font)
    w, _ = text_size(draw, text, font)
    return x + w


def draw_celsius(draw: ImageDraw.ImageDraw, x: int, y: int, temp_str: str, color, font) -> int:
    """
    Draws e.g. "32℃" as: '32' + small degree dot + 'C'
    degree dot: 2x2 px square placed 3 px left of 'C' and at y-1.
    """
    # Split numeric part and literal 'C'
    assert temp_str.endswith('C')
    num = temp_str[:-1]
    x = draw_text(draw, x, y, num, color, font)
    # measure width of 'C' to know its placement (for consistency)
    c_w, c_h = text_size(draw, 'C', font)
    # The 'C' will start at x
    # Draw 2x2 square degree dot 3 px left of 'C' and y-1
    deg_x = max(LM, x - 3)
    deg_y = max(TM, y - 1)
    draw.rectangle((deg_x, deg_y, deg_x + 1, deg_y + 1), fill=color)
    # Now draw 'C'
    x = draw_text(draw, x, y, 'C', color, font)
    return x


def hline(draw: ImageDraw.ImageDraw, x0: int, x1: int, y: int, color) -> None:
    draw.line((x0, y, x1, y), fill=color)


def draw_boot() -> Image.Image:
    img, draw = new_canvas()
    f = font_bitmap()

    # Title centered (row 1)
    title = 'UPS120'
    tw, th = text_size(draw, title, f)
    tx = (W - tw) // 2
    ty = TM + 0  # first row
    draw_text(draw, tx, ty, title, WHITE, f)

    # Subtitle (row 2, gray)
    sub = 'System Boot'
    sw, sh = text_size(draw, sub, f)
    sx = (W - sw) // 2
    sy = ty + 10  # bitmap font height ~8-10, add small gap
    draw_text(draw, sx, sy, sub, GRAY, f)

    # Progress bar (center-ish), 152x8 per spec, margin respected
    pb_w, pb_h = 152, 8
    pb_x = (W - pb_w) // 2
    pb_y = H - BM - pb_h - 8  # keep gap above bottom line for status
    # frame
    draw.rectangle((pb_x, pb_y, pb_x + pb_w - 1, pb_y + pb_h - 1), outline=GRAY)
    # fill to 72%
    pct = 72
    fill_w = int((pb_w - 2) * pct / 100)
    draw.rectangle((pb_x + 1, pb_y + 1, pb_x + 1 + fill_w - 1, pb_y + pb_h - 2), fill=GREEN)

    # Status above the bar (no overlap)
    status = f'Init SC8815  {pct}%'
    sw, _ = text_size(draw, status, f)
    sx = max(LM, (W - sw) // 2)
    draw_text(draw, sx, pb_y - 10, status, CYAN, f)

    return img


def right_text(draw: ImageDraw.ImageDraw, text: str, y: int, color, font) -> int:
    w, _ = text_size(draw, text, font)
    x = W - RM - w
    draw_text(draw, x, y, text, color, font)
    return x


def draw_dashboard(mode: str) -> Image.Image:
    assert mode in ('Discharge', 'Charge', 'Standby')
    img, draw = new_canvas()
    f = font_bitmap()

    # Row topology (8px grid from top, starting at y=1)
    row_y = [TM + i * 8 for i in range(4)]

    # Row 1: MODE left, SoC right
    mode_color = {'Charge': CYAN, 'Discharge': WHITE, 'Standby': GRAY}[mode]
    draw_text(draw, LM, row_y[0], f'MODE: {mode}', mode_color, f)
    right_text(draw, '85%', row_y[0], WHITE, f)

    # Row 2: IN V/A/W
    x = LM
    x = draw_text(draw, x, row_y[1], 'IN ', CYAN, f)
    x = draw_text(draw, x, row_y[1], '48.0V', ORANGE, f)
    x = draw_text(draw, x, row_y[1], ' ', WHITE, f)
    x = draw_text(draw, x, row_y[1], '2.5A', RED, f)
    x = draw_text(draw, x, row_y[1], ' ', WHITE, f)
    x = draw_text(draw, x, row_y[1], '120W', GREEN, f)

    # Row 3: depends on mode
    if mode == 'Discharge':
        x = LM
        x = draw_text(draw, x, row_y[2], 'OUT ', CYAN, f)
        x = draw_text(draw, x, row_y[2], '48.0V', ORANGE, f)
        x = draw_text(draw, x, row_y[2], ' ', WHITE, f)
        x = draw_text(draw, x, row_y[2], '2.0A', RED, f)
        x = draw_text(draw, x, row_y[2], ' ', WHITE, f)
        x = draw_text(draw, x, row_y[2], '100W', GREEN, f)
    elif mode == 'Charge':
        x = LM
        x = draw_text(draw, x, row_y[2], 'CHG ', CYAN, f)
        x = draw_text(draw, x, row_y[2], '80W', GREEN, f)
    else:  # Standby
        x = LM
        x = draw_text(draw, x, row_y[2], 'IDLE ', CYAN, f)
        x = draw_text(draw, x, row_y[2], '01d02:03', WHITE, f)

    # Row 4: temps + fan. Use flexible spacing to avoid overflow
    # Compose segment images to measure widths
    def seg_text(text, color):
        return text, color

    segs = [
        seg_text('BAT ', CYAN),
        seg_text('32C', WHITE),  # we'll draw degree dot before 'C'
        seg_text('    ', WHITE),
        seg_text('UPS ', CYAN),
        seg_text('36C', WHITE),
        seg_text('    ', WHITE),
        seg_text('FAN ', CYAN),
        seg_text('45%', WHITE),
    ]

    # Measure total width, then compress padding if needed
    # Build a function to render with a given spacer width
    def render_row4(spaces: int) -> None:
        x = LM
        # BAT
        x = draw_text(draw, x, row_y[3], 'BAT ', CYAN, f)
        x = draw_celsius(draw, x, row_y[3], '32C', WHITE, f)
        x = draw_text(draw, x, row_y[3], ' ' * spaces, WHITE, f)
        # UPS
        x = draw_text(draw, x, row_y[3], 'UPS ', CYAN, f)
        x = draw_celsius(draw, x, row_y[3], '36C', WHITE, f)
        x = draw_text(draw, x, row_y[3], ' ' * spaces, WHITE, f)
        # FAN
        x = draw_text(draw, x, row_y[3], 'FAN ', CYAN, f)
        x = draw_text(draw, x, row_y[3], '45%', WHITE, f)
        # Nothing else; rely on margins

    # Try with 4 spaces, fallback to fewer if needed
    for spaces in (4, 3, 2, 1):
        test_img, test_draw = new_canvas()
        # dry run: compute width by drawing onto a temp image and getting last x
        x = LM
        x += text_size(test_draw, 'BAT ', f)[0]
        x += text_size(test_draw, '32', f)[0]
        # degree dot + 'C' adds roughly 3 px + width('C')
        x += 3 + text_size(test_draw, 'C', f)[0]
        x += text_size(test_draw, ' ' * spaces, f)[0]
        x += text_size(test_draw, 'UPS ', f)[0]
        x += text_size(test_draw, '36', f)[0]
        x += 3 + text_size(test_draw, 'C', f)[0]
        x += text_size(test_draw, ' ' * spaces, f)[0]
        x += text_size(test_draw, 'FAN ', f)[0]
        x += text_size(test_draw, '45%', f)[0]
        if x <= W - RM:
            # draw on real
            render_row4(spaces)
            break
    else:
        # ultimate fallback: no spaces
        render_row4(1)

    # Simple validation: ensure no pixel drawn beyond bounds by verifying last characters would fit
    # (A stricter draw-time validator would require a DrawTarget abstraction; keep it simple here.)
    return img


def main():
    # Boot
    boot_img = draw_boot()
    save_scaled(boot_img, 'boot.png')

    # Dashboards
    for mode, name in (
        ('Discharge', 'dashboard-discharge.png'),
        ('Charge', 'dashboard-charge.png'),
        ('Standby', 'dashboard-standby.png'),
    ):
        img = draw_dashboard(mode)
        save_scaled(img, name)

    print('Generated: boot.png, dashboard-*.png')


if __name__ == '__main__':
    main()

