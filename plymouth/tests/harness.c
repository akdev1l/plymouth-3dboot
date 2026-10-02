/* SPDX-License-Identifier: GPL-3.0-or-later */
/* Headless test harness for the plymouth-3dboot splash plugin.
 *
 * The plugin is dlopen()ed and driven with a real event loop, key file and
 * pixel buffer. This executable is linked with --export-dynamic and defines
 * the ply_pixel_display_* functions (a fake display) and ply_get_timestamp
 * (controllable time); the plugin's references bind to these definitions
 * instead of libply's (ELF symbol interposition).
 *
 *   harness PLUGIN.so MODEL */
#define _GNU_SOURCE
#include <dlfcn.h>
#include <stdbool.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>
#include <unistd.h>

#include <ply-boot-splash-plugin.h>
#include <ply-event-loop.h>
#include <ply-key-file.h>
#include <ply-pixel-buffer.h>
#include <ply-pixel-display.h>
#include <ply-utils.h>

#include <plymouth-3dboot.h>

#define CHECK(cond)                                                                 \
    do {                                                                            \
        if (!(cond)) {                                                              \
            fprintf(stderr, "%s:%d: check failed: %s\n", __FILE__, __LINE__, #cond); \
            exit(1);                                                                \
        }                                                                           \
    } while (0)

enum { W = 64, H = 48 };

/* --- Interposed time ------------------------------------------------------ */

static bool time_frozen = true;
static double frozen_time = 1000.0;

double ply_get_timestamp(void) {
    if (time_frozen)
        return frozen_time;
    struct timespec ts;
    clock_gettime(CLOCK_MONOTONIC, &ts);
    return (double) ts.tv_sec + ts.tv_nsec / 1e9;
}

/* --- Fake pixel display ------------------------------------------------------ */

struct _ply_pixel_display {
    unsigned long width, height;
    ply_pixel_display_draw_handler_t handler;
    void *user_data;
    ply_pixel_buffer_t *buffer;
    int draws;
};

unsigned long ply_pixel_display_get_width(ply_pixel_display_t *display) { return display->width; }
unsigned long ply_pixel_display_get_height(ply_pixel_display_t *display) { return display->height; }
int ply_pixel_display_get_device_scale(ply_pixel_display_t *display) { (void) display; return 1; }

void ply_pixel_display_set_draw_handler(ply_pixel_display_t *display, ply_pixel_display_draw_handler_t handler,
                                        void *user_data) {
    display->handler = handler;
    display->user_data = user_data;
}

void ply_pixel_display_draw_area(ply_pixel_display_t *display, int x, int y, int width, int height) {
    display->draws++;
    if (display->handler != NULL)
        display->handler(display->user_data, display->buffer, x, y, width, height, display);
}

/* --- Helpers --------------------------------------------------------------- */

/* The frame the plugin should show at animation time `t`, via the C API. */
static void expected_frame(const char *model_path, double t, uint32_t *out) {
    p3b_model *model = NULL;
    CHECK(p3b_model_load_file(model_path, &model) == P3B_STATUS_OK);
    p3b_render_options options = p3b_render_options_default();
    p3b_renderer *renderer = NULL;
    CHECK(p3b_renderer_new(model, 0, W, H, &options, &renderer) == P3B_STATUS_OK);
    CHECK(p3b_render_frame(renderer, t, (uint8_t *) out, W * H * 4, W * 4, P3B_PIXEL_FORMAT_ARGB32_PREMULTIPLIED) ==
          P3B_STATUS_OK);
    p3b_renderer_free(renderer);
    p3b_model_free(model);
}

static void check_frame(ply_pixel_display_t *display, const char *model_path, double t) {
    static uint32_t expected[W * H];
    expected_frame(model_path, t, expected);
    uint32_t *actual = ply_pixel_buffer_get_argb32_data(display->buffer);
    CHECK(memcmp(actual, expected, sizeof expected) == 0);
}

static void quit_loop(void *user_data, ply_event_loop_t *loop) {
    (void) user_data;
    ply_event_loop_exit(loop, 0);
}

/* Runs the event loop for `seconds` of real time. */
static void run_for(ply_event_loop_t *loop, double seconds) {
    ply_event_loop_watch_for_timeout(loop, seconds, quit_loop, NULL);
    ply_event_loop_run(loop);
}

int main(int argc, char **argv) {
    CHECK(argc == 3);
    const char *model_path = argv[2];

    char theme[] = "/tmp/p3b-theme-XXXXXX";
    int fd = mkstemp(theme);
    CHECK(fd >= 0);
    FILE *f = fdopen(fd, "w");
    CHECK(f != NULL);
    fprintf(f, "[Plymouth Theme]\nModuleName=plymouth-3dboot\n\n[plymouth-3dboot]\nModelFile=%s\nFramesPerSecond=30\n",
            model_path);
    fclose(f);
    ply_key_file_t *key_file = ply_key_file_new(theme);
    CHECK(ply_key_file_load(key_file));

    void *module = dlopen(argv[1], RTLD_NOW | RTLD_LOCAL);
    if (module == NULL)
        fprintf(stderr, "dlopen: %s\n", dlerror());
    CHECK(module != NULL);
    ply_boot_splash_plugin_interface_t *(*get_interface)(void) =
        (ply_boot_splash_plugin_interface_t * (*) (void)) dlsym(module, "ply_boot_splash_plugin_get_interface");
    CHECK(get_interface != NULL);
    ply_boot_splash_plugin_interface_t *iface = get_interface();

    ply_boot_splash_plugin_t *plugin = iface->create_plugin(key_file);
    CHECK(plugin != NULL);

    struct _ply_pixel_display display = {.width = W, .height = H, .buffer = ply_pixel_buffer_new(W, H)};
    iface->add_pixel_display(plugin, &display);
    CHECK(display.handler != NULL);

    /* Before the splash is shown, a redraw shows time 0. */
    ply_pixel_display_draw_area(&display, 0, 0, W, H);
    check_frame(&display, model_path, 0.0);

    /* Shown at frozen time T: redraws follow the virtual clock exactly. */
    ply_event_loop_t *loop = ply_event_loop_new();
    CHECK(iface->show_splash_screen(plugin, loop, NULL, PLY_BOOT_SPLASH_MODE_BOOT_UP));
    check_frame(&display, model_path, 0.0);
    frozen_time += 0.75;
    ply_pixel_display_draw_area(&display, 0, 0, W, H);
    check_frame(&display, model_path, 0.75);
    frozen_time += 2.0;
    ply_pixel_display_draw_area(&display, 10, 10, 5, 5); /* partial redraw */
    check_frame(&display, model_path, 2.75);

    /* In real time the frame timer keeps redrawing (30 fps)... */
    time_frozen = false;
    int before = display.draws;
    run_for(loop, 0.3);
    int during = display.draws - before;
    printf("draws in 0.3 s: %d\n", during);
    CHECK(during >= 3);

    /* ...and stops when the splash is hidden. */
    iface->hide_splash_screen(plugin, loop);
    before = display.draws;
    run_for(loop, 0.15);
    CHECK(display.draws == before);

    iface->remove_pixel_display(plugin, &display);
    CHECK(display.handler == NULL);
    iface->destroy_plugin(plugin);

    /* A theme without a model is rejected cleanly. */
    ply_key_file_t *empty = ply_key_file_new("/dev/null");
    CHECK(iface->create_plugin(empty) == NULL);
    ply_key_file_free(empty);

    ply_event_loop_free(loop);
    ply_pixel_buffer_free(display.buffer);
    ply_key_file_free(key_file);
    unlink(theme);
    dlclose(module);
    puts("harness: ok");
    return 0;
}
