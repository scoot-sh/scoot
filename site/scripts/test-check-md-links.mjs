// Fixture test for the link gate (`check-md-links.mjs`): the `broken`
// fixture carries three dead fragments (one in-page, two cross-page, one of
// them the real `#shape-and-opacity` vs `shape-opacity-and-depth` slip) next
// to two valid fragment links; the `ok` fixture exercises duplicate `-1`
// ids, a percent-encoded id, `#`, an external URL with a fragment
// and a fragment-less page link, none of which may fail.
import { dirname, join } from 'node:path';
import { checkLinks } from './check-md-links.mjs';

const here = dirname(new URL(import.meta.url).pathname);
const fixture = (name) => join(here, 'fixtures', 'check-md-links', name, 'dist');

const failures = [];
const fail = (message) => failures.push(message);

const broken = checkLinks(fixture('broken'));
const expect = ['#missing-local', '#shape-and-opacity', '#nope'];
if (broken.failures.length !== expect.length) {
  fail(`broken fixture: expected ${expect.length} failures, got ${broken.failures.length}: ${broken.failures.join(' | ')}`);
}
for (const needle of expect) {
  if (!broken.failures.some((f) => f.includes(needle))) fail(`broken fixture: no failure naming ${needle}`);
}

const ok = checkLinks(fixture('ok'));
if (ok.failures.length !== 0) fail(`ok fixture: unexpected failures: ${ok.failures.join(' | ')}`);
if (ok.fragments < 4) fail(`ok fixture: expected at least 4 fragments checked, got ${ok.fragments}`);

if (failures.length > 0) {
  console.error('test-check-md-links: FAIL');
  for (const f of failures) console.error(`  - ${f}`);
  process.exit(1);
}
console.log(`test-check-md-links: ok (broken fixture fails with ${broken.failures.length}, ok fixture passes with ${ok.fragments} fragments)`);
