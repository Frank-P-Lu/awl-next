/*
 * Syntax gallery sample — C.
 *
 * This block comment is prose: an explanation of what the file
 * demonstrates, so it should render prominent rather than receding to the
 * muted commented-out code ink used below.
 */

// int retries = 3;
// connect(host, retries);

#include <stdio.h>
#include <stdbool.h>

#define MAX_RETRIES 5

static const char *GREETING = "hello, awl";
static const double TAU = 6.283185;

struct Config {
    char name[32];
    bool verbose;
};

union Value {
    int as_int;
    float as_float;
};

enum Mode {
    MODE_READ,
    MODE_WRITE,
    MODE_IDLE,
};

struct Config connect(const char *host, int retries) {
    char marker = 'c';
    bool ok = retries > 0 && host[0] != '\0' && marker == 'c';
    struct Config cfg;
    cfg.verbose = ok ? false : true;
    return cfg;
}

int main(void) {
    const char *unset = NULL;
    struct Config cfg = connect(GREETING, MAX_RETRIES);
    if (cfg.verbose) {
        printf("verbose config\n");
    } else {
        printf("quiet config, %s\n", GREETING);
    }
    return 0;
}
