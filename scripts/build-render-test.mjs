import { build } from "esbuild";
await build({
  entryPoints: ["scripts/render-test.tsx"],
  bundle: true,
  platform: "node",
  outfile: "/tmp/render-test.cjs",
  jsx: "automatic",
  loader: { ".tsx": "tsx" },
});
console.log("bundled");
