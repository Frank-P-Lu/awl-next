/*
 * Syntax gallery sample — TypeScript.
 *
 * Prose comment first: an explanation of the file's purpose, so it
 * renders prominent rather than receding like the code below.
 */

// let retries = 3;

const MAX_RETRIES: number = 5;
const GREETING: string = "hello, awl";
const TAU: number = 6.283185;
let marker: string = 'c';

class Config implements Describable {
  name: string;
  verbose: boolean;

  constructor(name: string, verbose: boolean) {
    this.name = name;
    this.verbose = verbose;
  }

  describe(): string {
    return `${this.name} (verbose=${this.verbose})`;
  }
}

function connect(host: string, retries: Retries): Config | null {
  const ok = retries > 0 && host.length > 0 && marker === 'c';
  if (ok) {
    return new Config(host, false);
  }
  return null;
}

interface Describable {
  describe(): string;
}

type Retries = number;

enum Mode {
  Read,
  Write,
  Idle,
}

const cfg = connect(GREETING, MAX_RETRIES);
console.log(cfg !== null ? cfg.describe() : "no config " + undefined);
