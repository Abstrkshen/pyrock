# import-c

`kern.c` is compiled into the binary. `wave.cpp` is a shared library loaded from Python.

```sh
g++ -shared -fPIC -O2 -o libwave.so wave.cpp
pyrock check main.py
pyrock explain main.py
pyrock run main.py
```
