# Playground programs

The programs in the [browser playground](https://abstrkshen.github.io/pyrock/play.html).
The website loads them from here (`scripts/build_site.sh` copies them into `site/`), and
the test suite runs each one: compiled by pyrock and under CPython, the output must be
identical (`crates/driver/tests/suites.rs`, test `differential`).

A first line `# mode: native-only` makes the playground select that mode and the test
compile in it. `# expect: compile error (P301)` marks a program pyrock must reject with
that code.

The `svg_*`, `ppm_*` and `html_*` programs draw by printing: SVG, HTML or plain-text PPM
images (`P3`). The playground renders that output; several SVGs or PPM images in a row
play as an animation. Since a picture is only text, the native executable prints the same
one. To see it outside the browser:

```console
pyrock run svg_chart.py --mode native-only > chart.svg
pyrock run ppm_mandelbrot.py --release > mandelbrot.ppm
```

To add a program, put the file here and add a line for it to `EXAMPLES` in
`site/play.html`.
