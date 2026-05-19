import { promises as fs } from "node:fs";
import { dirname, join } from "node:path";
import { createRequire } from "node:module";
import satori from "satori";
import { Resvg } from "@resvg/resvg-js";

const require = createRequire(import.meta.url);

export type OgPageInfo = {
  title: string;
  description?: string;
  category?: string;
};

const WIDTH = 1200;
const HEIGHT = 630;

const BRAND_BLUE = "#0a84ff";
const BRAND_BLUE_SOFT = "#e6f2ff";
const TEXT_PRIMARY = "#0f172a";
const TEXT_SECONDARY = "#475569";
const TEXT_MUTED = "#94a3b8";
const BACKGROUND = "#ffffff";
const FOOTER_BORDER = "#e2e8f0";

const SITE_HOSTNAME = "simdeck.nativescript.org";

type SatoriElement = {
  type: string;
  key?: string | number | null;
  props: {
    style?: Record<string, unknown>;
    children?: SatoriChildren;
    [key: string]: unknown;
  };
};

type SatoriChildren =
  | string
  | number
  | SatoriElement
  | null
  | undefined
  | false
  | Array<SatoriChildren>;

function el(
  type: string,
  props: SatoriElement["props"] = {},
  children?: SatoriChildren,
): SatoriElement {
  return {
    type,
    key: null,
    props: { ...props, children: children ?? props.children },
  };
}

let fontsPromise:
  | Promise<
      Array<{ name: string; data: Buffer; weight: 400 | 700; style: "normal" }>
    >
  | undefined;

async function loadFonts() {
  if (!fontsPromise) {
    fontsPromise = (async () => {
      const regularPath =
        require.resolve("@fontsource/inter/files/inter-latin-400-normal.woff");
      const boldPath =
        require.resolve("@fontsource/inter/files/inter-latin-700-normal.woff");
      const [regular, bold] = await Promise.all([
        fs.readFile(regularPath),
        fs.readFile(boldPath),
      ]);
      return [
        {
          name: "Inter",
          data: regular,
          weight: 400 as const,
          style: "normal" as const,
        },
        {
          name: "Inter",
          data: bold,
          weight: 700 as const,
          style: "normal" as const,
        },
      ];
    })();
  }
  return fontsPromise;
}

function buildTemplate({
  title,
  description,
  category,
}: OgPageInfo): SatoriElement {
  const topRowChildren: SatoriChildren = [
    el("div", { style: { display: "flex", alignItems: "center" } }, [
      el("div", {
        style: {
          width: "44px",
          height: "44px",
          borderRadius: "10px",
          background: `linear-gradient(135deg, ${BRAND_BLUE} 0%, #4ba3ff 100%)`,
          marginRight: "18px",
          display: "flex",
        },
      }),
      el(
        "div",
        {
          style: {
            fontSize: "34px",
            fontWeight: 700,
            color: TEXT_PRIMARY,
            letterSpacing: "-0.02em",
          },
        },
        "SimDeck",
      ),
    ]),
  ];

  if (category) {
    (topRowChildren as Array<SatoriChildren>).push(
      el(
        "div",
        {
          style: {
            fontSize: "18px",
            fontWeight: 600,
            color: BRAND_BLUE,
            letterSpacing: "0.1em",
            textTransform: "uppercase",
            padding: "10px 20px",
            borderRadius: "999px",
            backgroundColor: BRAND_BLUE_SOFT,
            display: "flex",
          },
        },
        category,
      ),
    );
  }

  const middleChildren: Array<SatoriChildren> = [
    el(
      "div",
      {
        style: {
          fontSize: title.length > 32 ? "68px" : "84px",
          fontWeight: 700,
          color: TEXT_PRIMARY,
          letterSpacing: "-0.025em",
          lineHeight: 1.05,
          maxWidth: "1040px",
          display: "block",
          // satori clamps text overflow with lineClamp
          // @ts-expect-error - satori-specific
          lineClamp: 2,
        },
      },
      title,
    ),
  ];

  if (description) {
    middleChildren.push(
      el(
        "div",
        {
          style: {
            fontSize: "30px",
            fontWeight: 400,
            color: TEXT_SECONDARY,
            lineHeight: 1.4,
            marginTop: "28px",
            maxWidth: "1040px",
            display: "block",
            // @ts-expect-error - satori-specific
            lineClamp: 3,
          },
        },
        description,
      ),
    );
  }

  return el(
    "div",
    {
      style: {
        width: "100%",
        height: "100%",
        display: "flex",
        flexDirection: "column",
        backgroundColor: BACKGROUND,
        padding: "72px 80px",
        fontFamily: "Inter",
        position: "relative",
      },
    },
    [
      el(
        "div",
        {
          style: {
            display: "flex",
            alignItems: "center",
            justifyContent: "space-between",
            width: "100%",
          },
        },
        topRowChildren,
      ),
      el("div", { style: { flex: 1, display: "flex" } }),
      el(
        "div",
        { style: { display: "flex", flexDirection: "column" } },
        middleChildren,
      ),
      el(
        "div",
        {
          style: {
            display: "flex",
            alignItems: "center",
            marginTop: "56px",
            paddingTop: "32px",
            borderTop: `2px solid ${FOOTER_BORDER}`,
            fontSize: "24px",
            color: TEXT_MUTED,
            fontWeight: 500,
            letterSpacing: "-0.005em",
          },
        },
        SITE_HOSTNAME,
      ),
    ],
  );
}

