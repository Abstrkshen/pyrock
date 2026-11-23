// The window is the C raylib library, loaded from lib/ next to this file.
// pyrock: link dl
#include <dlfcn.h>
#include <stdio.h>
#include <string.h>

#include "raylib.h"

/* A path next to this source file (pyrock compiles it by its absolute path,
   so __FILE__ is absolute). */
static const char *beside(const char *name) {
    static char buf[4096];
    const char *slash = strrchr(__FILE__, '/');
    int n = slash ? (int)(slash - __FILE__) : 0;
    snprintf(buf, sizeof buf, "%.*s%s%s", n, __FILE__, n ? "/" : "", name);
    return buf;
}

typedef void (*init_fn)(int, int, const char *);
typedef void (*fps_fn)(int);
typedef void (*void_fn)(void);
typedef void (*clear_fn)(Color);
typedef void (*circle_fn)(int, int, float, Color);
typedef void (*text_fn)(const char *, int, int, int, Color);

static const char *note(const char *text) {
    static char buf[512];
    snprintf(buf, sizeof buf, "%s", text);
    return buf;
}

const char *show(int frames) {
    static char buf[128];
    void *lib = dlopen(beside("lib/libraylib.so.550"), RTLD_NOW | RTLD_LOCAL);
    if (!lib) {
        snprintf(buf, sizeof buf, "raylib: %s", dlerror());
        return buf;
    }
    init_fn init_window = (init_fn)dlsym(lib, "InitWindow");
    fps_fn set_fps = (fps_fn)dlsym(lib, "SetTargetFPS");
    void_fn begin_drawing = (void_fn)dlsym(lib, "BeginDrawing");
    clear_fn clear_background = (clear_fn)dlsym(lib, "ClearBackground");
    circle_fn draw_circle = (circle_fn)dlsym(lib, "DrawCircle");
    text_fn draw_text = (text_fn)dlsym(lib, "DrawText");
    void_fn end_drawing = (void_fn)dlsym(lib, "EndDrawing");
    void_fn close_window = (void_fn)dlsym(lib, "CloseWindow");
    if (!init_window || !set_fps || !begin_drawing || !clear_background || !draw_circle || !draw_text || !end_drawing || !close_window) {
        dlclose(lib);
        return note("raylib: a required symbol is missing");
    }
    init_window(640, 480, "pyrock + raylib");
    set_fps(60);
    for (int i = 0; i < frames; i++) {
        int x = 80 + (i * 3) % 480;
        begin_drawing();
        clear_background((Color){20, 24, 36, 255});
        draw_circle(x, 250, 36.0f, GOLD);
        draw_circle(320, 120 + (i % 180), 16.0f, SKYBLUE);
        draw_text("pyrock + raylib", 36, 40, 32, RAYWHITE);
        end_drawing();
    }
    close_window();
    dlclose(lib);
    snprintf(buf, sizeof buf, "raylib 5.5, frames %d", frames);
    return buf;
}
