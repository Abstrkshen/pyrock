# Lumen

A short platformer. Lumen, the moths, and the beetles are SVG characters with frame animation. Wisps, the gate, the moon, hills, clouds, and ground are SVG files too. librsvg rasterizes them, and `lib/liblumen.so` draws with the C raylib library.

There is a title menu, a level select, and three levels. Collect every wisp, then walk into the gate.

Move with A and D or the arrow keys. Jump with W, space, or up. P or esc pauses. On the menus, up and down change the selection and enter confirms.

`play(0)` runs until you quit or close the window.

<!-- readme_check: skip-ci (opens a window, needs librsvg and cairo) -->

`bash build.sh` builds `lib/liblumen.so` and downloads raylib 5.5 next to it. The SVG
loading needs librsvg and cairo (Debian/Ubuntu: `sudo apt install librsvg2-2 libcairo2`).

```sh
bash build.sh
pyrock check main.py
pyrock explain main.py
pyrock run main.py
```
