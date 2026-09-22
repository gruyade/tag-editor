import type { Page } from "@playwright/test";

/**
 * frontend/main.js が参照する window.__TAURI__（core/dialog/event/opener）の
 * モックをページへ注入する。実 Tauri ランタイムを起動せず、フロントのロジック
 * （DOM操作・ボタンのイベントハンドラ・invoke呼び出しの発火）だけを検証する。
 *
 * invoke/openPath 呼び出しはすべて window.__calls__ へ記録される。テストは
 * page.evaluate(() => window.__calls__) で呼び出し履歴を検証できる。
 */
export async function installTauriMock(page: Page): Promise<void> {
  await page.addInitScript(() => {
    (window as any).__calls__ = {
      invoke: [] as { command: string; args: unknown }[],
      openPath: [] as string[],
      dialogOpen: [] as unknown[],
      listen: [] as string[],
    };

    // invoke へ渡すコマンドごとの戻り値を差し替えられるようにする。
    // 未設定のコマンドは既定値（空配列/空オブジェクト）を返す。
    (window as any).__invokeResponses__ = {};

    const defaultResponseFor = (command: string): unknown => {
      switch (command) {
        case "list_catalog":
          return { variants: [], excluded: [], model_dir_present: false };
        case "get_model_dir_path":
          return "C:\\mock\\app_data\\models";
        case "list_images":
          return { items: [], truncated: false, total: 0 };
        default:
          return {};
      }
    };

    (window as any).__TAURI__ = {
      core: {
        invoke: async (command: string, args?: unknown) => {
          (window as any).__calls__.invoke.push({ command, args });
          const responses = (window as any).__invokeResponses__;
          if (command in responses) {
            const entry = responses[command];
            if (entry && entry.throw) {
              throw entry.value;
            }
            return entry.value;
          }
          return defaultResponseFor(command);
        },
        convertFileSrc: (path: string) => `mock://${path}`,
      },
      dialog: {
        open: async (opts: unknown) => {
          (window as any).__calls__.dialogOpen.push(opts);
          return null;
        },
      },
      event: {
        listen: async (eventName: string, _handler: (ev: unknown) => void) => {
          (window as any).__calls__.listen.push(eventName);
          // unlisten 関数（main.js が保持するだけで実際には呼ばない前提でも安全に動く）。
          return () => {};
        },
      },
      opener: {
        openPath: async (path: string) => {
          (window as any).__calls__.openPath.push(path);
          return undefined;
        },
      },
    };
  });
}

/** 指定コマンドの invoke 戻り値（または例外）をテスト側から設定する。 */
export async function mockInvokeResponse(
  page: Page,
  command: string,
  value: unknown,
  opts: { throw?: boolean } = {}
): Promise<void> {
  await page.evaluate(
    ({ command, value, shouldThrow }) => {
      (window as any).__invokeResponses__[command] = { value, throw: shouldThrow };
    },
    { command, value, shouldThrow: !!opts.throw }
  );
}

/** 記録された invoke 呼び出し履歴を取得する。 */
export async function getInvokeCalls(
  page: Page
): Promise<{ command: string; args: unknown }[]> {
  return page.evaluate(() => (window as any).__calls__.invoke);
}

/** 記録された openPath 呼び出し履歴を取得する。 */
export async function getOpenPathCalls(page: Page): Promise<string[]> {
  return page.evaluate(() => (window as any).__calls__.openPath);
}
