# Syntax gallery sample — Python.
#
# This paragraph is a prose comment: several stacked line comments that read
# as an explanation, not code, so they should render prominent (full content
# ink plus the comment wash) rather than fading to the muted grey below.

# retries = 3;
# connect(host, retries);

import json

MAX_RETRIES = 5
GREETING = "hello, awl"
TAU = 6.283185
MARKER = 'c'


class Config:
    """A tiny connection config, just for the gallery."""

    def __init__(self, name, verbose=False):
        self.name = name
        self.verbose = verbose

    def describe(self):
        return f"{self.name} (verbose={self.verbose})"


class Mode:
    READ = 1
    WRITE = 2
    IDLE = 3


def connect(host, retries=MAX_RETRIES):
    ok = retries > 0 and len(host) > 0 and MARKER == 'c'
    if ok:
        return Config(host, verbose=False)
    return None


def main():
    cfg = connect(GREETING)
    if cfg is not None:
        print(cfg.describe())
    else:
        print("no config, retries were", None)


if __name__ == "__main__":
    main()
