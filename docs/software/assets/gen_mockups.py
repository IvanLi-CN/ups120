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
  - 8x12 mono grid (7x10 glyph + 1px horizontal spacing + 2px leading)
  - Element colors: V=ORANGE, A=RED, W=GREEN
  - Celsius rendered as 'C' with a crisp 2x2 degree dot at y-1
  - 4-row dashboard layout with mode-specific third row

NOTE: This is a host-side renderer to verify layout; not firmware code.
"""
from PIL import Image, ImageDraw

W, H = 160, 50
LM, RM, TM, BM = 4, 4, 1, 1  # margins
CELL_W = 8
GLYPH_W = 7
GLYPH_H = 10
LINE_H = 12
LABEL_WIDTH_CELLS = 4
VOLT_WIDTH_CELLS = 5
CURRENT_WIDTH_CELLS = 4
POWER_WIDTH_CELLS = 4
VALUE_GAP_CELLS = 1
TEMP_LABEL_CELLS = 4
TEMP_VALUE_CELLS = 4
AUX_GAP_CELLS = 3
FAN_LABEL_CELLS = 3
FAN_VALUE_CELLS = 4
COL_LABEL = 0
COL_VOLT = LABEL_WIDTH_CELLS
COL_CURR = COL_VOLT + VOLT_WIDTH_CELLS + VALUE_GAP_CELLS
COL_POWER = COL_CURR + CURRENT_WIDTH_CELLS + VALUE_GAP_CELLS
AUX_TEMP_VALUE_COL = TEMP_LABEL_CELLS
AUX_FAN_LABEL_COL = TEMP_LABEL_CELLS + TEMP_VALUE_CELLS + AUX_GAP_CELLS
AUX_FAN_VALUE_COL = AUX_FAN_LABEL_COL + FAN_LABEL_CELLS
PLACEHOLDER = '--'

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
    pixels = img.load()
    return img, draw, pixels


def save_scaled(img: Image.Image, path: str, scale: int = 4) -> None:
    out = img.resize((img.width * scale, img.height * scale), resample=Image.NEAREST)
    out.save(path)


def text_width(text: str) -> int:
    return len(text) * CELL_W


# 7x10 glyph bitmaps copied from firmware (binary -> int)
GLYPHS: dict[str, list[int]] = {
    ' ': [
        0b0000000, 0b0000000, 0b0000000, 0b0000000, 0b0000000,
        0b0000000, 0b0000000, 0b0000000, 0b0000000, 0b0000000,
    ],
    '%': [
        0b1110000, 0b1110001, 0b1110001, 0b0000110, 0b0001000,
        0b0001000, 0b0110000, 0b1000111, 0b1000111, 0b0000111,
    ],
    '-': [
        0b0000000, 0b0000000, 0b0000000, 0b0000000, 0b0111110,
        0b0111110, 0b0000000, 0b0000000, 0b0000000, 0b0000000,
    ],
    '.': [
        0b0000000, 0b0000000, 0b0000000, 0b0000000, 0b0000000,
        0b0000000, 0b0000000, 0b0001000, 0b0001000, 0b0001000,
    ],
    '0': [
        0b0111110, 0b1000001, 0b1000001, 0b1000111, 0b1001001,
        0b1001001, 0b1110001, 0b1000001, 0b1000001, 0b0111110,
    ],
    '1': [
        0b0001000, 0b0111000, 0b0111000, 0b0001000, 0b0001000,
        0b0001000, 0b0001000, 0b0001000, 0b0001000, 0b0111110,
    ],
    '2': [
        0b0111110, 0b1000001, 0b1000001, 0b0000001, 0b0000110,
        0b0000110, 0b0001000, 0b0110000, 0b0110000, 0b1111111,
    ],
    '3': [
        0b1111110, 0b0000001, 0b0000001, 0b0000001, 0b0111110,
        0b0111110, 0b0000001, 0b0000001, 0b0000001, 0b1111110,
    ],
    '4': [
        0b0000110, 0b0001110, 0b0001110, 0b0110110, 0b1000110,
        0b1000110, 0b1111111, 0b0000110, 0b0000110, 0b0000110,
    ],
    '5': [
        0b1111111, 0b1000000, 0b1000000, 0b1111110, 0b0000001,
        0b0000001, 0b0000001, 0b1000001, 0b1000001, 0b0111110,
    ],
    '6': [
        0b0001110, 0b0110000, 0b0110000, 0b1000000, 0b1111110,
        0b1111110, 0b1000001, 0b1000001, 0b1000001, 0b0111110,
    ],
    '7': [
        0b1111111, 0b0000001, 0b0000001, 0b0000110, 0b0001000,
        0b0001000, 0b0110000, 0b0110000, 0b0110000, 0b0110000,
    ],
    '8': [
        0b0111110, 0b1000001, 0b1000001, 0b1000001, 0b0111110,
        0b0111110, 0b1000001, 0b1000001, 0b1000001, 0b0111110,
    ],
    '9': [
        0b0111110, 0b1000001, 0b1000001, 0b1000001, 0b0111111,
        0b0111111, 0b0000001, 0b0000110, 0b0000110, 0b0111000,
    ],
    ':': [
        0b0000000, 0b0001000, 0b0001000, 0b0001000, 0b0000000,
        0b0000000, 0b0001000, 0b0001000, 0b0001000, 0b0000000,
    ],
    'A': [
        0b0111110, 0b1000001, 0b1000001, 0b1000001, 0b1111111,
        0b1111111, 0b1000001, 0b1000001, 0b1000001, 0b1000001,
    ],
    'B': [
        0b1111110, 0b1000001, 0b1000001, 0b1000001, 0b1111110,
        0b1111110, 0b1000001, 0b1000001, 0b1000001, 0b1111110,
    ],
    'C': [
        0b0111110, 0b1000001, 0b1000001, 0b1000000, 0b1000000,
        0b1000000, 0b1000000, 0b1000001, 0b1000001, 0b0111110,
    ],
    'D': [
        0b1111000, 0b1000110, 0b1000110, 0b1000001, 0b1000001,
        0b1000001, 0b1000001, 0b1000110, 0b1000110, 0b1111000,
    ],
    'E': [
        0b1111111, 0b1000000, 0b1000000, 0b1000000, 0b1111110,
        0b1111110, 0b1000000, 0b1000000, 0b1000000, 0b1111111,
    ],
    'F': [
        0b1111111, 0b1000000, 0b1000000, 0b1000000, 0b1111110,
        0b1111110, 0b1000000, 0b1000000, 0b1000000, 0b1000000,
    ],
    'G': [
        0b0111110, 0b1000001, 0b1000001, 0b1000000, 0b1001111,
        0b1001111, 0b1000001, 0b1000001, 0b1000001, 0b0111110,
    ],
    'H': [
        0b1000001, 0b1000001, 0b1000001, 0b1000001, 0b1111111,
        0b1111111, 0b1000001, 0b1000001, 0b1000001, 0b1000001,
    ],
    'I': [
        0b0111110, 0b0001000, 0b0001000, 0b0001000, 0b0001000,
        0b0001000, 0b0001000, 0b0001000, 0b0001000, 0b0111110,
    ],
    'L': [
        0b1000000, 0b1000000, 0b1000000, 0b1000000, 0b1000000,
        0b1000000, 0b1000000, 0b1000000, 0b1000000, 0b1111111,
    ],
    'M': [
        0b1000001, 0b1110111, 0b1110111, 0b1001001, 0b1001001,
        0b1001001, 0b1000001, 0b1000001, 0b1000001, 0b1000001,
    ],
    'N': [
        0b1000001, 0b1000001, 0b1000001, 0b1110001, 0b1001001,
        0b1001001, 0b1000111, 0b1000001, 0b1000001, 0b1000001,
    ],
    'O': [
        0b0111110, 0b1000001, 0b1000001, 0b1000001, 0b1000001,
        0b1000001, 0b1000001, 0b1000001, 0b1000001, 0b0111110,
    ],
    'P': [
        0b1111110, 0b1000001, 0b1000001, 0b1000001, 0b1111110,
        0b1111110, 0b1000000, 0b1000000, 0b1000000, 0b1000000,
    ],
    'R': [
        0b1111110, 0b1000001, 0b1000001, 0b1000001, 0b1111110,
        0b1111110, 0b1001000, 0b1000110, 0b1000110, 0b1000001,
    ],
    'S': [
        0b0111111, 0b1000000, 0b1000000, 0b1000000, 0b0111110,
        0b0111110, 0b0000001, 0b0000001, 0b0000001, 0b1111110,
    ],
    'T': [
        0b1111111, 0b0001000, 0b0001000, 0b0001000, 0b0001000,
        0b0001000, 0b0001000, 0b0001000, 0b0001000, 0b0001000,
    ],
    'U': [
        0b1000001, 0b1000001, 0b1000001, 0b1000001, 0b1000001,
        0b1000001, 0b1000001, 0b1000001, 0b1000001, 0b0111110,
    ],
    'V': [
        0b1000001, 0b1000001, 0b1000001, 0b1000001, 0b1000001,
        0b1000001, 0b0110110, 0b0110110, 0b0110110, 0b0001000,
    ],
    'W': [
        0b1000001, 0b1000001, 0b1000001, 0b1001001, 0b1001001,
        0b1001001, 0b1001001, 0b1110111, 0b1110111, 0b1000001,
    ],
    'Y': [
        0b1000001, 0b1000001, 0b1000001, 0b0110110, 0b0001000,
        0b0001000, 0b0001000, 0b0001000, 0b0001000, 0b0001000,
    ],
}


def normalize_char(ch: str) -> str:
    if 'a' <= ch <= 'z':
        return chr(ord(ch) - 32)
    return ch


def draw_char(pixels, x: int, y: int, ch: str, color) -> int:
    ch = normalize_char(ch)
    pattern = GLYPHS.get(ch)
    if pattern is None:
        # fallback: 1px outline box
        for rx in range(GLYPH_W):
            for ry in range(GLYPH_H):
                if pixels is None:
                    continue
                draw_pixel = (
                    ry == 0 or ry == GLYPH_H - 1 or rx == 0 or rx == GLYPH_W - 1
                )
                if draw_pixel:
                    px = x + rx
                    py = y + ry
                    if 0 <= px < W and 0 <= py < H:
                        pixels[px, py] = color
        return x + CELL_W

    for ry, row in enumerate(pattern):
        mask = 1 << (GLYPH_W - 1)
        for rx in range(GLYPH_W):
            if row & mask:
                if pixels is not None:
                    px = x + rx
                    py = y + ry
                    if 0 <= px < W and 0 <= py < H:
                        pixels[px, py] = color
            mask >>= 1
    return x + CELL_W


def draw_text(pixels, x: int, y: int, text: str, color) -> int:
    for ch in text:
        x = draw_char(pixels, x, y, ch, color)
    return x


def cell_to_x(cell: int) -> int:
    return LM + cell * CELL_W


def draw_value_right(pixels, col_start: int, width_cells: int, y: int, text: str, color) -> int:
    text_cells = len(text)
    start_cell = col_start if text_cells >= width_cells else col_start + (width_cells - text_cells)
    x = cell_to_x(start_cell)
    return draw_text(pixels, x, y, text, color)


def fmt_voltage(mv: int) -> str:
    clamped = min(mv, 99_990)
    v_tenths = (clamped + 50) // 100
    whole = v_tenths // 10
    if whole >= 100:
        return '>99V'
    frac = v_tenths % 10
    if whole >= 10:
        return f'{whole:02}.{frac}V'
    hundredths = ((clamped + 5) // 10) % 10
    return f'{whole}.{frac}{hundredths}V'


def fmt_current(ma: int) -> str:
    clamped = min(ma, 99_900)
    if clamped >= 10_000:
        amps = (clamped + 500) // 1000
        if amps >= 100:
            return '>99A'
        return f'{amps:>3}A'
    a_tenths = (clamped + 50) // 100
    whole = a_tenths // 10
    frac = a_tenths % 10
    return f'{whole}.{frac}A'


def fmt_power(mw: int) -> str:
    watts = (mw + 500) // 1000
    if watts > 999:
        return '999W'
    return f'{watts:>3}W'


def fmt_soc(pct: int) -> str:
    return f'{min(pct, 100):>3}%'


def fmt_idle(secs: int) -> str:
    d = secs // 86400
    h = (secs % 86400) // 3600
    m = (secs % 3600) // 60
    return f'{d:02}D{h:02}:{m:02}'


def fmt_temp_digits(c: int) -> str:
    capped = max(-99, min(199, c))
    return f'{capped}'


def fmt_fan(pct: int) -> str:
    return f'{min(pct, 100):>3}%'


def draw_celsius(pixels, x: int, y: int, digits: str, color) -> int:
    x = draw_text(pixels, x, y, digits, color)
    deg_x = max(LM, x - 3)
    deg_y = max(TM, y - 1)
    if pixels is not None:
        for dx in range(2):
            for dy in range(2):
                px = deg_x + dx
                py = deg_y + dy
                if 0 <= px < W and 0 <= py < H:
                    pixels[px, py] = color
    x = draw_char(pixels, x, y, 'C', color)
    return x


def draw_boot() -> Image.Image:
    img, draw, pixels = new_canvas()

    # Title centered (row 1)
    title = 'UPS120'
    tx = (W - text_width(title)) // 2
    ty = TM  # first row baseline
    draw_text(pixels, tx, ty, title, WHITE)

    # Subtitle (row 2, gray)
    sub = 'SYSTEM BOOT'
    sx = (W - text_width(sub)) // 2
    sy = ty + LINE_H
    draw_text(pixels, sx, sy, sub, GRAY)

    # Progress bar (center-ish), 152x8 per spec, margin respected
    pb_w, pb_h = 152, 8
    pb_x = (W - pb_w) // 2
    pb_y = H - BM - pb_h - LINE_H // 2  # keep gap above bottom line for status
    # frame
    draw.rectangle((pb_x, pb_y, pb_x + pb_w - 1, pb_y + pb_h - 1), outline=GRAY)
    # fill to 72%
    pct = 72
    fill_w = int((pb_w - 2) * pct / 100)
    draw.rectangle((pb_x + 1, pb_y + 1, pb_x + 1 + fill_w - 1, pb_y + pb_h - 2), fill=GREEN)

    # Status above the bar (no overlap)
    status = f'INIT SC8815  {pct}%'
    sw = text_width(status)
    sx = max(LM, (W - sw) // 2)
    draw_text(pixels, sx, pb_y - LINE_H, status, CYAN)

    return img


def right_text(pixels, text: str, y: int, color) -> int:
    x = W - RM - text_width(text)
    draw_text(pixels, x, y, text, color)
    return x


def draw_trio_line(pixels, y: int, label: str, v: str, a: str, w: str) -> None:
    draw_text(pixels, cell_to_x(COL_LABEL), y, label, CYAN)
    volt_color = GRAY if v == PLACEHOLDER else ORANGE
    curr_color = GRAY if a == PLACEHOLDER else RED
    power_color = GRAY if w == PLACEHOLDER else GREEN
    draw_value_right(pixels, COL_VOLT, VOLT_WIDTH_CELLS, y, v, volt_color)
    draw_value_right(pixels, COL_CURR, CURRENT_WIDTH_CELLS, y, a, curr_color)
    draw_value_right(pixels, COL_POWER, POWER_WIDTH_CELLS, y, w, power_color)


def draw_aux_line(pixels, y: int, temp_label: str, temp_c: int, fan_pct: int) -> None:
    draw_text(pixels, cell_to_x(COL_LABEL), y, temp_label, CYAN)
    digits = fmt_temp_digits(temp_c)
    total_cells = len(digits) + 1
    start_cell = AUX_TEMP_VALUE_COL if total_cells >= TEMP_VALUE_CELLS else AUX_TEMP_VALUE_COL + (TEMP_VALUE_CELLS - total_cells)
    draw_celsius(pixels, cell_to_x(start_cell), y, digits, WHITE)
    draw_text(pixels, cell_to_x(AUX_FAN_LABEL_COL), y, 'FAN', CYAN)
    fan_text = fmt_fan(fan_pct)
    draw_value_right(pixels, AUX_FAN_VALUE_COL, FAN_VALUE_CELLS, y, fan_text, WHITE)


def draw_dashboard(mode: str, temp_slot: str) -> Image.Image:
    assert mode in ('Discharge', 'Charge', 'Standby')
    assert temp_slot in ('BAT', 'UPS')
    img, draw, pixels = new_canvas()

    # Row topology (12px line height from top, starting at y=1)
    row_y = [TM + i * LINE_H for i in range(4)]

    # Row 1: MODE left, SoC right
    mode_color = {'Charge': CYAN, 'Discharge': WHITE, 'Standby': GRAY}[mode]
    draw_text(pixels, LM, row_y[0], f'MODE: {mode.upper()}', mode_color)
    soc = fmt_soc(85)
    right_text(pixels, soc, row_y[0], WHITE)

    # Row 2: IN V/A/W
    draw_trio_line(
        pixels,
        row_y[1],
        'IN',
        fmt_voltage(48_000),
        fmt_current(2_500),
        fmt_power(120_000),
    )

    # Row 3: depends on mode
    temp_label = temp_slot
    temp_value = 32 if temp_slot == 'BAT' else 36
    if mode == 'Discharge':
        draw_trio_line(
            pixels,
            row_y[2],
            'OUT',
            fmt_voltage(48_000),
            fmt_current(2_000),
            fmt_power(100_000),
        )
    elif mode == 'Charge':
        draw_trio_line(
            pixels,
            row_y[2],
            'CHG',
            PLACEHOLDER,
            PLACEHOLDER,
            fmt_power(80_000),
        )
    else:  # Standby
        draw_trio_line(
            pixels,
            row_y[2],
            'IDLE',
            fmt_idle(1 * 86_400 + 2 * 3_600 + 3 * 60),
            PLACEHOLDER,
            PLACEHOLDER,
        )

    # Row 4: temperature (single slot) + fan
    draw_aux_line(pixels, row_y[3], temp_label, temp_value, 45)

    return img


def main():
    # Boot
    boot_img = draw_boot()
    save_scaled(boot_img, 'boot.png')

    # Dashboards
    for mode, temp_slot, name in (
        ('Discharge', 'BAT', 'dashboard-discharge.png'),
        ('Charge', 'UPS', 'dashboard-charge.png'),
        ('Standby', 'BAT', 'dashboard-standby.png'),
    ):
        img = draw_dashboard(mode, temp_slot)
        save_scaled(img, name)

    print('Generated: boot.png, dashboard-*.png')


if __name__ == '__main__':
    main()
