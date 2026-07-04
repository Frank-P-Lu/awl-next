/*
 * Syntax gallery sample — Java.
 *
 * Prose comment first: it explains the file's purpose in full sentences,
 * so it renders prominent rather than fading like the code below.
 */

// int retries = 3;

import java.util.Optional;

public class Connection {
    static final int MAX_RETRIES = 5;
    static final String GREETING = "hello, awl";
    static final double TAU = 6.283185;

    record Config(String name, boolean verbose) {
        public String describe() {
            return name + " (verbose=" + verbose + ")";
        }
    }

    static Optional<Config> connect(String host, int retries) {
        char marker = 'c';
        boolean ok = retries > 0 && host.length() > 0 && marker == 'c';
        if (ok) {
            return Optional.of(new Config(host, false));
        }
        return Optional.empty();
    }

    interface Describable {
        String describe();
    }

    enum Mode {
        READ,
        WRITE,
        IDLE,
    }

    public static void main(String[] args) {
        Optional<Config> cfg = connect(GREETING, MAX_RETRIES);
        if (cfg.isPresent()) {
            System.out.println(cfg.get().describe());
        } else {
            System.out.println("no config, was " + null);
        }
    }
}
