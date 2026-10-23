# JPEG XL test assets (pending companion PR)

These files belong into [`typst-dev-assets`](https://github.com/typst/typst-dev-assets)
as `files/images/jxl/*` (plus entries in the `files!` list in its `src/lib.rs`).
They are kept here only until that companion change is merged. Afterwards,
delete this directory, bump the `typst-dev-assets` revision in `Cargo.toml`, and
replace `/tests/jxl-assets/` with `/assets/images/jxl/` in
`tests/suite/visualize/image.typ`.

All images are synthetic and were generated from scratch with `generate.py`
(which reproduces them byte for byte with `cjxl` v0.7.0). They are dedicated to
the public domain (CC0).

| File                           | Content                                               |
| ------------------------------ | ----------------------------------------------------- |
| `lossy.jxl`                    | 8-bit RGB, lossy (VarDCT), bare codestream            |
| `lossless.jxl`                 | 8-bit RGB, lossless (modular)                         |
| `alpha.jxl`                    | 8-bit RGBA, lossy                                     |
| `gray.jxl`                     | 8-bit grayscale, lossless                             |
| `hdr-pq.jxl`                   | 16-bit HDR (Rec. 2100 PQ, 1000 nits), container       |
| `float.jxl`                    | 32-bit float, linear sRGB, lossy                      |
| `icc.jxl`                      | Embedded ICC profile (Display P3 primaries, LUT TRC)  |
| `orientation.jxl`              | Orientation 6 in the image header                     |
| `orientation-recompressed.jxl` | Recompressed JPEG with orientation 6                  |
| `animated.jxl`                 | Three-frame animation                                 |
| `recompressed.jxl`             | Losslessly recompressed `recompressed.jpg` (`jbrd`)   |
| `recompressed.jpg`             | The original JPEG of `recompressed.jxl`               |
| `linked.svg`                   | SVG linking to `lossless.jxl`                         |
