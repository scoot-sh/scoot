// Build gate: every asset a built page or stylesheet points at must exist
// in `dist/`. Run after `astro build` (`pnpm verify`, the nix `docs-site`
// derivation, and CI).
//
// Background: Vite leaves a CSS `url()` untouched when its file is absent
// and the build still exits 0. During PR #466 a new font file was
// referenced before it was staged, so the build shipped an `@font-face`
// that would 404 while every gate passed. This gate closes that hole:
//   1. Every CSS `url()` in `dist/**/*.css` (plus `<style>` blocks and
//      `style=""` attributes in `dist/**/*.html`, which are stylesheet
//      content too) must resolve to a file in `dist/`.
//   2. Every HTML `src`, `href` and `srcset` in `dist/**/*.html` that names
//      an asset must resolve to a file in `dist/`.
//   3. Resolution is base-aware: root-absolute URLs are resolved under the
//      base the site was built with (inferred from the `/_astro/` prefix
//      in the built pages, so this stays correct under the project-pages
//      `/scoot/` base as well as `/`); page-relative URLs resolve against
//      the linking file's own directory, CSS-relative URLs against the
//      stylesheet's.
//   4. Scope is assets only: images, fonts, CSS/JS, icons (see ASSET_EXTS).
//      Intentional `.md` twin links are excluded (the llms gate already
//      covers those; a naive version of this check false-positived on 202
//      of them in the #466 review), as are `.txt` bundles, feeds, page
//      links (extensionless or trailing-`/`), and external/data URLs.
//      External links are out of scope for this ticket (a link checker,
//      with network).
//
// Only `src`/`href`/`srcset` attributes and CSS `url()` are scanned:
// `content=""` metadata (e.g. `og:image`, absolute here) and `url()`
// inside `<script>` blocks are deliberately not (the former is out of the
// ticket's mechanism, the latter is code, not a reference).
import { existsSync, readFileSync, readdirSync, statSync } from 'node:fs';
import { dirname, extname, join, resolve, sep } from 'node:path';

const root = new URL('..', import.meta.url).pathname;
const defaultDist = join(root, 'dist');

// Images, fonts, CSS/JS, icons — the ticket's scope. Everything else
// (`.md` twins, `.txt` bundles, feeds, extensionless page links) is
// another gate's or out of scope, and is skipped, never failed.
const ASSET_EXTS = new Set([
  '.png',
  '.jpg',
  '.jpeg',
  '.webp',
  '.avif',
  '.gif',
  '.svg',
  '.ico',
  '.bmp',
  '.tif',
  '.tiff',
  '.woff',
  '.woff2',
  '.ttf',
  '.otf',
  '.eot',
  '.css',
  '.js',
  '.mjs',
]);

const isExternal = (target) => /^[a-z][a-z0-9+.-]*:/i.test(target);

const stripSuffix = (url) => url.split('#')[0].split('?')[0];

const walk = (dir, suffix) =>
  readdirSync(dir, { withFileTypes: true }).flatMap((entry) => {
    const path = join(dir, entry.name);
    if (entry.isDirectory()) return walk(path, suffix);
    return entry.name.endsWith(suffix) ? [path] : [];
  });

// One CSS `url(...)` reference per match: quoted (`"…"`, `'…`) or bare.
// `data:` and other schemes are returned and skipped by the caller.
function cssUrls(css) {
  const found = [];
  const pattern = /url\(\s*(?:"([^"]*)"|'([^']*)'|([^)"'\s][^)]*?))\s*\)/g;
  for (const match of css.matchAll(pattern)) {
    found.push(match[1] ?? match[2] ?? match[3].trim());
  }
  return found;
}

// Every asset URL named by one HTML file: `src`/`href` attributes,
// `srcset` candidates (first token of each comma-separated entry), and
// `url()` inside `<style>` blocks and `style=""` attributes (resolved
// against this file's directory, like a stylesheet beside it). Inline
// `<script>` bodies are stripped first: they are code, and a JS string
// like `img.src="…"` is not a reference (only the `<script src>`
// tag's own attributes are kept).
function htmlUrls(html) {
  const noscript = html.replace(
    /<script[^>]*>[\s\S]*?<\/script>/g,
    (block) => block.match(/^<script[^>]*>/)[0],
  );
  const found = [];
  for (const match of noscript.matchAll(/(?:src|href)\s*=\s*"([^"]*)"/g)) {
    found.push(match[1]);
  }
  for (const match of noscript.matchAll(/srcset\s*=\s*"([^"]*)"/g)) {
    for (const candidate of match[1].split(',')) {
      const url = candidate.trim().split(/\s+/)[0];
      if (url) found.push(url);
    }
  }
  for (const match of noscript.matchAll(/<style[^>]*>([\s\S]*?)<\/style>/g)) {
    found.push(...cssUrls(match[1]));
  }
  for (const match of noscript.matchAll(/style\s*=\s*"([^"]*)"/g)) {
    found.push(...cssUrls(match[1]));
  }
  return found;
}

