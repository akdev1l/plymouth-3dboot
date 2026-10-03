/* SPDX-License-Identifier: GPL-3.0-or-later */
/* Headless test harness for the plymouth-3dboot splash plugin.
 *
 * The plugin is dlopen()ed and driven with a real event loop, key file and
 * pixel buffer. This executable is linked with --export-dynamic and defines
 * the ply_pixel_display_* functions (a fake display) and ply_get_timestamp
 * (controllable time), and wraps ply_event_loop_watch_for_timeout to capture
 * the plugin's frame timer; the plugin's references bind to these
 * definitions instead of libply's (ELF symbol interposition).
 *
 *   harness PLUGIN.so MODEL */
#define _GNU_SOURCE
#define PLY_ENABLE_TRACING 1 /* ply_logger_toggle_tracing */
#include <dlfcn.h>
#include <errno.h>
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
#include <ply-logger.h>
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
    ply_rectangle_t last_area; /* of the last draw */
};

unsigned long ply_pixel_display_get_width(ply_pixel_display_t *display) { return display->width; }
unsigned long ply_pixel_display_get_height(ply_pixel_display_t *display) { return display->height; }
int ply_pixel_display_get_device_scale(ply_pixel_display_t *display) { (void) display; return 1; }

void ply_pixel_display_set_draw_handler(ply_pixel_display_t *display, ply_pixel_display_draw_handler_t handler,
                                        void *user_data) {
    display->handler = handler;
    display->user_data = user_data;
}

/* Like Plymouth's: the handler draws into the buffer clipped to the area,
 * so pixels outside keep their previous content. */
void ply_pixel_display_draw_area(ply_pixel_display_t *display, int x, int y, int width, int height) {
    display->draws++;
    ply_rectangle_t area = {.x = x, .y = y, .width = (unsigned long) width, .height = (unsigned long) height};
    display->last_area = area;
    if (display->handler != NULL) {
        ply_pixel_buffer_push_clip_area(display->buffer, &area);
        display->handler(display->user_data, display->buffer, x, y, width, height, display);
        ply_pixel_buffer_pop_clip_area(display->buffer);
    }
}

/* --- Captured frame timer ---------------------------------------------------- */

static ply_event_loop_timeout_handler_t frame_handler; /* the plugin's last timeout */
static void *frame_user_data;
static double frame_delay;

void ply_event_loop_watch_for_timeout(ply_event_loop_t *loop, double seconds,
                                      ply_event_loop_timeout_handler_t handler, void *user_data) {
    static void (*real)(ply_event_loop_t *, double, ply_event_loop_timeout_handler_t, void *);
    if (real == NULL)
        *(void **) &real = dlsym(RTLD_NEXT, "ply_event_loop_watch_for_timeout");
    Dl_info info;
    if (dladdr(*(void **) &handler, &info) != 0 && info.dli_fname != NULL &&
        strstr(info.dli_fname, "plymouth-3dboot.so") != NULL) {
        frame_handler = handler;
        frame_user_data = user_data;
        frame_delay = seconds;
    }
    real(loop, seconds, handler, user_data);
}

/* Runs the plugin's frame timer now (cancelling the pending one). */
static void tick(ply_event_loop_t *loop) {
    CHECK(frame_handler != NULL);
    ply_event_loop_stop_watching_for_timeout(loop, frame_handler, frame_user_data);
    frame_handler(frame_user_data, loop);
}

static bool near(double a, double b) { return a - b < 1e-6 && b - a < 1e-6; }

/* --- Helpers --------------------------------------------------------------- */

