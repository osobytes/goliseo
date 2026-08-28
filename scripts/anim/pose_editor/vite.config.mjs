const THREE = "/home/oscar/Coding/galactic-cup/ts/node_modules/.pnpm/three@0.180.0/node_modules/three";
export default {
  root: "/home/oscar/Coding/galactic-cup/scripts/anim/pose_editor",
  build: {
    outDir: "dist",
    emptyOutDir: true,
    minify: false,
    lib: { entry: "src/main.js", name: "PoseEditor", formats: ["iife"], fileName: () => "editor.js" },
  },
  resolve: {
    alias: [
      { find: /^three-addons\//, replacement: THREE + "/examples/jsm/" },
      { find: /^three$/, replacement: THREE + "/build/three.module.js" },
    ],
  },
};
