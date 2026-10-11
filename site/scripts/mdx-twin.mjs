// Shared MDX → plain-Markdown twin conversion (M1: MDX pages pass every
// gate; hardened follow-up: F1–F7). Imported by the per-page twin route
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
//     inner line gains a `> ` prefix. `<Aside>` open tags may span
//     multiple lines (one attribute per line); they convert exactly like
//     single-line opens.
//   - `<CardGrid>` wrappers are dropped; each `<LinkCard title="T"
//     href="H" description="D" />` becomes a `- [T](H) — D` link-list
//     entry (description omitted when absent).
//
// Hardening notes (F1–F7):
//   - Inline-code spans (single-backtick and multi-backtick, possibly
//     spanning lines) are exempt from BOTH the prose rewriting and the
//     JSX gate: `` `<TabItem>` ``, `` `<N>` `` and `` ``<Aside>`` ``
//     survive verbatim and never trip the gate. Fenced blocks are
//     likewise untouched. So `.mdx` authors backtick-quote component
//     names and prose generics.
//   - Attribute values are parsed quote-aware: a `>` inside a quoted
//     value (`title="a > b"`) does NOT end the tag, and backslash
//     escapes inside quotes work (`label="A \"quoted\" label"` →
//     `A "quoted" label`). Both quote styles are accepted. A tag whose
//     quotes never terminate simply does not match, so it stays in the
//     twin and the gate below fails loudly (naming the file via
//     check-llms and the line via the `line N:` prefix) instead of
//     silently corrupting content.
//   - An unclosed `<Aside>` block (and any unbalanced `<Tabs>` /
//     `<Steps>` / `<CardGrid>` / `<TabItem>` / `<LinkCard>` wrapper)
//     emits an explicit `<TAG-UNCLOSED>` sentinel so the gate fails
//     instead of silently swallowing prose into a quote or dropping a
//     wrapper half.
//   - An unterminated fenced block emits an `UNTERMINATED-FENCE` gate
//     finding (line-numbered) instead of silently exempting the rest of
//     the page from the gate.
//   - CRLF (`\r\n`) sources are normalized to LF up front, so twins are
//     stable regardless of checkout line endings.
//
// Anything else matching `<[A-Z]` or `</[A-Z]` (an unhandled component
// or stray close) is left in place on purpose so the gate below fails
// loudly instead of silently shipping JSX to agents.
const FENCE_MARKER = /^[ \t]*(`{3,}|~{3,})/;

// Extract one `name="…"` / `name='…'` attribute value, quote-aware:
// `>` inside the quotes is data, backslash escapes the next character
// (`\"` → `"`, `\\` → `\`). Returns null when absent or unterminated
// (the caller leaves the tag for the gate in that case).
export function attr(attrs, name) {
  const marker = attrs.match(new RegExp(`(?:^|\\s)${name}\\s*=\\s*(["'])`));
  if (!marker) return null;
  const quote = marker[1];
  let i = marker.index + marker[0].length;
  let out = '';
  while (i < attrs.length) {
    const c = attrs[i];
    if (c === '\\' && i + 1 < attrs.length) {
      out += attrs[i + 1];
      i += 2;
      continue;
    }
    if (c === quote) return out;
    out += c;
    i += 1;
  }
  return null;
}

// Index of the `>` closing a tag opened at or before `from`, ignoring
// `>` inside single/double quotes (backslash escapes inside quotes).
// Returns -1 when no such `>` exists (unterminated tag).
function tagCloseIndex(text, from = 0) {
  let single = false;
  let double = false;
  for (let i = from; i < text.length; i++) {
    const c = text[i];
    if (c === '\\' && (single || double) && i + 1 < text.length) {
      i += 1;
      continue;
    }
    if (c === '"' && !single) double = !double;
    else if (c === "'" && !double) single = !single;
    else if (c === '>' && !single && !double) return i;
  }
  return -1;
}

function normalizeNewlines(text) {
  return text.replace(/\r\n/g, '\n').replace(/\r/g, '\n');
}

// Split text into inline-code / prose parts (CommonMark-style: an
// opening run of N backticks closes at the next run of exactly N
// backticks; other-length runs inside are content). Unclosed runs are
// literal prose. Code parts may span lines.
function splitInlineCode(text) {
  const parts = [];
  let i = 0;
  let plain = '';
  const flushPlain = () => {
    if (plain) {
      parts.push({ code: false, text: plain });
      plain = '';
    }
  };
  while (i < text.length) {
    if (text[i] !== '`') {
      plain += text[i];
      i += 1;
      continue;
    }
    let j = i;
    while (j < text.length && text[j] === '`') j += 1;
    const n = j - i;
    const opener = text.slice(i, j);
    let k = j;
    let close = -1;
    let closeEnd = -1;
    while (k < text.length) {
      const p = text.indexOf('`'.repeat(n), k);
      if (p === -1) break;
      let q = p;
      while (q < text.length && text[q] === '`') q += 1;
      if (q - p === n) {
        close = p;
        closeEnd = q;
        break;
      }
      k = q;
    }
    if (close === -1) {
      plain += opener;
      i = j;
      continue;
    }
    flushPlain();
    parts.push({ code: true, text: text.slice(i, closeEnd) });
    i = closeEnd;
  }
  flushPlain();
  return parts;
}

const PLACEHOLDER = (n) => `\u0000CODE${n}\u0000`;

// Hide inline-code spans behind placeholders so tag rewriting never
// sees inside them. Returns the protected text plus the hidden slots.
function protectInlineCode(prose) {
  const slots = [];
  let out = '';
  for (const part of splitInlineCode(prose)) {
    if (part.code) {
      slots.push(part.text);
      out += PLACEHOLDER(slots.length - 1);
    } else {
      out += part.text;
    }
  }
  return { protected: out, slots };
}

function restoreInlineCode(text, slots) {
  return text.replace(/\u0000CODE(\d+)\u0000/g, (_, n) => slots[Number(n)] ?? '');
}

// Like protect, but for the gate: mask code spans with spaces (keeping
// newlines) so `<[A-Z]` inside backticks never trips it.
function maskInlineCode(prose) {
  return splitInlineCode(prose)
    .map((part) =>
      part.code ? part.text.replace(/[^\n]/g, ' ') : part.text,
    )
    .join('');
}

// Split text into fenced / prose segments so the conversion below never
// touches fenced code (a TOML snippet quoting `<TabItem>` must survive).
// CRLF is normalized up front (F7). A fence that never closes is kept
// as a fenced segment but flagged `unterminated` with its opening line,
// so the gate can fail loudly instead of silently exempting the tail.
export function splitFences(text) {
  const normalized = normalizeNewlines(text);
  const segments = [];
  let prose = [];
  let open = null;
  let fenced = null;
  let openLine = 0;
  let lineNo = 0;
  const proseStart = () => lineNo - prose.length + 1;
  for (const line of normalized.split('\n')) {
    lineNo += 1;
    const marker = line.match(FENCE_MARKER)?.[1] ?? null;
    if (open === null && marker !== null) {
      if (prose.length > 0) {
        segments.push({
          fenced: false,
          text: `${prose.join('\n')}\n`,
          startLine: proseStart(),
        });
        prose = [];
      }
      open = marker;
      fenced = line;
      openLine = lineNo;
    } else if (open !== null) {
      fenced += `\n${line}`;
      if (
        marker !== null &&
        marker[0] === open[0] &&
        marker.length >= open.length
      ) {
        segments.push({
          fenced: true,
          text: `${fenced}\n`,
          startLine: openLine,
        });
        open = null;
        fenced = null;
      }
    } else {
      prose.push(line);
    }
  }
  if (fenced !== null)
    segments.push({
      fenced: true,
      text: fenced,
      startLine: openLine,
      unterminated: true,
      marker: open,
    });
  if (prose.length > 0)
    segments.push({
      fenced: false,
      text: prose.join('\n'),
      startLine: proseStart(),
    });
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

// Quote-aware attribute tail: quoted values (either style, backslash
// escapes) or any run without `>`, `"`, `'`. Covers `>` inside quotes
// and spans lines (`[^>"']` matches `\n`).
const QUOTED = `"(?:[^"\\\\]|\\\\.)*"|'(?:[^'\\\\]|\\\\.)*'`;
const ATTRS = `((?:${QUOTED}|[^>"'])*)`;

function convertAsides(prose, startInAside = false) {
  const lines = prose.split('\n');
  const out = [];
  let inAside = startInAside;
  let pending = null;
  const inlineAside = new RegExp(`^\\s*<Aside\\b${ATTRS}>([\\s\\S]*)<\\/Aside>\\s*$`);
  for (const line of lines) {
    if (pending !== null) {
      pending.lines.push(line);
      const joined = pending.lines.join('\n');
      const close = tagCloseIndex(joined, pending.tagStart);
      if (close === -1) continue;
      const attrs = joined.slice(pending.tagStart + '<Aside'.length, close);
      const rest = joined.slice(close + 1);
      if (rest.trim() !== '') {
        out.push(...pending.lines);
        pending = null;
        continue;
      }
      const type = attr(attrs, 'type') ?? 'note';
      const title = attr(attrs, 'title');
      out.push(
        `> **${type[0].toUpperCase()}${type.slice(1)}:**${title ? ` ${title}` : ''}`,
      );
      inAside = true;
      pending = null;
      continue;
    }
    if (!inAside) {
      // Single-line form first: `<Aside type="note">text</Aside>`.
      const inline = line.match(inlineAside);
      if (inline) {
        const type = attr(inline[1], 'type') ?? 'note';
        const title = attr(inline[1], 'title');
        const head = `**${type[0].toUpperCase()}${type.slice(1)}:**`;
        out.push(
          `> ${head}${title ? ` ${title}` : ''}${inline[2] ? ` ${inline[2].trim()}` : ''}`,
        );
        continue;
      }
      const asideStart = line.match(/^\s*<Aside\b/);
      if (asideStart) {
        const tagStart = line.indexOf('<Aside');
        const close = tagCloseIndex(line, tagStart);
        if (close === -1) {
          // Multi-line open tag: buffer until the `>` outside quotes.
          pending = { lines: [line], tagStart };
          continue;
        }
        const attrs = line.slice(tagStart + '<Aside'.length, close);
        const rest = line.slice(close + 1);
        if (rest.trim() === '') {
          const type = attr(attrs, 'type') ?? 'note';
          const title = attr(attrs, 'title');
          out.push(
            `> **${type[0].toUpperCase()}${type.slice(1)}:**${title ? ` ${title}` : ''}`,
          );
          inAside = true;
          continue;
        }
      }
      out.push(line);
    } else if (/^\s*<\/Aside>\s*$/.test(line)) {
      out.push('');
      inAside = false;
    } else {
      out.push(line === '' ? '>' : `> ${line}`);
    }
  }
  if (pending !== null) {
    // An open tag that never closed is author error: leave it so the
    // JSX gate fails rather than silently eating lines.
    out.push(...pending.lines);
  }
  // The caller (mdxToTwinMarkdown) carries `inAside` across prose
  // segments split by fences and emits one `<Aside-UNCLOSED>` sentinel
  // at document end when still open, so an `<Aside>` spanning a fence
  // does not false-positive per segment.
  return { text: out.join('\n'), inAside };
}

// Count unbalanced handled wrappers on the whole document's protected
// prose (all non-fenced segments combined, inline code already hidden).
// Must run document-wide: a `<TabItem>` legitimately spans fenced
// blocks, so per-segment counts false-positive. Returns one
// `<TAG-UNCLOSED>` sentinel per tag whose opens and closes disagree, so
// the gate fails loudly.
function unclosedWrapperSentinels(protectedProse) {
  const sentinels = [];
  for (const tag of ['Tabs', 'Steps', 'CardGrid', 'TabItem', 'LinkCard']) {
    const openRe = new RegExp(`<${tag}\\b${ATTRS}>`, 'g');
    let opens = 0;
    for (const m of protectedProse.matchAll(openRe)) {
      if (m[0].endsWith('/>')) continue;
      opens += 1;
    }
    const closes = (protectedProse.match(new RegExp(`<\\/${tag}\\s*>`, 'g')) ?? []).length;
    if (opens !== closes) sentinels.push(`<${tag}-UNCLOSED>`);
  }
  return sentinels;
}

function convertProse(prose, asideState = null) {
  const trailingNewline = prose.endsWith('\n');
  const { protected: hidden, slots } = protectInlineCode(prose);
  const linkSelfLine = new RegExp(`^[ \\t]*<LinkCard\\b${ATTRS}\\/>\\s*$`, 'gm');
  const linkSelf = new RegExp(`<LinkCard\\b${ATTRS}\\/>[ \\t]*`, 'g');
  const linkPaired = new RegExp(`<LinkCard\\b${ATTRS}>([\\s\\S]*?)<\\/LinkCard>[ \\t]*`, 'g');
  const tabLine = new RegExp(`^[ \\t]*<TabItem\\b${ATTRS}>[ \\t]*$`, 'gm');
  const tabInline = new RegExp(`<TabItem\\b${ATTRS}>[ \\t]*`, 'g');
  const wrapLine = new RegExp(`^\\s*<\\/?(Tabs|Steps|CardGrid)\\b${ATTRS}>\\s*$`, 'gm');
  const wrapInline = new RegExp(`<\\/?(Tabs|Steps|CardGrid)\\b${ATTRS}>`, 'g');
  let out = hidden
    // Import lines (single- or multi-line `import … from '…';`).
    .replace(/^\s*import\s+[\s\S]*?from\s+['"][^'"]+['"];?\s*$/gm, '')
    // LinkCard (self-closing, then paired with children). Whole-line tags
    // dedent to column 0 so the twin reads as a plain list.
    .replace(linkSelfLine, (_, attrs) => linkCardLine(attrs))
    .replace(linkSelf, (_, attrs) => linkCardLine(attrs))
    .replace(
      linkPaired,
      (_, attrs, children) =>
        `${linkCardLine(attrs)}${children.trim() ? `\n${children.trim()}\n\n` : ''}`,
    )
    // Tabs: headings carry the labels, wrappers go. Whole-line tags
    // dedent to column 0; inline occurrences keep their position.
    .replace(tabLine, (_, attrs) => tabHeading(attrs))
    .replace(tabInline, (_, attrs) => tabHeading(attrs))
    .replace(wrapLine, '')
    .replace(wrapInline, '')
    .replace(/<\/TabItem>[ \t]*/g, '');
  const asideResult = convertAsides(out, asideState ? asideState.inAside : false);
  out = asideResult.text;
  if (asideState) asideState.inAside = asideResult.inAside;
  else if (asideResult.inAside) out += '\n<Aside-UNCLOSED>\n';
  out = restoreInlineCode(out, slots);
  // Tag removals leave indented blank lines behind; strip trailing
  // whitespace so the twin has none (fences never reach this function;
  // inline code was restored after the protected conversion, and a
  // code span ending a line keeps its bytes because only `[ \t]+`
  // outside the span location is stripped — spans containing trailing
  // spaces are pathological and not preserved exactly).
  out = out.replace(/[ \t]+$/gm, '');
  // convertAsides rebuilds via join, dropping the source's trailing
  // newline; restore it so fence-split segments rejoin cleanly (else a
  // `>` quote line glues onto the next fence marker).
  if (trailingNewline && !out.endsWith('\n')) out += '\n';
  return out;
}

// The twin body for one MDX source body (frontmatter already removed by
// the caller): plain Markdown, fences intact, CRLF normalized to LF.
// Wrapper balance (Tabs/Steps/CardGrid/TabItem/LinkCard opens vs closes)
// is checked document-wide over all prose segments: those wrappers
// legitimately span fenced blocks. Aside open state likewise carries
// across fence-split segments; a still-open Aside at document end (and
// any unbalanced wrapper) appends one `<TAG-UNCLOSED>` sentinel so the
// gate fails loudly.
export function mdxToTwinMarkdown(body) {
  const segments = splitFences(body);
  const combinedHidden = segments
    .filter((segment) => !segment.fenced)
    .map((segment) => protectInlineCode(segment.text).protected)
    .join('\n');
  const sentinels = unclosedWrapperSentinels(combinedHidden);
  const asideState = { inAside: false };
  let twin = segments
    .map((segment) =>
      segment.fenced ? segment.text : convertProse(segment.text, asideState),
    )
    .join('')
    .replace(/\n{3,}/g, '\n\n');
  if (asideState.inAside) twin += '\n<Aside-UNCLOSED>\n';
  if (sentinels.length > 0) twin += `\n${sentinels.join('\n')}\n`;
  return twin.replace(/\n{3,}/g, '\n\n');
}

// The M1 gate (hardened): every `<[A-Z]` or `</[A-Z]` left in a
// converted twin's prose is an unhandled component leaking JSX to
// agents. Fences are exempt (they are code, quoted verbatim), as are
// inline-code spans (`` `<N>` `` is prose, not a component). An
// unterminated fence is its own finding. Returns `line N: …` strings,
// empty when clean; check-llms prefixes the file (slug), so a failure
// names both file and line.
export function jsxLeakLines(twin) {
  const leaks = [];
  for (const segment of splitFences(twin)) {
    if (segment.fenced) {
      if (segment.unterminated) {
        leaks.push(
          `line ${segment.startLine}: UNTERMINATED-FENCE: fence opened with ${segment.marker} never closed`,
        );
      }
      continue;
    }
    const masked = maskInlineCode(segment.text);
    const lines = masked.split('\n');
    const rawLines = segment.text.split('\n');
    for (let i = 0; i < lines.length; i++) {
      if (/<\/?[A-Z]/.test(lines[i])) {
        leaks.push(`line ${segment.startLine + i}: ${(rawLines[i] ?? '').trim()}`);
      }
    }
  }
  return leaks;
}
