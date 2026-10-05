import starlight from '@astrojs/starlight';
import starlightLlmsTxt from 'starlight-llms-txt';
import { defineConfig } from 'astro/config';

// `site` has to be absolute: the llms.txt bundles link every page with a
// full URL (an agent handed the file as a blob has no base to resolve
// against), and the plugin throws at config time without it. This is the
// default GitHub project-pages address for github.com/scoot-sh/scoot.
// `base` must match `site`'s path: starlight-llms-txt builds its bundle
// URLs from Astro's `base`, so a subpath `site` with the default base `/`
// emits bundle links at the domain root (verified against the pinned
// plugin source, llms.txt.ts: `new URL(base, site)`). Both change together
// when the maintainer picks the real domain (see site/README.md).
export default defineConfig({
  site: 'https://scoot-sh.github.io/scoot/',
  base: '/scoot/',
  integrations: [
    starlight({
      title: 'scoot',
      description:
        'A scrolling-tiling Wayland compositor that runs without a GPU: install it, learn it, configure it, drive it from an agent.',
      social: [{ label: 'GitHub', href: 'https://github.com/scoot-sh/scoot', icon: 'github' }],
      customCss: ['./src/styles/custom.css'],
      sidebar: [
        {
          label: 'Start',
          items: [
            { label: 'What is scoot', slug: 'index' },
            { label: 'Install', slug: 'install' },
            { label: 'First session', slug: 'first-session' },
            { label: 'Keybindings', slug: 'keybindings' },
          ],
        },
        // Phase 2 groups land here as their pages do (see site/OUTLINE.md):
        // { label: 'Configure', items: [...], collapsed: true },
        // { label: 'Reference', items: [...], collapsed: true },
      ],
      plugins: [
        starlightLlmsTxt({
          projectName: 'scoot',
          description:
            'A scrolling-tiling Wayland compositor that runs without a GPU.',
          details:
            'Task-first guides (install, first session, keybindings) plus the full reference. Commands in fenced blocks, nothing meaningful only in an image.',
          promote: ['index*', 'install*', 'first-session*', 'keybindings*'],
        }),
      ],
    }),
  ],
});
