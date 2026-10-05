// Snippet gate (phase-1 slice of the docs-bar standard): every fenced code
// block in the docs sources must be at least mechanically sound, so no
// example can rot unnoticed.
//
//   - ```toml blocks must parse (smol-toml).
//   - ```sh blocks must pass `bash -n`.
//   - ```nix blocks are skipped: only `nix build` type-checks them, and
//     that runs in CI's docs-site job via the derivation itself.
//   - Prose must not say "colour" or "window manager" (repo wording rules:
//     CLAUDE.md). Quoted upstream text is the only exception, and there is
//     none in the scaffold.
//
// What this does NOT do yet (recorded in the docs-site ticket as the next
// step): execute snippets against a headless scoot. That needs the Linux
// binary beside the site build; the derivation and the CI job are shaped
// for it (both run this script already).
import { execFileSync } from 'node:child_process';
import { readFileSync, readdirSync } from 'node:fs';
import { join } from 'node:path';
import { parse as parseToml } from 'smol-toml';

const root = new URL('..', import.meta.url).pathname;
const contentDir = join(root, 'src', 'content', 'docs');

const failures = [];
const fail = (file, message) => failures.push(`${file}: ${message}`);

const fence = /```(\w+)\n([\s\S]*?)```/g;

for (const file of readdirSync(contentDir).filter((f) => f.endsWith('.md'))) {
  const text = readFileSync(join(contentDir, file), 'utf8');
  if (/\bcolour\b/i.test(text)) fail(file, 'says "colour", spell it "color"');
  if (/\bwindow manager\b/i.test(text)) {
    fail(file, 'says "window manager", say "compositor"');
  }
  for (const match of text.matchAll(fence)) {
    const [, language, body] = match;
    if (language === 'toml') {
      try {
        parseToml(body);
      } catch (error) {
        fail(file, `unparseable toml block: ${error.message.split('\n')[0]}`);
      }
    } else if (language === 'sh') {
      try {
        execFileSync('bash', ['-n'], { input: body });
      } catch {
        fail(file, 'sh block fails `bash -n`');
      }
    }
  }
}

if (failures.length > 0) {
  console.error('test-snippets: FAIL');
  for (const failure of failures) console.error(`  - ${failure}`);
  process.exit(1);
}
console.log('test-snippets: ok (toml parses, sh syntax-checks, wording rules hold)');
