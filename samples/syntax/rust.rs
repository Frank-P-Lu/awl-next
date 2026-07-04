//! Syntax gallery sample — Rust.
//!
//! Prose comment first: it reads as an explanation, not code, so it renders
//! prominent (full content ink plus the warm wash) rather than fading.

// let retries = 3;

const MAX_RETRIES: u32 = 5;
const GREETING: &str = "hello, awl";
const TAU: f64 = 6.283185;

struct Config {
    name: String,
    verbose: bool,
}

fn connect(host: &str, retries: u32) -> Option<Config> {
    let marker = 'c';
    let ok = retries > 0 && !host.is_empty() && marker == 'c';
    if ok {
        Some(Config { name: host.to_string(), verbose: false })
    } else {
        None
    }
}

enum Mode {
    Read,
    Write,
    Idle,
}

trait Describe {
    fn describe(&self) -> String;
}

impl Describe for Config {
    fn describe(&self) -> String {
        format!("{} (verbose={})", self.name, self.verbose)
    }
}

type Retries = u32;

fn main() {
    let cfg = connect(GREETING, MAX_RETRIES);
    match cfg {
        Some(c) => println!("{}", c.describe()),
        None => println!("no config"),
    }
}
