# The pnpm deps hash (`nix/docs-site.nix`)

`nix/docs-site.nix` pins the vendored site dependencies with
`pnpmDeps.hash`. It must match `site/pnpm-lock.yaml`. The hash is
Linux-specific: a Mac computes a different value, so Linux is canonical.

## When to run the helper

After any change to `site/pnpm-lock.yaml`: a dependabot site PR, a manual
`pnpm update`, an added or removed site dependency. The CI `docs-site` job
fails with `hash mismatch` until the pin is updated, and its log ends with
an annotation naming this command.

```sh
scripts/update-pnpm-deps-hash     # on Linux, e.g. the dev VM
git diff nix/docs-site.nix        # one line
```

Commit the one-line change to the PR branch. The script builds only
`.#docs-site.pnpmDeps`, not the whole site. It restores the file if the
build fails or is interrupted, so the pin is never left blank, and a pin
that is already right is left untouched. It refuses to run off Linux.

Nothing pushes the fix for you: CI checks the pin (the `docs-site` job), it
does not edit the branch.
