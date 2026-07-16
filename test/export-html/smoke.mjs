// smoke.mjs — the repeatable Playwright pass over awl's HTML export.
//
// Renders the exported `rich.html` golden (src/export/testdata/rich.html) in a
// REAL headless Chromium and asserts the rich document actually paints: no empty
// list item, the table grid carries every fixture cell, both task checkboxes are
// present with the right checked state, the embedded image decodes, the heading
// ladder h1–h3 is present, and the print stylesheet's `@page` rule is reachable.
//
// This is the browser-side counterpart to the Rust golden gate: the goldens prove
// byte-stable BYTES; this proves those bytes render as a real document (the user
// reported Pages mangling the docx table — the HTML/PDF path must stay sound).
//
// Run via scripts/smoke-export-html.sh (bootstraps Playwright + regenerates the
// golden). Standalone:  node test/export-html/smoke.mjs [path-to-rich.html]
//
// Exit code 0 = every assertion held; 1 = a failure (message on stderr).

import { chromium } from "playwright";
import { fileURLToPath, pathToFileURL } from "node:url";
import { dirname, resolve } from "node:path";
import { existsSync } from "node:fs";

const HERE = dirname(fileURLToPath(import.meta.url));
const REPO = resolve(HERE, "..", "..");
const htmlPath = resolve(
  process.argv[2] ?? resolve(REPO, "src/export/testdata/rich.html"),
);

if (!existsSync(htmlPath)) {
  console.error(`smoke-export-html: golden not found: ${htmlPath}`);
  console.error("  regenerate with: AWL_BLESS=1 cargo test export::tests::html_golden");
  process.exit(1);
}

// The fixture's covered surface, mirrored from src/export/tests.rs FIXTURE so a
// silent content drop (the c9bead0 tight-list bug class) fails here too.
const EXPECT = {
  headings: { h1: "Export Fixture", h2: "Section Two", h3: "Subsection" },
  // Every list item's own words — no <li> may render empty.
  listItems: [
    "first bullet",
    "second bullet",
    "nested bullet",
    "third bullet",
    "one",
    "two",
    "three",
  ],
  tasks: [
    { label: "open task", checked: false },
    { label: "done task", checked: true },
  ],
  // The GFM table, header + body, exactly as the fixture declares it.
  tableGrid: [
    ["Left", "Center", "Right"],
    ["a", "b", "c"],
    ["dee", "eee", "eff"],
  ],
};

const failures = [];
const check = (cond, msg) => {
  if (!cond) failures.push(msg);
};

const browser = await chromium.launch();
try {
  const page = await browser.newPage();
  await page.goto(pathToFileURL(htmlPath).href, { waitUntil: "load" });

  // 1. Headings h1–h3 present with the fixture's text.
  for (const [tag, text] of Object.entries(EXPECT.headings)) {
    const got = await page.locator(tag).first().textContent().catch(() => null);
    check(got !== null, `missing <${tag}>`);
    check(
      got !== null && got.trim() === text,
      `<${tag}> text = ${JSON.stringify(got)}, want ${JSON.stringify(text)}`,
    );
  }

  // 2. Every <li> has non-empty text (the empty-<li> content-loss bug).
  const liTexts = await page.$$eval("li", (els) =>
    els.map((e) => (e.textContent ?? "").trim()),
  );
  check(liTexts.length > 0, "no <li> elements rendered");
  liTexts.forEach((t, i) => check(t.length > 0, `<li> #${i} rendered empty`));
  for (const word of EXPECT.listItems) {
    check(
      liTexts.some((t) => t.includes(word)),
      `list item text "${word}" missing from the rendered document`,
    );
  }

  // 3. Both task checkboxes present with the correct checked state.
  const boxes = await page.$$eval("li.task", (els) =>
    els.map((li) => ({
      text: (li.textContent ?? "").trim(),
      checked: li.querySelector('input[type="checkbox"]')?.checked ?? null,
      hasBox: !!li.querySelector('input[type="checkbox"]'),
    })),
  );
  check(boxes.length === 2, `expected 2 task items, got ${boxes.length}`);
  for (const want of EXPECT.tasks) {
    const li = boxes.find((b) => b.text.includes(want.label));
    check(li !== undefined, `task item "${want.label}" not found`);
    if (li) {
      check(li.hasBox, `task "${want.label}" has no checkbox`);
      check(
        li.checked === want.checked,
        `task "${want.label}" checked=${li.checked}, want ${want.checked}`,
      );
    }
  }

  // 4. The table grid matches the fixture cell-for-cell.
  const grid = await page.$$eval("table tr", (rows) =>
    rows.map((r) =>
      [...r.querySelectorAll("th,td")].map((c) => (c.textContent ?? "").trim()),
    ),
  );
  check(
    JSON.stringify(grid) === JSON.stringify(EXPECT.tableGrid),
    `table grid = ${JSON.stringify(grid)}, want ${JSON.stringify(EXPECT.tableGrid)}`,
  );

  // 5. The embedded image decodes (naturalWidth > 0 = the data: URI is real).
  const imgOk = await page.$$eval("img", (els) =>
    els.map((e) => ({ w: e.naturalWidth, h: e.naturalHeight })),
  );
  check(imgOk.length >= 1, "no <img> rendered");
  imgOk.forEach((d, i) =>
    check(d.w > 0 && d.h > 0, `<img> #${i} did not decode (naturalWidth=${d.w})`),
  );

  // 6. The print stylesheet is reachable: some rule tree contains an `@page`
  //    (CSSPageRule, type 6), possibly nested inside the `@media print` block.
  const hasPageRule = await page.evaluate(() => {
    const PAGE = 6; // CSSRule.PAGE_RULE
    const scan = (rules) => {
      for (const r of rules) {
        if (r.type === PAGE) return true;
        if (r.cssRules && scan(r.cssRules)) return true;
      }
      return false;
    };
    for (const sheet of document.styleSheets) {
      try {
        if (scan(sheet.cssRules)) return true;
      } catch {
        /* cross-origin — not our inline sheet */
      }
    }
    return false;
  });
  check(hasPageRule, "no @page rule reachable (print stylesheet missing)");
} finally {
  await browser.close();
}

if (failures.length) {
  console.error(`smoke-export-html: ${failures.length} FAILED`);
  for (const f of failures) console.error(`  ✗ ${f}`);
  process.exit(1);
}
console.log(`smoke-export-html: PASS — ${htmlPath}`);
console.log(
  "  ✓ headings h1–h3  ✓ non-empty <li>×" +
    "  ✓ 2 task checkboxes (state)  ✓ table grid  ✓ image decoded  ✓ @page rule",
);
