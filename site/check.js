// site/check.js — pure version-comparison logic for site/check.html.
//
// The awl BINARY never makes a network request (see CLAUDE.md's zero-network
// law). "Check for Updates" opens this page in the OS browser with the
// installed build's version as a `?v=` query param; this SITE does the
// comparison against its own same-origin version.json (generated at deploy,
// never committed — see .github/workflows/deploy-web.yml). No trackers, no
// external resources, no cookies.
//
// Kept as plain functions (no ES module syntax) so this file loads directly
// via a <script> tag in the browser AND via `require()` in a plain Node test
// runner — one file, two runtimes, no build step for either.

/**
 * Parse a version string ("v0.2.0" or "0.2.0") into an array of integers,
 * or null if it doesn't look like a dotted numeric version at all.
 */
function parseVersion(v) {
  if (typeof v !== "string" || v.length === 0) return null;
  var parts = v.replace(/^v/i, "").split(".").map(function (n) {
    return parseInt(n, 10);
  });
  for (var i = 0; i < parts.length; i++) {
    if (isNaN(parts[i])) return null;
  }
  return parts;
}

/**
 * Compare two version strings component-wise (missing trailing components
 * count as 0, so "0.2" == "0.2.0"). Returns -1 / 0 / 1, or 0 (treated as
 * "equal, nothing to report") if either string doesn't parse — never throws.
 */
function compareVersions(a, b) {
  var pa = parseVersion(a);
  var pb = parseVersion(b);
  if (!pa || !pb) return 0;
  var len = Math.max(pa.length, pb.length);
  for (var i = 0; i < len; i++) {
    var x = pa[i] || 0;
    var y = pb[i] || 0;
    if (x !== y) return x < y ? -1 : 1;
  }
  return 0;
}

var RELEASES_URL = "https://github.com/Frank-P-Lu/awl-next/releases";

/**
 * The ONE decision this page makes, as a pure function of its three inputs —
 * never touches the DOM or the network itself, so it's directly testable.
 *
 *   siteVersion — the site's own version.json `version` field, or null if the
 *                 fetch failed / the file is missing / malformed.
 *   prerelease  — version.json's `prerelease` flag (true when no tag has
 *                 shipped yet — "0.0.0" is not a real version to compare
 *                 against).
 *   param       — the installed build's version from the page's own `?v=`
 *                 query string, or null if absent.
 *
 * Returns `{ kind, text, showDownload }`:
 *   - kind "current":   param is present and >= the site's version.
 *   - kind "available": param is present and older than the site's version.
 *   - kind "unknown":   no param, the fetch failed, or the site has no real
 *                       tagged release yet — the honest "can't say, here's
 *                       where to look" fallback, matter-of-fact either way.
 */
function checkState(siteVersion, prerelease, param) {
  if (siteVersion === null || siteVersion === undefined) {
    return {
      kind: "unknown",
      text: "could not check just now. downloads are here:",
      showDownload: true,
    };
  }
  if (prerelease) {
    return {
      kind: "unknown",
      text: "no tagged release yet. downloads will appear here once one ships:",
      showDownload: true,
    };
  }
  if (!param) {
    return {
      kind: "unknown",
      text: "latest is " + siteVersion + ". downloads here:",
      showDownload: true,
    };
  }
  if (compareVersions(param, siteVersion) < 0) {
    return {
      kind: "available",
      text: "v" + siteVersion + " available (you have v" + param + "):",
      showDownload: true,
    };
  }
  return {
    kind: "current",
    text: "you're current (v" + param + ").",
    showDownload: false,
  };
}

if (typeof module !== "undefined" && module.exports) {
  module.exports = { parseVersion: parseVersion, compareVersions: compareVersions, checkState: checkState, RELEASES_URL: RELEASES_URL };
}
