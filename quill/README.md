# quill

Takeover display host for the reMarkable Paper Pro: stops xochitl and drives
the e-ink panel directly through the vendor waveform engine (libqsgepaper's
EPFramebuffer, via asivery's epfb-re interposition shim), with raw evdev input.

This is the lowest-latency third-party drawing path that exists on this device
short of reverse-engineering the FPGA transport frame format.

- `src/epfb.cpp`, `src/epframebuffer.h` — epfb-re (QImage constructor
  interposition to capture the engine's internal buffers)
- `src/quill_c.cpp` — C ABI over the engine (init/buffer/swap) for C and Rust apps
- `src/scribble.c` — C1 milestone: pen-to-glass latency demo (exit: pen
  side-button in hover, 5-finger tap, power button, or SIGTERM)
- `src/drawlab.c` — no-AI live drawing experiments built on the scribble core
- `src/map_demo.c` — Marauder's-Map-style demo: full map render + tiny
  partial-update animated footsteps (`marauders_map.png` is its map;
  `medium-map.gif` / `deviant-map.gif` are recordings of it running)
- `src/image_demo.cpp` — render a PNG/JPEG/etc. image through Qt's QImage
  loader, scaled to the panel
- `src/image_anim_demo.cpp`, `src/gif_demo.cpp` — partial-update animation
  experiments (sprites over a still image; GIF playback)
- `scripts/takeover.sh` — stop xochitl, run app, ALWAYS restore xochitl
- `build.sh` — cross-build against the ferrari SDK (~/rm-sdk-3.26) +
  `vendor/libqsgepaper.so` pulled from the device (the SDK comes from
  reMarkable's developer program; build.sh expects it unpacked at
  `~/rm-sdk-3.26` and the tablet reachable over ssh to fetch the vendor lib)

Exit the demos: power button, 5-finger tap, or SIGTERM.
