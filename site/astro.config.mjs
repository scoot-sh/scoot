import starlight from '@astrojs/starlight';
import starlightLlmsTxt from 'starlight-llms-txt';
import { defineConfig } from 'astro/config';

// `site` has to be absolute: the llms.txt bundles link every page with a
// full URL (an agent handed the file as a blob has no base to resolve
// against), and the plugin throws at config time without it.
// `base` must match `site`'s path: starlight-llms-txt builds its bundle
// URLs from Astro's `base`, so a subpath `site` with the default base `/`
// emits bundle links at the domain root (verified against the pinned
// plugin source, llms.txt.ts: `new URL(base, site)`).
//
// Deployment target — the one-line switch for the scoot.sh move (steps in
// site/README.md, "Moving to scoot.sh"). `'pages'` serves the repo's
// project-pages address today; `'apex'` serves the root domain. `site`
// and `base` both derive from this single value so the switch cannot
// leave them mismatched.
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
    starlight({
      title: 'scoot',
      description:
        'A scrolling-tiling Wayland compositor that runs without a GPU: install it, learn it, configure it, drive it from an agent.',
      social: [{ label: 'GitHub', href: 'https://github.com/scoot-sh/scoot', icon: 'github' }],
      customCss: ['./src/styles/custom.css'],
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
            { label: 'Theming', slug: 'scoot/theming' },
            { label: 'Protocols', slug: 'scoot/protocols' },
            { label: 'Troubleshooting', slug: 'scoot/troubleshooting' },
          ],
        },
        {
          label: 'scootctl / IPC',
          items: [
            { label: 'Overview', slug: 'scootctl' },
            { label: 'Requests', slug: 'scootctl/requests' },
            { label: 'Actions', slug: 'scootctl/actions' },
            { label: 'Events', slug: 'scootctl/events' },
            { label: 'Screenshots', slug: 'scootctl/screenshots' },
            { label: 'Troubleshooting', slug: 'scootctl/troubleshooting' },
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
              label: 'scootctl / IPC',
              paths: ['scootctl', 'scootctl/**'],
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
