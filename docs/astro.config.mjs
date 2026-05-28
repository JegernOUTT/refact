import { defineConfig } from 'astro/config';
import starlight from '@astrojs/starlight';

const site = 'https://docs.refact.ai/';

export default defineConfig({
  site,
  integrations: [
    starlight({
      title: 'Refact Documentation',
      components: {
        Search: './src/components/Search.astro',
        Head: './src/components/Head.astro',
      },
      logo: {
        light: '/src/assets/logo-light.svg',
        dark: '/src/assets/logo-dark.svg',
        replacesTitle: true,
      },
      social: {
        github: 'https://github.com/smallcloudai',
      },
      head: [
        {
          tag: 'meta',
          attrs: { property: 'og:image', content: site + 'og.jpg' },
        },
        {
          tag: 'meta',
          attrs: { property: 'twitter:image', content: site + 'og.jpg' },
        },
      ],
      sidebar: [
        {
          label: 'Start Here',
          items: [
            {
              label: 'Overview',
              link: '/',
              attrs: {
                'aria-label': 'Refact documentation overview',
              },
            },
            {
              label: 'Quickstart',
              link: '/introduction/quickstart/',
              attrs: {
                'aria-label': 'Get started with Refact',
              },
            },
            {
              label: 'Installation Overview',
              link: '/installation/installation-hub/',
              attrs: {
                'aria-label': 'Browse Refact installation options',
              },
            },
            {
              label: 'VS Code',
              link: '/installation/vs-code/',
              attrs: {
                'aria-label': 'Install Refact for VS Code',
              },
            },
            {
              label: 'JetBrains IDEs',
              link: '/installation/jetbrains/',
              attrs: {
                'aria-label': 'Install Refact for JetBrains IDEs',
              },
            },
            {
              label: 'Configure Providers',
              link: '/byok/',
              attrs: {
                'aria-label': 'Configure Refact providers',
              },
            },
            {
              label: 'Supported Models',
              link: '/supported-models/',
              attrs: {
                'aria-label': 'Understand supported Refact models',
              },
            },
            {
              label: 'Privacy',
              link: '/privacy/',
              attrs: {
                'aria-label': 'Understand Refact privacy',
              },
            },
          ],
        },
        {
          label: 'Core Features',
          collapsed: true,
          items: [
            {
              label: 'AI Chat',
              link: '/features/ai-chat/',
              attrs: {
                'aria-label': 'Learn about AI Chat',
              },
            },
            {
              label: 'Autonomous Agent',
              collapsed: true,
              items: [
                { label: 'Getting Started', link: '/features/autonomous-agent/getting-started/' },
                { label: 'Overview', link: '/features/autonomous-agent/overview/' },
                {
                  label: 'Tools',
                  link: '/features/autonomous-agent/tools/',
                  attrs: {
                    'aria-label': 'Learn about agent tools',
                  },
                },
                {
                  label: 'Rollback',
                  link: '/features/autonomous-agent/rollback/',
                  attrs: {
                    'aria-label': 'Learn about agent rollback',
                  },
                },
                {
                  label: 'Worktrees',
                  link: '/features/autonomous-agent/worktrees/',
                  attrs: {
                    'aria-label': 'Learn about agent worktrees',
                  },
                },
              ],
            },
            {
              label: 'Code Completion',
              link: '/features/code-completion/',
              attrs: {
                'aria-label': 'Learn about code completion',
              },
            },
            {
              label: 'Hidden Roles',
              link: '/roles/',
              attrs: {
                'aria-label': 'Understand hidden roles and plans',
              },
            },
            {
              label: 'Scheduler',
              link: '/scheduler/',
              attrs: {
                'aria-label': 'Schedule agent prompts with cron',
              },
            },
            {
              label: 'Processes and PTY',
              link: '/processes/',
              attrs: {
                'aria-label': 'Manage background processes and PTY sessions',
              },
            },
            {
              label: 'Context',
              link: '/features/context/',
              attrs: {
                'aria-label': 'Understand context features',
              },
            },
            {
              label: 'AI Toolbox',
              link: '/features/ai-toolbox/',
              attrs: {
                'aria-label': 'Explore AI Toolbox features',
              },
            },
          ],
        },
        {
          label: 'Agent Integrations',
          collapsed: true,
          items: [
            { label: 'Overview', link: '/features/autonomous-agent/integrations/' },
            { label: 'Chrome', link: '/features/autonomous-agent/integrations/chrome/' },
            { label: 'Shell Commands', link: '/features/autonomous-agent/integrations/shell-commands/' },
            { label: 'Command Line Tool', link: '/features/autonomous-agent/integrations/command-line-tool/' },
            { label: 'Command Line Service', link: '/features/autonomous-agent/integrations/command-line-service/' },
            {
              label: 'MCP',
              link: '/features/autonomous-agent/integrations/mcp/',
              attrs: {
                'aria-label': 'Connect to Model Context Protocol servers',
              },
            },
            { label: 'GitHub', link: '/features/autonomous-agent/integrations/github/' },
            { label: 'GitLab', link: '/features/autonomous-agent/integrations/gitlab/' },
            { label: 'Bitbucket', link: '/features/autonomous-agent/integrations/bitbucket/' },
            { label: 'PostgreSQL', link: '/features/autonomous-agent/integrations/postgresql/' },
            { label: 'MySQL', link: '/features/autonomous-agent/integrations/mysql/' },
            { label: 'PDB', link: '/features/autonomous-agent/integrations/pdb/' },
          ],
        },
        {
          label: 'Guides',
          collapsed: true,
          items: [
            {
              label: 'JetBrains Troubleshooting',
              link: '/guides/plugins/jetbrains/troubleshooting/',
              attrs: {
                'aria-label': 'JetBrains IDEs troubleshooting guide',
              },
            },
          ],
        },
        {
          label: 'FAQ',
          link: '/faq/',
          attrs: {
            'aria-label': 'Frequently asked questions',
          },
        },
        {
          label: 'Contributing',
          link: '/contributing/',
          attrs: {
            'aria-label': 'Learn how to contribute to Refact',
          },
        },
      ],
      customCss: ['./src/styles/index.css'],
      editLink: {
        baseUrl: 'https://github.com/smallcloudai/refact/edit/main/docs/',
      },
      lastUpdated: true,
    }),
  ],
});
