// Build gate: fails if the llms surface misses a page. Run after
// `astro build` (`pnpm verify`, the nix `docs-site` derivation, and CI).
//
// The division of labor: `starlight-llms-txt` owns the three aggregate
// bundles (`llms.txt` index, `llms-full.txt`, `llms-small.txt`); the
// first-party route `src/pages/[...page].md.ts` owns the per-page `.md`
// twins. This script checks, against `dist/` and the sources:
//   1. All three bundles exist, and `llms.txt` links both bundle files
//      (catches a `site`/`base` mismatch: the plugin builds bundle URLs
//      from Astro's `base`, so a subpath `site` with the default base
//      emits domain-root links — see astro.config.mjs).
//   2. Every content page (src/content/docs/**/*.md, nested sections
//      included) appears in `llms-full.txt` AND `llms-small.txt`, by its
//      frontmatter title.
//   3. Every page's `.md` twin exists in `dist/` at its nested path
//      (`scoot/keybindings` → `dist/scoot/keybindings.md`) and is
//      non-empty.
//   4. Every `.md` / `.txt` link in every source page resolves: a
//      page-relative link (`./x.md`, `../scoot/y.md`) to a source page in
//      the tree (so the twin it points at exists), a root-absolute link
//      (`/llms-full.txt`) to a file in `dist/`. Source-level checking is
//      strictly stronger than scanning the bundles, whose prose is copied
//      verbatim from the sources — every bundle link is checked at its
//      source, where the relative path has a known base. Absolute links
//      inside the built bundles are checked too, against `dist/`.
//   5. Every per-app set (`customSets`, served under `_llms-txt/`) matches
//      at least one page (no silent empty sets).
import {
  existsSync,
  readFileSync,
  readdirSync,
  statSync,
} from 'node:fs';
import { dirname, join, resolve } from 'node:path';

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

// Every page under the docs tree, nested sections included. The slug is
// the source path relative to the docs root, minus `.md`
// (`scoot/keybindings.md` → `scoot/keybindings`).
const walk = (dir) =>
  readdirSync(dir, { withFileTypes: true }).flatMap((entry) => {
    const path = join(dir, entry.name);
    if (entry.isDirectory()) return walk(path);
    return entry.name.endsWith('.md') ? [path] : [];
  });

const pages = walk(contentDir).map((file) => {
  const text = readFileSync(file, 'utf8');
  const frontmatter = text.match(/^---\n([\s\S]*?)\n---\n/);
  const title =
    frontmatter?.[1].match(/^title:\s*(.+)$/m)?.[1].trim() ?? file;
  const slug = file
    .replace(`${contentDir}/`, '')
    .replace(/\.md$/, '');
  return { file, slug, title, text };
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

// Source-level link check: every `.md` / `.txt` link in every page must
// resolve — page-relative links against the page's own directory (so the
// linked twin exists), root-absolute links against `dist/`. External
// URLs and pure `#anchor` links are skipped.
const isExternal = (target) => /^[a-z][a-z0-9+.-]*:/i.test(target);

for (const page of pages) {
  const dir = dirname(page.file);
  for (const match of page.text.matchAll(/\]\(([^)]+)\)/g)) {
    const raw = match[1].split('#')[0].trim();
    if (!raw || isExternal(raw) || raw.startsWith('#')) continue;
    const base = raw.split('?')[0];
    if (!base.endsWith('.md') && !base.endsWith('.txt')) continue;
    if (base.startsWith('/')) {
      if (!existsSync(join(dist, base))) {
        fail(`${page.slug}: links ${raw}, which has no file in dist/`);
      }
    } else {
      const resolved = resolve(dir, base);
      if (!existsSync(resolved)) {
        fail(`${page.slug}: links ${match[1]}, which is no page in the tree`);
      }
    }
  }
}

// Absolute `.md` / `.txt` links inside the built bundles and sets must
// resolve to a file in `dist/` (page-relative prose links are covered at
// their source, above, where the base is known).
const checkDistLinks = (name, body) => {
  for (const match of body.matchAll(/\]\((\/[^)]+)\)/g)) {
    const target = match[1].split('#')[0];
    if (target.endsWith('.md') || target.endsWith('.txt')) {
      if (!existsSync(join(dist, target))) {
        fail(`${name} links ${target}, which has no file in dist/`);
      }
    }
  }
};

for (const [name, body] of [
  ['llms.txt', llms],
  ['llms-full.txt', full],
  ['llms-small.txt', small],
]) {
  if (body) checkDistLinks(name, body);
}

if (existsSync(join(dist, '_llms-txt'))) {
  for (const file of readdirSync(join(dist, '_llms-txt'))) {
    if (!file.endsWith('.txt')) continue;
    const setBody = readFileSync(join(dist, '_llms-txt', file), 'utf8');
    checkDistLinks(`_llms-txt/${file}`, setBody);
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