function isAsset(url) {
  if (!url || url.startsWith('#')) return false;
  if (url.startsWith('//')) return false;
  if (isExternal(url)) return false;
  return ASSET_EXTS.has(extname(stripSuffix(url)).toLowerCase());
}

// The base the site was built with, inferred from the built pages: the
// portion before `/_astro/` in root-absolute asset URLs (`''` under `/`,
// `'/scoot'` under the project-pages `/scoot/` base). Majority vote across
// every such URL, so one odd link cannot move it.
function inferBase(absolutePaths) {
  const votes = new Map();
  for (const path of absolutePaths) {
    const at = path.indexOf('/_astro/');
    if (at === -1) continue;
    const base = path.slice(0, at);
    votes.set(base, (votes.get(base) ?? 0) + 1);
  }
  let best = '';
  let bestCount = 0;
  for (const [base, count] of votes) {
    if (count > bestCount) {
      best = base;
      bestCount = count;
    }
  }
  return best;
}

export function checkAssets(distDir = defaultDist) {
  const failures = [];
  const checked = [];

  if (!existsSync(distDir)) {
    return {
      checked: 0,
      missing: [
        {
          file: '(no dist)',
          url: '(no dist)',
          resolved: distDir,
          note: 'no dist/ directory (run `astro build` first)',
        },
      ],
    };
  }

  const htmlFiles = walk(distDir, '.html');
  const cssFiles = walk(distDir, '.css');

  // Pass one: collect every asset URL, so the base vote sees them all.
  const refs = [];
  for (const file of htmlFiles) {
    const html = readFileSync(file, 'utf8');
    for (const url of htmlUrls(html)) {
      if (isAsset(url)) refs.push({ file, dir: dirname(file), url });
    }
  }
  for (const file of cssFiles) {
    const css = readFileSync(file, 'utf8');
    for (const url of cssUrls(css)) {
      if (isAsset(url)) refs.push({ file, dir: dirname(file), url });
    }
  }

  const base = inferBase(
    refs.map((ref) => stripSuffix(ref.url)).filter((url) => url.startsWith('/')),
  );

  // Pass two: resolve each against dist/.
  for (const ref of refs) {
    const pathPart = stripSuffix(ref.url);
    let resolved;
    if (pathPart.startsWith('/')) {
      if (base !== '' && pathPart !== base && !pathPart.startsWith(`${base}/`)) {
        failures.push({
          ...ref,
          resolved: join(distDir, pathPart.slice(1)),
          note: `outside the site base '${base}'`,
        });
        continue;
      }
      const inner = base !== '' ? pathPart.slice(base.length + 1) : pathPart.slice(1);
      resolved = join(distDir, inner);
    } else {
      resolved = resolve(ref.dir, pathPart);
      const inside =
        resolved === distDir || resolved.startsWith(`${distDir}${sep}`);
      if (!inside) {
        failures.push({
          ...ref,
          resolved,
          note: 'escapes dist/',
        });
        continue;
      }
    }
    checked.push({ ...ref, resolved });
    if (!existsSync(resolved) || !statSync(resolved).isFile()) {
      failures.push({ ...ref, resolved });
    }
  }

  return { checked: checked.length, missing: failures, base };
}

const cliDist = (() => {
  const at = process.argv.indexOf('--dist');
  const raw = at === -1 ? defaultDist : process.argv[at + 1];
  return resolve(raw);
})();

// Imported by test-check-assets.mjs: only run the CLI on a direct run.
if (import.meta.url === `file://${process.argv[1]}`) {
  const { checked, missing, base } = checkAssets(cliDist);
  if (missing.length > 0) {
    console.error('check-assets: FAIL');
    for (const { file, url, resolved, note } of missing) {
      const rel = String(file).startsWith(`${cliDist}${sep}`)
        ? String(file).slice(cliDist.length + 1)
        : file;
      console.error(`  - ${rel}: ${url} -> no file at ${resolved}${note ? ` (${note})` : ''}`);
    }
    process.exit(1);
  }
  console.log(
    `check-assets: ok (${checked} asset references resolve under base '${base || '/'}')`,
  );
}
