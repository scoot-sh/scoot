// Fixture test for the asset gate (`check-assets.mjs`): the `missing`
// fixture (one missing font via CSS `url()`, one missing image via HTML
// `src`) must fail the check with exactly those two, and the `ok`
// fixture — same shape, every asset present — must pass. The present
// assets, the `.md` twin link, the external URL and the `data:` URI in
// both fixtures pin the no-false-positive side: the checker must see
// references (non-zero `checked`) yet only fail on the two absent files.
import { dirname, join } from 'node:path';
import { checkAssets } from './check-assets.mjs';

const here = dirname(new URL(import.meta.url).pathname);
const missingDist = join(here, 'fixtures', 'check-assets', 'missing', 'dist');
const okDist = join(here, 'fixtures', 'check-assets', 'ok', 'dist');

const failures = [];
const fail = (message) => failures.push(message);

const missing = checkAssets(missingDist);
const missingUrls = missing.missing.map((entry) => entry.url).sort();
if (missingUrls.length !== 2) {
  fail(`missing fixture: expected 2 missing, got ${missingUrls.length} (${missingUrls.join(', ')})`);
} else {
  if (missingUrls[0] !== './fonts/missing-font.woff2') {
    fail(`missing fixture: expected the missing font, got ${missingUrls[0]}`);
  }
  if (missingUrls[1] !== './images/missing-photo.avif') {
    fail(`missing fixture: expected the missing image, got ${missingUrls[1]}`);
  }
}
if (missing.checked < 6) {
  fail(`missing fixture: expected at least 6 references checked, got ${missing.checked}`);
}

const ok = checkAssets(okDist);
if (ok.missing.length !== 0) {
  fail(
    `ok fixture: expected no missing, got ${ok.missing.map((entry) => entry.url).join(', ')}`,
  );
}
if (ok.checked < 5) {
  fail(`ok fixture: expected at least 5 references checked, got ${ok.checked}`);
}

if (failures.length > 0) {
  console.error('test-check-assets: FAIL');
  for (const failure of failures) console.error(`  - ${failure}`);
  process.exit(1);
}
console.log(
  `test-check-assets: ok (missing fixture fails with ${missing.missing.length}, ok fixture passes with ${ok.checked} references)`,
);
