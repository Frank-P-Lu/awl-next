// site/check.test.js — plain-Node unit tests for check.js's pure logic.
// Run with: node site/check.test.js
// No test framework dependency — a tiny hand-rolled assert runner, matching
// this repo's "no build step for the landing page" discipline.

const assert = require("assert");
const { parseVersion, compareVersions, checkState } = require("./check.js");

let passed = 0;
function test(name, fn) {
  try {
    fn();
    passed++;
    console.log("ok - " + name);
  } catch (e) {
    console.error("FAIL - " + name);
    console.error(e);
    process.exitCode = 1;
  }
}

// --- parseVersion --------------------------------------------------------

test("parseVersion parses a bare dotted version", () => {
  assert.deepStrictEqual(parseVersion("0.2.10"), [0, 2, 10]);
});

test("parseVersion strips a leading v", () => {
  assert.deepStrictEqual(parseVersion("v1.0.0"), [1, 0, 0]);
});

test("parseVersion rejects garbage", () => {
  assert.strictEqual(parseVersion("not-a-version"), null);
  assert.strictEqual(parseVersion(""), null);
  assert.strictEqual(parseVersion(null), null);
  assert.strictEqual(parseVersion(undefined), null);
});

// --- compareVersions -------------------------------------------------------

test("compareVersions orders patch/minor/major correctly", () => {
  assert.strictEqual(compareVersions("0.1.0", "0.1.0"), 0);
  assert.strictEqual(compareVersions("0.1.0", "0.1.1"), -1);
  assert.strictEqual(compareVersions("0.1.1", "0.1.0"), 1);
  assert.strictEqual(compareVersions("0.1.0", "0.2.0"), -1);
  assert.strictEqual(compareVersions("1.0.0", "0.9.9"), 1);
});

test("compareVersions treats a missing trailing component as zero", () => {
  assert.strictEqual(compareVersions("0.2", "0.2.0"), 0);
});

test("compareVersions is inert (0) on unparseable input", () => {
  assert.strictEqual(compareVersions("garbage", "0.1.0"), 0);
  assert.strictEqual(compareVersions("0.1.0", "garbage"), 0);
});

// --- checkState: the three rendered states --------------------------------

test("STATE 1: current — the param matches the site version", () => {
  const s = checkState("0.1.0", false, "0.1.0");
  assert.strictEqual(s.kind, "current");
  assert.strictEqual(s.showDownload, false);
  assert.ok(s.text.includes("current"));
  assert.ok(s.text.includes("0.1.0"));
});

test("STATE 1b: current — the param is NEWER than the site (never behind)", () => {
  const s = checkState("0.1.0", false, "0.2.0");
  assert.strictEqual(s.kind, "current");
});

test("STATE 2: available — the param is older than the site version", () => {
  const s = checkState("0.2.0", false, "0.1.0");
  assert.strictEqual(s.kind, "available");
  assert.strictEqual(s.showDownload, true);
  assert.ok(s.text.includes("0.2.0"));
  assert.ok(s.text.includes("0.1.0"));
});

test("STATE 3a: unknown — no ?v= param at all", () => {
  const s = checkState("0.1.0", false, null);
  assert.strictEqual(s.kind, "unknown");
  assert.strictEqual(s.showDownload, true);
  assert.ok(s.text.includes("0.1.0"));
});

test("STATE 3b: unknown — version.json fetch failed (siteVersion null)", () => {
  const s = checkState(null, false, "0.1.0");
  assert.strictEqual(s.kind, "unknown");
  assert.strictEqual(s.showDownload, true);
});

test("STATE 3c: unknown — no tagged release yet (prerelease flag honest)", () => {
  const s = checkState("0.0.0", true, "0.1.0");
  assert.strictEqual(s.kind, "unknown");
  assert.strictEqual(s.showDownload, true);
  assert.ok(s.text.toLowerCase().includes("no tagged release"));
});

console.log(passed + " passed");
if (process.exitCode) {
  process.exit(process.exitCode);
}
