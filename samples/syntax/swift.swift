/*
 * Syntax gallery sample — Swift.
 *
 * Prose comment first: it explains the file's purpose in full sentences,
 * so it renders prominent rather than fading like the code below.
 */

// let retries = 3;

let maxRetries = 5
let greeting = "hello, awl"
let tau = 6.283185
let marker: Character = "c"

struct Config: Describable {
    var name: String
    var verbose: Bool

    func describe() -> String {
        return "\(name) (verbose=\(verbose))"
    }
}

func connect(host: String, retries: Int) -> Config? {
    let ok = retries > 0 && !host.isEmpty && marker == "c"
    if ok {
        return Config(name: host, verbose: false)
    }
    return nil
}

protocol Describable {
    func describe() -> String
}

enum Mode {
    case read
    case write
    case idle
}

extension Config {
    var isQuiet: Bool {
        return !verbose
    }
}

let cfg = connect(host: greeting, retries: maxRetries)
if let c = cfg {
    print(c.describe())
} else {
    print("no config, was \(String(describing: nil as Config?))")
}
