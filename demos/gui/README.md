# gui: 300 colliding balls with raylib

`sim.py` is the physics (gravity, walls, pairwise elastic collisions: 45,000 pair checks
per frame), compiled natively. `main.py` draws it with the `raylib` Python package, so the
drawing calls go through CPython. Create the virtualenv once:
`python3 -m venv .venv && .venv/bin/pip install raylib`.

```sh
pyfast run headless.py -- 300
pyfast bench headless.py --release --runs 3
pyfast run main.py --python .venv/bin/python -- --frames 120
```

`headless.py` runs the simulation without a window: about 17x faster than CPython.
Without `--frames`, the window stays open until you close it.
