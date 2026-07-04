# Syntax gallery sample — Python.
#
# Prose comment first: it reads as an explanation, not code, so it renders
# prominent (full content ink plus the comment wash) rather than fading.

# retries = 3;

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


def connect(host, retries=MAX_RETRIES):
    ok = retries > 0 and len(host) > 0 and MARKER == 'c'
    if ok:
        return Config(host, verbose=False)
    return None


class Mode:
    READ = 1
    WRITE = 2
    IDLE = 3


def main():
    cfg = connect(GREETING)
    if cfg is not None:
        print(cfg.describe())
    else:
        print("no config, retries were", None)


if __name__ == "__main__":
    main()
