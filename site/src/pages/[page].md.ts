import type { APIRoute, GetStaticPaths } from 'astro';

// Per-page Markdown twins at stable `/<slug>.md` URLs (the home page at
// `/index.md`). `starlight-llms-txt` covers the three aggregate bundles;
// this route covers the per-page requirement from the docs-site ticket.
// Only top-level pages are served: nested sections gain their own twins
// when they land (extend the glob, add the check — check-llms.mjs
// enforces both).
//
// Served from the Markdown sources (bundled raw at build time via
// `import.meta.glob`, so this works prerendered with no filesystem), as
// the page's own prose: title heading, description quote, body without
// frontmatter.
const sources = import.meta.glob<string>('../content/docs/*.md', {
  query: '?raw',
  import: 'default',
  eager: true,
});

const slugOf = (path: string) =>
  path.split('/').pop()?.replace(/\.md$/, '') ?? '';

const twinBody = (raw: string, slug: string) => {
  const frontmatter = raw.match(/^---\n([\s\S]*?)\n---\n/);
  const title =
    frontmatter?.[1].match(/^title:\s*(.+)$/m)?.[1].trim() ?? slug;
  const description =
    frontmatter?.[1].match(/^description:\s*(.+)$/m)?.[1].trim() ?? '';
  const body = raw.replace(/^---\n[\s\S]*?\n---\n/, '').trim();
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
  return new Response(twinBody(sources[path], params.page ?? 'page'), {
    headers: { 'Content-Type': 'text/markdown; charset=utf-8' },
  });
};
