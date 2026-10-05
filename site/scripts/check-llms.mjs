// Build gate: fails if the llms surface misses a page. Run after
// `astro build` (`pnpm verify`, the nix `docs-site` derivation, and CI).
//
// The division of labor: `starlight-llms-txt` owns the three aggregate
// bundles (`llms.txt` index, `llms-full.txt`, `llms-small.txt`); the
// first-party route `src/pages/[page].md.ts` owns the per-page `.md`
// twins. This script checks, against `dist/`:
//   1. All three bundles exist, and `llms.txt` links both bundle files
//      (catches a `site`/`base` mismatch: the plugin builds bundle URLs
//      from Astro's `base`, so a subpath `site` with the default base
//      emits domain-root links — see astro.config.mjs).
//   2. Every content page (src/content/docs/*.md, top-level) appears in
//      `llms-full.txt` AND `llms-small.txt`, by its frontmatter title.
//   3. Every page's `.md` twin exists in `dist/` and is non-empty.
//   4. Every root-relative (`/…`) and page-relative (`./…`) `.md` /
//      `.txt` link inside every bundle and per-app set resolves to a
//      file in `dist/` (no dangling bundle links), and every per-app
//      set matches at least one page (no silent empty sets). The index
//      `llms.txt` carries absolute bundle URLs while the full/small
//      bundles (and the `_llms-txt/` sets) preserve the prose's
//      source-relative `./x.md` links — so all of them are scanned.
import { existsSync, readFileSync, readdirSync, statSync } from 'node:fs';
import { join } from 'node:path';

const root = new URL('..', import.meta.url).pathname;
const dist = join(root, 'dist');
const contentDir = join(root, 'src', 'content', 'docs');

const failures = [];
const fail = (message) => failures.push(message);

const bundles = ['llms.txt', 'llms-full.txt', 'llms-small.txt'];
for (const bundle of bundles) {
  if (!existsSync(join(dist, bundle))) {
    fail(`missing bundle: dist/${bundle}`);
  }
}

const read = (file) =>
  existsSync(join(dist, file)) ? readFileSync(join(dist, file), 'utf8') : '';

const llms = read('llms.txt');
const full = read('llms-full.txt');
const small = read('llms-small.txt');

for (const bundle of ['llms-full.txt', 'llms-small.txt']) {
  if (llms && !llms.includes(`/${bundle}`)) {
    fail(`llms.txt does not link /${bundle}`);
  }
}

const pages = readdirSync(contentDir)
  .filter((file) => file.endsWith('.md'))
  .map((file) => {
    const text = readFileSync(join(contentDir, file), 'utf8');
    const frontmatter = text.match(/^---\n([\s\S]*?)\n---\n/);
    const title =
      frontmatter?.[1].match(/^title:\s*(.+)$/m)?.[1].trim() ?? file;
    return { slug: file.replace(/\.md$/, ''), title };
  });

for (const page of pages) {
  for (const [name, body] of [
    ['llms-full.txt', full],
    ['llms-small.txt', small],
  ]) {
    if (body && !body.includes(`# ${page.title}`)) {
      fail(`${name} misses page "${page.title}" (${page.slug})`);
    }
  }
  const twinFile = join(dist, `${page.slug}.md`);
  if (!existsSync(twinFile)) {
    fail(`missing per-page twin: dist/${page.slug}.md`);
  } else if (statSync(twinFile).size === 0) {
    fail(`empty per-page twin: dist/${page.slug}.md`);
  }
}

// Page prose links land in the bundles verbatim: the index `llms.txt`
// carries only absolute bundle URLs, while `llms-full.txt` /
// `llms-small.txt` preserve the sources' root-relative (`/x.md`) and
// page-relative (`./x.md`) links — so every bundle's `.md` / `.txt`
// links must resolve to a file in `dist/`. Every page is top-level, so
// `./x.md` must be the twin `dist/x.md`. The per-app sets
// (`customSets`, served under `_llms-txt/`) carry the same prose, so
// they get the same rules.
const checkLinks = (name, body) => {
  for (const match of body.matchAll(/\]\((\/[^)]+)\)/g)) {
    const target = match[1].split('#')[0];
    if (target.endsWith('.md') || target.endsWith('.txt')) {
      if (!existsSync(join(dist, target))) {
        fail(`${name} links ${target}, which has no file in dist/`);
      }
    }
  }
  for (const match of body.matchAll(/\]\((\.[^)]+)\)/g)) {
    const target = match[1].split('#')[0];
    if (target.endsWith('.md') || target.endsWith('.txt')) {
      const resolved = target.replace(/^\.\//, '');
      if (!existsSync(join(dist, resolved))) {
        fail(`${name} links ${match[1]}, which has no file in dist/`);
      }
    }
  }
};

for (const [name, body] of [
  ['llms.txt', llms],
  ['llms-full.txt', full],
  ['llms-small.txt', small],
]) {
  if (body) checkLinks(name, body);
}

if (existsSync(join(dist, '_llms-txt'))) {
  for (const file of readdirSync(join(dist, '_llms-txt'))) {
    if (!file.endsWith('.txt')) continue;
    const setBody = readFileSync(join(dist, '_llms-txt', file), 'utf8');
    checkLinks(`_llms-txt/${file}`, setBody);
    // A set that matches no pages is a silent lie (wrong `paths`):
    // every set file must carry at least one page heading.
    if (!setBody.match(/^# .+/m)) {
      fail(`_llms-txt/${file} matches no pages (check customSets paths)`);
    }
  }
}

if (failures.length > 0) {
  console.error('check-llms: FAIL');
  for (const failure of failures) console.error(`  - ${failure}`);
  process.exit(1);
}
console.log(
  `check-llms: ok (${pages.length} pages in both bundles with twins: ${pages.map((page) => page.slug).join(', ')})`,
);
