# textlens

Select a region of a Wayland screen, OCR the text inside it, and copy the result to the
clipboard. The whole pipeline runs in memory — no screenshot is ever written to disk.

```
slurp  →  grim -g - -  →  tesseract stdin stdout -l <lang>  →  wl-copy
```

## Requirements

- A compositor with `wlr-screencopy` (sway, Hyprland, …) and `wlr-layer-shell` (slurp)
- `slurp`, `grim`, `tesseract`, `wl-copy` (from wl-clipboard)
- For `--focused`: `grim` 1.5+ and a compositor exporting `ext-foreign-toplevel-list`
  and `ext-image-copy-capture` (river; sway does not)

Arch Linux:

```sh
sudo pacman -S slurp grim tesseract tesseract-data-eng wl-clipboard
```

## Build

```sh
cargo build --release
install -Dm755 target/release/textlens ~/.local/bin/textlens
```

## Usage

```sh
textlens                        # English only
textlens --lang eng+chi_sim     # mix several tesseract languages
```

Select the region with the pointer; the text is copied as soon as you release.

In sway, bind it to a key:

```
bindsym $mod+Shift+t exec textlens
```

Any other tesseract language needs its data package, e.g. `tesseract-data-chi_sim`,
`tesseract-data-jpn`, `tesseract-data-deu`.

## Keyboard only (no pointer)

When the pointer is unusable, name the region instead of dragging it:

```sh
textlens --full                     # every output
textlens --focused                  # the focused window (needs grim 1.5+)
textlens --geometry '20,10 640x480'
```

`--geometry`, `--focused` and `--full` are mutually exclusive; without any of them,
the region is picked with the pointer.

## Behaviour

- Escape or a click during selection cancels: nothing is copied, no output, exit 1.
- No text recognised: the clipboard is left untouched, message on stderr, exit 1.
- Exit codes: `0` copied, `1` cancelled or failed, `2` bad arguments.
