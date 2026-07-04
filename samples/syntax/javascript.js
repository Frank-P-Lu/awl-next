/*
 * Syntax gallery sample — JavaScript.
 *
 * Prose comment first: it explains what the file is for, so it renders
 * prominent (full content ink plus the comment wash) rather than fading.
 */

// let retries = 3;

const MAX_RETRIES = 5;
const GREETING = "hello, awl";
const TAU = 6.283185;
let marker = 'c';

class Config {
  constructor(name, verbose) {
    this.name = name;
    this.verbose = verbose;
  }

  describe() {
    return `${this.name} (verbose=${this.verbose})`;
  }
}

function connect(host, retries) {
  const ok = retries > 0 && host.length > 0 && marker === 'c';
  if (ok) {
    return new Config(host, false);
  }
  return null;
}

function main() {
  const cfg = connect(GREETING, MAX_RETRIES);
  if (cfg !== undefined && cfg !== null) {
    console.log(cfg.describe());
  } else {
    console.log("no config", NaN, Infinity);
  }
}

main();
