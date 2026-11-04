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
//   3. Every `#fragment` (on a page link, or a bare in-page `#x`) names an
//      `id` that exists in the target page's built HTML. Checking the
//      rendered ids rather than re-deriving slugs means Starlight's own
//      github-slugger output, explicit `{#id}` anchors and duplicate
//      `-1` suffixes are right by construction.
//
// Only relative hrefs are checked: Starlight's own sidebar/pagination
// links are root-absolute (generated from slugs, never `.md`), and
// absolute/external URLs and asset links are out of scope. Asset hrefs
// (a file extension in the last path segment: `.css`, `.webp`, …) are
// skipped by the resolution check.
import { existsSync, readFileSync, readdirSync, statSync } from 'node:fs';
import { dirname, join, resolve, sep } from 'node:path';

const walk = (dir) =>
  readdirSync(dir, { withFileTypes: true }).flatMap((entry) => {
    const path = join(dir, entry.name);
    if (entry.isDirectory()) return walk(path);
    return entry.name.endsWith('.html') ? [path] : [];
  });

const isExternal = (target) => /^[a-z][a-z0-9+.-]*:/i.test(target);

// Every `id="…"` in a page. A fragment may also match a `name=` anchor in
// browsers, but nothing here emits one, so ids are the whole contract.
const idCache = new Map();
const idsOf = (file) => {
  let ids = idCache.get(file);
  if (!ids) {
    ids = new Set();
    for (const m of readFileSync(file, 'utf8').matchAll(/\sid=(?:"([^"]*)"|'([^']*)')/g)) {
      ids.add(m[1] ?? m[2]);
    }
    idCache.set(file, ids);
  }
  return ids;
};

const decode = (fragment) => {
  try {
    return decodeURIComponent(fragment);
  } catch {
    return fragment;
  }
};

// Returns `{ failures, rewritten, fragments }` for a built `dist/` tree.
export function checkLinks(dist) {
  const failures = [];
  const fail = (message) => failures.push(message);
  idCache.clear();
  let rewritten = 0;
  let fragments = 0;
  const checkFragment = (rel, raw, page, fragment) => {
    // `#` and `#top` always scroll to the top of the page.
    if (fragment === '' || fragment === 'top') return;
    fragments += 1;
    if (!idsOf(page).has(decode(fragment))) {
      fail(`${rel}: links ${raw}, but ${page.replace(`${dist}${sep}`, '')} has no #${fragment} anchor`);
    }
  };
  for (const file of walk(dist)) {
    const html = readFileSync(file, 'utf8');
    const dir = dirname(file);
    const rel = file.replace(`${dist}${sep}`, '');
    for (const match of html.matchAll(/href="([^"]*)"/g)) {
      const raw = match[1];
      if (raw.startsWith('#')) {
        checkFragment(rel, raw, file, raw.slice(1));
        continue;
      }
      if (!raw || raw.startsWith('/') || raw.startsWith('//')) continue;
      if (isExternal(raw)) continue;
      const [beforeHash, fragment = ''] = raw.split(/#(.*)/s);
      const pathPart = beforeHash.split('?')[0];
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
      checkFragment(rel, raw, page, fragment);
    }
  }
  return { failures, rewritten, fragments };
}

// Imported by test-check-md-links.mjs: only run the CLI on a direct run.
if (import.meta.url === `file://${process.argv[1]}`) {
  const dist = join(new URL('..', import.meta.url).pathname, 'dist');
  if (!existsSync(dist)) {
    console.error('check-md-links: FAIL\n  - no dist/ directory (run `astro build` first)');
    process.exit(1);
  }
  const { failures, rewritten, fragments } = checkLinks(dist);
  if (failures.length > 0) {
    console.error('check-md-links: FAIL');
    for (const failure of failures) console.error(`  - ${failure}`);
    process.exit(1);
  }
  console.log(
    `check-md-links: ok (no .md hrefs in built HTML, ${rewritten} relative page links resolve, ${fragments} fragments match an id)`,
  );
}
