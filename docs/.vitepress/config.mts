import { defineConfig } from "vitepress";
import {
  describePage,
  ogImageOutPath,
  ogImageRelPath,
  writeOgImage,
  type OgPageInfo,
} from "./og-image.mts";

const repoName = "SimDeck";
const githubUrl = `https://github.com/NativeScript/${repoName}`;
const siteUrl = "https://simdeck.sh";
const homeOgImageRelPath = "images/codex-screenshot.png";
const homeOgImageWidth = "3216";
const homeOgImageHeight = "2090";

type CollectedPage = OgPageInfo & {
  slug: string;
  urlPath: string;
};

const collectedPages = new Map<string, CollectedPage>();

export default defineConfig({
  title: "SimDeck",
  description:
    "Stream, inspect, and automate iOS Simulators and Android emulators from a browser, CLI, or test.",
  lang: "en-US",
  cleanUrls: true,
  lastUpdated: true,
  srcExclude: ["**/public/**"],

  head: [
    ["meta", { name: "theme-color", content: "#0a84ff" }],
    ["meta", { property: "og:type", content: "website" }],
    ["meta", { property: "og:site_name", content: "SimDeck" }],
    ["meta", { name: "twitter:card", content: "summary_large_image" }],
    ["meta", { name: "twitter:site", content: "@NativeScript" }],
  ],

  transformPageData(pageData) {
    if (pageData.relativePath === "404.md") return;

    const { slug, urlPath, category } = describePage(pageData.relativePath);

    const isHome = pageData.frontmatter.layout === "home";
    const hero = (pageData.frontmatter.hero ?? {}) as {
      name?: string;
      text?: string;
      tagline?: string;
    };

    const title = isHome
      ? hero.text || hero.name || "SimDeck"
      : pageData.title || "SimDeck";

    const description =
      (pageData.frontmatter.description as string | undefined) ||
      (isHome ? hero.tagline : undefined) ||
      pageData.description ||
      "Stream, inspect, and automate iOS Simulators and Android emulators from a browser, CLI, or test.";

    if (!isHome) {
      collectedPages.set(slug, {
        slug,
        urlPath,
        title,
        description,
        category,
      });
    }

    const pageUrl = `${siteUrl}${urlPath}`;
    const imageRelPath = isHome ? homeOgImageRelPath : ogImageRelPath(slug);
    const imageUrl = `${siteUrl}/${imageRelPath}`;
    const imageWidth = isHome ? homeOgImageWidth : "1200";
    const imageHeight = isHome ? homeOgImageHeight : "630";

    pageData.frontmatter.head ??= [];
    pageData.frontmatter.head.push(
      ["link", { rel: "canonical", href: pageUrl }],
      ["meta", { property: "og:title", content: title }],
      ["meta", { property: "og:description", content: description }],
      ["meta", { property: "og:url", content: pageUrl }],
      ["meta", { property: "og:image", content: imageUrl }],
      ["meta", { property: "og:image:width", content: imageWidth }],
      ["meta", { property: "og:image:height", content: imageHeight }],
      ["meta", { property: "og:image:alt", content: title }],
      ["meta", { name: "twitter:title", content: title }],
      ["meta", { name: "twitter:description", content: description }],
      ["meta", { name: "twitter:image", content: imageUrl }],
    );
  },

  async buildEnd(siteConfig) {
    const outDir = siteConfig.outDir;
    if (collectedPages.size === 0) {
      console.warn("[og-image] No pages collected; skipping image generation.");
      return;
    }

    const tasks: Array<Promise<void>> = [];
    for (const page of collectedPages.values()) {
      tasks.push(
        writeOgImage(
          {
            title: page.title,
            description: page.description,
            category: page.category,
          },
          ogImageOutPath(outDir, page.slug),
        ),
      );
    }
    await Promise.all(tasks);
    console.log(`[og-image] Generated ${tasks.length} social card images.`);
  },

  themeConfig: {
    siteTitle: "SimDeck",

    nav: [
      { text: "Guide", link: "/guide/", activeMatch: "/guide/" },
      { text: "CLI", link: "/cli/", activeMatch: "/cli/" },
      {
        text: "Inspectors",
        link: "/inspector/",
        activeMatch: "/inspector/",
      },
      { text: "Support", link: "/support" },
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
            { text: "Quick start", link: "/guide/quick-start" },
          ],
        },
        {
          text: "Use",
          items: [
            { text: "Daemon", link: "/guide/daemon" },
            { text: "Video and streaming", link: "/guide/video" },
            { text: "LAN access", link: "/guide/lan-access" },
            { text: "Testing", link: "/guide/testing" },
            { text: "GitHub Actions", link: "/guide/github-actions" },
          ],
        },
        {
          text: "Reference",
          items: [
            { text: "Troubleshooting", link: "/guide/troubleshooting" },
            { text: "How it works", link: "/guide/architecture" },
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
