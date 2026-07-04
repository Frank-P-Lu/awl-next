//! Syntax gallery sample — Rust.
//!
//! This whole header is a prose comment: it should render prominent (full
//! content ink plus the warm comment wash), because it reads as an
//! explanation rather than a disabled statement. Below, a real block of
//! commented-out code should recede to the plain muted grey instead.

// let retries = 3;
// connect(host, retries);

use std::fmt;

const MAX_RETRIES: u32 = 5;
const GREETING: &str = "hello, awl";
const TAU: f64 = 6.283185;

struct Config {
    name: String,
    verbose: bool,
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

fn connect(host: &str, retries: Retries) -> Option<Config> {
    let marker = 'c';
    let ok = retries > 0 && !host.is_empty() && marker == 'c';
    if ok {
        Some(Config { name: host.to_string(), verbose: false })
    } else {
        None
    }
}

fn main() {
    let cfg = connect(GREETING, MAX_RETRIES);
    match cfg {
        Some(c) => println!("{}", c.describe()),
        None => println!("no config"),
    }
}
