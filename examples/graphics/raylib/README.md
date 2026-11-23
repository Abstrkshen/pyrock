# raylib

Opens a window, draws two moving circles, then closes. `window.c` calls the C raylib library in `lib/libraylib.so.550` (raylib 5.5). There is no Python raylib package on this path.

<!-- readme_check: skip-ci (opens a window) -->

`bash build.sh` downloads the raylib 5.5 shared library into `lib/` first.

```sh
bash build.sh
pyrock check main.py
pyrock explain main.py
pyrock run main.py
```
