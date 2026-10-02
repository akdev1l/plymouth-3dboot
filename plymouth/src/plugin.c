/* SPDX-License-Identifier: GPL-3.0-or-later */
/* Plymouth splash plugin rendering an animated 3D model with
 * libplymouth_3dboot. See docs/plymouth.md. */
#define _GNU_SOURCE
#include <stdbool.h>
#include <stdint.h>
#include <stdlib.h>
#include <string.h>
#include <strings.h>

#include <ply-boot-splash-plugin.h>
#include <ply-event-loop.h>
#include <ply-key-file.h>
#include <ply-logger.h>
#include <ply-pixel-buffer.h>
#include <ply-pixel-display.h>
#include <ply-utils.h>

#include <plymouth-3dboot.h>

#define GROUP "plymouth-3dboot"
#define EXPORT __attribute__((visibility("default")))

/* One display (head) and its renderer. */
typedef struct view {
    ply_boot_splash_plugin_t *plugin;
    ply_pixel_display_t *display;
    p3b_renderer *renderer;
    uint32_t *pixels;       /* last rendered frame, width * height ARGB32 */
    unsigned long width, height;
    double rendered_time;   /* animation time of `pixels`, or -1 */
} view_t;

struct _ply_boot_splash_plugin {
    p3b_model *model;
    size_t clip;            /* clip to play, or SIZE_MAX for none */
    p3b_render_options options;
    double fps;

    view_t **views;
    size_t view_count;

    ply_event_loop_t *loop; /* set while the splash is shown */
    double start_time;      /* ply_get_timestamp() when shown */
};

/* Parses "RRGGBB" into options->background (opaque); false if invalid. */
static bool parse_color(const char *text, uint8_t rgba[4]) {
    if (text == NULL || strlen(text) != 6)
        return false;
    char *end = NULL;
    unsigned long v = strtoul(text, &end, 16);
    if (end == NULL || *end != '\0')
        return false;
    rgba[0] = (uint8_t) (v >> 16);
    rgba[1] = (uint8_t) (v >> 8);
    rgba[2] = (uint8_t) v;
    rgba[3] = 255;
    return true;
}

static ply_boot_splash_plugin_t *create_plugin(ply_key_file_t *key_file) {
    char *path = ply_key_file_get_value(key_file, GROUP, "ModelFile");
    if (path == NULL) {
        ply_trace("plymouth-3dboot: theme has no ModelFile");
        return NULL;
    }
    p3b_model *model = NULL;
    p3b_status status = p3b_model_load_file(path, &model);
    if (status != P3B_STATUS_OK) {
        ply_trace("plymouth-3dboot: cannot load %s: %s", path, p3b_last_error());
        free(path);
        return NULL;
    }
    free(path);

    ply_boot_splash_plugin_t *plugin = calloc(1, sizeof *plugin);
    if (plugin == NULL) {
        p3b_model_free(model);
        return NULL;
    }
    plugin->model = model;
    plugin->options = p3b_render_options_default();
    plugin->clip = p3b_model_clip_count(model) > 0 ? 0 : SIZE_MAX;

    double period = ply_key_file_get_double(key_file, GROUP, "TurntablePeriod", 6.0);
    if (plugin->clip == SIZE_MAX && period > 0 &&
        p3b_model_add_turntable(model, 0, 1, 0, (float) period) == P3B_STATUS_OK)
        plugin->clip = 0;

    char *shading = ply_key_file_get_value(key_file, GROUP, "Shading");
    if (shading != NULL) {
        if (strcasecmp(shading, "lambert") == 0)
            plugin->options.shading = P3B_SHADING_LAMBERT;
        else if (strcasecmp(shading, "blinn-phong") == 0)
            plugin->options.shading = P3B_SHADING_BLINN_PHONG;
        free(shading);
    }
    char *color = ply_key_file_get_value(key_file, GROUP, "BackgroundColor");
    if (color != NULL && !parse_color(color, plugin->options.background)) {
        ply_trace("plymouth-3dboot: ignoring invalid BackgroundColor %s", color);
    }
    free(color);

    plugin->fps = ply_key_file_get_double(key_file, GROUP, "FramesPerSecond", 30.0);
    if (!(plugin->fps >= 1.0 && plugin->fps <= 60.0))
        plugin->fps = 30.0;
    return plugin;
}

/* Animation time: seconds since the splash was shown (0 before). */
static double animation_time(ply_boot_splash_plugin_t *plugin) {
    return plugin->loop != NULL ? ply_get_timestamp() - plugin->start_time : 0.0;
}

static void free_view_resources(view_t *view) {
    p3b_renderer_free(view->renderer);
    view->renderer = NULL;
    free(view->pixels);
    view->pixels = NULL;
}

/* (Re)creates the view's renderer for the display's current size. */
static bool prepare_view(view_t *view) {
    unsigned long width = ply_pixel_display_get_width(view->display);
    unsigned long height = ply_pixel_display_get_height(view->display);
    if (view->renderer != NULL && width == view->width && height == view->height)
        return true;
    free_view_resources(view);
    view->width = width;
    view->height = height;
    view->rendered_time = -1;
    if (width == 0 || height == 0 || width > P3B_MAX_SIZE || height > P3B_MAX_SIZE)
        return false;
    view->pixels = malloc(width * height * sizeof *view->pixels);
    if (view->pixels == NULL)
        return false;
    ply_boot_splash_plugin_t *plugin = view->plugin;
    if (p3b_renderer_new(plugin->model, plugin->clip, (uint32_t) width, (uint32_t) height, &plugin->options,
                         &view->renderer) != P3B_STATUS_OK) {
        ply_trace("plymouth-3dboot: renderer for %lux%lu: %s", width, height, p3b_last_error());
        free_view_resources(view);
        return false;
    }
    return true;
}

