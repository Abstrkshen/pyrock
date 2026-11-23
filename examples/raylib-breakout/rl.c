/* Flat C API over libraylib. Python calls these; it does not import the raylib package. */
#define _GNU_SOURCE
#include <dlfcn.h>
#include <stdio.h>
#include <string.h>

#include "raylib.h"

/* A path next to this shared library (lib/liblumen.so), found with dladdr. */
static const char *beside_self(const char *name) {
    static char buf[4096];
    Dl_info info;
    const char *me = "";
    if (dladdr((void *)beside_self, &info) && info.dli_fname) {
        me = info.dli_fname;
    }
    const char *slash = strrchr(me, '/');
    int n = slash ? (int)(slash - me) : 0;
    snprintf(buf, sizeof buf, "%.*s%s%s", n, me, n ? "/" : "", name);
    return buf;
}

typedef void (*init_fn)(int, int, const char *);
typedef void (*key_fn)(int);
typedef void (*fps_fn)(int);
typedef bool (*bool_fn)(void);
typedef bool (*keystate_fn)(int);
typedef void (*void_fn)(void);
typedef void (*rect_fn)(int, int, int, int, Color);
typedef void (*circle_fn)(int, int, float, Color);
typedef void (*text_fn)(const char *, int, int, int, Color);
typedef int (*measure_fn)(const char *, int);
typedef Texture2D (*from_image_fn)(Image);
typedef void (*filter_fn)(Texture2D, int);
typedef void (*unload_fn)(Texture2D);
typedef void (*pro_fn)(Texture2D, Rectangle, Rectangle, Vector2, float, Color);

static init_fn p_init;
static key_fn p_exit_key;
static fps_fn p_fps;
static bool_fn p_should_close;
static void_fn p_begin;
static void_fn p_end;
static void_fn p_close;
static rect_fn p_rect;
static circle_fn p_circle;
static text_fn p_text;
static measure_fn p_measure;
static keystate_fn p_down;
static keystate_fn p_pressed;
static from_image_fn p_from_image;
static filter_fn p_filter;
static unload_fn p_unload;
static pro_fn p_pro;

static Texture2D textures[96];
static int live[96];

static int ready(void) {
    if (p_init) {
        return 1;
    }
    void *lib = dlopen(beside_self("libraylib.so.550"), RTLD_NOW | RTLD_LOCAL);
    if (!lib) {
        fprintf(stderr, "raylib: %s\n", dlerror());
        return 0;
    }
    p_init = (init_fn)dlsym(lib, "InitWindow");
    p_exit_key = (key_fn)dlsym(lib, "SetExitKey");
    p_fps = (fps_fn)dlsym(lib, "SetTargetFPS");
    p_should_close = (bool_fn)dlsym(lib, "WindowShouldClose");
    p_begin = (void_fn)dlsym(lib, "BeginDrawing");
    p_end = (void_fn)dlsym(lib, "EndDrawing");
    p_close = (void_fn)dlsym(lib, "CloseWindow");
    p_rect = (rect_fn)dlsym(lib, "DrawRectangle");
    p_circle = (circle_fn)dlsym(lib, "DrawCircle");
    p_text = (text_fn)dlsym(lib, "DrawText");
    p_measure = (measure_fn)dlsym(lib, "MeasureText");
    p_down = (keystate_fn)dlsym(lib, "IsKeyDown");
    p_pressed = (keystate_fn)dlsym(lib, "IsKeyPressed");
    p_from_image = (from_image_fn)dlsym(lib, "LoadTextureFromImage");
    p_filter = (filter_fn)dlsym(lib, "SetTextureFilter");
    p_unload = (unload_fn)dlsym(lib, "UnloadTexture");
    p_pro = (pro_fn)dlsym(lib, "DrawTexturePro");
    if (!p_init || !p_rect || !p_from_image || !p_pro) {
        fprintf(stderr, "raylib: a required symbol is missing\n");
        return 0;
    }
    return 1;
}

static Color color(unsigned char r, unsigned char g, unsigned char b, unsigned char a) {
    Color c;
    c.r = r;
    c.g = g;
    c.b = b;
    c.a = a;
    return c;
}

static Texture2D *slot(int id) {
    if (id < 1 || id >= 96 || !live[id]) {
        return NULL;
    }
    return &textures[id];
}

