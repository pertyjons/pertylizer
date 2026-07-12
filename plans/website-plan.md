# Pertylizer Website Plan

> **Status:** Draft / not started. This document captures the agreed approach for a
> public Pertylizer website (marketing + show-off + documentation) so the build can
> begin later from a settled plan.
>
> **Privacy note:** Hosting provider, DNS records, certificate/redirect settings, and
> any account details are **deliberately kept out of this file** (the repo is intended
> to go public). Those are configured out-of-band. Only the repo-side pieces (the
> static-site build and its CI) are described here. Domains are referred to abstractly
> as the **primary (canonical) domain** and the **secondary domain**.

## Goal

A real product website — not just a docs "book". It should:

- **Market / show off** the app: hero, feature highlights, screenshots, examples.
- **Document** it: getting started, user guide, the MCP/AI-agent integration, the YAMS
  scripting language, reference material, and a changelog.
- Be **low-maintenance** and **updatable from this repo** (edit markdown → push → auto-deploy).

## Decisions (settled)

| Decision         | Choice                                                                                                                    | 
|------------------|---------------------------------------------------------------------------------------------------------------------------|
| Build tool       | **Astro + Starlight** (Astro pages for landing/marketing, Starlight for docs)                                             |
| Location in repo | New top-level `website/` directory (kept separate from `docs/`, which stays the markdown source of truth)                 |
| Deploy target    | **GitHub Pages**, built in this repo's own GitHub Actions                                                                 |
| Canonical domain | Primary domain is canonical; secondary domain issues an **HTTPS 301** to it (configured out-of-band, not documented here) |
| Node toolchain   | Node available locally (v22 / npm 10); Astro requires Node 18+                                                            |

## Why Astro + Starlight

- Best fit for "marketing + show off **and** docs" in one site.
- Landing/marketing pages are free-form Astro components (hero, feature grid, gallery, CTA).
- Docs get Starlight for free: client-side search (Pagefind), dark mode, auto-generated
  sidebar from markdown frontmatter, good typography.
- Only trade-off vs a Rust-native generator (Zola/mdBook) is that CI pulls in Node/npm —
  accepted.

## Repository layout (target)

```
website/
  package.json
  astro.config.mjs          # site URL, Starlight integration, base
  tsconfig.json
  public/
    CNAME                    # canonical domain, copied verbatim into the build output
    favicon / logo / og-image
  src/
    pages/
      index.astro           # landing page (hero, highlights, CTA)
      screenshots.astro     # gallery (optional; could be a docs page instead)
      download.astro        # per-platform download, points at GitHub Releases
    content/
      docs/                 # Starlight docs (see content plan below)
    components/             # Hero, FeatureGrid, ScreenshotGallery, etc.
    styles/                 # dark theme matching the app's egui vibe
  scripts/
    sync-docs.mjs           # pulls canonical markdown from ../docs + ../screenshots
```

## Content plan (reuse what already exists)

Almost everything is already markdown — the site should **reuse** it, not fork it.

| Source in repo                      | Destination on site                                    |
|-------------------------------------|--------------------------------------------------------|
| `README.md` ("How This Came About") | Landing intro + an **About** page                      |
| `README.md` "Highlights"            | Landing **feature grid** + a **Features/Tour** section |
| `screenshots/` (+ its README)       | **Screenshot gallery** / feature illustrations         |
| `docs/README_MCP.md`                | **MCP / AI Integration** guide (a headline feature)    |
| `docs/yams.md`                      | **YAMS** scripting reference                           |
| `docs/param-kinds.md`               | Reference: parameter kinds                             |
| `docs/references.md`                | **Resources** page                                     |
| `docs/history.md`                   | **Changelog** page                                     |
| `packaging/` (install scaffolding)  | **Getting Started / Install** + **Download** page      |
| GitHub Releases (tag workflow)      | Download links per platform (Linux/macOS/Windows)      |

**To be written new** (docs are currently sparse per the README's own admission):
a proper Getting Started, a short User Guide (building an instrument, the sequencer/tracker,
the sample bank), and landing copy.

### Keeping docs in sync (the "how to upgrade the site" answer)

Keep `docs/*.md` and `screenshots/` as the **single source of truth**. A small prebuild
step (`website/scripts/sync-docs.mjs`, run by `npm run build`) copies the canonical
markdown into `src/content/docs/`, injecting the Starlight frontmatter each page needs.
Net effect: updating `docs/history.md` (as the release flow already requires) or a
screenshot automatically flows to the site on the next deploy — no double authoring.

Newly-authored, site-only docs (Getting Started, User Guide) live directly in
`src/content/docs/` and are edited there.

## Deploy / CI

A **new, separate** workflow `.github/workflows/pages.yml`, fully decoupled from the
existing tag-triggered release build (`build.yml` stays untouched):

- **Trigger:** push to `main` with a path filter (`website/**`, `docs/**`, `screenshots/**`,
  and the workflow file), plus `workflow_dispatch` for manual runs.
- **Steps:** checkout → setup-node → `npm ci` (in `website/`) → `npm run build`
  (Astro → `website/dist`) → `actions/upload-pages-artifact` → `actions/deploy-pages`.
- **Permissions:** `pages: write`, `id-token: write`; a `concurrency` group so overlapping
  pushes don't race.
- **Custom domain:** the canonical domain is emitted via `public/CNAME` into the build
  output; the repo's Pages custom-domain + "Enforce HTTPS" toggle is set out-of-band.

Result: docs/marketing changes push to `main` and redeploy in ~a minute, with **zero**
interaction with the version-tag release pipeline. Pushing `v*` tags still only triggers
app releases; pushing site changes never triggers a release.

## Design / feel

Match the app's identity: dark, immediate, a little "last-century" (the egui vibe the
README describes). Reuse the existing logo (`crates/pertylizer/assets/images/pertylizer.png`)
and derive an accent palette from it. Style Starlight via custom CSS overrides.

Open design questions to settle before/while building:

- Embedded **audio demos** on the landing/features pages? (render short WAV/MP3 examples
  via the engine's render-to-wav, host as static assets.)
- **Analytics** — default to none / privacy-friendly.
- Whether the screenshot gallery is a landing section, a standalone page, or a docs page.

## Suggested phasing

0. **Scaffold** Astro + Starlight in `website/`; green local `npm run build`. New git
   branch (e.g. `feat/website`), independent of unrelated in-flight app branches.
1. **Landing page** — hero, highlights grid, CTA; branding/theme.
2. **Docs migration** — `sync-docs.mjs` + Starlight sidebar: MCP guide, YAMS, Changelog,
   Reference; author Getting Started.
3. **Screenshots gallery** + Features/Tour pages.
4. **Download page** wired to GitHub Releases.
5. **CI deploy workflow**; custom domain + HTTPS 301 for the secondary domain configured
   out-of-band; verify end-to-end.
6. **Document the update flow** in `CLAUDE.md` (a short "How to update the website" section)
   so it becomes a fixed convention.

## Out of scope for this plan

- Actual hosting/DNS/redirect configuration (kept private, done out-of-band).
- Any CMS or server-side component — the site is fully static.