export async function renderOgImage(info: OgPageInfo): Promise<Buffer> {
  const fonts = await loadFonts();
  const svg = await satori(buildTemplate(info) as unknown as never, {
    width: WIDTH,
    height: HEIGHT,
    fonts,
  });
  const resvg = new Resvg(svg, {
    fitTo: { mode: "width", value: WIDTH },
  });
  return resvg.render().asPng();
}

export async function writeOgImage(
  info: OgPageInfo,
  outPath: string,
): Promise<void> {
  const png = await renderOgImage(info);
  await fs.mkdir(dirname(outPath), { recursive: true });
  await fs.writeFile(outPath, png);
}

/**
 * Map a page's relative source path (e.g. `guide/installation.md`) to:
 * - `slug`: stable filename for the generated PNG (e.g. `guide-installation`)
 * - `urlPath`: site-relative URL (e.g. `/guide/installation`) honoring cleanUrls
 * - `category`: human label for the top-level section, or undefined for root
 */
export function describePage(relativePath: string): {
  slug: string;
  urlPath: string;
  category?: string;
} {
  const noExt = relativePath.replace(/\.md$/, "");
  const segments = noExt.split("/").filter(Boolean);

  let slug: string;
  let urlPath: string;
  if (noExt === "index") {
    slug = "index";
    urlPath = "/";
  } else if (segments[segments.length - 1] === "index") {
    const dir = segments.slice(0, -1).join("/");
    slug = dir.replace(/\//g, "-");
    urlPath = `/${dir}/`;
  } else {
    slug = segments.join("-");
    urlPath = `/${segments.join("/")}`;
  }

  const category = categoryFor(segments);
  return { slug, urlPath, category };
}

function categoryFor(segments: string[]): string | undefined {
  if (segments.length === 0 || segments[0] === "index") return undefined;
  const top = segments[0];
  switch (top) {
    case "guide":
      return "Guide";
    case "cli":
      return "CLI";
    case "api":
      return "API";
    case "inspector":
      return "Inspectors";
    case "extensions":
      return "Extensions";
    case "contributing":
      return "Contributing";
    default:
      return undefined;
  }
}

export function ogImageRelPath(slug: string): string {
  return `og-images/${slug}.png`;
}

export function ogImageOutPath(outDir: string, slug: string): string {
  return join(outDir, "og-images", `${slug}.png`);
}