/* The frame the plugin should show at animation time `t`, via the C API. */
static void expected_frame(const char *model_path, double t, uint32_t *out) {
    p3b_model *model = NULL;
    CHECK(p3b_model_load_file(model_path, &model) == P3B_STATUS_OK);
    const uint8_t floor[3] = {0xC8, 0xC8, 0xC8}; /* as in the harness theme */
    CHECK(p3b_model_add_floor(model, floor, 4.0f) == P3B_STATUS_OK);
    p3b_render_options options = p3b_render_options_default();
    const uint8_t sky[4] = {0x87, 0xCE, 0xEB, 0xFF};
    memcpy(options.background, sky, sizeof sky);
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
    fprintf(f,
            "[Plymouth Theme]\nModuleName=plymouth-3dboot\n\n[plymouth-3dboot]\nModelFile=%s\nFramesPerSecond=30\n"
            "BackgroundColor=87CEEB\nFloorColor=C8C8C8\nFloorSize=4\n",
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
    /* plymouthd asserts these exist (ply-boot-splash.c) instead of checking. */
    CHECK(iface->show_splash_screen != NULL);
    CHECK(iface->hide_splash_screen != NULL);
    CHECK(iface->update_status != NULL);

    ply_boot_splash_plugin_t *plugin = iface->create_plugin(key_file);
    CHECK(plugin != NULL);

    struct _ply_pixel_display display = {.width = W, .height = H, .buffer = ply_pixel_buffer_new(W, H)};
    iface->add_pixel_display(plugin, &display);
    CHECK(display.handler != NULL);

    /* Before the splash is shown, a redraw shows time 0. */
    ply_pixel_display_draw_area(&display, 0, 0, W, H);
    check_frame(&display, model_path, 0.0);

    /* Shown at frozen time T: frames follow the virtual clock exactly, and
     * are due every 1/30 s from T whatever the time spent drawing. */
    ply_event_loop_t *loop = ply_event_loop_new();
    CHECK(iface->show_splash_screen(plugin, loop, NULL, PLY_BOOT_SPLASH_MODE_BOOT_UP));
    check_frame(&display, model_path, 0.0);
    CHECK(near(frame_delay, 1.0 / 30));
    frozen_time += 0.75; /* 22.5 frame periods: the next is due at 23 */
    tick(loop);
    check_frame(&display, model_path, 0.75);
    CHECK(near(frame_delay, 23.0 / 30 - 0.75));
    /* Only the changed area was redrawn (the logo moves; the background
     * does not), and the clipped buffer still matches the whole frame. */
    CHECK(display.last_area.width * display.last_area.height < (unsigned long) (W * H));
    ply_pixel_display_draw_area(&display, 10, 10, 5, 5); /* Plymouth's own redraw */
    check_frame(&display, model_path, 0.75);
    for (int i = 0; i < 20; i++) {
        frozen_time += 0.137;
        tick(loop);
        check_frame(&display, model_path, frozen_time - 1000.0); /* as the plugin computes it */
    }
    /* Frame statistics every 5 s go to Plymouth's debug log (ply_trace
     * writes to the error logger, which plymouthd --debug sends there). */
    char log_path[] = "/tmp/p3b-log-XXXXXX";
    int log_fd = mkstemp(log_path);
    CHECK(log_fd >= 0);
    close(log_fd);
    ply_logger_t *logger = ply_logger_get_error_default();
    CHECK(ply_logger_open_file(logger, log_path));
    ply_logger_toggle_tracing(logger);
    frozen_time = 1000.0 + 6.0;
    tick(loop);
    check_frame(&display, model_path, 6.0);
    ply_logger_flush(logger);
    ply_logger_toggle_tracing(logger);
    ply_logger_close_file(logger);
    char log_text[4096] = {0};
    FILE *log_file = fopen(log_path, "r");
    CHECK(log_file != NULL);
    size_t log_len = fread(log_text, 1, sizeof log_text - 1, log_file);
    fclose(log_file);
    unlink(log_path);
    CHECK(log_len > 0 && strstr(log_text, "plymouth-3dboot: ") != NULL && strstr(log_text, " fps; per frame: ") != NULL);

    /* Prompts and messages redraw and never disturb the model frame
     * (no label plugin is installed in the test environment). */
    int draws = display.draws;
    iface->display_password(plugin, "Disk password", 3);
    iface->display_question(plugin, "Continue?", "yes");
    iface->display_prompt(plugin, NULL, "secret", true);
    iface->display_message(plugin, "Checking disks");
    iface->hide_message(plugin, "Other message"); /* not shown: ignored */
    iface->hide_message(plugin, "Checking disks");
    iface->display_normal(plugin);
    iface->update_status(plugin, "systemd-udevd.service"); /* as during boot */
    CHECK(display.draws >= draws + 6);
    check_frame(&display, model_path, 6.0);

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
