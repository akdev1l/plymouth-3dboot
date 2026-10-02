/* SPDX-License-Identifier: GPL-3.0-or-later */
/* Minimal assertion helpers for the C API tests. */
#ifndef P3B_TEST_CHECK_H
#define P3B_TEST_CHECK_H

#include <stdio.h>
#include <stdlib.h>

#include "plymouth-3dboot.h"

#define CHECK(cond)                                                                 \
    do {                                                                            \
        if (!(cond)) {                                                              \
            fprintf(stderr, "%s:%d: check failed: %s\n", __FILE__, __LINE__, #cond); \
            exit(1);                                                                \
        }                                                                           \
    } while (0)

#define CHECK_STATUS(call, expected)                                                   \
    do {                                                                               \
        p3b_status status_ = (call);                                                   \
        if (status_ != (expected)) {                                                   \
            fprintf(stderr, "%s:%d: %s returned %d (expected %d): %s\n", __FILE__,     \
                    __LINE__, #call, (int) status_, (int) (expected), p3b_last_error()); \
            exit(1);                                                                   \
        }                                                                              \
    } while (0)

#endif
