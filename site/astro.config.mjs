import starlight from '@astrojs/starlight';
import starlightLlmsTxt from 'starlight-llms-txt';
import { defineConfig } from 'astro/config';
import { mdLinksToPages } from './scripts/md-links.mjs';

// Post-build `.md` → page rewrite and its gate: see scripts/md-links.mjs
// and scripts/check-md-links.mjs. This must stay a post-build HTML pass,
// not a remark/rehype plugin: the llms.txt bundles render through the same
// markdown pipeline, and any pipeline rewrite would leak into them, where
// agents must keep following Markdown (`.md`) links.

// `site` has to be absolute: the llms.txt bundles link every page with a
// full URL (an agent handed the file as a blob has no base to resolve
// against), and the plugin throws at config time without it.
// `base` must match `site`'s path: starlight-llms-txt builds its bundle
// URLs from Astro's `base`, so a subpath `site` with the default base `/`
// emits bundle links at the domain root (verified against the pinned
// plugin source, llms.txt.ts: `new URL(base, site)`).
//
// Deployment target — the one-line switch for the domain (steps in
// site/README.md). `'www'` serves `https://www.scoot.sh/` today;
// `'apex'` serves the bare root domain; anything else serves the repo's
// project-pages address. `site` and `base` both derive from this single
// value so the switch cannot leave them mismatched.
const DEPLOY_TARGET = 'www';
const { site, base } =
  DEPLOY_TARGET === 'www'
    ? { site: 'https://www.scoot.sh/', base: '/' }
    : DEPLOY_TARGET === 'apex'
      ? { site: 'https://scoot.sh/', base: '/' }
      : { site: 'https://scoot-sh.github.io/scoot/', base: '/scoot/' };
