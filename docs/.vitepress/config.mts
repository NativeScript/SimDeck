import { defineConfig } from "vitepress";

const repoName = "xcode-canvas-web";
const githubUrl = `https://github.com/DjDeveloperr/${repoName}`;

export default defineConfig({
  title: "Simdeck",
  description:
    "A local-first iOS Simulator control plane with a browser UI, REST API, and WebTransport video.",
  lang: "en-US",
  cleanUrls: true,
  lastUpdated: true,
  base: `/${repoName}/`,

  head: [
    ["meta", { name: "theme-color", content: "#0a84ff" }],
    ["meta", { property: "og:type", content: "website" }],
    ["meta", { property: "og:title", content: "Simdeck" }],
    [
      "meta",
      {
        property: "og:description",
        content:
          "A local iOS Simulator control plane with a browser UI, REST API, and WebTransport video.",
      },
    ],
    [
      "meta",
      {
        property: "og:url",
        content: `https://djdeveloperr.github.io/${repoName}/`,
      },
    ],
  ],

  themeConfig: {
    siteTitle: "Simdeck",

    nav: [
      { text: "Guide", link: "/guide/", activeMatch: "/guide/" },
      { text: "CLI", link: "/cli/", activeMatch: "/cli/" },
      { text: "API", link: "/api/rest", activeMatch: "/api/" },
      {
        text: "Inspector",
        link: "/inspector/",
        activeMatch: "/inspector/",
      },
      {
        text: "Extensions",
        link: "/extensions/vscode",
        activeMatch: "/extensions/",
      },
      {
        text: "0.1.0",
        items: [
          {
            text: "Changelog",
            link: `${githubUrl}/releases`,
          },
          {
            text: "Contributing",
            link: "/contributing",
          },
        ],
      },
    ],

    sidebar: {
      "/guide/": [
        {
          text: "Getting Started",
          items: [
            { text: "Introduction", link: "/guide/" },
            { text: "Installation", link: "/guide/installation" },
            { text: "Quick Start", link: "/guide/quick-start" },
          ],
        },
        {
          text: "Concepts",
          items: [
            { text: "Architecture", link: "/guide/architecture" },
            { text: "Video Pipeline", link: "/guide/video" },
            { text: "LAN Access", link: "/guide/lan-access" },
            { text: "Background Service", link: "/guide/service" },
          ],
        },
        {
          text: "Operating Simdeck",
          items: [
            { text: "Troubleshooting", link: "/guide/troubleshooting" },
            { text: "Contributing", link: "/contributing" },
          ],
        },
      ],

      "/cli/": [
        {
          text: "CLI",
          items: [
            { text: "Overview", link: "/cli/" },
            { text: "Command Reference", link: "/cli/commands" },
            { text: "Flags & Options", link: "/cli/flags" },
          ],
        },
      ],

      "/api/": [
        {
          text: "HTTP API",
          items: [
            { text: "REST Endpoints", link: "/api/rest" },
            { text: "Health & Metrics", link: "/api/health" },
          ],
        },
        {
          text: "Live Video",
          items: [
            { text: "WebTransport", link: "/api/webtransport" },
            { text: "Packet Format", link: "/api/packet-format" },
          ],
        },
        {
          text: "Inspectors",
          items: [
            { text: "Inspector Protocol", link: "/api/inspector-protocol" },
          ],
        },
      ],

      "/inspector/": [
        {
          text: "Inspector",
          items: [
            { text: "Overview", link: "/inspector/" },
            { text: "Accessibility (AXe)", link: "/inspector/accessibility" },
            {
              text: "Swift In-App Agent",
              link: "/inspector/swift",
            },
            {
              text: "NativeScript Runtime",
              link: "/inspector/nativescript",
            },
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
      copyright: `Copyright © 2026 Simdeck contributors.`,
    },

    outline: {
      level: [2, 3],
    },
  },
});
