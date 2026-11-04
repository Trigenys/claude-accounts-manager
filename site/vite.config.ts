import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";
import { defineConfig, type Plugin } from "vite";

const phosphorDir = join(dirname(fileURLToPath(import.meta.url)), "node_modules", "@phosphor-icons", "core", "assets", "light");

/** Inlines Phosphor (light) icons at build time: `<i data-icon="bell"></i>` becomes the SVG, no runtime JS. */
function phosphorInline(): Plugin {
  return {
    name: "phosphor-inline",
    transformIndexHtml(html) {
      return html.replace(/<i data-icon="([a-z-]+)"><\/i>/g, (_, name: string) => {
        const svg = readFileSync(join(phosphorDir, `${name}-light.svg`), "utf8");
        return svg.replace("<svg ", '<svg class="icon" aria-hidden="true" focusable="false" ');
      });
    },
  };
}

export default defineConfig({
  plugins: [phosphorInline()],
  build: { target: "es2022", assetsInlineLimit: 0 },
});
