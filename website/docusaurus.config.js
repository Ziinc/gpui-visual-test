// @ts-check
import { themes as prismThemes } from "prism-react-renderer";

/** @type {import('@docusaurus/types').Config} */
const config = {
  title: "gpui-visual-test",
  tagline: "Visual tests for GPUI, judged in plain English",
  favicon: "img/favicon.svg",
  url: "https://ziinc.github.io",
  baseUrl: process.env.BASE_URL ?? "/gpui-visual-test/",
  organizationName: "Ziinc",
  projectName: "gpui-visual-test",
  onBrokenLinks: "throw",
  i18n: { defaultLocale: "en", locales: ["en"] },
  presets: [
    [
      "classic",
      /** @type {import('@docusaurus/preset-classic').Options} */
      ({
        // A single page: no docs section, no blog.
        docs: false,
        blog: false,
        theme: { customCss: "./src/css/custom.css" },
      }),
    ],
  ],
  themeConfig:
    /** @type {import('@docusaurus/preset-classic').ThemeConfig} */
    ({
      colorMode: { respectPrefersColorScheme: true },
      navbar: {
        title: "gpui-visual-test",
        logo: { alt: "", src: "img/favicon.svg" },
        items: [
          { to: "/#quick-start", label: "Quick start", position: "left" },
          { to: "/#pixel-diff", label: "Pixel diff", position: "left" },
          { to: "/#judges", label: "Judges", position: "left" },
          { to: "/#agents", label: "Agents & CLI", position: "left" },
          { to: "/#integrations", label: "Integrations", position: "left" },
          { to: "/#reference", label: "Reference", position: "left" },
          { href: "https://github.com/Ziinc/gpui-visual-test", label: "GitHub", position: "right" },
        ],
      },
      footer: {
        style: "dark",
        copyright: "gpui-visual-test · Apache-2.0",
      },
      prism: {
        theme: prismThemes.github,
        darkTheme: prismThemes.dracula,
        additionalLanguages: ["rust", "toml", "bash", "json", "elixir"],
      },
    }),
};

export default config;
