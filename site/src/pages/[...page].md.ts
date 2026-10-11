import type { APIRoute, GetStaticPaths } from 'astro';

// Per-page Markdown twins at stable `/<slug>.md` URLs (the home page at
// `/index.md`, a nested page like `scoot/keybindings` at
// `/scoot/keybindings.md`). `starlight-llms-txt` covers the three
// aggregate bundles; this route covers the per-page requirement from the
// docs-site ticket.
//
// Served from the Markdown sources (bundled raw at build time via
// `import.meta.glob`, so this works prerendered with no filesystem), as
// the page's own prose: title heading, description quote, body without
// frontmatter. `.mdx` pages convert through `scripts/mdx-twin.mjs` (M1):
// imports stripped, TabItem/Aside/Steps/LinkCard flattened to plain
// Markdown, fenced code untouched — so agents never see JSX.
import { mdxToTwinMarkdown } from '../../scripts/mdx-twin.mjs';

const sources = import.meta.glob<string>(
  [
    '../content/docs/**/*.{md,mdx}',
    // Mirror Starlight's docsLoader (`**/[^_]*`, pinned source
    // `dist/loaders.js`): underscore-led files are drafts, never pages —
    // and never twins, so the twin set stays exactly the page set.
    '!../content/docs/**/_*.{md,mdx}',
  ],
  {
    query: '?raw',
    import: 'default',
    eager: true,
  },
);

const slugOf = (path: string) =>
  path.replace(/^.*\/content\/docs\//, '').replace(/\.mdx?$/, '');

const twinBody = (raw: string, slug: string, isMdx: boolean) => {
  const frontmatter = raw.match(/^---\n([\s\S]*?)\n---\n/);
  const title =
    frontmatter?.[1].match(/^title:\s*(.+)$/m)?.[1].trim() ?? slug;
  const description =
    frontmatter?.[1].match(/^description:\s*(.+)$/m)?.[1].trim() ?? '';
  // Twins read as plain Markdown (the docs-bar agent-friendly rule), so
  // `<kbd>` keycaps become backtick-quoted keys rather than literal tags:
  // `<kbd>Super</kbd>+<kbd>Return</kbd>` reads as `` `Super`+`Return` ``.
  const plain = raw.replace(/^---\n[\s\S]*?\n---\n/, '').trim();
  const body = (isMdx ? mdxToTwinMarkdown(plain) : plain).replaceAll(
    /<kbd>(.*?)<\/kbd>/g,
    '`$1`',
  );
  return `# ${title}\n\n${description}\n\n${body}\n`;
};

export const getStaticPaths = (async () =>
  Object.keys(sources).map((path) => ({ params: { page: slugOf(path) } }))
) satisfies GetStaticPaths;

export const GET: APIRoute = async ({ params }) => {
  const path = Object.keys(sources).find(
    (candidate) => slugOf(candidate) === params.page,
  );
  if (!path) {
    return new Response('Not found', { status: 404 });
  }
  return new Response(
    twinBody(sources[path], params.page ?? 'page', path.endsWith('.mdx')),
    {
      headers: { 'Content-Type': 'text/markdown; charset=utf-8' },
    },
  );
};
