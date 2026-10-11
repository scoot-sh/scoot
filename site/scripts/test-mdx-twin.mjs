// Gate test for the MDX twin conversion (M1, hardened F1–F7): proves
// both directions — the good fixture converts to plain Markdown with
// fences intact and no JSX left, and the broken fixture's twin still
// trips the gate — plus one regression pin per hardening finding. Each
// hardening test FAILS on the pre-hardening converter and passes after.
// The fixtures live outside the content tree on purpose: they must never
// publish (no sidebar entry, no build output), only pin this conversion.
import { execFileSync } from 'node:child_process';
import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { parse as parseToml } from 'smol-toml';
import { jsxLeakLines, mdxToTwinMarkdown, splitFences } from './mdx-twin.mjs';

const root = new URL('.', import.meta.url).pathname;
const fixture = (name) =>
  readFileSync(join(root, 'fixtures/mdx-twin', name), 'utf8');
const good = fixture('good.mdx');
const broken = fixture('broken.mdx');

const failures = [];
const check = (name, ok) => {
  console.log(`  ${ok ? 'ok' : 'FAIL'}: ${name}`);
  if (!ok) failures.push(name);
};

// Frontmatter strip only (CRLF-aware); the body keeps its original line
// endings so the CRLF fixture pins the converter's own normalization.
const stripFrontmatter = (text) =>
  text.replace(/^---\r?\n[\s\S]*?\r?\n---\r?\n/, '');

// --- Good fixture: the twin must be plain Markdown. ---
const twin = mdxToTwinMarkdown(stripFrontmatter(good));

check('import lines are stripped', !/^import /m.test(twin));
check(
  'TabItem labels become ### headings',
  twin.includes('### Debian/Ubuntu') && twin.includes('### Fedora'),
);
check('Tabs wrapper is gone', !/<\/?Tabs/.test(twin));
check(
  'Steps wrapper is gone but the ordered list stays',
  !/<\/?Steps/.test(twin) && /^1\. Start scoot from a VT:/m.test(twin),
);
check(
  'Aside with type+title becomes a blockquote',
  twin.includes('> **Caution:** Login screen'),
);
check(
  'Aside without title defaults to its type',
  twin.includes('> **Note:**'),
);
check('Aside bodies are quoted', /^>\s+Picking scoot at the login screen/m.test(twin));
check(
  'LinkCards become a link list',
  twin.includes('- [Install](/start/install/) — Get scoot onto your machine') &&
    twin.includes('- [First run](/start/first-session/)'),
);
check('no JSX leaks in the good twin', jsxLeakLines(twin).length === 0);

// Fences must survive byte-for-byte: extract them before and after and
// compare, and prove the TOML/sh blocks are still sound.
const fencesOf = (text) =>
  splitFences(text)
    .filter((segment) => segment.fenced)
    .map((segment) => segment.text);
