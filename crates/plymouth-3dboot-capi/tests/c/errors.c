/* SPDX-License-Identifier: GPL-3.0-or-later */
/* Error paths: every bad input is a status plus message, never a crash. */
#include <stdint.h>
#include <string.h>

#include "check.h"

static int resolve_nothing(void *user, const char *name, const uint8_t **data, size_t *len) {
    (void) user;
    (void) name;
    (void) data;
    (void) len;
    return 1;
}

int main(int argc, char **argv) {
    CHECK(argc == 2);
    p3b_model *model = (p3b_model *) 1; /* must be reset to NULL */
    CHECK_STATUS(p3b_model_load_file("/nonexistent/model.obj", &model), P3B_STATUS_IO);
    CHECK(model == NULL);
    CHECK(strstr(p3b_last_error(), "model.obj") != NULL);
    CHECK_STATUS(p3b_model_load_file("model.ply", &model), P3B_STATUS_UNSUPPORTED_FORMAT);
    CHECK_STATUS(p3b_model_load_file(NULL, &model), P3B_STATUS_NULL_POINTER);
    CHECK_STATUS(p3b_model_load_file(argv[1], NULL), P3B_STATUS_NULL_POINTER);

    const char *bad = "v 1 2\n";
    CHECK_STATUS(p3b_model_load_memory("obj", (const uint8_t *) bad, strlen(bad), resolve_nothing, NULL, &model), P3B_STATUS_PARSE);
    CHECK(strstr(p3b_last_error(), "line 1") != NULL);
    const char *dae = "<COLLADA><library_nodes>";
    CHECK_STATUS(p3b_model_load_memory("dae", (const uint8_t *) dae, strlen(dae), NULL, NULL, &model), P3B_STATUS_PARSE);
    CHECK_STATUS(p3b_model_load_memory("gltf", (const uint8_t *) bad, strlen(bad), NULL, NULL, &model), P3B_STATUS_UNSUPPORTED_FORMAT);
    const uint8_t invalid_utf8[] = {0xff, 0xfe};
    CHECK_STATUS(p3b_model_load_memory("obj", invalid_utf8, sizeof invalid_utf8, NULL, NULL, &model), P3B_STATUS_INVALID_UTF8);
    CHECK_STATUS(p3b_model_load_memory("obj", NULL, 4, NULL, NULL, &model), P3B_STATUS_NULL_POINTER);

    /* A valid model, then bad renderer and frame arguments. */
    CHECK_STATUS(p3b_model_load_file(argv[1], &model), P3B_STATUS_OK);
    CHECK(strcmp(p3b_last_error(), "") == 0);
    p3b_renderer *renderer = (p3b_renderer *) 1;
    CHECK_STATUS(p3b_renderer_new(model, 5, 16, 16, NULL, &renderer), P3B_STATUS_INVALID_ARGUMENT);
    CHECK(renderer == NULL);
    CHECK_STATUS(p3b_renderer_new(model, SIZE_MAX, 0, 16, NULL, &renderer), P3B_STATUS_INVALID_ARGUMENT);
    p3b_render_options options = p3b_render_options_default();
    options.shading = 42;
    CHECK_STATUS(p3b_renderer_new(model, SIZE_MAX, 16, 16, &options, &renderer), P3B_STATUS_INVALID_ARGUMENT);
    CHECK_STATUS(p3b_renderer_new(model, SIZE_MAX, 16, 16, NULL, &renderer), P3B_STATUS_OK);

    static uint8_t buf[16 * 16 * 4];
    CHECK_STATUS(p3b_render_frame(renderer, 0, buf, sizeof buf - 1, 64, P3B_PIXEL_FORMAT_RGBA8888), P3B_STATUS_BUFFER_TOO_SMALL);
    CHECK_STATUS(p3b_render_frame(renderer, 0, buf, sizeof buf, 32, P3B_PIXEL_FORMAT_RGBA8888), P3B_STATUS_INVALID_ARGUMENT);
    CHECK_STATUS(p3b_render_frame(renderer, 0, buf, sizeof buf, 64, 99), P3B_STATUS_INVALID_ARGUMENT);
    CHECK_STATUS(p3b_render_frame(renderer, 0, NULL, sizeof buf, 64, P3B_PIXEL_FORMAT_RGBA8888), P3B_STATUS_NULL_POINTER);
    CHECK_STATUS(p3b_render_frame(NULL, 0, buf, sizeof buf, 64, P3B_PIXEL_FORMAT_RGBA8888), P3B_STATUS_NULL_POINTER);
    CHECK_STATUS(p3b_render_frame(renderer, 0, buf, sizeof buf, 64, P3B_PIXEL_FORMAT_RGBA8888), P3B_STATUS_OK);

    /* NULL handles are accepted by queries and frees. */
    CHECK(p3b_model_clip_count(NULL) == 0);
    CHECK(p3b_model_warning_count(NULL) == 0);
    CHECK(p3b_model_warning(model, 1000) == NULL);
    p3b_renderer_free(NULL);
    p3b_model_free(NULL);

    p3b_renderer_free(renderer);
    p3b_model_free(model);
    puts("errors: ok");
    return 0;
}
