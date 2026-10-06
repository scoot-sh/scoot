// Build gate: no built HTML page may link to a raw `.md` twin, and every
// rewritten page link must resolve to a page in `dist/`. Run after
// `astro build` (`pnpm verify`, the nix `docs-site` derivation, and CI).
//
// Background: the sources write relative `.md` links (good for editors,
// GitHub and the raw twins at `/<slug>.md`), and a remark plugin in
// `astro.config.mjs` rewrites them in the HTML render only
// (`foo/bar.md` → `foo/bar/`, `foo/index.md` → `foo/`, anchors and query
// strings kept). The twins and llms.txt keep their `.md` links; this gate
// covers the HTML side:
//   1. No relative `href` in any built page ends in `.md` or `.md#…`
//      (an unrewritten link: a human clicking it lands on raw Markdown).
//   2. Every rewritten relative page link resolves to a page in `dist/`
//      (`./scoot/theming/` → `dist/scoot/theming/index.html`).
//
// Only relative hrefs are checked: Starlight's own sidebar/pagination
// links are root-absolute (generated from slugs, never `.md`), and
// absolute/external URLs and asset links are out of scope. Asset hrefs
// (a file extension in the last path segment: `.css`, `.webp`, …) are
// skipped by the resolution check.
import { existsSync, readFileSync, readdirSync, statSync } from 'node:fs';
import { dirname, join, resolve, sep } from 'node:path';

const root = new URL('..', import.meta.url).pathname;
const dist = join(root, 'dist');

const failures = [];
const fail = (message) => failures.push(message);

const walk = (dir) =>
  readdirSync(dir, { withFileTypes: true }).flatMap((entry) => {
    const path = join(dir, entry.name);
    if (entry.isDirectory()) return walk(path);
    return entry.name.endsWith('.html') ? [path] : [];
  });

if (!existsSync(dist)) {
  console.error('check-md-links: FAIL\n  - no dist/ directory (run `astro build` first)');
  process.exit(1);
}

const isExternal = (target) => /^[a-z][a-z0-9+.-]*:/i.test(target);

let rewritten = 0;
for (const file of walk(dist)) {
  const html = readFileSync(file, 'utf8');
  const dir = dirname(file);
  const rel = file.replace(`${dist}${sep}`, '');
  for (const match of html.matchAll(/href="([^"]*)"/g)) {
    const raw = match[1];
    if (!raw || raw.startsWith('#') || raw.startsWith('/') || raw.startsWith('//')) continue;
    if (isExternal(raw)) continue;
    const pathPart = raw.split('#')[0].split('?')[0];
    if (pathPart.endsWith('.md')) {
      fail(`${rel}: links ${raw}, a raw Markdown twin instead of a page`);
      continue;
    }
    // Asset hrefs carry a file extension in the last segment; page links
    // do not (they end in `/`, or are `./`, `../`, `..`).
    const last = pathPart.split('/').pop();
    if (last !== '' && last !== '.' && last !== '..' && last.includes('.')) continue;
    const resolved = resolve(dir, pathPart || '.');
    const page =
      existsSync(resolved) && statSync(resolved).isDirectory()
        ? join(resolved, 'index.html')
        : `${resolved}.html`;
    if (!existsSync(page)) {
      fail(`${rel}: links ${raw}, which is no page in dist/`);
      continue;
    }
    rewritten += 1;
  }
}

if (failures.length > 0) {
  console.error('check-md-links: FAIL');
  for (const failure of failures) console.error(`  - ${failure}`);
  process.exit(1);
}
console.log(`check-md-links: ok (no .md hrefs in built HTML, ${rewritten} relative page links resolve)`);