check(
  'fenced blocks survive byte-for-byte (incl. the quoted <TabItem> line)',
  JSON.stringify(fencesOf(stripFrontmatter(good))) === JSON.stringify(fencesOf(twin)),
);
let tomlOk = true;
for (const fence of fencesOf(twin)) {
  const body = fence.replace(/^```\w*\n/, '').replace(/```\s*$/, '');
  const language = fence.match(/^```(\w*)/)?.[1] ?? '';
  try {
    if (language === 'toml') parseToml(body);
    if (language === 'sh') execFileSync('bash', ['-n'], { input: body });
  } catch {
    tomlOk = false;
  }
}
check('surviving toml parses and sh passes bash -n', tomlOk);

// --- Broken fixture: the gate must fail on its twin. ---
const brokenTwin = mdxToTwinMarkdown(stripFrontmatter(broken));
const leaks = jsxLeakLines(brokenTwin);
check(
  'broken twin trips the gate (Badge leaks)',
  leaks.length > 0 && leaks.some((line) => line.includes('<Badge')),
);

// --- F1+F3: inline-code spans are exempt from rewriting and the gate. ---
const inlineTwin = mdxToTwinMarkdown(
  stripFrontmatter(fixture('inline-code.mdx')),
);
check(
  'backticked <N> survives verbatim',
  inlineTwin.includes('`<N>`'),
);
check(
  'backticked <TabItem> is not rewritten to a heading',
  inlineTwin.includes('`<TabItem>`') && !inlineTwin.includes('`### Tab`'),
);
check(
  'multi-backtick <Aside> survives verbatim',
  inlineTwin.includes('``<Aside>``'),
);
check(
  'inline-code twin trips no gate',
  jsxLeakLines(inlineTwin).length === 0,
);

// --- F2: unclosed wrappers fail the gate. ---
const unclosedAsideTwin = mdxToTwinMarkdown(
  stripFrontmatter(fixture('unclosed-aside.mdx')),
);
const unclosedAsideLeaks = jsxLeakLines(unclosedAsideTwin);
check(
  'unclosed Aside fails the gate',
  unclosedAsideLeaks.length > 0 &&
    unclosedAsideLeaks.some((line) => line.includes('Aside-UNCLOSED')),
);
const unclosedTabsTwin = mdxToTwinMarkdown(
  stripFrontmatter(fixture('unclosed-tabs.mdx')),
);
const unclosedTabsLeaks = jsxLeakLines(unclosedTabsTwin);
check(
  'unclosed Tabs/TabItem fails the gate',
  unclosedTabsLeaks.length > 0 &&
    unclosedTabsLeaks.some((line) => line.includes('UNCLOSED')),
);

// --- F4: `>` inside a quoted attribute value does not end the tag. ---
const attrGtTwin = mdxToTwinMarkdown(
  stripFrontmatter(fixture('attr-gt.mdx')),
);
check(
  'Aside title keeps its inner >',
  attrGtTwin.includes('> **Note:** a > b') &&
    /^>\s+Body text\./m.test(attrGtTwin),
);
check(
  'title-with-> twin trips no gate',
  jsxLeakLines(attrGtTwin).length === 0,
);

// --- F5: escaped quotes inside a label parse. ---
const attrEscapedTwin = mdxToTwinMarkdown(
  stripFrontmatter(fixture('attr-escaped.mdx')),
);
check(
  'TabItem label unescapes its quoted quotes',
  attrEscapedTwin.includes('### A "quoted" label'),
);
check(
  'escaped-quote twin trips no gate',
  jsxLeakLines(attrEscapedTwin).length === 0,
);

// --- F6: an unterminated fence fails the gate loudly. ---
const unfencedTwin = mdxToTwinMarkdown(
  stripFrontmatter(fixture('unterminated-fence.mdx')),
);
const unfencedLeaks = jsxLeakLines(unfencedTwin);
check(
  'unterminated fence fails the gate',
  unfencedLeaks.length > 0 &&
    unfencedLeaks.some((line) => line.includes('UNTERMINATED-FENCE')),
);

// --- F7: CRLF normalizes to LF, and multi-line Aside opens convert. ---
const crlfTwin = mdxToTwinMarkdown(
  stripFrontmatter(fixture('crlf.mdx')),
);
check('CRLF twin holds no carriage returns', !crlfTwin.includes('\r'));
check(
  'CRLF prose survives (both lines)',
  crlfTwin.includes('First line.') && crlfTwin.includes('Second line.'),
);
const asideMultiTwin = mdxToTwinMarkdown(
  stripFrontmatter(fixture('aside-multiline.mdx')),
);
check(
  'multi-line Aside open converts like a single-line one',
  asideMultiTwin.includes('> **Note:** Hello') &&
    /^>\s+Body text\./m.test(asideMultiTwin),
);
check(
  'multi-line Aside twin trips no gate',
  jsxLeakLines(asideMultiTwin).length === 0,
);

if (failures.length > 0) {
  console.error('test-mdx-twin: FAIL');
  process.exit(1);
}
console.log('test-mdx-twin: ok (good converts clean, broken fails the gate)');