export default defineConfig({
  site,
  base,
  integrations: [
    mdLinksToPages(),
    starlight({
      title: 'scoot',
      description:
        'A scrolling-tiling Wayland compositor that runs without a GPU: install it, learn it, configure it, drive it from an agent.',
      social: [{ label: 'GitHub', href: 'https://github.com/scoot-sh/scoot', icon: 'github' }],
      // The cat's face in a black circle, from the project logo
      // (`docs/assets/logo.png`, face box (985,235)-(1315,565)); the black
      // surround melts into the dark page ground. The "scoot" title text
      // beside it is set in League Spartan 900 (see custom.css).
      logo: {
        src: './src/assets/cat-head.png',
        alt: 'scoot — a grumpy ginger cat about to swat a window sideways',
      },
      favicon: '/cat-favicon-32.png',
      head: [
        {
          tag: 'meta',
          attrs: { name: 'theme-color', content: '#000000' },
        },
        {
          tag: 'link',
          attrs: {
            rel: 'apple-touch-icon',
            href: `${base}apple-touch-icon.png`,
          },
        },
        {
          tag: 'meta',
          attrs: { property: 'og:image', content: `${site}og-cat.jpg` },
        },
        {
          tag: 'meta',
          attrs: {
            property: 'og:image:alt',
            content:
              'scoot — a grumpy ginger cat about to swat a terminal window sideways',
          },
        },
        {
          tag: 'meta',
          attrs: { name: 'twitter:card', content: 'summary_large_image' },
        },
      ],
      customCss: ['./src/styles/custom.css'],
      // Dark only (brand decision 2026-10-06, site/OUTLINE.md): the site
      // renders the dark theme regardless of the visitor's OS preference or
      // any stored choice. `ThemeSelect` renders nothing (no Auto/Light/Dark
      // picker in the header or mobile menu); `ThemeProvider` forces
      // `data-theme="dark"` before first paint and drops any stale
      // `starlight-theme` stored choice. Component paths resolve against the
      // project root (verified against the pinned Starlight source:
      // `resolveId` in `integrations/vite-virtual-modules.js` resolves
      // `.`-relative ids against the project root).
      components: {
        ThemeSelect: './src/components/ThemeSelect.astro',
        ThemeProvider: './src/components/ThemeProvider.astro',
      },
      // Code blocks keep only the dark theme (`starlight-dark`, the dark
      // half of Starlight's default pair). With a single theme Expressive
      // Code applies it unconditionally — no `data-theme` scoping, no
      // `prefers-color-scheme` media query (verified against the pinned
      // `@expressive-code/core` source: `getThemeStyles` emits the base
      // theme under bare `&`, and `useDarkModeMediaQuery` defaults to false
      // unless exactly two themes of opposite types are given).
      // `useStarlightUiThemeColors` stays on so the frames keep Starlight's
      // surface colors (the current dark look) rather than Night Owl's own
      // chrome.
      expressiveCode: {
        themes: ['starlight-dark'],
        useStarlightUiThemeColors: true,
      },
      // One group per app (see site/OUTLINE.md): cross-cutting Start
      // first, the scoot desktop as the primary path, then each app
      // overview → configure → reference → troubleshooting, then agents,
      // cross-cutting troubleshooting, and the reference hub. Slugs nest
      // under their app (`scoot/keybindings`); every group has a matching
      // `customSets` entry below so each app is its own llms.txt set.
      sidebar: [
        {
          label: 'Start',
          items: [
            { label: 'What is scoot', slug: 'index' },
            { label: 'Install', slug: 'start/install' },
            { label: 'First session', slug: 'start/first-session' },
          ],
        },
        {
          label: 'The scoot desktop',
          items: [{ label: 'Desktop profile', slug: 'desktop' }],
        },
        {
          label: 'scoot',
          items: [
            { label: 'Overview', slug: 'scoot' },
            { label: 'Configure', slug: 'scoot/configure' },
            { label: 'Layout', slug: 'scoot/layout' },
            { label: 'Appearance', slug: 'scoot/appearance' },
            { label: 'Outputs', slug: 'scoot/outputs' },
            { label: 'Keybindings', slug: 'scoot/keybindings' },
            { label: 'Windows', slug: 'scoot/windows' },
            { label: 'Backends and rendering', slug: 'scoot/backends' },
            { label: 'XWayland', slug: 'scoot/xwayland' },
            { label: 'Remote desktop', slug: 'scoot/remote-desktop' },
            { label: 'Theming', slug: 'scoot/theming' },
            { label: 'Protocols', slug: 'scoot/protocols' },
            { label: 'Troubleshooting', slug: 'scoot/troubleshooting' },
          ],
        },
        {
          label: 'scoot msg / IPC',
          items: [
            { label: 'Overview', slug: 'msg' },
            { label: 'Requests', slug: 'msg/requests' },
            { label: 'Actions', slug: 'msg/actions' },
            { label: 'Events', slug: 'msg/events' },
            { label: 'Screenshots', slug: 'msg/screenshots' },
            { label: 'Troubleshooting', slug: 'msg/troubleshooting' },
          ],
        },
        {
          label: 'scootbar',
          items: [
            { label: 'Overview', slug: 'scootbar' },
            { label: 'Configure', slug: 'scootbar/configure' },
            { label: 'Modules', slug: 'scootbar/modules' },
            { label: 'Theming', slug: 'scootbar/theming' },
            { label: 'CLI reference', slug: 'scootbar/cli' },
            { label: 'Troubleshooting', slug: 'scootbar/troubleshooting' },
          ],
        },
        {
          label: 'scootbg',
          items: [
            { label: 'Overview', slug: 'scootbg' },
            { label: 'Images and color', slug: 'scootbg/images' },
            { label: 'Outputs', slug: 'scootbg/outputs' },
            { label: 'Wallpaper from a link', slug: 'scootbg/from-url' },
            { label: 'Restore', slug: 'scootbg/restore' },
            { label: 'CLI reference', slug: 'scootbg/cli' },
            { label: 'Troubleshooting', slug: 'scootbg/troubleshooting' },
          ],
        },
        {
          label: 'Agents & webtop',
          items: [
            { label: 'Agents', slug: 'agents' },
            { label: 'Webtop', slug: 'agents/webtop' },
          ],
        },
        {
          label: 'Troubleshooting',
          items: [{ label: 'Symptom index', slug: 'troubleshooting' }],
        },
        {
          label: 'Reference',
          items: [
            { label: 'Reference hub', slug: 'reference' },
            { label: 'Generated CLI pages', slug: 'reference/cli' },
          ],
        },
      ],
      plugins: [
        starlightLlmsTxt({
          projectName: 'scoot',
          description:
            'A scrolling-tiling Wayland compositor that runs without a GPU.',
          details:
            'Task-first guides (install, the desktop profile, first session, keybindings) plus per-app references. Commands in fenced blocks, nothing meaningful only in an image.',
          promote: [
            'index',
            'start/install',
            'start/first-session',
            'desktop',
            'scoot/keybindings',
          ],
          // Per-app documentation sets (see site/OUTLINE.md): one set per
          // sidebar section, served at `/<base>/_llms-txt/<slug>.txt` and
          // linked from llms.txt's "Documentation Sets". Paths match
          // Starlight's extensionless page ids; `<app>/**` covers each
          // app's nested slugs.
          customSets: [
            {
              label: 'Start',
              paths: ['index', 'start/**'],
              description:
                'What scoot is, installing it, and your first session.',
            },
            {
              label: 'The scoot desktop',
              paths: ['desktop', 'desktop/**'],
              description:
                'The full desktop profile: one switch plus a look.',
            },
            {
              label: 'scoot',
              paths: ['scoot', 'scoot/**'],
              description:
                'The scoot compositor: configure, keybindings, backends, theming, protocols.',
            },
            {
              label: 'scoot msg / IPC',
              paths: ['msg', 'msg/**'],
              description:
                'Drive scoot from a script or agent: requests, actions, events, screenshots.',
            },
            {
              label: 'scootbar',
              paths: ['scootbar', 'scootbar/**'],
              description:
                'The status bar: configure, modules, theming, CLI.',
            },
            {
              label: 'scootbg',
              paths: ['scootbg', 'scootbg/**'],
              description:
                'The wallpaper daemon: images, outputs, links, restore, CLI.',
            },
            {
              label: 'Agents & webtop',
              paths: ['agents', 'agents/**'],
              description:
                'Agent setup, headless operation, and the webtop image.',
            },
            {
              label: 'Troubleshooting',
              paths: ['troubleshooting', 'troubleshooting/**'],
              description: 'Fixes by symptom, across every app.',
            },
            {
              label: 'Reference',
              paths: ['reference', 'reference/**'],
              description:
                'Every complete reference in one place, plus generated CLI pages.',
            },
          ],
        }),
      ],
    }),
  ],
});
