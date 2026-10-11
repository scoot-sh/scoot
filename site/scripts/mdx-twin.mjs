// Shared MDX → plain-Markdown twin conversion (M1: MDX pages pass every
// gate). Imported by the per-page twin route
// (`src/pages/[...page].md.ts`) and pinned by `test-mdx-twin.mjs` over the
// fixtures in `scripts/fixtures/mdx-twin/` (which never publish: they live
// outside the content tree, so no sidebar entry or build output names
// them).
//
// The twin for an `.mdx` page must read as plain Markdown for agents, so
// every Starlight component becomes its Markdown equivalent and fenced
// code is left byte-for-byte untouched:
//
//   - `import … from '…'` lines are dropped (they would leak into the
//     twin as prose otherwise).
//   - `<Tabs syncKey="…">` / `</Tabs>` wrappers are dropped; each
//     `<TabItem label="X">` becomes a `### X` heading, `</TabItem>` is
//     dropped. The tab labels stay findable; the sync behavior is a
//     browser concern the twin does not need.
//   - `<Steps>` / `</Steps>` wrappers are dropped; the ordered list they
//     wrap stays as-is.
//   - `<Aside type="caution" title="T">…</Aside>` becomes a `> **Caution:**
//     T`-style blockquote (type defaults to `note`, capitalized); every
//     inner line gains a `> ` prefix.
//   - `<CardGrid>` wrappers are dropped; each `<LinkCard title="T"
//     href="H" description="D" />` becomes a `- [T](H) — D` link-list
//     entry (description omitted when absent).
//
// Anything else matching `<[A-Z]` (an unhandled component) is left in
// place on purpose so the gate below fails loudly instead of silently
// shipping JSX to agents. Prose generics such as `<N>` trip the same
// gate, so `.mdx` sources must backtick-quote them (`` `<N>` ``).
const FENCE_MARKER = /^[ \t]*(`{3,}|~{3,})/;

const attr = (attrs, name) => {
  const double = attrs.match(new RegExp(`${name}="([^"]*)"`));
  if (double) return double[1];
  const single = attrs.match(new RegExp(`${name}='([^']*)'`));
  return single ? single[1] : null;
};

// Split text into fenced / prose segments so the conversion below never
// touches fenced code (a TOML snippet quoting `<TabItem>` must survive).
export function splitFences(text) {
  const segments = [];
  let prose = [];
  let open = null;
  let fenced = null;
  for (const line of text.split('\n')) {
    const marker = line.match(FENCE_MARKER)?.[1] ?? null;
    if (open === null && marker !== null) {
      if (prose.length > 0) {
        segments.push({ fenced: false, text: `${prose.join('\n')}\n` });
        prose = [];
      }
      open = marker;
      fenced = line;
    } else if (open !== null) {
      fenced += `\n${line}`;
      if (marker !== null && marker[0] === open[0] && marker.length >= open.length) {
        segments.push({ fenced: true, text: `${fenced}\n` });
        open = null;
        fenced = null;
      }
    } else {
      prose.push(line);
    }
  }
  if (fenced !== null) segments.push({ fenced: true, text: fenced });
  if (prose.length > 0) segments.push({ fenced: false, text: prose.join('\n') });
  return segments;
}

const linkCardLine = (attrs) => {
  const title = attr(attrs, 'title') ?? 'Link';
  const href = attr(attrs, 'href');
  const description = attr(attrs, 'description');
  const link = href ? `[${title}](${href})` : title;
  return `- ${link}${description ? ` — ${description}` : ''}\n\n`;
};

const tabHeading = (attrs) => `### ${attr(attrs, 'label') ?? 'Tab'}\n\n`;

