---
title: Images and color
description: "Colors, image formats, fit modes, filters, and what scale each output draws at."
---

Show a color or an image, fitted your way.

## Colors and images

**Colors** are `#rrggbb`, six hex digits in either case; quote them, since
the shell reads `#` as a comment. No `#rgb` shorthand, no alpha (a
wallpaper is opaque). **Anything not starting with `#` is an image**: a
path, or an `http://` or `https://` URL ([below](./from-url.md#a-wallpaper-from-a-link));
a file whose name starts with `#` is given as `./#name.png`. A path is
made absolute before it is sent (the daemon's working directory is not
yours) and must be valid UTF-8. PNG, JPEG, GIF and WebP are read, told
apart by content, not by name (an animated GIF, APNG or animated WebP
shows its first frame: at most 64 frames and 64 MiB of frames; past
that the `set` is refused — pass `--no-animate` for the first frame
instead; playing frame by frame is a follow-up; a `--no-animate` still
is live-only, re-checked after a restart);
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
filter (default `lanczos3`; `nearest` keeps pixel art hard). Before
scaling, the daemon probes the scaler's whole budget
and refuses the draw when the
probe fails (see [Troubleshooting](./troubleshooting.md#symptoms));
`center` and `tile` scale nothing
and never probe. `--mode`,
`--fill` and `--filter` with a color are a usage error. The image is
decoded and scaled on a worker thread and the decoded pixels are dropped
once drawn. Outputs of one size showing one image share one buffer's
memory. An output plugged
in later shares the pixels of an output of its size already showing the
image, with no decode; otherwise it reads the file again. Images over 16384×16384 pixels are refused.
An image is also not scaled along a side longer than 65536 pixels:
a 20,000,000×1 strip set with
`fit` or `stretch` is a drawing error, while `fill` (which crops the long side away
first), `center` and `tile` show it.



## Scale

**Images are drawn at each output's real device pixels, fractional scales
included.** On a compositor with `wp_fractional_scale_v1` and
`wp_viewporter` (scoot, sway, and most others) an image the size of the
output with `--mode center` comes out exact to the pixel. Without those
protocols it is drawn at the integer scale and the compositor scales it
down: sharp, but larger than the output. The same
happens for a moment when the compositor has not yet told a surface its
new scale: the image is on
screen when `set` returns, drawn larger and scaled down, never stretched,
and redrawn exact once the compositor sends the scale.
