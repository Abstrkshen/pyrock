// pyrock calls this module. The window, the camera, and the model all live in
// libvision.so, which is C++ linked to OpenCV, ONNX Runtime, and raylib.
// pyrock: link dl
#include <dlfcn.h>
#include <stdio.h>
#include <string.h>

/* A path next to this source file (pyrock compiles it by its absolute path,
   so __FILE__ is absolute). */
static const char *beside(const char *name) {
    static char buf[4096];
    const char *slash = strrchr(__FILE__, '/');
    int n = slash ? (int)(slash - __FILE__) : 0;
    snprintf(buf, sizeof buf, "%.*s%s%s", n, __FILE__, n ? "/" : "", name);
    return buf;
}

const char *run(int frames, const char *source) {
    static void *lib;
    static const char *(*vision_run)(int, const char *);
    static char err[512];
    if (!vision_run) {
        lib = dlopen(beside("lib/libvision.so"), RTLD_NOW | RTLD_LOCAL);
        if (!lib) {
            snprintf(err, sizeof err, "vision: %s", dlerror());
            return err;
        }
        vision_run = (const char *(*)(int, const char *))dlsym(lib, "vision_run");
        if (!vision_run) {
            snprintf(err, sizeof err, "vision: %s", dlerror());
            return err;
        }
    }
    return vision_run(frames, source ? source : "");
}
