/* SPDX-License-Identifier: GPL-3.0-or-later */
/* Load from memory through the resolver callback, repeatedly, to expose
 * leaks under valgrind. */
#include <stdint.h>
#include <string.h>

#include "check.h"

struct files {
    const char *mtl;
    size_t mtl_len;
    int calls;
};

static char *read_file(const char *path, size_t *len) {
    FILE *f = fopen(path, "rb");
    CHECK(f != NULL);
    CHECK(fseek(f, 0, SEEK_END) == 0);
    long n = ftell(f);
    CHECK(n > 0);
    rewind(f);
    char *data = malloc((size_t) n);
    CHECK(data != NULL);
    CHECK(fread(data, 1, (size_t) n, f) == (size_t) n);
    fclose(f);
    *len = (size_t) n;
    return data;
}

static int resolve(void *user, const char *name, const uint8_t **data, size_t *len) {
    struct files *files = user;
    files->calls++;
    if (strcmp(name, "n64_logo.mtl") != 0)
        return 1;
    *data = (const uint8_t *) files->mtl;
    *len = files->mtl_len;
    return 0;
}

int main(int argc, char **argv) {
    CHECK(argc == 3);
    size_t obj_len, mtl_len;
    char *obj = read_file(argv[1], &obj_len);
    char *mtl = read_file(argv[2], &mtl_len);
    struct files files = {mtl, mtl_len, 0};

    for (int i = 0; i < 20; i++) {
        p3b_model *model = NULL;
        CHECK_STATUS(p3b_model_load_memory("obj", (const uint8_t *) obj, obj_len, resolve, &files, &model), P3B_STATUS_OK);
        /* Only the ignored OBJ/MTL directives warn; the library was found. */
        for (size_t w = 0; w < p3b_model_warning_count(model); w++)
            CHECK(strstr(p3b_model_warning(model, w), "ignored directive") != NULL);
        CHECK_STATUS(p3b_model_add_turntable(model, 0, 1, 0, 2.0f), P3B_STATUS_OK);
        p3b_renderer *renderer = NULL;
        CHECK_STATUS(p3b_renderer_new(model, 0, 24, 24, NULL, &renderer), P3B_STATUS_OK);
        uint32_t pixels[24 * 24];
        CHECK_STATUS(p3b_render_frame(renderer, i * 0.1, (uint8_t *) pixels, sizeof pixels, 24 * 4, P3B_PIXEL_FORMAT_ARGB32_PREMULTIPLIED), P3B_STATUS_OK);
        p3b_renderer_free(renderer);
        p3b_model_free(model);
    }
    CHECK(files.calls == 20);
    free(obj);
    free(mtl);
    puts("memory: ok");
    return 0;
}
