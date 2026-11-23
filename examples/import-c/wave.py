import ctypes
import os


def sample(turns):
    try:
        path = os.path.join(os.path.dirname(os.path.abspath(__file__)), "libwave.so")
        lib = ctypes.CDLL(path)
        lib.sine_wave.argtypes = [ctypes.c_double]
        lib.sine_wave.restype = ctypes.c_double
        return lib.sine_wave(float(turns))
    except OSError as exc:
        return str(exc)
