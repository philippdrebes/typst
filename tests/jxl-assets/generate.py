# Generates the JPEG XL test images in this directory from synthetic source
# images. Requires numpy, Pillow, and libjxl's `cjxl` (tested with v0.7.0).
#
# Usage: python3 generate.py <work-dir> <icc-profile>
#
# The ICC profile used for `icc.jxl` is a Display P3 profile created with
# `moxcms::ColorProfile::new_display_p3()`, with all tone curves replaced by a
# 256-entry gamma 1.8 LUT (so that encoders cannot replace it with an enum
# color encoding) and the header version set to 4.2.
import sys, struct, zlib
import numpy as np
from PIL import Image

out = sys.argv[1]
W, H = 64, 48

def base_rgb():
    x = np.linspace(0, 1, W)[None, :]
    y = np.linspace(0, 1, H)[:, None]
    r = np.clip(1.0 - x, 0, 1) * (0.4 + 0.6 * (1 - y))
    g = np.clip(x * (1 - x) * 4, 0, 1) * (0.4 + 0.6 * y)
    b = np.clip(x, 0, 1) * (0.5 + 0.5 * y)
    img = np.stack([r * np.ones_like(y), g, b * np.ones_like(y)], axis=-1)
    img = np.broadcast_to(img, (H, W, 3)).copy()
    # An asymmetric "F" glyph in the top-left so that orientation is obvious.
    img[6:30, 6:11] = 1.0
    img[6:11, 6:24] = 1.0
    img[15:20, 6:19] = 1.0
    # A dark disc on the right.
    yy, xx = np.mgrid[0:H, 0:W]
    disc = (xx - 46) ** 2 + (yy - 30) ** 2 < 10 ** 2
    img[disc] = [0.1, 0.1, 0.15]
    return img

rgb = base_rgb()
Image.fromarray((rgb * 255).round().astype(np.uint8), "RGB").save(f"{out}/rgb.png")

# RGBA: alpha fades out to the right, with a fully transparent hole.
yy, xx = np.mgrid[0:H, 0:W]
alpha = np.clip(1.2 - xx / W, 0, 1)
alpha[(xx - 20) ** 2 + (yy - 34) ** 2 < 8 ** 2] = 0
rgba = np.concatenate([rgb, alpha[..., None]], axis=-1)
Image.fromarray((rgba * 255).round().astype(np.uint8), "RGBA").save(f"{out}/rgba.png")

# Grayscale.
gray = rgb @ np.array([0.2126, 0.7152, 0.0722])
Image.fromarray((gray * 255).round().astype(np.uint8), "L").save(f"{out}/gray.png")

# HDR: 16-bit PQ-encoded RGB. Linear light in nits, peak 1000 nits on the
# right, so that the tone-mapped result is visibly a smooth ramp.
def pq_encode(nits):
    m1, m2 = 2610 / 16384, 2523 / 4096 * 128
    c1, c2, c3 = 3424 / 4096, 2413 / 4096 * 32, 2392 / 4096 * 32
    y = np.clip(nits / 10000.0, 0, 1)
    return ((c1 + c2 * y ** m1) / (1 + c3 * y ** m1)) ** m2
lin = rgb ** 2.2  # roughly linearize the sRGB base image
nits = lin * (100 + 900 * np.linspace(0, 1, W)[None, :, None])
pq = (pq_encode(nits) * 65535).round().astype(">u2")
def write_png16(path, arr):
    h, w, c = arr.shape
    raw = b"".join(b"\x00" + arr[i].tobytes() for i in range(h))
    def chunk(t, d):
        return struct.pack(">I", len(d)) + t + d + struct.pack(">I", zlib.crc32(t + d) & 0xffffffff)
    png = b"\x89PNG\r\n\x1a\n"
    png += chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 16, 2, 0, 0, 0))
    png += chunk(b"IDAT", zlib.compress(raw, 9))
    png += chunk(b"IEND", b"")
    open(path, "wb").write(png)
write_png16(f"{out}/hdr-pq16.png", pq)

# Animation: three frames, the first one is the base image.
frames = [Image.fromarray((rgb * 255).round().astype(np.uint8), "RGB")]
for c in [(255, 0, 0), (0, 0, 255)]:
    frames.append(Image.new("RGB", (W, H), c))
frames[0].save(f"{out}/anim.gif", save_all=True, append_images=frames[1:], duration=200, loop=0)

# JPEG to be recompressed losslessly.
Image.fromarray((rgb * 255).round().astype(np.uint8), "RGB").save(f"{out}/photo.jpg", quality=85)

# Orientation 6: Store the image rotated, so that the orientation from the
# Exif metadata (which cjxl moves into the image header) makes it upright.
from PIL import Image as _I
_rot = _I.open(f"{out}/rgb.png").transpose(_I.Transpose.ROTATE_90)
_ex = _I.Exif()
_ex[0x0112] = 6
_rot.save(f"{out}/orient6.png", exif=_ex.tobytes())
_rot.save(f"{out}/orient6.jpg", exif=_ex.tobytes(), quality=90)

# Linear float image.
_h, _w, _ = rgb.shape
with open(f"{out}/float.pfm", "wb") as f:
    f.write(b"PF\n%d %d\n-1.0\n" % (_w, _h))
    rgb8 = np.asarray(_I.open(f"{out}/rgb.png")).astype(np.float32) / 255
    f.write((rgb8 ** 2.2)[::-1].astype("<f4").tobytes())

import os, shutil, subprocess
here = os.path.dirname(os.path.abspath(__file__))
icc = sys.argv[2]
def cjxl(src, dst, *args):
    subprocess.run(["cjxl", f"{out}/{src}", f"{here}/{dst}", *args], check=True)
cjxl("rgb.png", "lossy.jxl", "-d", "1.0", "-e", "7")
cjxl("rgb.png", "lossless.jxl", "-d", "0", "-e", "7")
cjxl("rgba.png", "alpha.jxl", "-d", "1.0")
cjxl("gray.png", "gray.jxl", "-d", "0")
cjxl("hdr-pq16.png", "hdr-pq.jxl", "-d", "0", "-x", "color_space=RGB_D65_202_Rel_PeQ", "--intensity_target=1000")
cjxl("float.pfm", "float.jxl", "-d", "1.0", "-x", "color_space=RGB_D65_SRG_Rel_Lin")
cjxl("rgb.png", "icc.jxl", "-d", "0", "-x", f"icc_pathname={icc}")
cjxl("orient6.png", "orientation.jxl", "-d", "0")
cjxl("orient6.jpg", "orientation-recompressed.jxl")
cjxl("anim.gif", "animated.jxl", "-d", "0")
cjxl("photo.jpg", "recompressed.jxl")
shutil.copy(f"{out}/photo.jpg", f"{here}/recompressed.jpg")
