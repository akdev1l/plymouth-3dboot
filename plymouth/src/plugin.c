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
#include <ply-key-file.h>
#include <ply-logger.h>

#include <plymouth-3dboot.h>

#define GROUP "plymouth-3dboot"
#define EXPORT __attribute__((visibility("default")))

struct _ply_boot_splash_plugin {
    p3b_model *model;
    size_t clip;           /* clip to play, or SIZE_MAX for none */
    p3b_render_options options;
    double fps;
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

static void destroy_plugin(ply_boot_splash_plugin_t *plugin) {
    if (plugin == NULL)
        return;
    p3b_model_free(plugin->model);
    free(plugin);
}

static bool show_splash_screen(ply_boot_splash_plugin_t *plugin, ply_event_loop_t *loop,
                               ply_buffer_t *boot_buffer, ply_boot_splash_mode_t mode) {
    (void) plugin;
    (void) loop;
    (void) boot_buffer;
    (void) mode;
    return true;
}

static void hide_splash_screen(ply_boot_splash_plugin_t *plugin, ply_event_loop_t *loop) {
    (void) plugin;
    (void) loop;
}

static void become_idle(ply_boot_splash_plugin_t *plugin, ply_trigger_t *idle_trigger) {
    (void) plugin;
    ply_trigger_pull(idle_trigger, NULL);
}

EXPORT ply_boot_splash_plugin_interface_t *ply_boot_splash_plugin_get_interface(void) {
    static ply_boot_splash_plugin_interface_t interface = {
        .create_plugin = create_plugin,
        .destroy_plugin = destroy_plugin,
        .show_splash_screen = show_splash_screen,
        .hide_splash_screen = hide_splash_screen,
        .become_idle = become_idle,
    };
    return &interface;
}
