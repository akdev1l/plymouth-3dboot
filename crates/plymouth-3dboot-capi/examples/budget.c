/* SPDX-License-Identifier: GPL-3.0-or-later */
/* Measures what matters at boot: time to the first frame and the steady
 * frame time, at a given resolution, through the C API.
 *
 *   budget MODEL WIDTH HEIGHT FRAMES
 *
 * Prints "first_frame_ms=<ms> frame_ms=<median ms>". */
#define _POSIX_C_SOURCE 199309L
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <time.h>

#include <plymouth-3dboot.h>

static double now_ms(void) {
    struct timespec ts;
    clock_gettime(CLOCK_MONOTONIC, &ts);
    return ts.tv_sec * 1e3 + ts.tv_nsec / 1e6;
}

static int cmp(const void *a, const void *b) {
    double x = *(const double *) a, y = *(const double *) b;
    return (x > y) - (x < y);
}

int main(int argc, char **argv) {
    if (argc != 5) {
        fprintf(stderr, "usage: %s MODEL WIDTH HEIGHT FRAMES\n", argv[0]);
        return 2;
    }
    uint32_t w = (uint32_t) atoi(argv[2]), h = (uint32_t) atoi(argv[3]);
    int frames = atoi(argv[4]);
    if (w == 0 || h == 0 || frames < 1)
        return 2;
    uint32_t *pixels = malloc((size_t) w * h * 4);
    double *times = malloc(sizeof *times * (size_t) frames);
    if (pixels == NULL || times == NULL)
        return 1;

    double start = now_ms();
    p3b_model *model = NULL;
    p3b_renderer *renderer = NULL;
    if (p3b_model_load_file(argv[1], &model) != P3B_STATUS_OK ||
        (p3b_model_clip_count(model) == 0 && p3b_model_add_turntable(model, 0, 1, 0, 6) != P3B_STATUS_OK) ||
        p3b_renderer_new(model, 0, w, h, NULL, &renderer) != P3B_STATUS_OK ||
        p3b_render_frame(renderer, 0, (uint8_t *) pixels, (size_t) w * h * 4, w * 4,
                         P3B_PIXEL_FORMAT_ARGB32_PREMULTIPLIED) != P3B_STATUS_OK) {
        fprintf(stderr, "error: %s\n", p3b_last_error());
        return 1;
    }
    double first = now_ms() - start;

    for (int i = 0; i < frames; i++) {
        double t0 = now_ms();
        p3b_render_frame(renderer, 0.05 * (i + 1), (uint8_t *) pixels, (size_t) w * h * 4, w * 4,
                         P3B_PIXEL_FORMAT_ARGB32_PREMULTIPLIED);
        times[i] = now_ms() - t0;
    }
    qsort(times, (size_t) frames, sizeof *times, cmp);
    printf("first_frame_ms=%.1f frame_ms=%.1f\n", first, times[frames / 2]);

    p3b_renderer_free(renderer);
    p3b_model_free(model);
    free(times);
    free(pixels);
    return 0;
}
