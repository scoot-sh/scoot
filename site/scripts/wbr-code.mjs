// Post-build `<wbr>` pass: let long dotted tokens in inline code break
// after their dots. Plus the `.mtable-card` label pass: tag the 4-5 column
// reference tables so narrow-screen CSS can stack each row as a card.
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
// The `<wbr>` splice applies to every inline `<code>` span outside the
// opaque regions, not just tables: 495 spans in `td`, 449 in `li`, 326 in
// paragraphs and other prose, 10 in links (measured on this tree). That
// breadth is harmless — `wbr` is `display: none` above the narrow-screen
// breakpoint and only a preferred break under constraint below it — but
// the comment says so plainly after a review caught it claiming
// table-only.
//
// Why post-build and not a remark/rehype plugin (same reason as
// `md-links.mjs`, whose header documents it): `starlight-llms-txt`
// renders each page through the same markdown pipeline
// (`render(entry)` → HTML → Markdown), so any pipeline rewrite would leak
// literal `<wbr>`s (and `data-label`/`role` attributes) into the bundles,
// where agents read them as text.
// (Astro 7.3.5's `markdown.rehypePlugins` additionally never runs without
// the uninstalled `@astrojs/markdown-remark` legacy package.) Rewriting
// only `dist/**/*.html` after the build keeps the split exact: the `.md`
// twins and the llms.txt bundles are byte-identical with or without these
// passes (`check-llms` pins that).
//
// Safety properties:
// - `<wbr>` contributes no characters: copy-paste, pagefind text, screen
//   readers and heading ids/slugs (all computed at build time, before
//   this pass) are unchanged. Only HTML line-breaking changes, and only
//   under constraint — with room to spare no break is taken, so desktop
//   rendering is unchanged.
// - `data-label`/`role` on card tables likewise contribute no visible
//   text: labels repeat the column headings (already indexed once), and
//   above the breakpoint the attributes are inert (no CSS reads them
//   there), so desktop layout and the search index are unchanged.
// - Fenced code blocks (`<pre>`), scripts, styles and comments are opaque
//   to both passes: snippet contents stay byte-identical for
//   `test-snippets`, and no `<wbr>` can land inside a copyable block.
// - A `<code>` span containing nested markup is left alone (the inner
//   pattern requires pure text), so future Starlight markup inside code
//   fails safe instead of being spliced mid-tag.
// - A table that already carries `mtable-card` (re-run), has no `<thead>`
//   headings, has a heading count unlike its columns, holds a `<kbd>`
//   key grid, or has other than 4-5 columns is left alone: it keeps the
//   scrolling table treatment from `custom.css` instead of cards.
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

// Card tables: 4-5 column reference tables become stacked cards below the
// breakpoint (see the `mtable-card` rules in `custom.css`). Each body cell
// needs its column heading as a `data-label` (the card's row label via
// `td::before`), and the table keeps explicit `table`/`row`/`cell` roles
// so the accessibility tree still exposes table semantics once the CSS
// restacks the elements as blocks (verified with CDP
// Accessibility.getFullAXTree: table, row, columnheader and cell nodes
// all present). Key-grid (`<kbd>`) matrices stay scrolling tables.
const TABLE_BLOCK = /<table\b[^>]*>[\s\S]*?<\/table\s*>/gi;
const THEAD_BLOCK = /<thead\b[^>]*>([\s\S]*?)<\/thead\s*>/i;
const TH_CELL = /<th\b[^>]*>([\s\S]*?)<\/th\s*>/gi;
const FIRST_ROW = /<tr\b[^>]*>([\s\S]*?)<\/tr\s*>/i;
const CELL_TAG = /<(th|td)\b[^>]*>/gi;
const TAG_STRIP = /<[^>]*>/g;

const escAttr = (s) => s.replace(/&/g, '&amp;').replace(/"/g, '&quot;');
// Headings arrive as HTML (`<code>`, entities); the label is plain text,
// decoded first so `escAttr` cannot double-escape an existing entity.
const headText = (h) =>
  h
    .replace(TAG_STRIP, '')
    .replace(/&quot;/g, '"')
    .replace(/&#39;/g, "'")
    .replace(/&lt;/g, '<')
    .replace(/&gt;/g, '>')
    .replace(/&amp;/g, '&')
    .replace(/\s+/g, ' ')
    .trim();

function labelOneTable(table) {
  if (/\bmtable-card\b/.test(table)) return { table, count: 0 };
  if (/<kbd\b/i.test(table)) return { table, count: 0 };
  const thead = table.match(THEAD_BLOCK);
  if (!thead) return { table, count: 0 };
  const headers = [...thead[1].matchAll(TH_CELL)].map((m) => headText(m[1]));
  const firstRow = table.match(FIRST_ROW);
  if (!firstRow) return { table, count: 0 };
  const ncols = [...firstRow[1].matchAll(CELL_TAG)].length;
  if ((ncols !== 4 && ncols !== 5) || headers.length !== ncols) {
    return { table, count: 0 };
  }
  let out = table.replace(/<table\b/, '<table class="mtable-card" role="table"');
  out = out.replace(/<tr\b/g, '<tr role="row"');
  out = out.replace(/<th\b/g, '<th role="columnheader"');
  let cells = 0;
  out = out.replace(/<tbody\b[^>]*>([\s\S]*?)<\/tbody\s*>/gi, (m, body) => {
    const labelled = body.replace(/<tr\b[^>]*>[\s\S]*?<\/tr\s*>/gi, (row) => {
      let idx = 0;
      return row.replace(/<td\b/g, () => {
        const label = escAttr(headers[idx] ?? '');
        idx += 1;
        cells += 1;
        return `<td data-label="${label}" role="cell"`;
      });
    });
    return m.replace(body, labelled);
  });
  return { table: out, count: cells > 0 ? 1 : 0 };
}

// Pure rewrite of one HTML string. Exported for tests.
export function insertCardLabels(html) {
  let count = 0;
  const parts = html.split(PROTECTED);
  const seps = html.match(PROTECTED) || [];
  const out = parts.map((seg) =>
    seg.replace(TABLE_BLOCK, (table) => {
      const { table: next, count: c } = labelOneTable(table);
      count += c;
      return next;
    }),
  );
  let merged = '';
  for (let i = 0; i < out.length; i++) {
    merged += out[i];
    if (i < seps.length) merged += seps[i];
  }
  return { html: merged, count };
}

export function tableCardLabels() {
  return {
    name: 'scoot-table-card-labels',
    hooks: {
      'astro:build:done': async ({ dir, logger }) => {
        const distRoot = fileURLToPath(dir);
        let files = 0;
        let tables = 0;
        for (const file of walkHtml(distRoot)) {
          const html = readFileSync(file, 'utf8');
          const { html: next, count } = insertCardLabels(html);
          if (count > 0) {
            files += 1;
            tables += count;
            writeFileSync(file, next);
          }
        }
        logger.info(`labelled ${tables} card tables in ${files} HTML files`);
      },
    },
  };
}
