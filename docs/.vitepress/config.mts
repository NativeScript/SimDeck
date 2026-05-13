import { defineConfig } from "vitepress";

const repoName = "SimDeck";
const githubUrl = `https://github.com/NativeScript/${repoName}`;
const siteUrl = "https://simdeck.nativescript.org";

export default defineConfig({
  title: "SimDeck",
  description:
    "Stream, inspect, and automate iOS Simulators and Android emulators from a browser, CLI, or test.",
  lang: "en-US",
  cleanUrls: true,
  lastUpdated: true,

  head: [
    ["meta", { name: "theme-color", content: "#0a84ff" }],
    ["meta", { property: "og:type", content: "website" }],
    ["meta", { property: "og:title", content: "SimDeck" }],
    [
      "meta",
      {
        property: "og:description",
        content:
          "Stream, inspect, and automate iOS Simulators and Android emulators from a browser, CLI, or test.",
      },
    ],
    ["meta", { property: "og:url", content: `${siteUrl}/` }],
    ["link", { rel: "canonical", href: `${siteUrl}/` }],
  ],

  themeConfig: {
    siteTitle: "SimDeck",

    nav: [
      { text: "Guide", link: "/guide/", activeMatch: "/guide/" },
      { text: "CLI", link: "/cli/", activeMatch: "/cli/" },
      { text: "API", link: "/api/rest", activeMatch: "/api/" },
      {
        text: "Inspectors",
        link: "/inspector/",
        activeMatch: "/inspector/",
      },
      {
        text: "Extensions",
        link: "/extensions/vscode",
        activeMatch: "/extensions/",
      },
      {
        text: "0.1.5",
        items: [
          { text: "Changelog", link: `${githubUrl}/releases` },
          { text: "Contributing", link: "/contributing" },
        ],
      },
    ],

    sidebar: {
      "/guide/": [
        {
          text: "Start",
          items: [
            { text: "Overview", link: "/guide/" },
            { text: "Install", link: "/guide/installation" },
            { text: "Quick Start", link: "/guide/quick-start" },
          ],
        },
        {
          text: "Use",
          items: [
            { text: "Daemon", link: "/guide/daemon" },
            { text: "Video & Streaming", link: "/guide/video" },
            { text: "LAN Access", link: "/guide/lan-access" },
            { text: "Testing", link: "/guide/testing" },
            { text: "GitHub Actions", link: "/guide/github-actions" },
          ],
        },
        {
          text: "Reference",
          items: [
            { text: "Troubleshooting", link: "/guide/troubleshooting" },
            { text: "How It Works", link: "/guide/architecture" },
            { text: "Contributing", link: "/contributing" },
          ],
        },
      ],

      "/cli/": [
        {
          text: "CLI",
          items: [
            { text: "Overview", link: "/cli/" },
            { text: "Commands", link: "/cli/commands" },
            { text: "Flags", link: "/cli/flags" },
          ],
        },
      ],

      "/api/": [
        {
          text: "API",
          items: [
            { text: "REST", link: "/api/rest" },
            { text: "Health & Metrics", link: "/api/health" },
            { text: "Inspector Protocol", link: "/api/inspector-protocol" },
          ],
        },
      ],

      "/inspector/": [
        {
          text: "Inspectors",
          items: [
            { text: "Overview", link: "/inspector/" },
            { text: "Accessibility", link: "/inspector/accessibility" },
            { text: "Swift", link: "/inspector/swift" },
            { text: "NativeScript", link: "/inspector/nativescript" },
            { text: "React Native", link: "/inspector/react-native" },
            { text: "Flutter", link: "/inspector/flutter" },
          ],
        },
      ],

      "/extensions/": [
        {
          text: "Extensions",
          items: [
            { text: "VS Code", link: "/extensions/vscode" },
            { text: "Browser Client", link: "/extensions/browser-client" },
          ],
        },
      ],
    },

    socialLinks: [{ icon: "github", link: githubUrl }],

    editLink: {
      pattern: `${githubUrl}/edit/main/docs/:path`,
      text: "Edit this page on GitHub",
    },

    search: {
      provider: "local",
    },

    footer: {
      message: "Released under the Apache-2.0 License.",
      copyright: "Copyright (c) 2026 SimDeck contributors.",
    },

    outline: {
      level: [2, 3],
    },
  },
});
