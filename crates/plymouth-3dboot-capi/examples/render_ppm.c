/* SPDX-License-Identifier: GPL-3.0-or-later */
/* Renders one frame of a model with the plymouth-3dboot C API and writes it
 * as a binary PPM image.
 *
 *   cc render_ppm.c $(pkg-config --cflags --libs plymouth-3dboot) -o render_ppm
 *   ./render_ppm model.dae frame.ppm [seconds]
 *
 * The model's first clip is played; models without animation spin on a
 * turntable. Pixels are requested in Plymouth's premultiplied ARGB32 format
 * and composited over black for the PPM. */
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>

#include <plymouth-3dboot.h>

enum { WIDTH = 320, HEIGHT = 240 };

static int fail(const char *what) {
    fprintf(stderr, "%s: %s\n", what, p3b_last_error());
    return 1;
}

int main(int argc, char **argv) {
    if (argc < 3) {
        fprintf(stderr, "usage: %s MODEL OUTPUT.ppm [SECONDS]\n", argv[0]);
        return 2;
    }
    double seconds = argc > 3 ? atof(argv[3]) : 0.0;

    p3b_model *model = NULL;
    if (p3b_model_load_file(argv[1], &model) != P3B_STATUS_OK)
        return fail("load");
    for (size_t i = 0; i < p3b_model_warning_count(model); i++)
        fprintf(stderr, "warning: %s\n", p3b_model_warning(model, i));
    if (p3b_model_clip_count(model) == 0 && p3b_model_add_turntable(model, 0, 1, 0, 6.0f) != P3B_STATUS_OK) {
        p3b_model_free(model);
        return fail("turntable");
    }

    p3b_render_options options = p3b_render_options_default();
    options.shading = P3B_SHADING_LAMBERT;
    p3b_renderer *renderer = NULL;
    if (p3b_renderer_new(model, 0, WIDTH, HEIGHT, &options, &renderer) != P3B_STATUS_OK) {
        p3b_model_free(model);
        return fail("renderer");
    }

    static uint32_t pixels[WIDTH * HEIGHT];
    int status = 0;
    if (p3b_render_frame(renderer, seconds, (uint8_t *) pixels, sizeof pixels, WIDTH * 4,
                         P3B_PIXEL_FORMAT_ARGB32_PREMULTIPLIED) != P3B_STATUS_OK) {
        status = fail("render");
    } else {
        FILE *out = fopen(argv[2], "wb");
        if (!out) {
            perror(argv[2]);
            status = 1;
        } else {
            fprintf(out, "P6\n%d %d\n255\n", WIDTH, HEIGHT);
            for (int i = 0; i < WIDTH * HEIGHT; i++) {
                /* Premultiplied colour over black is the colour itself. */
                uint8_t rgb[3] = {(uint8_t) (pixels[i] >> 16), (uint8_t) (pixels[i] >> 8), (uint8_t) pixels[i]};
                fwrite(rgb, 1, sizeof rgb, out);
            }
            fclose(out);
        }
    }

    p3b_renderer_free(renderer); /* renderers before their model */
    p3b_model_free(model);
    return status;
}
