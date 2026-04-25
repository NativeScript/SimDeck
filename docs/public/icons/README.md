# Home page icons

These SVGs are used by the feature grid in [`docs/index.md`](../../index.md). They are vendored locally so the site has no runtime CDN dependency.

## Source

- **Library**: [Lucide](https://lucide.dev) — distributed via the [`lucide-static`](https://www.npmjs.com/package/lucide-static) npm package.
- **License**: ISC. The license comment at the top of each SVG (`<!-- @license lucide-static v<version> - ISC -->`) must stay intact.

## Manual edits applied to each icon

After downloading, two small tweaks are applied:

1. `stroke="currentColor"` → `stroke="#0a84ff"` so the icon picks up the Simdeck accent color in both light and dark themes (VitePress renders feature icons via `<img>`, which doesn't inherit the page color).
2. `stroke-width="2"` → `stroke-width="1.75"` for a slightly lighter feel at small sizes.

## Adding a new icon

1. Pick the icon at [lucide.dev](https://lucide.dev) and note its kebab-case name (e.g., `rocket`).
2. From the repo root, fetch and patch it:

   ```sh
   ICON=rocket
   curl -sSfL "https://unpkg.com/lucide-static@latest/icons/${ICON}.svg" \
     -o "docs/public/icons/${ICON}.svg"
   sed -i '' \
     -e 's/stroke="currentColor"/stroke="#0a84ff"/' \
     -e 's/stroke-width="2"/stroke-width="1.75"/' \
     "docs/public/icons/${ICON}.svg"
   ```

   (On Linux, drop the empty `''` after `sed -i`.)

3. Reference it from `docs/index.md`:

   ```yaml
   - icon:
       src: /icons/rocket.svg
       width: 28
       height: 28
     title: ...
     details: ...
   ```

4. `npm run docs:dev` to preview, `npm run docs:build` to verify the production build.

## Replacing the icon set

If you ever swap Lucide for another library (Phosphor, Tabler, Heroicons), keep the same constraints:

- Inline SVG, not a sprite or icon font.
- `stroke` (or `fill`) set to a fixed color that works on both backgrounds — `<img>` won't inherit `currentColor`.
- Roughly 24×24 viewBox so the existing `width: 28 / height: 28` in `index.md` keeps the visual rhythm.
- License attribution preserved in a comment at the top of each file.