void rl_init(int w, int h, const char *title) {
    if (!ready()) {
        return;
    }
    p_init(w, h, title);
}

void rl_exit_key(int key) {
    if (p_exit_key) {
        p_exit_key(key);
    }
}

void rl_fps(int fps) {
    if (p_fps) {
        p_fps(fps);
    }
}

int rl_should_close(void) {
    return p_should_close && p_should_close();
}

void rl_begin(void) {
    if (p_begin) {
        p_begin();
    }
}

void rl_end(void) {
    if (p_end) {
        p_end();
    }
}

void rl_close(void) {
    if (p_close) {
        p_close();
    }
}

void rl_rect(int x, int y, int w, int h, unsigned char r, unsigned char g, unsigned char b, unsigned char a) {
    if (p_rect) {
        p_rect(x, y, w, h, color(r, g, b, a));
    }
}

void rl_circle(int x, int y, float radius, unsigned char r, unsigned char g, unsigned char b, unsigned char a) {
    if (p_circle) {
        p_circle(x, y, radius, color(r, g, b, a));
    }
}

void rl_text(const char *text, int x, int y, int size, unsigned char r, unsigned char g, unsigned char b, unsigned char a) {
    if (p_text) {
        p_text(text, x, y, size, color(r, g, b, a));
    }
}

int rl_text_width(const char *text, int size) {
    if (!p_measure) {
        return 0;
    }
    return p_measure(text, size);
}

int rl_down(int key) {
    return p_down && p_down(key);
}

int rl_pressed(int key) {
    return p_pressed && p_pressed(key);
}

int rl_upload(const unsigned char *rgba, int n, int w, int h) {
    if (!ready() || !rgba || w < 1 || h < 1 || n < w * h * 4) {
        return 0;
    }
    int id = 1;
    while (id < 96 && live[id]) {
        id += 1;
    }
    if (id >= 96) {
        return 0;
    }
    Image image;
    image.data = (void *)rgba;
    image.width = w;
    image.height = h;
    image.mipmaps = 1;
    image.format = PIXELFORMAT_UNCOMPRESSED_R8G8B8A8;
    textures[id] = p_from_image(image);
    p_filter(textures[id], TEXTURE_FILTER_BILINEAR);
    live[id] = 1;
    return id;
}

void rl_sprite(int id, float x, float y, float w, float h, int flip, unsigned char r, unsigned char g, unsigned char b, unsigned char a) {
    Texture2D *tex = slot(id);
    if (!tex || !p_pro) {
        return;
    }
    float sw = (float)tex->width;
    float sh = (float)tex->height;
    Rectangle src;
    src.x = flip ? sw : 0.0f;
    src.y = 0.0f;
    src.width = flip ? -sw : sw;
    src.height = sh;
    Rectangle dst;
    dst.x = x;
    dst.y = y;
    dst.width = w;
    dst.height = h;
    Vector2 origin;
    origin.x = 0.0f;
    origin.y = 0.0f;
    p_pro(*tex, src, dst, origin, 0.0f, color(r, g, b, a));
}

void rl_sprite_src(int id, float sx, float sy, float sw, float sh, float dx, float dy, float dw, float dh, unsigned char r, unsigned char g, unsigned char b, unsigned char a) {
    Texture2D *tex = slot(id);
    if (!tex || !p_pro) {
        return;
    }
    Rectangle src;
    src.x = sx;
    src.y = sy;
    src.width = sw;
    src.height = sh;
    Rectangle dst;
    dst.x = dx;
    dst.y = dy;
    dst.width = dw;
    dst.height = dh;
    Vector2 origin;
    origin.x = 0.0f;
    origin.y = 0.0f;
    p_pro(*tex, src, dst, origin, 0.0f, color(r, g, b, a));
}

int rl_tex_w(int id) {
    Texture2D *tex = slot(id);
    return tex ? tex->width : 0;
}

int rl_tex_h(int id) {
    Texture2D *tex = slot(id);
    return tex ? tex->height : 0;
}

void rl_free(int id) {
    Texture2D *tex = slot(id);
    if (!tex || !p_unload) {
        return;
    }
    p_unload(*tex);
    live[id] = 0;
    memset(&textures[id], 0, sizeof textures[id]);
}
