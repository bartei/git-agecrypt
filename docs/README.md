# git-agecrypt — site

Source for the [git-agecrypt](https://github.com/bartei/git-agecrypt) product page, docs, and quickstart. Hosted via GitHub Pages.

## Deploy on GitHub Pages

This folder is the entire site. There is **no build step** — JSX is transpiled in-browser by Babel Standalone, so the workflow uploads `docs/` as-is.

Deployment is automated by [`.github/workflows/pages.yml`](../.github/workflows/pages.yml). It runs on every push to `main` that touches `docs/**`, and it can be triggered manually from the Actions tab.

One-time repo setup:

1. Repo → **Settings** → **Pages**.
2. Under **Build and deployment**, set **Source** to **GitHub Actions**.
3. Push a change under `docs/` (or trigger the workflow via *Actions → Pages → Run workflow*).

The site will be live at `https://<user>.github.io/git-agecrypt/`.

## Local preview

Any static file server works. Two zero-install options:

```sh
# Python
python3 -m http.server 8000 --directory docs

# Node (via npx)
npx serve docs
```

Then open <http://localhost:8000>.

## Files

```
docs/
  index.html             # entry point
  styles.css             # all styles (CSS variables, light + dark themes)
  tweaks-panel.jsx       # in-page Tweaks panel (theme + logo variant)
  assets/
    data.js              # features, comparison rows, commands, workflows
    logos.jsx            # 4 abstract logo variants (lock + git node)
    terminal.jsx         # animated quickstart terminal
    app-top.jsx          # Nav, Hero, Features, Compare
    app-mid.jsx          # Quickstart, Docs
    app-bottom.jsx       # How it works, CTA, Footer
    app-root.jsx         # root component + Tweaks wiring
```

## Notes

- The Tweaks panel is for design iteration — it's harmless on the live site but you can strip the `<script type="text/babel" src="tweaks-panel.jsx"></script>` line from `index.html` and remove the panel render in `app-root.jsx` if you want to keep the bundle minimal.
- All third-party JS (React 18, Babel Standalone, Google Fonts) is loaded from public CDNs with subresource-integrity hashes pinned. Nothing gets pulled at build time.
- Custom domain? Drop a `CNAME` file (with your domain) into `docs/`.