static void on_draw(void *user_data, ply_pixel_buffer_t *pixel_buffer, int x, int y, int width, int height,
                    ply_pixel_display_t *display) {
    (void) x;
    (void) y;
    (void) width;
    (void) height;
    (void) display;
    view_t *view = user_data;
    if (!prepare_view(view))
        return;
    /* Plymouth may draw several areas per frame: render once per time. */
    double t = animation_time(view->plugin);
    if (t != view->rendered_time) {
        size_t stride = view->width * sizeof *view->pixels;
        if (p3b_render_frame(view->renderer, t, (uint8_t *) view->pixels, stride * view->height, stride,
                             P3B_PIXEL_FORMAT_ARGB32_PREMULTIPLIED) != P3B_STATUS_OK) {
            ply_trace("plymouth-3dboot: render: %s", p3b_last_error());
            return;
        }
        view->rendered_time = t;
    }
    ply_rectangle_t area = {.x = 0, .y = 0, .width = view->width, .height = view->height};
    ply_pixel_buffer_fill_with_argb32_data(pixel_buffer, &area, view->pixels);
}

static void redraw_all(ply_boot_splash_plugin_t *plugin) {
    for (size_t i = 0; i < plugin->view_count; i++) {
        view_t *view = plugin->views[i];
        ply_pixel_display_draw_area(view->display, 0, 0, (int) ply_pixel_display_get_width(view->display),
                                    (int) ply_pixel_display_get_height(view->display));
    }
}

static void on_timeout(void *user_data, ply_event_loop_t *loop) {
    ply_boot_splash_plugin_t *plugin = user_data;
    if (plugin->loop == NULL)
        return;
    redraw_all(plugin);
    /* Time-based animation: a late tick drops frames, it never slows down. */
    ply_event_loop_watch_for_timeout(loop, 1.0 / plugin->fps, on_timeout, plugin);
}

static void add_pixel_display(ply_boot_splash_plugin_t *plugin, ply_pixel_display_t *display) {
    view_t *view = calloc(1, sizeof *view);
    view_t **views = realloc(plugin->views, (plugin->view_count + 1) * sizeof *views);
    if (view == NULL || views == NULL) {
        free(view);
        if (views != NULL)
            plugin->views = views;
        return;
    }
    plugin->views = views;
    view->plugin = plugin;
    view->display = display;
    view->rendered_time = -1;
    plugin->views[plugin->view_count++] = view;
    ply_pixel_display_set_draw_handler(display, on_draw, view);
}

static void remove_pixel_display(ply_boot_splash_plugin_t *plugin, ply_pixel_display_t *display) {
    for (size_t i = 0; i < plugin->view_count; i++) {
        view_t *view = plugin->views[i];
        if (view->display != display)
            continue;
        ply_pixel_display_set_draw_handler(display, NULL, NULL);
        free_view_resources(view);
        free(view);
        plugin->views[i] = plugin->views[--plugin->view_count];
        return;
    }
}

static void stop_animation(ply_boot_splash_plugin_t *plugin) {
    if (plugin->loop == NULL)
        return;
    ply_event_loop_stop_watching_for_timeout(plugin->loop, on_timeout, plugin);
    plugin->loop = NULL;
}

static void destroy_plugin(ply_boot_splash_plugin_t *plugin) {
    if (plugin == NULL)
        return;
    stop_animation(plugin);
    while (plugin->view_count > 0)
        remove_pixel_display(plugin, plugin->views[plugin->view_count - 1]->display);
    free(plugin->views);
    p3b_model_free(plugin->model); /* after the renderers */
    free(plugin);
}

static bool show_splash_screen(ply_boot_splash_plugin_t *plugin, ply_event_loop_t *loop,
                               ply_buffer_t *boot_buffer, ply_boot_splash_mode_t mode) {
    (void) boot_buffer;
    (void) mode;
    stop_animation(plugin);
    plugin->loop = loop;
    plugin->start_time = ply_get_timestamp();
    redraw_all(plugin);
    ply_event_loop_watch_for_timeout(loop, 1.0 / plugin->fps, on_timeout, plugin);
    return true;
}

static void hide_splash_screen(ply_boot_splash_plugin_t *plugin, ply_event_loop_t *loop) {
    (void) loop;
    stop_animation(plugin);
}

static void become_idle(ply_boot_splash_plugin_t *plugin, ply_trigger_t *idle_trigger) {
    (void) plugin;
    ply_trigger_pull(idle_trigger, NULL);
}

EXPORT ply_boot_splash_plugin_interface_t *ply_boot_splash_plugin_get_interface(void) {
    static ply_boot_splash_plugin_interface_t interface = {
        .create_plugin = create_plugin,
        .destroy_plugin = destroy_plugin,
        .add_pixel_display = add_pixel_display,
        .remove_pixel_display = remove_pixel_display,
        .show_splash_screen = show_splash_screen,
        .hide_splash_screen = hide_splash_screen,
        .become_idle = become_idle,
    };
    return &interface;
}
