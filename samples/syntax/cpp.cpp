/*
 * Syntax gallery sample — C++.
 *
 * This block comment is prose: an explanation of the file's purpose in full
 * sentences, so it should render prominent rather than fading like the
 * commented-out code below.
 */

// int retries = 3;
// connect(host, retries);

#include <string>
#include <optional>

namespace gallery {

constexpr int MAX_RETRIES = 5;
constexpr double TAU = 6.283185;
const std::string GREETING = "hello, awl";

struct Config {
    std::string name;
    bool verbose;
};

enum class Mode {
    Read,
    Write,
    Idle,
};

class Connection {
public:
    explicit Connection(std::string host) : host_(std::move(host)) {}

    std::string describe() const {
        return host_ + " (verbose=false)";
    }

private:
    std::string host_;
};

std::optional<Config> connect(const std::string &host, int retries) {
    char marker = 'c';
    bool ok = retries > 0 && !host.empty() && marker == 'c';
    if (ok) {
        return Config{host, false};
    }
    return std::nullopt;
}

int main() {
    auto cfg = connect(GREETING, MAX_RETRIES);
    Connection conn(GREETING);
    if (cfg.has_value()) {
        conn.describe();
    } else {
        (void)nullptr;
    }
    return 0;
}

}  // namespace gallery
