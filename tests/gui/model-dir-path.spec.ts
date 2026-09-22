import { test, expect } from "@playwright/test";
import path from "path";
import { installTauriMock, mockInvokeResponse, getInvokeCalls } from "./tauri-mock";

const indexPath = "file://" + path.resolve(__dirname, "../../frontend/index.html");

test.describe("モデル管理タブ: 保存先パスのテキスト表示", () => {
  test.beforeEach(async ({ page }) => {
    await installTauriMock(page);
  });

  test("モデル管理タブを開くと保存先パスが取得・表示される", async ({ page }) => {
    await page.goto(indexPath);
    await mockInvokeResponse(page, "get_model_dir_path", "C:\\mock\\app_data\\models");

    // 起動時の refreshModels 呼び出しでは beforeEach 前のモック未設定応答
    // （既定値）が使われるため、一覧更新ボタンで再取得させる。
    await page.locator('.ops-tab[data-tab="model"]').click();
    await page.locator("#model-refresh").click();

    await expect(page.locator("#model-dir-path")).toHaveText("C:\\mock\\app_data\\models");

    const calls = await getInvokeCalls(page);
    expect(calls.some((c) => c.command === "get_model_dir_path")).toBeTruthy();
  });

  test("開くボタンは存在しない（ボタン機能は廃止）", async ({ page }) => {
    await page.goto(indexPath);
    await page.locator('.ops-tab[data-tab="model"]').click();
    await expect(page.locator("#model-open-dir")).toHaveCount(0);
  });

  test("パス取得に失敗した場合は取得不可の旨を表示する", async ({ page }) => {
    await page.goto(indexPath);
    await mockInvokeResponse(
      page,
      "get_model_dir_path",
      { kind: "Io", message: "取得失敗" },
      { throw: true }
    );

    await page.locator('.ops-tab[data-tab="model"]').click();
    await page.locator("#model-refresh").click();

    await expect(page.locator("#model-dir-path")).toHaveText("（取得できません）");
  });
});
