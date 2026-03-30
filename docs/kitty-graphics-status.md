# Kitty Graphics Protocol — Implementation Status

Tracking bcon's implementation of the [Kitty Graphics Protocol](https://sw.kovidgoyal.net/kitty/graphics-protocol/) compared to other terminals.

Last updated: 2026-03-31 (v1.3.0)

## Transfer Modes

| Mode | Key | bcon | kitty | Ghostty | WezTerm |
|------|-----|------|-------|---------|---------|
| Direct (base64 inline) | `t=d` | Yes | Yes | Yes | Yes |
| File path | `t=f` | Yes | Yes | Yes | Yes |
| Temporary file | `t=t` | Yes | Yes | Yes | Yes |
| Shared memory | `t=s` | Yes | Yes | Yes | Yes |

All 4 modes are enabled by default. Can be disabled via `[security] allow_kitty_remote = false` for hardening.

## Actions

| Action | Key | bcon | kitty | Ghostty | WezTerm |
|--------|-----|------|-------|---------|---------|
| Transmit only | `a=t` | Yes | Yes | Yes | Yes |
| Transmit & display | `a=T` | Yes | Yes | Yes | Yes |
| Display (put) | `a=p` | Yes | Yes | Yes | Yes |
| Query | `a=q` | Yes | Yes | Yes | Yes |
| Delete | `a=d` | Yes | Yes | Yes | Partial |
| Frame data | `a=f` | Yes | Yes | No | Yes |
| Animation control | `a=a` | Yes | Yes | No | Partial |
| Compose frames | `a=c` | Yes | Yes | No | Yes |

## Delete Targets (`a=d`)

| Target | Key | bcon | kitty | Ghostty | WezTerm |
|--------|-----|------|-------|---------|---------|
| All visible | `d=a/A` | Yes | Yes | Yes | Yes |
| By image ID | `d=i/I` | Yes | Yes | Yes | Yes |
| By image number | `d=n/N` | Yes | Yes | Yes | No |
| At cursor position | `d=c/C` | Yes | Yes | Yes | No |
| At cell coordinate | `d=p/P` | Yes | Yes | Yes | No |
| By column | `d=x/X` | Yes | Yes | Yes | No |
| By row | `d=y/Y` | Yes | Yes | Yes | No |
| By z-index | `d=z/Z` | Yes | Yes | Yes | No |
| By ID range | `d=r/R` | Yes | Yes | Yes | No |
| Animation frames | `d=f/F` | Yes | Yes | Yes (no-op) | No |

## Image Management

| Feature | bcon | kitty | Ghostty | WezTerm |
|---------|------|-------|---------|---------|
| Storage model | Texture cache (HashMap) | Out-of-band (verstable hashmap) | Out-of-band (Pin tracking) | Cell-attached |
| Scroll tracking | Yes (absolute row) | Yes (row offset) | Yes (Pin) | Yes (implicit via cells) |
| Z-order | Yes (2-pass) | Yes (3-pass) | Yes (3-layer) | Yes (z-index sort) |
| Unicode placeholder | Yes (U+10EEEE) | Yes | Yes | No |
| Storage limit | 128 textures | 320MB | 320MB | 320MB |
| Screen clear (`ESC[2J`) clears images | Yes | Yes | Yes | Yes |

## Image Formats

| Format | Key | bcon | kitty | Ghostty | WezTerm |
|--------|-----|------|-------|---------|---------|
| RGBA (32bpp) | `f=32` | Yes | Yes | Yes | Yes |
| RGB (24bpp) | `f=24` | Yes | Yes | Yes | Yes |
| PNG | `f=100` | Yes | Yes | Yes | Yes |

## Other Features

| Feature | bcon | kitty | Ghostty | WezTerm |
|---------|------|-------|---------|---------|
| Chunked transfer (`m=1`) | Yes | Yes | Yes | Yes |
| zlib compression (`o=z`) | Yes | Yes | Yes | Yes |
| Response (`q=0/1/2`) | Yes | Yes | Yes | Yes |
| Cursor movement (`C=0/1`) | Yes | Yes | Yes | Yes |
| Cell offset (`X`, `Y`) | Yes | Yes | Yes | ? |
| Display size (`c`, `r`) | Yes | Yes | Yes | ? |
| Relative placement (`P`, `Q`) | Yes | Yes | Yes | No |

## Known Limitations

- Z<0 images: text background appears black over image area (LCD subpixel rendering constraint, same behavior as kitty)

## Test Suite

Run the test suite on bcon:

```bash
python3 tests/generate-test-images.py
bash tests/kitty-graphics-test.sh
```

26 tests covering: display, deletion (all targets), scroll tracking, overlay, z-order, cell offset, relative placement, ID range deletion, and click persistence.
