---
title: Images and color
description: "Colors, image formats, fit modes, filters, and what scale each output draws at."
---

Show a color or an image, fitted your way, drawn at each output's real device pixels.

## Colors and images

**Colors** are `#rrggbb`, six hex digits in either case; quote them, since
the shell reads `#` as a comment. No `#rgb` shorthand, no alpha (a
wallpaper is opaque). **Anything not starting with `#` is an image**: a
path, or an `http://` or `https://` URL ([below](./from-url.md#a-wallpaper-from-a-link));
a file whose name starts with `#` is given as `./#name.png`. A path is
made absolute before it is sent (the daemon's working directory is not
yours) and must be valid UTF-8. PNG, JPEG and WebP are read, told apart by
content, not by name (an animated PNG or WebP shows its first frame);
transparency is shown over the fill color, and EXIF orientation is
applied (a JPEG's, a WebP's, or a PNG's `eXIf` chunk). `--mode` fits it to each output:

| `--mode` | |
|---|---|
| `fill` (default) | cover the output, cropping what overflows, centred |
| `fit` | all of it, as large as fits, centred, the rest in `--fill` |
| `stretch` | the output's size, whatever the aspect |
| `center` | unscaled, centred: cropped if larger, the rest in `--fill` |
| `tile` | unscaled, repeated from the top-left corner |

`--fill '#rrggbb'` is the color around a fitted or centred image (default
`#000000`), `--filter lanczos3|catmull-rom|bilinear|nearest` the scaling
filter (default `lanczos3`; `nearest` keeps pixel art hard). `--mode`,
`--fill` and `--filter` with a color are a usage error. The image is
decoded and scaled on a worker thread and the decoded pixels are dropped
once drawn. Outputs of one size showing one image share one buffer's
memory (32.4 MB at 4K, however many outputs show it). An output plugged
in later shares the pixels of an output of its size already showing the
image, with no decode; otherwise it, like a new scale that needs a new
size, reads the file again. Images over 16384×16384 pixels are refused.
An image is also not scaled along a side longer than 65536 pixels (the
scaler's limit, far past any screen): a 20,000,000×1 strip set with
`fit` or `stretch` is a drawing error, reported by `set` and by
`query`'s `draw_error`, while `fill` (which crops the long side away
first), `center` and `tile` show it.



## Scale

**Images are drawn at each output's real device pixels, fractional scales
included.** On a compositor with `wp_fractional_scale_v1` and
`wp_viewporter` (scoot, sway, and most others) the buffer is the surface's
logical size times the scale the compositor asks for, rounded as that
protocol says: 1601×1001 for scoot's 1067×667 surface at 1.5 on a
1600×1000 output, which scoot draws one buffer pixel to one device pixel
(the last column and row fall off the edge). An image the size of the
output with `--mode center` comes out exact to the pixel. Without those
protocols it is drawn at the integer scale (the larger of `wl_surface`'s
preferred buffer scale and `wl_output`'s) and the compositor scales it
down: sharp, but larger than the output (2134×1334 there). The same
happens for a moment when the compositor has not yet told a surface its
new scale (sway does not while nothing is shown on it): the image is on
screen when `set` returns, drawn larger and scaled down, never stretched,
and redrawn exact once the compositor sends the scale. A compositor scale that is
not a multiple of 1/120 (1.33, say) cannot be drawn exactly by any client,
since the protocol cannot say it.
