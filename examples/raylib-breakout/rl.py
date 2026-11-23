"""Calls into libraylib through liblumen.so. No Python raylib package."""

import ctypes
import os
from ctypes import c_char_p, c_float, c_int, c_ubyte

_lib = ctypes.CDLL(os.path.join(os.path.dirname(os.path.abspath(__file__)), "lib", "liblumen.so"))

_lib.rl_init.argtypes = [c_int, c_int, c_char_p]
_lib.rl_exit_key.argtypes = [c_int]
_lib.rl_fps.argtypes = [c_int]
_lib.rl_should_close.restype = c_int
_lib.rl_rect.argtypes = [c_int, c_int, c_int, c_int, c_ubyte, c_ubyte, c_ubyte, c_ubyte]
_lib.rl_circle.argtypes = [c_int, c_int, c_float, c_ubyte, c_ubyte, c_ubyte, c_ubyte]
_lib.rl_text.argtypes = [c_char_p, c_int, c_int, c_int, c_ubyte, c_ubyte, c_ubyte, c_ubyte]
_lib.rl_text_width.argtypes = [c_char_p, c_int]
_lib.rl_text_width.restype = c_int
_lib.rl_down.argtypes = [c_int]
_lib.rl_down.restype = c_int
_lib.rl_pressed.argtypes = [c_int]
_lib.rl_pressed.restype = c_int
_lib.rl_upload.argtypes = [ctypes.POINTER(c_ubyte), c_int, c_int, c_int]
_lib.rl_upload.restype = c_int
_lib.rl_sprite.argtypes = [c_int, c_float, c_float, c_float, c_float, c_int, c_ubyte, c_ubyte, c_ubyte, c_ubyte]
_lib.rl_sprite_src.argtypes = [c_int, c_float, c_float, c_float, c_float, c_float, c_float, c_float, c_float, c_ubyte, c_ubyte, c_ubyte, c_ubyte]
_lib.rl_tex_w.argtypes = [c_int]
_lib.rl_tex_w.restype = c_int
_lib.rl_tex_h.argtypes = [c_int]
_lib.rl_tex_h.restype = c_int
_lib.rl_free.argtypes = [c_int]

KEY_NULL = 0
KEY_SPACE = 32
KEY_A = 65
KEY_D = 68
KEY_P = 80
KEY_S = 83
KEY_W = 87
KEY_ESCAPE = 256
KEY_ENTER = 257
KEY_RIGHT = 262
KEY_LEFT = 263
KEY_DOWN = 264
KEY_UP = 265
WHITE = (255, 255, 255, 255)


def _bytes(text):
    if isinstance(text, bytes):
        return text
    return str(text).encode()


def _parts(color):
    return int(color[0]), int(color[1]), int(color[2]), int(color[3])


def init(w, h, title):
    _lib.rl_init(int(w), int(h), _bytes(title))


def exit_key(key):
    _lib.rl_exit_key(int(key))


def fps(value):
    _lib.rl_fps(int(value))


def should_close():
    return _lib.rl_should_close()


def begin():
    _lib.rl_begin()


def end():
    _lib.rl_end()


def close():
    _lib.rl_close()


def rect(x, y, w, h, color):
    r, g, b, a = _parts(color)
    _lib.rl_rect(int(x), int(y), int(w), int(h), r, g, b, a)


def circle(x, y, radius, color):
    r, g, b, a = _parts(color)
    _lib.rl_circle(int(x), int(y), float(radius), r, g, b, a)


def text(message, x, y, size, color):
    r, g, b, a = _parts(color)
    _lib.rl_text(_bytes(message), int(x), int(y), int(size), r, g, b, a)


def text_width(message, size):
    return _lib.rl_text_width(_bytes(message), int(size))


def down(key):
    return _lib.rl_down(int(key))


def pressed(key):
    return _lib.rl_pressed(int(key))


def upload(rgba, width, height):
    buf = (c_ubyte * len(rgba)).from_buffer_copy(rgba)
    return _lib.rl_upload(buf, len(rgba), int(width), int(height))


def sprite(texture, x, y, w, h, flip, color):
    r, g, b, a = _parts(color)
    _lib.rl_sprite(int(texture), float(x), float(y), float(w), float(h), 1 if flip else 0, r, g, b, a)


def sprite_src(texture, sx, sy, sw, sh, dx, dy, dw, dh, color):
    r, g, b, a = _parts(color)
    _lib.rl_sprite_src(int(texture), float(sx), float(sy), float(sw), float(sh), float(dx), float(dy), float(dw), float(dh), r, g, b, a)


def tex_w(texture):
    return _lib.rl_tex_w(int(texture))


def tex_h(texture):
    return _lib.rl_tex_h(int(texture))


def free(texture):
    _lib.rl_free(int(texture))


def IsKeyDown(key):
    return down(key)


def IsKeyPressed(key):
    return pressed(key)


def DrawText(message, x, y, size, color):
    text(message, x, y, size, color)


def MeasureText(message, size):
    return text_width(message, size)


def DrawRectangle(x, y, w, h, color):
    rect(x, y, w, h, color)


def DrawCircle(x, y, radius, color):
    circle(x, y, radius, color)


def BeginDrawing():
    begin()


def EndDrawing():
    end()


def UnloadTexture(texture):
    free(texture)


def InitWindow(w, h, title):
    init(w, h, title)


def SetExitKey(key):
    exit_key(key)


def SetTargetFPS(value):
    fps(value)


def WindowShouldClose():
    return should_close()


def CloseWindow():
    close()
