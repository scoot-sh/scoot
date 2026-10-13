// Post-build `<wbr>` pass: let long dotted tokens in inline code break
// after their dots.
//
// The docs are full of unbreakable dotted tokens in inline code — option
// paths (`desktop.notifications.bar.icons.unread`), version triples,
// file paths. Dots are not line-break opportunities, so on a phone such a
// token behaves like one 40-character word: it either overflows its table
// column (crushing the neighbouring columns into a sliver — the
// mobile-tables bug) or, once wrapping is allowed, breaks mid-word at a
// greedy fill point (`desktop.notifi|cations.enable`). A `<wbr>` after
// each intra-token dot turns the dots into *preferred* break
// opportunities, so names break at their segments; the emergency
// `overflow-wrap: anywhere` rule in `custom.css` then only fires for a
// segment still longer than the line.
//
// Why post-build and not a remark/rehype plugin (same reason as
// `md-links.mjs`, whose header documents it): `starlight-llms-txt`
// renders each page through the same markdown pipeline
// (`render(entry)` → HTML → Markdown), so any pipeline rewrite would leak
// literal `<wbr>`s into the bundles, where agents read them as text.
// (Astro 7.3.5's `markdown.rehypePlugins` additionally never runs without
// the uninstalled `@astrojs/markdown-remark` legacy package.) Rewriting
// only `dist/**/*.html` after the build keeps the split exact: the `.md`
// twins and the llms.txt bundles are byte-identical with or without this
// pass (`check-llms` pins that).
//
// Safety properties:
// - `<wbr>` contributes no characters: copy-paste, pagefind text, screen
//   readers and heading ids/slugs (all computed at build time, before
//   this pass) are unchanged. Only HTML line-breaking changes, and only
//   under constraint — with room to spare no break is taken, so desktop
//   rendering is unchanged.
// - Fenced code blocks (`<pre>`), scripts, styles and comments are opaque
//   to the pass: snippet contents stay byte-identical for
//   `test-snippets`, and no `<wbr>` can land inside a copyable block.
// - A `<code>` span containing nested markup is left alone (the inner
//   pattern requires pure text), so future Starlight markup inside code
//   fails safe instead of being spliced mid-tag.
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { readdirSync, readFileSync, writeFileSync } from 'node:fs';

// Opaque regions: never splice inside these.
const PROTECTED =
  /<!--[\s\S]*?-->|<script\b[^>]*>[\s\S]*?<\/script\s*>|<style\b[^>]*>[\s\S]*?<\/style\s*>|<pre\b[^>]*>[\s\S]*?<\/pre\s*>/gi;

// An inline code span with pure-text contents. Nested markup inside the
// span simply does not match and is left alone.
const CODE_SPAN = /<code\b([^>]*)>([^<]*)<\/code>/g;

// A dot followed by a token character: not a leading, trailing, doubled
// (`...`) or sentence (`e. `) dot.
const DOT_BOUNDARY = /(?<=\.)(?=[^.\s])/g;

// Pure rewrite of one HTML string. Exported for tests.
export function insertWbr(html) {
  let count = 0;
  const parts = html.split(PROTECTED);
  // `split` with a global regex drops the separators; recover them.
  const seps = html.match(PROTECTED) || [];
  const out = parts.map((seg) =>
    seg.replace(CODE_SPAN, (match, attrs, text) => {
      DOT_BOUNDARY.lastIndex = 0;
      if (!DOT_BOUNDARY.test(text)) return match;
      count += 1;
      // Zero-width match: insertion only, the dot itself is untouched.
      return `<code${attrs}>${text.replace(DOT_BOUNDARY, '<wbr>')}</code>`;
    }),
  );
  let merged = '';
  for (let i = 0; i < out.length; i++) {
    merged += out[i];
    if (i < seps.length) merged += seps[i];
  }
  return { html: merged, count };
}

const walkHtml = (dir) =>
  readdirSync(dir, { withFileTypes: true }).flatMap((entry) => {
    const path = join(dir, entry.name);
    if (entry.isDirectory()) return walkHtml(path);
    return entry.name.endsWith('.html') ? [path] : [];
  });

export function wbrDottedCode() {
  return {
    name: 'scoot-wbr-dotted-code',
    hooks: {
      'astro:build:done': async ({ dir, logger }) => {
        const distRoot = fileURLToPath(dir);
        let files = 0;
        let spans = 0;
        for (const file of walkHtml(distRoot)) {
          const html = readFileSync(file, 'utf8');
          const { html: next, count } = insertWbr(html);
          if (count > 0) {
            files += 1;
            spans += count;
            writeFileSync(file, next);
          }
        }
        logger.info(`inserted <wbr> into ${spans} code spans in ${files} HTML files`);
      },
    },
  };
}
