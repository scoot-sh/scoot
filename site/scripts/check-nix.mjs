// Nix gate: every fenced ```nix block in the docs tree must parse, so a
// flake/module snippet cannot rot unnoticed. Runs in node (`pnpm
// test-nix`), in the nix `docs-site` derivation (which provides the `nix`
// binary), and in CI through both.
//
// Four shapes are accepted, tried in order: the block as written (a full
// file, e.g. a `flake.nix`, or a bare expression); the block as an
// attrset of assignments (`{ <block> }`, e.g. `programs.scoot = ...`);
// the same with the usual module and package names bound
// (`{ inputs, pkgs, lib, config, scootbar, ... }:` — this
// `nix-instantiate` resolves names while parsing, so a fragment naming
// `inputs` or `pkgs` needs them in scope); and the block as a bare
// expression under that lambda (e.g. `scootbar.override { ... }`).
// Placeholders are comments, never literal `...` outside real Nix
// syntax (which `...` only is in attr patterns); alternatives go in
// separate fences (two assignments to one path in one fence is a
// duplicate, not documentation).
import { execFileSync } from 'node:child_process';
import { mkdtempSync, readFileSync, readdirSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

const root = new URL('..', import.meta.url).pathname;
const contentDir = join(root, 'src', 'content', 'docs');

const failures = [];
const fail = (file, message) => failures.push(`${file}: ${message}`);

try {
  execFileSync('nix-instantiate', ['--version'], { stdio: 'ignore' });
} catch {
  console.error(
    'check-nix: FAIL\n  - no `nix-instantiate` on PATH (run through nix/develop: the site build provides it)',
  );
  process.exit(1);
}

const walk = (dir) =>
  readdirSync(dir, { withFileTypes: true }).flatMap((entry) => {
    const path = join(dir, entry.name);
    if (entry.isDirectory()) return walk(path);
    return entry.name.endsWith('.md') || entry.name.endsWith('.mdx')
      ? [path]
      : [];
  });

const fence = /```nix\n([\s\S]*?)```/g;
const scratch = mkdtempSync(join(tmpdir(), 'scoot-nix-snippets-'));

let count = 0;
for (const file of walk(contentDir)) {
  const text = readFileSync(file, 'utf8');
  for (const match of text.matchAll(fence)) {
    const body = match[1];
    count += 1;
    const direct = join(scratch, `block-${count}.nix`);
    const wrapped = join(scratch, `block-${count}-wrapped.nix`);
    const modWrap = join(scratch, `block-${count}-module.nix`);
    const modExpr = join(scratch, `block-${count}-modexpr.nix`);
    writeFileSync(direct, body);
    writeFileSync(wrapped, `{\n${body}\n}\n`);
    writeFileSync(modWrap, `{ inputs, pkgs, lib, config, scootbar, ... }:\n{\n${body}\n}\n`);
    writeFileSync(modExpr, `{ inputs, pkgs, lib, config, scootbar, ... }:\n${body}\n`);
    let ok = false;
    for (const candidate of [direct, wrapped, modWrap, modExpr]) {
      try {
        execFileSync('nix-instantiate', ['--parse', candidate], {
          stdio: 'ignore',
        });
        ok = true;
        break;
      } catch {
        // Try the next shape.
      }
    }
    if (!ok)
      fail(
        file,
        `unparseable nix block #${count} (as file, fragment, module body, and module expression)`,
      );
  }
}

if (failures.length > 0) {
  console.error('check-nix: FAIL');
  for (const failure of failures) console.error(`  - ${failure}`);
  process.exit(1);
}
console.log(`check-nix: ok (${count} nix blocks parse)`);
