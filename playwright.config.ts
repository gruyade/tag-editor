import { defineConfig } from "@playwright/test";

// frontend/main.js は window.__TAURI__（core/dialog/event/opener）に依存する。
// 実 Tauri ランタイムを起動せずロジックを検証するため、各テストで
// addInitScript によりモックした window.__TAURI__ を注入した上で
// frontend/index.html を file:// で直接開く（ビルド不要の静的アセットのため）。
export default defineConfig({
  testDir: "./tests/gui",
  timeout: 30_000,
  fullyParallel: true,
  reporter: [["list"]],
  use: {
    trace: "retain-on-failure",
  },
});
