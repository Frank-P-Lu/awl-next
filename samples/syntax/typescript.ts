/*
 * Syntax gallery sample — TypeScript.
 *
 * This block comment is prose: an explanation of the file's purpose, so it
 * should render prominent rather than receding to the muted commented-out
 * code ink used below.
 */

// let retries = 3;
// connect(host, retries);

const MAX_RETRIES: number = 5;
const GREETING: string = "hello, awl";
const TAU: number = 6.283185;
let marker: string = 'c';

interface Describable {
  describe(): string;
}

type Retries = number;

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

enum Mode {
  Read,
  Write,
  Idle,
}

function connect(host: string, retries: Retries): Config | null {
  const ok = retries > 0 && host.length > 0 && marker === 'c';
  if (ok) {
    return new Config(host, false);
  }
  return null;
}

const cfg = connect(GREETING, MAX_RETRIES);
console.log(cfg !== null ? cfg.describe() : "no config " + undefined);