function convertAsides(prose) {
  const lines = prose.split('\n');
  const out = [];
  let inAside = false;
  for (const line of lines) {
    if (!inAside) {
      // Single-line form first: `<Aside type="note">text</Aside>`.
      const inline = line.match(/^\s*<Aside\b([^>]*)>([\s\S]*)<\/Aside>\s*$/);
      if (inline) {
        const type = attr(inline[1], 'type') ?? 'note';
        const title = attr(inline[1], 'title');
        const head = `**${type[0].toUpperCase()}${type.slice(1)}:**`;
        out.push(`> ${head}${title ? ` ${title}` : ''}${inline[2] ? ` ${inline[2].trim()}` : ''}`);
        continue;
      }
      const open = line.match(/^\s*<Aside\b([^>]*)>\s*$/);
      if (open) {
        const type = attr(open[1], 'type') ?? 'note';
        const title = attr(open[1], 'title');
        out.push(`> **${type[0].toUpperCase()}${type.slice(1)}:**${title ? ` ${title}` : ''}`);
        inAside = true;
        continue;
      }
      out.push(line);
    } else if (/^\s*<\/Aside>\s*$/.test(line)) {
      out.push('');
      inAside = false;
    } else {
      out.push(line === '' ? '>' : `> ${line}`);
    }
  }
  // An unclosed `<Aside>` is author error: leave the marker so the JSX
  // gate fails rather than silently swallowing the rest of the page into
  // a quote.
  if (inAside) out.push('</Aside>');
  return out.join('\n');
}

function convertProse(prose) {
  let out = prose
    // Import lines (single- or multi-line `import … from '…';`).
    .replace(/^\s*import\s+[\s\S]*?from\s+['"][^'"]+['"];?\s*$/gm, '')
    // LinkCard (self-closing, then paired with children). Whole-line tags
    // dedent to column 0 so the twin reads as a plain list.
    .replace(/^[ \t]*<LinkCard\b([^>]*)\/>\s*$/gm, (_, attrs) => linkCardLine(attrs))
    .replace(/<LinkCard\b([^>]*)\/>[ \t]*/g, (_, attrs) => linkCardLine(attrs))
    .replace(
      /<LinkCard\b([^>]*)>([\s\S]*?)<\/LinkCard>[ \t]*/g,
      (_, attrs, children) => `${linkCardLine(attrs)}${children.trim() ? `\n${children.trim()}\n\n` : ''}`,
    )
    // Tabs: headings carry the labels, wrappers go. Whole-line tags
    // dedent to column 0; inline occurrences keep their position.
    .replace(/^[ \t]*<TabItem\b([^>]*)>[ \t]*$/gm, (_, attrs) => tabHeading(attrs))
    .replace(/<TabItem\b([^>]*)>[ \t]*/g, (_, attrs) => tabHeading(attrs))
    .replace(/^\s*<\/?(Tabs|Steps|CardGrid)\b[^>]*>\s*$/gm, '')
    .replace(/<\/?(Tabs|Steps|CardGrid)\b[^>]*>/g, '')
    .replace(/<\/TabItem>[ \t]*/g, '');
  out = convertAsides(out);
  // Tag removals leave indented blank lines behind; strip trailing
  // whitespace so the twin has none (fences never reach this function).
  return out.replace(/[ \t]+$/gm, '');
}

// The twin body for one MDX source body (frontmatter already removed by
// the caller): plain Markdown, fences intact.
export function mdxToTwinMarkdown(body) {
  return splitFences(body)
    .map((segment) => (segment.fenced ? segment.text : convertProse(segment.text)))
    .join('')
    .replace(/\n{3,}/g, '\n\n');
}

// The M1 gate: every `<[A-Z]` left in a converted twin's prose is an
// unhandled component leaking JSX to agents. Fences are exempt (they are
// code, quoted verbatim). Returns the offending lines, empty when clean.
export function jsxLeakLines(twin) {
  const leaks = [];
  for (const segment of splitFences(twin)) {
    if (segment.fenced) continue;
    for (const line of segment.text.split('\n')) {
      if (/<[A-Z]/.test(line)) leaks.push(line.trim());
    }
  }
  return leaks;
}
