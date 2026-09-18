// @ts-check

/** @type {import('@docusaurus/types').Config} */
const config = {
  title: 'Memeloop Workspace Control',
  tagline: 'A Kubernetes control plane for isolated, per-user development workspaces',
  url: 'https://memeloop-online.github.io',
  baseUrl: '/memeloop-workspace-control/',
  organizationName: 'memeloop-online',
  projectName: 'memeloop-workspace-control',
  deploymentBranch: 'gh-pages',
  trailingSlash: true,

  onBrokenLinks: 'throw',
  markdown: {
    hooks: {
      onBrokenMarkdownLinks: 'throw',
    },
  },

  // Browser-only modules: locale auto-detection and dropdown persistence.
  clientModules: ['./src/client/localePreference.js'],

  i18n: {
    defaultLocale: 'en',
    locales: ['en', 'zh'],
    localeConfigs: {
      en: {
        label: 'English',
        htmlLang: 'en',
      },
      zh: {
        label: '简体中文',
        htmlLang: 'zh-CN',
      },
    },
  },

  presets: [
    [
      'classic',
      /** @type {import('@docusaurus/preset-classic').Options} */
      ({
        docs: {
          path: '../docs',
          routeBasePath: 'docs',
          sidebarPath: './sidebars.js',
          editUrl:
            'https://github.com/memeloop-online/memeloop-workspace-control/tree/main/',
        },
        blog: false,
        theme: {
          customCss: './src/css/custom.css',
        },
      }),
    ],
  ],

  themeConfig:
    /** @type {import('@docusaurus/preset-classic').ThemeConfig} */
    ({
      colorMode: {
        defaultMode: 'light', // stable light fallback
        respectPrefersColorScheme: true,
        disableSwitch: false, // manual light/dark switch stays enabled
      },
      navbar: {
        title: 'MWC',
        items: [
          { to: '/', label: 'Home', position: 'left' },
          { to: '/features', label: 'Features', position: 'left' },
          { to: '/preview', label: 'Preview', position: 'left' },
          {
            type: 'doc',
            docId: 'intro',
            label: 'Documentation',
            position: 'left',
          },
          {
            type: 'doc',
            docId: 'api',
            label: 'API',
            position: 'left',
          },
          {
            type: 'doc',
            docId: 'plugin-development',
            label: 'Plugins',
            position: 'left',
          },
          { type: 'localeDropdown', position: 'right' },
          {
            href: 'https://github.com/memeloop-online/memeloop-workspace-control',
            label: 'GitHub',
            position: 'right',
          },
        ],
      },
      footer: {
        style: 'dark',
        links: [
          {
            title: 'Docs',
            items: [
              { label: 'Quick start', to: '/docs/quickstart' },
              { label: 'Workspaces', to: '/docs/workspaces' },
              { label: 'Access', to: '/docs/access' },
            ],
          },
          {
            title: 'Platform',
            items: [
              { label: 'Administration', to: '/docs/administration' },
              { label: 'API reference', to: '/docs/api' },
              { label: 'Plugin development', to: '/docs/plugin-development' },
            ],
          },
          {
            title: 'More',
            items: [
              { label: 'Security model', to: '/docs/security-model' },
              { label: 'FAQ', to: '/docs/faq' },
              {
                label: 'GitHub',
                href: 'https://github.com/memeloop-online/memeloop-workspace-control',
              },
            ],
          },
        ],
        copyright: `Copyright © ${new Date().getFullYear()} Memeloop. Built with Docusaurus.`,
      },
      prism: {
        additionalLanguages: ['bash', 'json', 'rust'],
      },
    }),
};

module.exports = config;
