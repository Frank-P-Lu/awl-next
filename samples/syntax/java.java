/*
 * Syntax gallery sample — Java.
 *
 * This block comment is prose: it explains the file's purpose in full
 * sentences, so it should render prominent rather than fading like the
 * commented-out code below.
 */

// int retries = 3;
// connect(host, retries);

import java.util.Optional;

public class Connection {
    static final int MAX_RETRIES = 5;
    static final String GREETING = "hello, awl";
    static final double TAU = 6.283185;

    interface Describable {
        String describe();
    }

    enum Mode {
        READ,
        WRITE,
        IDLE,
    }

    record Config(String name, boolean verbose) implements Describable {
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

    public static void main(String[] args) {
        Optional<Config> cfg = connect(GREETING, MAX_RETRIES);
        if (cfg.isPresent()) {
            System.out.println(cfg.get().describe());
        } else {
            System.out.println("no config, was " + null);
        }
    }
}
