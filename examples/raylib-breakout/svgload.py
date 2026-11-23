"""Load SVG assets with librsvg and upload them as raylib textures.

This raylib build has no SVG decoder. Frames live in each SVG as `<g id="...">`
groups. The loader isolates one group, rasterizes it, and hands the pixels to
raylib. Call `load_clip` only after `InitWindow`.
"""

import ctypes
import os
import xml.etree.ElementTree as ET
from ctypes import POINTER, byref, c_bool, c_char_p, c_double, c_int, c_size_t, c_void_p

import rl

_rsvg = ctypes.CDLL("librsvg-2.so.2")
_cairo = ctypes.CDLL("libcairo.so.2")
_gobj = ctypes.CDLL("libgobject-2.0.so.0")

_rsvg.rsvg_handle_new_from_data.restype = c_void_p
_rsvg.rsvg_handle_new_from_data.argtypes = [c_char_p, c_size_t, POINTER(c_void_p)]
_rsvg.rsvg_handle_render_cairo.restype = c_bool
_rsvg.rsvg_handle_render_cairo.argtypes = [c_void_p, c_void_p]
_rsvg.rsvg_handle_get_intrinsic_size_in_pixels.restype = c_bool
_rsvg.rsvg_handle_get_intrinsic_size_in_pixels.argtypes = [c_void_p, POINTER(c_double), POINTER(c_double)]

_cairo.cairo_image_surface_create.restype = c_void_p
_cairo.cairo_image_surface_create.argtypes = [c_int, c_int, c_int]
_cairo.cairo_create.restype = c_void_p
_cairo.cairo_create.argtypes = [c_void_p]
_cairo.cairo_destroy.argtypes = [c_void_p]
_cairo.cairo_surface_destroy.argtypes = [c_void_p]
_cairo.cairo_surface_flush.argtypes = [c_void_p]
_cairo.cairo_image_surface_get_data.restype = POINTER(ctypes.c_ubyte)
_cairo.cairo_image_surface_get_data.argtypes = [c_void_p]
_cairo.cairo_image_surface_get_stride.restype = c_int
_cairo.cairo_image_surface_get_stride.argtypes = [c_void_p]
_gobj.g_object_unref.argtypes = [c_void_p]


def asset_path(name):
    return os.path.join(os.path.dirname(os.path.abspath(__file__)), "assets", name)


def frame_document(path, frame_id):
    raw = open(path, "rb").read()
    if frame_id is None:
        return raw
    root = ET.fromstring(raw)
    defs = []
    chosen = None
    for child in list(root):
        tag = child.tag.split("}")[-1]
        if tag == "defs":
            defs.append(child)
        elif tag == "g" and child.get("id") == frame_id:
            chosen = child
    if chosen is None:
        raise RuntimeError("frame `%s` is not in %s" % (frame_id, path))
    out = ET.Element(root.tag, dict(root.attrib))
    for item in defs:
        out.append(item)
    out.append(chosen)
    return ET.tostring(out, encoding="utf-8")


def rasterize(svg_bytes):
    err = c_void_p()
    handle = _rsvg.rsvg_handle_new_from_data(svg_bytes, len(svg_bytes), byref(err))
    if not handle:
        raise RuntimeError("librsvg could not read an SVG asset")
    surface = None
    cr = None
    try:
        iw = c_double()
        ih = c_double()
        if not _rsvg.rsvg_handle_get_intrinsic_size_in_pixels(handle, byref(iw), byref(ih)):
            raise RuntimeError("SVG has no pixel size; set width and height")
        width = max(1, int(round(iw.value)))
        height = max(1, int(round(ih.value)))
        surface = _cairo.cairo_image_surface_create(0, width, height)
        cr = _cairo.cairo_create(surface)
        if not _rsvg.rsvg_handle_render_cairo(handle, cr):
            raise RuntimeError("librsvg failed to render an SVG asset")
        _cairo.cairo_destroy(cr)
        cr = None
        _cairo.cairo_surface_flush(surface)
        stride = _cairo.cairo_image_surface_get_stride(surface)
        data = _cairo.cairo_image_surface_get_data(surface)
        rgba = bytearray(width * height * 4)
        o = 0
        y = 0
        while y < height:
            row = y * stride
            x = 0
            while x < width:
                i = row + x * 4
                b = data[i]
                g = data[i + 1]
                r = data[i + 2]
                a = data[i + 3]
                if a == 0:
                    r = 0
                    g = 0
                    b = 0
                elif a < 255:
                    r = (r * 255) // a
                    g = (g * 255) // a
                    b = (b * 255) // a
                rgba[o] = r
                rgba[o + 1] = g
                rgba[o + 2] = b
                rgba[o + 3] = a
                o += 4
                x += 1
            y += 1
        return bytes(rgba), width, height
    finally:
        if cr:
            _cairo.cairo_destroy(cr)
        if surface:
            _cairo.cairo_surface_destroy(surface)
        _gobj.g_object_unref(handle)


def upload(rgba, width, height):
    return rl.upload(rgba, width, height)


def load_clip(filename, frame_ids):
    path = asset_path(filename)
    frames = []
    for frame_id in frame_ids:
        rgba, width, height = rasterize(frame_document(path, frame_id))
        frames.append(upload(rgba, width, height))
    return frames


def draw_sprite(texture, x, y, w, h, flip, tint):
    rl.sprite(texture, x, y, w, h, flip, tint)
