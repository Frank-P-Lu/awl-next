# RELEASING.md — cutting a release + deploying the website

Two independent GitHub Actions pipelines, both `workflow_dispatch` (deliberate,
never automatic): `.github/workflows/deploy-web.yml` (the site + `/editor/`
demo, on Fly.io) and `.github/workflows/release.yml` (macOS / Linux / web
downloadable artifacts, on a `v*` tag push or a manual dry run). This doc is
the one-time setup for both, plus how to actually cut a release.

## 1. Apple setup (macOS signing + notarization)

Signing is **optional but gated** — without these five secrets, `release.yml`
still builds an unsigned universal `Awl.app` + `.dmg` (loudly logged as
unsigned). Set all five together or none; a partial set is treated as "not
configured."

**(a) Export your Developer ID Application certificate as a `.p12`:**

1. In Xcode or the [Apple Developer portal](https://developer.apple.com/account/resources/certificates/list),
   create/download a **"Developer ID Application"** certificate (requires a
   paid Apple Developer Program membership).
2. In Keychain Access, find the certificate + its private key, select both,
   right-click → **Export 2 items…** → save as `DeveloperIDApplication.p12`,
   set an export password.

```sh
base64 -i DeveloperIDApplication.p12 | pbcopy
gh secret set MACOS_CERT_P12 --body "$(pbpaste)"
gh secret set MACOS_CERT_PASSWORD --body "<the export password you set>"
```

**(b) Create an App Store Connect API key** (for `notarytool`, no separate
Apple ID password/2FA prompt needed in CI):

1. [App Store Connect](https://appstoreconnect.apple.com/) → Users and Access
   → Integrations → **App Store Connect API** → generate a key with the
   **Developer** role. Download the `.p8` **once** (Apple won't let you
   re-download it).

```sh
gh secret set APPLE_API_KEY_ID --body "<the Key ID shown in the portal>"
gh secret set APPLE_API_ISSUER --body "<the Issuer ID shown in the portal>"
base64 -i AuthKey_XXXXXXXXXX.p8 | pbcopy
gh secret set APPLE_API_KEY_B64 --body "$(pbpaste)"
```

That's all five secrets `release.yml`'s mac job checks for:
`MACOS_CERT_P12`, `MACOS_CERT_PASSWORD`, `APPLE_API_KEY_ID`,
`APPLE_API_ISSUER`, `APPLE_API_KEY_B64`.

## 2. Fly.io setup (website deploy)

```sh
fly tokens create deploy -a awl-editor    # scoped deploy token for the app in site/fly.toml
gh secret set FLY_API_TOKEN --body "<the token printed above>"
```

That's the one secret `deploy-web.yml` checks for. If it's missing, the
workflow fails immediately on its first step rather than burning a wasm build
for nothing.

## 3. Cutting a release

**Website (landing + `/editor/` wasm demo):**

```sh
gh workflow run deploy-web.yml
gh run watch   # or check the Actions tab
```

Builds a fresh `trunk build --release --public-url /editor/`, assembles it
over a copy of `site/`, and `flyctl deploy`s that assembled directory. Never
touches or commits `site/editor/`'s checked-in bundle (legacy — see below).

**Downloadable artifacts (macOS / Linux / web):**

```sh
# 1. bump Cargo.toml's package.version if this is a real version bump
# 2. tag and push
git tag v0.1.0
git push origin v0.1.0
```

The tag push triggers `release.yml`: builds a macOS universal `.app`/`.dmg`
(signed + notarized if the Apple secrets are set, unsigned otherwise), a
Linux `.tar.gz`, and a zipped web `dist/`, then attaches all of them to a new
GitHub Release at that tag.

**Dry run (no tag, nothing published) — verify the pipeline is healthy:**

```sh
gh workflow run release.yml -f dry_run=true
gh run watch
```

Every job still builds; artifacts land in the run's **Artifacts** tab
instead of a GitHub Release, and no tag or release is created.

### What lands where

| Artifact | Where |
|---|---|
| `Awl.app` (universal, signed+notarized if secrets set) + `Awl.dmg` | GitHub Release (tag) or workflow artifact `awl-macos` (dry run) |
| `awl-linux-x86_64.tar.gz` | GitHub Release or workflow artifact `awl-linux` |
| `awl-web-dist.zip` (the `trunk build --release` output) | GitHub Release or workflow artifact `awl-web` |
| the live website + `/editor/` demo | Fly.io (`awl-editor`, `site/fly.toml`) — via `deploy-web.yml`, separately |

### Icon TODO

`scripts/package-macos.sh` looks for `assets/macos/Awl.icns` and wires it
into the bundle's `Contents/Resources/` + `Info.plist` (`CFBundleIconFile`)
**only if that file exists** — the bundle builds and runs fine without one
today (generic app icon). Once the user's icon is ready: drop the `.icns` at
`assets/macos/Awl.icns` and uncomment the two `cp`/wiring lines flagged in
`scripts/package-macos.sh` (search for "ICON:").

## 4. The LICENSE gap (blocking a public release — the user's decision)

The repo ships a plain GPL-3.0 `LICENSE` file (matching `Cargo.toml`'s
`license = "GPL-3.0-only"`) and `assets/fonts/LICENSES.md` for the bundled
OFL font faces — but the bundled Hunspell dictionaries (`assets/dict/*.dic`
/ `*.aff`) have no accompanying license notice in-tree, and nothing states
who holds copyright on awl's own code (no `NOTICE` / copyright header, no
CONTRIBUTORS file) — both worth resolving explicitly before treating this as
a public release rather than a personal tool with a license file attached.
