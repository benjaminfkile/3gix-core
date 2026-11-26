/*
 * ABI check: proves a C caller can use the core library through
 * include/gx_core.h and the native shared library alone.
 *
 * Usage: abi_check REGISTRY_VECTOR MATTER_VECTOR MATTER_KEY
 *                  REGISTRY_V1_VECTOR MANIFEST_VECTOR BAD_MANIFEST_VECTOR
 *                  EPOCHS_VECTOR BAD_EPOCHS_VECTOR
 *                  FRAME_REGISTRY FRAME_RECORD DISAGREEING_RECORD
 *                  OVERLAY_TILE SURFACE_TILE
 *
 * Calls gx_format_version, then gx_validate with a valid registry vector
 * (registry format version 2), a valid matter vector (matter format
 * version 3), the registry vector under a cell key, the matter vector with a
 * retired version 2 header, and a retired version 1 registry, and checks the return
 * codes, the reason, the buffer truncation, and gx_error_name. Then checks
 * gx_manifest_validate on a valid and an unsorted manifest,
 * gx_epochs_validate and gx_epochs_manifest on the regular epoch index of
 * conformance/epochs/valid/regular.json and an unsorted index, and
 * gx_section_address against two cases of conformance/address/cases.json,
 * and gx_validate under a frame- key and gx_frame_record_agrees on the
 * vectors of conformance/frame-record/valid/, and gx_validate and
 * gx_validate_layer on an overlay tile of conformance/overlay/valid/ and a
 * surface tile of conformance/overlay/layer/ under an overlay layer and
 * another layer (matter-format.md section 12.5).
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

/* Lowercase hex of 32 bytes into `out`, which holds 65 chars. */
static void hex32(const uint8_t* b, char* out) {
    static const char digits[] = "0123456789abcdef";
    for (int i = 0; i < 32; i++) {
        out[2 * i] = digits[b[i] >> 4];
        out[2 * i + 1] = digits[b[i] & 15];
    }
    out[64] = '\0';
}

/* Parses 64 lowercase hex chars into 32 bytes. */
static void unhex32(const char* s, uint8_t* out) {
    for (int i = 0; i < 32; i++) {
        unsigned v = 0;
        for (int j = 0; j < 2; j++) {
            char c = s[2 * i + j];
            v = v * 16 + (unsigned)(c <= '9' ? c - '0' : c - 'a' + 10);
        }
        out[i] = (uint8_t)v;
    }
}

/* Computes an address and compares its hex with `want`. */
static void check_address(const char* module_hex, const char* manifest_hex,
                          int64_t origin, uint64_t span_seconds,
                          uint64_t span_index, const char* key,
                          const char* want, const char* what) {
    uint8_t module[32];
    uint8_t manifest[32];
    uint8_t out[32];
    char got[65];
    unhex32(module_hex, module);
    if (manifest_hex != NULL) {
        unhex32(manifest_hex, manifest);
    }
    int32_t rc = gx_section_address(module, manifest_hex ? manifest : NULL,
                                    origin, span_seconds, span_index,
                                    (const uint8_t*)key, strlen(key), out);
    hex32(out, got);
    printf("      %s: %s\n", what, got);
    check(rc == 0 && strcmp(got, want) == 0, what);
}

static int32_t validate(const char* key, const uint8_t* bytes, size_t len,
                        char* err, size_t cap) {
    return gx_validate((const uint8_t*)key, strlen(key), bytes, len, err, cap);
}

