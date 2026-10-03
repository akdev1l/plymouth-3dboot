/* SPDX-License-Identifier: GPL-3.0-or-later */
/* Plymouth splash plugin rendering an animated 3D model with
 * libplymouth_3dboot. See docs/plymouth.md. */
#define _GNU_SOURCE
/* ply_trace compiles to nothing unless this is defined (Plymouth's own
 * plugins get it from its build configuration); with it, messages appear in
 * plymouthd's debug log. */
#define PLY_ENABLE_TRACING 1
#include <errno.h>
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
#include <ply-label.h>
#include <ply-utils.h>

#include <plymouth-3dboot.h>

#define GROUP "plymouth-3dboot"
/* Seconds between frame statistics in the debug log (plymouthd --debug). */
#define STATS_INTERVAL 5.0
#define EXPORT __attribute__((visibility("default")))

/* One display (head) and its renderer. */
typedef struct view {
    ply_boot_splash_plugin_t *plugin;
    ply_pixel_display_t *display;
    p3b_renderer *renderer;
    uint32_t *pixels;       /* last rendered frame, width * height ARGB32 */
    unsigned long width, height;
    bool has_frame;         /* `pixels` holds this renderer's last frame */
    ply_label_t *prompt;    /* prompt and entered text */
    ply_label_t *message;   /* display_message text */
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

    /* Frame statistics since stats_start, logged every STATS_INTERVAL. */
    double stats_start;
    unsigned long stats_frames;
    double stats_render;    /* seconds rendering */
    double stats_present;   /* seconds in Plymouth (copy and flush) */
    double stats_redrawn;   /* sum of the redrawn fractions of the screen */

    char *prompt_text;      /* NULL when no prompt is shown */
    char *message_text;     /* NULL when no message is shown */
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

    double antialias = ply_key_file_get_double(key_file, GROUP, "Antialias", 1.0);
    plugin->options.antialias = antialias >= 1.0 && antialias <= 8.0 ? (uint8_t) antialias : 1;
    double threads = ply_key_file_get_double(key_file, GROUP, "Threads", 1.0);
    plugin->options.threads = threads >= 1.0 && threads <= 64.0 ? (uint8_t) threads : 1;

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
    view->has_frame = false;
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

/* Shows `text` centred at `y_fraction` of the display height, or hides
 * the label for NULL. Labels need a Plymouth label plugin at run time;
 * without one they stay invisible and the animation is unaffected. */
static void place_label(ply_label_t *label, ply_pixel_display_t *display, const char *text, double y_fraction) {
    if (text == NULL) {
        ply_label_hide(label);
        return;
    }
    unsigned long width = ply_pixel_display_get_width(display);
    ply_label_set_text(label, text);
    ply_label_set_width(label, (long) width);
    ply_label_set_alignment(label, PLY_LABEL_ALIGN_CENTER);
    ply_label_set_color(label, 1.0f, 1.0f, 1.0f, 1.0f);
    long y = (long) (ply_pixel_display_get_height(display) * y_fraction);
    ply_label_show(label, display, 0, y);
}

static void update_labels(ply_boot_splash_plugin_t *plugin) {
    for (size_t i = 0; i < plugin->view_count; i++) {
        view_t *view = plugin->views[i];
        place_label(view->prompt, view->display, plugin->prompt_text, 0.75);
        place_label(view->message, view->display, plugin->message_text, 0.88);
    }
}

/* Renders the frame at time `t` into the view's buffer, writing only what
 * changed since the view's previous frame. Returns the changed area (empty
 * on failure). */
static ply_rectangle_t render_view(view_t *view, double t) {
    ply_rectangle_t area = {.x = 0, .y = 0, .width = 0, .height = 0};
    size_t stride = view->width * sizeof *view->pixels;
    size_t len = stride * view->height;
    p3b_rect damage = {.x = 0, .y = 0, .width = (uint32_t) view->width, .height = (uint32_t) view->height};
    p3b_status status =
        view->has_frame
            ? p3b_render_frame_incremental(view->renderer, t, (uint8_t *) view->pixels, len, stride,
                                           P3B_PIXEL_FORMAT_ARGB32_PREMULTIPLIED, &damage)
            : p3b_render_frame(view->renderer, t, (uint8_t *) view->pixels, len, stride,
                               P3B_PIXEL_FORMAT_ARGB32_PREMULTIPLIED);
    if (status != P3B_STATUS_OK) {
        ply_trace("plymouth-3dboot: render: %s", p3b_last_error());
        view->has_frame = false;
        return area;
    }
    view->has_frame = true;
    area.x = damage.x;
    area.y = damage.y;
    area.width = damage.width;
    area.height = damage.height;
    return area;
}

/* Plymouth asks for an area to be redrawn (clipping the buffer to it):
 * show the last frame, rendering one first if there is none yet. Frames
 * advance in on_timeout, which redraws only what changed. */
static void on_draw(void *user_data, ply_pixel_buffer_t *pixel_buffer, int x, int y, int width, int height,
                    ply_pixel_display_t *display) {
    (void) display;
    view_t *view = user_data;
    if (!prepare_view(view))
        return;
    if (!view->has_frame)
        render_view(view, animation_time(view->plugin));
    if (!view->has_frame)
        return;
    ply_rectangle_t area = {.x = 0, .y = 0, .width = view->width, .height = view->height};
    ply_pixel_buffer_fill_with_argb32_data(pixel_buffer, &area, view->pixels);
    ply_label_draw_area(view->prompt, pixel_buffer, x, y, (unsigned long) width, (unsigned long) height);
    ply_label_draw_area(view->message, pixel_buffer, x, y, (unsigned long) width, (unsigned long) height);
}

static void redraw_all(ply_boot_splash_plugin_t *plugin) {
    for (size_t i = 0; i < plugin->view_count; i++) {
        view_t *view = plugin->views[i];
        ply_pixel_display_draw_area(view->display, 0, 0, (int) ply_pixel_display_get_width(view->display),
                                    (int) ply_pixel_display_get_height(view->display));
    }
}

static void reset_stats(ply_boot_splash_plugin_t *plugin, double now) {
    plugin->stats_start = now;
    plugin->stats_frames = 0;
    plugin->stats_render = 0;
    plugin->stats_present = 0;
    plugin->stats_redrawn = 0;
}

/* Accumulates one frame and logs the averages every STATS_INTERVAL. */
static void record_stats(ply_boot_splash_plugin_t *plugin, double render, double present, double redrawn) {
    plugin->stats_frames++;
    plugin->stats_render += render;
    plugin->stats_present += present;
    plugin->stats_redrawn += redrawn;
    double now = ply_get_timestamp();
    double elapsed = now - plugin->stats_start;
    if (elapsed < STATS_INTERVAL)
        return;
    double frames = (double) plugin->stats_frames;
    ply_trace("plymouth-3dboot: %.1f fps; per frame: render %.1f ms, display %.1f ms, %.0f%% of the screen redrawn",
              frames / elapsed, plugin->stats_render / frames * 1e3, plugin->stats_present / frames * 1e3,
              plugin->stats_redrawn / frames * 100.0);
    reset_stats(plugin, now);
}

static void on_timeout(void *user_data, ply_event_loop_t *loop);

/* Arms the timer for the next frame. Frames are due every 1/fps from the
 * start, so time spent drawing does not lower the frame rate; a late frame
 * skips to the next due time instead of piling up. */
static void schedule_frame(ply_boot_splash_plugin_t *plugin, ply_event_loop_t *loop) {
    double period = 1.0 / plugin->fps;
    double now = ply_get_timestamp();
    double elapsed = now > plugin->start_time ? now - plugin->start_time : 0.0;
    double next = plugin->start_time + ((double) (unsigned long) (elapsed / period) + 1.0) * period;
    ply_event_loop_watch_for_timeout(loop, next - now, on_timeout, plugin);
}

static void on_timeout(void *user_data, ply_event_loop_t *loop) {
    ply_boot_splash_plugin_t *plugin = user_data;
    if (plugin->loop == NULL)
        return;
    double t = animation_time(plugin);
    double render = 0, present = 0, redrawn = 0;
    for (size_t i = 0; i < plugin->view_count; i++) {
        view_t *view = plugin->views[i];
        if (!prepare_view(view))
            continue;
        double started = ply_get_timestamp();
        ply_rectangle_t changed = render_view(view, t);
        double rendered = ply_get_timestamp();
        if (changed.width > 0 && changed.height > 0)
            ply_pixel_display_draw_area(view->display, (int) changed.x, (int) changed.y, (int) changed.width,
                                        (int) changed.height);
        present += ply_get_timestamp() - rendered;
        render += rendered - started;
        redrawn += (double) (changed.width * changed.height) / (double) (view->width * view->height);
    }
    if (plugin->view_count > 0)
        record_stats(plugin, render, present, redrawn / (double) plugin->view_count);
    schedule_frame(plugin, loop);
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
    view->prompt = ply_label_new();
    view->message = ply_label_new();
    plugin->views[plugin->view_count++] = view;
    ply_pixel_display_set_draw_handler(display, on_draw, view);
    update_labels(plugin);
}

static void remove_pixel_display(ply_boot_splash_plugin_t *plugin, ply_pixel_display_t *display) {
    for (size_t i = 0; i < plugin->view_count; i++) {
        view_t *view = plugin->views[i];
        if (view->display != display)
            continue;
        ply_pixel_display_set_draw_handler(display, NULL, NULL);
        free_view_resources(view);
        ply_label_free(view->prompt);
        ply_label_free(view->message);
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
    free(plugin->prompt_text);
    free(plugin->message_text);
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
    reset_stats(plugin, plugin->start_time);
    for (size_t i = 0; i < plugin->view_count; i++)
        plugin->views[i]->has_frame = false; /* render the first frame afresh */
    redraw_all(plugin);
    schedule_frame(plugin, loop);
    return true;
}

static void hide_splash_screen(ply_boot_splash_plugin_t *plugin, ply_event_loop_t *loop) {
    (void) loop;
    stop_animation(plugin);
}

/* Replaces *slot with a copy of `text` (NULL clears it) and redraws. */
static void set_text(ply_boot_splash_plugin_t *plugin, char **slot, const char *text) {
    free(*slot);
    *slot = text != NULL ? strdup(text) : NULL;
    update_labels(plugin);
    redraw_all(plugin);
}

/* "prompt: entry" with the entry shown as bullets when secret. */
static void show_prompt(ply_boot_splash_plugin_t *plugin, const char *prompt, const char *entry, size_t bullets,
                        bool secret) {
    static const char bullet[] = "\u2022"; /* UTF-8 "•" */
    const char *label = prompt != NULL && prompt[0] != '\0' ? prompt : (secret ? "Password" : "");
    size_t entry_len = secret ? bullets * (sizeof bullet - 1) : (entry != NULL ? strlen(entry) : 0);
    char *text = malloc(strlen(label) + 2 + entry_len + 1);
    if (text == NULL)
        return;
    char *end = stpcpy(text, label);
    if (label[0] != '\0')
        end = stpcpy(end, ": ");
    if (secret)
        for (size_t i = 0; i < bullets; i++)
            end = stpcpy(end, bullet);
    else if (entry != NULL)
        end = stpcpy(end, entry);
    set_text(plugin, &plugin->prompt_text, text);
    free(text);
}

static void display_normal(ply_boot_splash_plugin_t *plugin) {
    set_text(plugin, &plugin->prompt_text, NULL);
}

static void display_password(ply_boot_splash_plugin_t *plugin, const char *prompt, int bullets) {
    show_prompt(plugin, prompt, NULL, bullets > 0 ? (size_t) bullets : 0, true);
}

static void display_question(ply_boot_splash_plugin_t *plugin, const char *prompt, const char *entry_text) {
    show_prompt(plugin, prompt, entry_text, 0, false);
}

static void display_prompt(ply_boot_splash_plugin_t *plugin, const char *prompt, const char *entry_text,
                           bool is_secret) {
    show_prompt(plugin, prompt, entry_text, entry_text != NULL ? strlen(entry_text) : 0, is_secret);
}

static void display_message(ply_boot_splash_plugin_t *plugin, const char *message) {
    set_text(plugin, &plugin->message_text, message);
}

static void hide_message(ply_boot_splash_plugin_t *plugin, const char *message) {
    if (plugin->message_text != NULL && message != NULL && strcmp(plugin->message_text, message) == 0)
        set_text(plugin, &plugin->message_text, NULL);
}

/* Boot status updates (systemd sends one per unit) are not shown. Plymouth
 * asserts that this callback exists, so it must not be NULL. */
static void update_status(ply_boot_splash_plugin_t *plugin, const char *status) {
    (void) plugin;
    (void) status;
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
        .update_status = update_status,
        .become_idle = become_idle,
        .display_normal = display_normal,
        .display_password = display_password,
        .display_question = display_question,
        .display_prompt = display_prompt,
        .display_message = display_message,
        .hide_message = hide_message,
    };
    return &interface;
}
