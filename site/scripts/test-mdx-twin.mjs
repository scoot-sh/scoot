// Gate test for the MDX twin conversion (M1): proves both directions —
// the good fixture converts to plain Markdown with fences intact and no
// JSX left, and the broken fixture's twin still trips the `<[A-Z]` gate.
// The fixtures live outside the content tree on purpose: they must never
// publish (no sidebar entry, no build output), only pin this conversion.
import { execFileSync } from 'node:child_process';
import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { parse as parseToml } from 'smol-toml';
import { jsxLeakLines, mdxToTwinMarkdown, splitFences } from './mdx-twin.mjs';

const root = new URL('.', import.meta.url).pathname;
const good = readFileSync(join(root, 'fixtures/mdx-twin/good.mdx'), 'utf8');
const broken = readFileSync(join(root, 'fixtures/mdx-twin/broken.mdx'), 'utf8');

const failures = [];
const check = (name, ok) => {
  console.log(`  ${ok ? 'ok' : 'FAIL'}: ${name}`);
  if (!ok) failures.push(name);
};

const stripFrontmatter = (text) => text.replace(/^---\n[\s\S]*?\n---\n/, '');

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

if (failures.length > 0) {
  console.error('test-mdx-twin: FAIL');
  process.exit(1);
}
console.log('test-mdx-twin: ok (good converts clean, broken fails the gate)');
