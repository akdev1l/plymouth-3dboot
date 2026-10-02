/* SPDX-License-Identifier: GPL-3.0-or-later */
/* Load the animated N64 sample, render frames in every pixel format and
 * check the result: deterministic, padding untouched, logo colours present. */
#include <stdint.h>
#include <string.h>

#include "check.h"

enum { W = 64, H = 48, STRIDE = W * 4 + 16 };

static int has_rgb(const uint8_t *buf, uint8_t r, uint8_t g, uint8_t b) {
    for (int y = 0; y < H; y++)
        for (int x = 0; x < W; x++) {
            const uint8_t *p = buf + y * STRIDE + x * 4;
            if (p[0] == r && p[1] == g && p[2] == b && p[3] == 255)
                return 1;
        }
    return 0;
}

int main(int argc, char **argv) {
    CHECK(argc == 2);
    CHECK(p3b_abi_version() == P3B_ABI_VERSION);
    CHECK(strlen(p3b_version()) > 0);

    p3b_model *model = NULL;
    CHECK_STATUS(p3b_model_load_file(argv[1], &model), P3B_STATUS_OK);
    CHECK(model != NULL);
    CHECK(p3b_model_clip_count(model) == 1);
    float duration = 0;
    CHECK_STATUS(p3b_model_clip_duration(model, 0, &duration), P3B_STATUS_OK);
    CHECK(duration > 3.3f && duration < 3.4f);
    float lo[3], hi[3];
    CHECK_STATUS(p3b_model_bounds(model, lo, hi), P3B_STATUS_OK);
    CHECK(hi[1] > lo[1]);

    p3b_render_options options = p3b_render_options_default();
    p3b_renderer *renderer = NULL;
    CHECK_STATUS(p3b_renderer_new(model, 0, W, H, &options, &renderer), P3B_STATUS_OK);

    static uint8_t a[STRIDE * H], b[STRIDE * H];
    memset(a, 0xAA, sizeof a);
    memset(b, 0x55, sizeof b);
    CHECK_STATUS(p3b_render_frame(renderer, 1.0, a, sizeof a, STRIDE, P3B_PIXEL_FORMAT_RGBA8888), P3B_STATUS_OK);
    CHECK_STATUS(p3b_render_frame(renderer, 1.0, b, sizeof b, STRIDE, P3B_PIXEL_FORMAT_RGBA8888), P3B_STATUS_OK);
    for (int y = 0; y < H; y++) {
        CHECK(memcmp(a + y * STRIDE, b + y * STRIDE, W * 4) == 0); /* deterministic */
        for (int i = W * 4; i < STRIDE; i++)
            CHECK(a[y * STRIDE + i] == 0xAA); /* padding untouched */
    }
    /* The unlit logo shows its Readme colours: green and blue faces. */
    CHECK(has_rgb(a, 6, 147, 48));
    CHECK(has_rgb(a, 2, 34, 169));

    /* BGRA swaps red and blue; premultiplied ARGB of opaque pixels matches. */
    CHECK_STATUS(p3b_render_frame(renderer, 1.0, b, sizeof b, STRIDE, P3B_PIXEL_FORMAT_BGRA8888), P3B_STATUS_OK);
    for (int i = 0; i < W * 4; i += 4)
        CHECK(a[i] == b[i + 2] && a[i + 1] == b[i + 1] && a[i + 2] == b[i]);
    CHECK_STATUS(p3b_render_frame(renderer, 1.0, b, sizeof b, STRIDE, P3B_PIXEL_FORMAT_ARGB32_PREMULTIPLIED), P3B_STATUS_OK);
    for (int x = 0; x < W; x++) {
        uint32_t px;
        memcpy(&px, b + x * 4, 4);
        CHECK(px == (0xFF000000u | (uint32_t) a[x * 4] << 16 | (uint32_t) a[x * 4 + 1] << 8 | a[x * 4 + 2]));
    }

    /* A later frame differs (the logo spins). */
    CHECK_STATUS(p3b_render_frame(renderer, 1.8, b, sizeof b, STRIDE, P3B_PIXEL_FORMAT_RGBA8888), P3B_STATUS_OK);
    CHECK(memcmp(a, b, sizeof a) != 0);

    /* Anti-aliasing keeps the size and adds intermediate edge colours. */
    p3b_renderer_free(renderer);
    options.antialias = 4;
    CHECK_STATUS(p3b_renderer_new(model, 0, W, H, &options, &renderer), P3B_STATUS_OK);
    CHECK_STATUS(p3b_render_frame(renderer, 1.0, b, sizeof b, STRIDE, P3B_PIXEL_FORMAT_RGBA8888), P3B_STATUS_OK);
    CHECK(memcmp(a, b, sizeof a) != 0);

    /* Threads do not change the output. */
    p3b_renderer_free(renderer);
    options.threads = 4;
    CHECK_STATUS(p3b_renderer_new(model, 0, W, H, &options, &renderer), P3B_STATUS_OK);
    CHECK_STATUS(p3b_render_frame(renderer, 1.0, a, sizeof a, STRIDE, P3B_PIXEL_FORMAT_RGBA8888), P3B_STATUS_OK);
    for (int y = 0; y < H; y++)
        CHECK(memcmp(a + y * STRIDE, b + y * STRIDE, W * 4) == 0);

    p3b_renderer_free(renderer);
    p3b_model_free(model);
    puts("render: ok");
    return 0;
}
