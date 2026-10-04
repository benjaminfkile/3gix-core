/*
 * ABI check: proves a C caller can use the core library through
 * include/gx_core.h and the native shared library alone.
 *
 * Usage: abi_check REGISTRY_VECTOR MATTER_VECTOR MATTER_KEY
 *
 * Calls gx_format_version, then gx_validate with a valid registry vector, a
 * valid matter vector, and the registry vector under a cell key, and checks
 * the return codes, the reason, the buffer truncation, and gx_error_name.
 * Run by scripts/ci.sh. Exits 0 when every check passes.
 */

#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "gx_core.h"

static int failures = 0;

static void check(int ok, const char* what) {
    printf("%s: %s\n", ok ? "ok  " : "FAIL", what);
    if (!ok) {
        failures++;
    }
}

static uint8_t* read_file(const char* path, size_t* len) {
    FILE* f = fopen(path, "rb");
    if (f == NULL) {
        perror(path);
        exit(2);
    }
    size_t cap = 4096;
    size_t n = 0;
    uint8_t* buf = malloc(cap);
    for (;;) {
        if (buf == NULL) {
            fputs("out of memory\n", stderr);
            exit(2);
        }
        n += fread(buf + n, 1, cap - n, f);
        if (n < cap) {
            break;
        }
        cap *= 2;
        buf = realloc(buf, cap);
    }
    if (ferror(f)) {
        perror(path);
        exit(2);
    }
    fclose(f);
    *len = n;
    return buf;
}

static int32_t validate(const char* key, const uint8_t* bytes, size_t len,
                        char* err, size_t cap) {
    return gx_validate((const uint8_t*)key, strlen(key), bytes, len, err, cap);
}

int main(int argc, char** argv) {
    if (argc != 4) {
        fputs("usage: abi_check REGISTRY_VECTOR MATTER_VECTOR MATTER_KEY\n", stderr);
        return 2;
    }
    size_t reg_len = 0;
    size_t mat_len = 0;
    uint8_t* reg = read_file(argv[1], &reg_len);
    uint8_t* mat = read_file(argv[2], &mat_len);
    const char* mat_key = argv[3];
    char err[256];

    check(gx_format_version() == 1, "gx_format_version() == 1");

    memset(err, 0x7f, sizeof err);
    check(validate("registry", reg, reg_len, err, sizeof err) == 0,
          "valid registry under \"registry\" returns 0");
    check((unsigned char)err[0] == 0x7f, "success leaves err_buf untouched");

    check(validate(mat_key, mat, mat_len, err, sizeof err) == 0,
          "valid matter section under its key returns 0");

    int32_t rc = validate("7-3-1-2-5", reg, reg_len, err, sizeof err);
    printf("      registry under 7-3-1-2-5: %d %s: %s\n", (int)rc,
           gx_error_name(rc), err);
    check(rc >= 100 && rc < 200, "registry under a cell key returns a 1xx code");
    check(strstr(err, "magic") != NULL, "the reason names the magic");
    check(strcmp(gx_error_name(rc), "BAD_MAGIC") == 0,
          "gx_error_name names the code");

    size_t full = strlen(err);
    char small[6];
    memset(small, 0x7f, sizeof small);
    check(validate("7-3-1-2-5", reg, reg_len, small, 4) == rc,
          "truncated call returns the same code");
    check(small[3] == '\0' && strncmp(small, err, 3) == 0 && small[4] == 0x7f,
          "reason truncated to err_cap - 1 bytes plus NUL");
    check(full > 3, "full reason is longer than the truncated one");

    check(validate("7-3-1-2-5", reg, reg_len, NULL, 0) == rc,
          "null err_buf still returns the code");
    check(gx_validate(NULL, 3, reg, reg_len, err, sizeof err) == 207,
          "null key with non-zero length returns 207");
    check(validate("registry", NULL, 9, err, sizeof err) == 106,
          "null bytes with non-zero length returns 106");
    check(validate("not-a-key", mat, mat_len, err, sizeof err) == 206,
          "malformed key returns 206");

    const char* unknown = gx_error_name(0);
    check(unknown != NULL && unknown[0] == '\0', "unknown code name is empty");

    free(reg);
    free(mat);
    if (failures != 0) {
        printf("abi-check: %d failed\n", failures);
        return 1;
    }
    puts("abi-check ok");
    return 0;
}
