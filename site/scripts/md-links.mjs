// Post-build rewrite: relative `.md` links in built HTML pages point at
// pages, not at the raw `.md` twins.
//
// Sources link pages as relative `.md` files (`./start/install.md`), which
// reads in editors, on GitHub, and in the raw twins. Left alone, the built
// HTML would link the twin instead of the page, so after `astro build` this
// Astro integration (`astro:build:done`) rewrites those hrefs in `*.html`
// only:
//
//   `foo/bar.md` → `foo/bar/`, `foo/index.md` → `foo/`,
//
// anchors (`#…`, including the `##…` form found in several pages) and query
// strings carried over verbatim. The rewrite is computed from the linking
// page's slug to the target's, as a relative URL, so it stays correct
// under any configured `base` (including the project-pages `/scoot/` one)
// and for self-links (`./modules.md#x` on `modules` stays on the page
// instead of pointing at a `modules/modules/` subpath, which a plain
// `.md` → `/` string swap would produce).
//
// Why post-build and not a remark/rehype plugin: the raw twins
// (`src/pages/[...page].md.ts`, served from the `?raw` sources) bypass
// markdown rendering and keep their `.md` links either way — but the
// llms.txt bundles do not. `starlight-llms-txt` renders each page through
// the same markdown pipeline (`render(entry)` → HTML → Markdown), so any
// remark/rehype rewrite would leak into the bundles, where agents should
// keep following Markdown (`.md`) links. Rewriting only `dist/**/*.html`
// after the build keeps the split exact. (Astro 7.3.5's deprecated
// `markdown.remarkPlugins` additionally never runs with Starlight in play;
// the supported `processor: unified(…)` form does run — and contaminates
// the bundles. Verified against the pinned sources; do not "simplify" this
// back into a remark plugin without re-checking `dist/llms-full.txt`.)
//
// Untouched: absolute URLs (`https:`, `mailto:`, …), root-absolute links,
// pure `#anchor` links, non-`.md` assets, and links escaping the docs tree
// (left alone so the `check-md-links` gate fails loudly instead of
// emitting a silently broken page link).
import { posix, relative, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { existsSync, readdirSync, readFileSync, writeFileSync } from 'node:fs';

const docsRoot = fileURLToPath(new URL('../src/content/docs/', import.meta.url));

const pageOf = (slug) =>
  slug === 'index' || slug.endsWith('/index') ? slug.replace(/\/?index$/, '') : slug;

// Pure rewrite of one link URL given the linking page. Exported for tests.
export function rewriteMdLink(url, sourceDir, currentPage, slug) {
  if (!url || url.startsWith('#') || url.startsWith('/') || url.startsWith('//')) return url;
  if (/^[a-z][a-z0-9+.-]*:/i.test(url)) return url;
  const cut = url.search(/[?#]/);
  const pathPart = cut === -1 ? url : url.slice(0, cut);
  const suffix = cut === -1 ? '' : url.slice(cut);
  if (!pathPart.endsWith('.md')) return url;
  const target = posix.normalize(posix.join(sourceDir, pathPart)).replace(/\.md$/, '');
  if (target === '..' || target.startsWith('../')) return url;
  const targetPage = pageOf(target);
  if (targetPage === currentPage) return `./${suffix}`;
  // Starlight serves the `404` slug as a root-level `404.html` file,
  // not a directory: relative links from it resolve against `/`.
  const baseDir = slug === '404' ? '' : currentPage;
  const href = posix.relative(baseDir === '' ? '.' : baseDir, targetPage);
  if (href === '') return `./${suffix}`;
  return `${href.startsWith('.') ? href : `./${href}`}/${suffix}`;
}

// The linking page's (sourceDir, currentPage, slug) for one built HTML
// file, resolved through the actual sources (a `dist/<p>/index.html` page
// comes from `src/content/docs/<p>.md` or `<p>/index.md`, whichever exists).
function contextForHtml(distFile, distRoot, root = docsRoot) {
  const relPath = relative(distRoot, distFile).replace(/\\/g, '/');
  let source;
  if (relPath === '404.html') {
    source = join(root, '404.md');
  } else if (relPath === 'index.html') {
    source = join(root, 'index.md');
  } else if (relPath.endsWith('/index.html')) {
    const page = relPath.slice(0, -'/index.html'.length);
    const direct = join(root, `${page}.md`);
    source = existsSync(direct) ? direct : join(root, page, 'index.md');
  } else {
    return null;
  }
  if (!existsSync(source)) return null;
  const slug = relative(root, source).replace(/\\/g, '/').replace(/\.md$/, '');
  return { sourceDir: posix.dirname(slug), currentPage: pageOf(slug), slug };
}

// Rewrite one href value. Links whose target is no page in the tree are
// left alone so `check-md-links` fails loudly on them.
function rewriteHref(raw, context, root = docsRoot) {
  const next = rewriteMdLink(raw, context.sourceDir, context.currentPage, context.slug);
  if (next === raw) return raw;
  const cut = raw.search(/[?#]/);
  const pathPart = cut === -1 ? raw : raw.slice(0, cut);
  const target = posix
    .normalize(posix.join(context.sourceDir, pathPart))
    .replace(/\.md$/, '');
  const candidate = existsSync(join(root, `${target}.md`))
    ? target
    : target === 'index' || target.endsWith('/index')
      ? null
      : `${target}/index`;
  if (candidate === null || !existsSync(join(root, `${candidate}.md`))) return raw;
  return next;
}

const walkHtml = (dir) =>
  readdirSync(dir, { withFileTypes: true }).flatMap((entry) => {
    const path = join(dir, entry.name);
    if (entry.isDirectory()) return walkHtml(path);
    return entry.name.endsWith('.html') ? [path] : [];
  });

export function mdLinksToPages() {
  return {
    name: 'scoot-md-links-to-pages',
    hooks: {
      'astro:build:done': async ({ dir, logger }) => {
        const distRoot = fileURLToPath(dir);
        let files = 0;
        let rewritten = 0;
        for (const file of walkHtml(distRoot)) {
          const context = contextForHtml(file, distRoot);
          if (!context) continue;
          const html = readFileSync(file, 'utf8');
          let changed = false;
          const next = html.replace(/href="([^"]*)"/g, (match, raw) => {
            const fixed = rewriteHref(raw, context);
            if (fixed !== raw) {
              changed = true;
              rewritten += 1;
              return `href="${fixed}"`;
            }
            return match;
          });
          if (changed) {
            files += 1;
            writeFileSync(file, next);
          }
        }
        logger.info(`rewrote ${rewritten} .md links to pages in ${files} HTML files`);
      },
    },
  };
}