int main(int argc, char** argv) {
    if (argc != 14) {
        fputs("usage: abi_check REGISTRY_VECTOR MATTER_VECTOR MATTER_KEY "
              "REGISTRY_V1_VECTOR MANIFEST_VECTOR BAD_MANIFEST_VECTOR "
              "EPOCHS_VECTOR BAD_EPOCHS_VECTOR FRAME_REGISTRY FRAME_RECORD "
              "DISAGREEING_RECORD OVERLAY_TILE SURFACE_TILE\n",
              stderr);
        return 2;
    }
    size_t reg_len = 0;
    size_t mat_len = 0;
    uint8_t* reg = read_file(argv[1], &reg_len);
    uint8_t* mat = read_file(argv[2], &mat_len);
    const char* mat_key = argv[3];
    char err[256];

    check(gx_format_version() == 3, "gx_format_version() == 3");

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

    /* Matter format version 3: a version 2 header is retired. */
    uint8_t* mat2 = malloc(mat_len);
    check(mat2 != NULL, "allocate a copy of the matter vector");
    if (mat2 != NULL) {
        memcpy(mat2, mat, mat_len);
        mat2[4] = 2;
        mat2[5] = 0;
        rc = validate(mat_key, mat2, mat_len, err, sizeof err);
        printf("      version 2 matter section: %d %s: %s\n", (int)rc,
               gx_error_name(rc), err);
        check(rc == 107, "version 2 matter section returns 107");
        check(strcmp(gx_error_name(rc), "VERSION_RETIRED") == 0,
              "gx_error_name names the retired matter version");
        check(strstr(err, "retired") != NULL,
              "the reason says the matter version is retired");
        free(mat2);
    }

    const char* unknown = gx_error_name(0);
    check(unknown != NULL && unknown[0] == '\0', "unknown code name is empty");

    /* Registry format version 2: version 1 is retired with its own code. */
    size_t v1_len = 0;
    uint8_t* v1 = read_file(argv[4], &v1_len);
    rc = validate("registry", v1, v1_len, err, sizeof err);
    printf("      version 1 registry: %d %s: %s\n", (int)rc, gx_error_name(rc), err);
    check(rc == 610, "version 1 registry returns 610");
    check(strcmp(gx_error_name(rc), "REGISTRY_VERSION_RETIRED") == 0,
          "gx_error_name names the retired version");
    check(strstr(err, "retired") != NULL, "the reason says the version is retired");
    free(v1);

    /* Source manifests. */
    size_t man_len = 0;
    size_t bad_len = 0;
    uint8_t* man = read_file(argv[5], &man_len);
    uint8_t* bad = read_file(argv[6], &bad_len);
    memset(err, 0x7f, sizeof err);
    check(gx_manifest_validate(man, man_len, err, sizeof err) == 0,
          "valid manifest returns 0");
    check((unsigned char)err[0] == 0x7f, "manifest success leaves err_buf untouched");
    rc = gx_manifest_validate(bad, bad_len, err, sizeof err);
    printf("      unsorted manifest: %d %s: %s\n", (int)rc, gx_error_name(rc), err);
    check(rc == 906 && strcmp(gx_error_name(rc), "MANIFEST_NOT_SORTED") == 0,
          "unsorted manifest returns 906");
    check(gx_manifest_validate(NULL, 3, err, sizeof err) == 106,
          "null manifest bytes with non-zero length returns 106");
    free(man);
    free(bad);

    /* Epoch indexes, cases from conformance/epochs/valid/regular.json. */
    size_t ep_len = 0;
    size_t bad_ep_len = 0;
    uint8_t* ep = read_file(argv[7], &ep_len);
    uint8_t* bad_ep = read_file(argv[8], &bad_ep_len);
    memset(err, 0x7f, sizeof err);
    check(gx_epochs_validate(ep, ep_len, err, sizeof err) == 0,
          "valid epoch index returns 0");
    check((unsigned char)err[0] == 0x7f, "epochs success leaves err_buf untouched");
    rc = gx_epochs_validate(bad_ep, bad_ep_len, err, sizeof err);
    printf("      unsorted epoch index: %d %s: %s\n", (int)rc, gx_error_name(rc), err);
    check(rc == 1007 && strcmp(gx_error_name(rc), "EPOCHS_NOT_ASCENDING") == 0,
          "unsorted epoch index returns 1007");
    check(gx_epochs_validate(NULL, 3, err, sizeof err) == 106,
          "null epochs bytes with non-zero length returns 106");
    uint8_t man_hash[32];
    char got[65];
    check(gx_epochs_manifest(ep, ep_len, 3600, 19, man_hash) == 0,
          "hour 19 lies in a listed epoch");
    hex32(man_hash, got);
    check(strcmp(got, "f0db36f05de86ecb6966a89e7001c144ddb3101a270ffdcb3b713fabdf755ab8") == 0,
          "hour 19 has the manifest of epoch 0");
    check(gx_epochs_manifest(ep, ep_len, 3600, 20, man_hash) == 0,
          "hour 20 lies in a listed epoch");
    hex32(man_hash, got);
    check(strcmp(got, "cde526ce98384a370093912025c8351ca9adf9ba6c646034172482a0acfb7793") == 0,
          "hour 20 has the manifest of epoch 1");
    memset(man_hash, 0x7f, sizeof man_hash);
    check(gx_epochs_manifest(ep, ep_len, 3600, 40, man_hash) == 1,
          "hour 40 lies in an epoch that is not listed");
    check(gx_epochs_manifest(ep, ep_len, 7, 0, man_hash) == -1010,
          "a span that does not divide the epoch returns -1010");
    check(gx_epochs_manifest(ep, ep_len, 0, 0, man_hash) == -1009,
          "a zero span returns -1009");
    check(gx_epochs_manifest(bad_ep, bad_ep_len, 3600, 0, man_hash) == -1007,
          "an unsorted index returns -1007");
    check(gx_epochs_manifest(ep, ep_len, 3600, 0, NULL) == -1011,
          "a null out32 returns -1011");
    check(man_hash[0] == 0x7f, "none and failures leave out32 untouched");
    free(ep);
    free(bad_ep);

    /* Frame records, from conformance/frame-record/valid/index.json. */
    size_t fr_reg_len = 0;
    size_t fr_len = 0;
    size_t fr_bad_len = 0;
    uint8_t* fr_reg = read_file(argv[9], &fr_reg_len);
    uint8_t* fr = read_file(argv[10], &fr_len);
    uint8_t* fr_bad = read_file(argv[11], &fr_bad_len);
    check(gx_validate((const uint8_t*)"frame-3", 7, fr, fr_len, err, sizeof err) == 0,
          "frame record validates under frame-3");
    rc = gx_validate((const uint8_t*)"frame-2", 7, fr, fr_len, err, sizeof err);
    printf("      frame record under frame-2: %d %s: %s\n", (int)rc, gx_error_name(rc), err);
    check(rc == 1026 && strcmp(gx_error_name(rc), "FRAME_RECORD_ID_MISMATCH") == 0,
          "frame record under another key returns 1026");
    check(gx_frame_record_agrees(fr_reg, fr_reg_len, fr, fr_len) == 0,
          "frame record agrees with the registry");
    check(gx_frame_record_agrees(fr_reg, fr_reg_len, fr_bad, fr_bad_len) ==
              GX_FRAME_DIFFERS_MASS,
          "a record with another mass returns GX_FRAME_DIFFERS_MASS");
    check(gx_frame_record_agrees(fr_reg, fr_reg_len, NULL, 3) == -106,
          "null record with non-zero length returns -106");
    free(fr_reg);
    free(fr);
    free(fr_bad);

    /* Overlay tiles under the key of conformance/overlay/. */
    {
        const char* ok = "s-7-2-3-5-6";
        size_t ov_len = 0;
        size_t sf_len = 0;
        uint8_t* ov = read_file(argv[12], &ov_len);
        uint8_t* sf = read_file(argv[13], &sf_len);
        check(gx_validate((const uint8_t*)ok, strlen(ok), ov, ov_len, err, sizeof err) == 0,
              "overlay tile validates under a surface key");
        check(gx_validate_layer((const uint8_t*)ok, strlen(ok), ov, ov_len, 1, err,
                                sizeof err) == 0,
              "overlay tile validates under an overlay layer");
        rc = gx_validate_layer((const uint8_t*)ok, strlen(ok), ov, ov_len, 0, err, sizeof err);
        printf("      overlay tile under another layer: %d %s: %s\n", (int)rc,
               gx_error_name(rc), err);
        check(rc == 1059 && strcmp(gx_error_name(rc), "OVERLAY_NOT_OVERLAY_LAYER") == 0,
              "overlay tile under another layer returns 1059");
        check(gx_validate_layer((const uint8_t*)ok, strlen(ok), sf, sf_len, 0, err,
                                sizeof err) == 0,
              "surface tile validates under another layer");
        rc = gx_validate_layer((const uint8_t*)ok, strlen(ok), sf, sf_len, 1, err, sizeof err);
        check(rc == 1058 && strcmp(gx_error_name(rc), "OVERLAY_LAYER_REFUSED") == 0,
              "surface tile under an overlay layer returns 1058");
        check(gx_validate_layer((const uint8_t*)ok, strlen(ok), NULL, 3, 1, err,
                                sizeof err) == 106,
              "null bytes with non-zero length returns 106");
        free(ov);
        free(sf);
    }

    /* Section addresses, two cases from conformance/address/cases.json. */
    check_address(
        "fb503059c9f88e93bf9c14aca0ab8b6cccb9da910e6296da024bdd41987cec17",
        NULL, 0, 0, 0, "registry",
        "8df746b277bc7fcdee07e4bf592af0f98ecc5a8cf076e1e014a9ab8116c156c6",
        "address of case registry-static");
    check_address(
        "44d9032ff40ad789b264a6ee0de404f5cd3fd627231d7324ac14c48e5957a0e4",
        "ed73fe4433eb779f192bc73a01e0549e2cbc504b91efae81f97255ef3a8807d1",
        INT64_C(1763130000000), 0, 0, "4-12-1024-2048-3071",
        "20feb104121014e8ebcdc8b4962bb1df05e9ba331f8c8817735c40ec666d97a2",
        "address of case depth-12-with-manifest-uses-origin");
    uint8_t out[32];
    memset(out, 0x7f, sizeof out);
    check(gx_section_address(NULL, NULL, 0, 0, 0, (const uint8_t*)"registry", 8,
                             out) == 962,
          "null module returns 962");
    check(gx_section_address(out, NULL, 0, 0, 0, (const uint8_t*)"x", 1, out) == 206,
          "malformed key returns 206");
    check(out[0] == 0x7f, "failure leaves out32 untouched");

    free(reg);
    free(mat);
    if (failures != 0) {
        printf("abi-check: %d failed\n", failures);
        return 1;
    }
    puts("abi-check ok");
    return 0;
}
