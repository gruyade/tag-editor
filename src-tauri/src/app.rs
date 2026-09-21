//! アプリ層の配線（タスク 21）。
//!
//! Tauri ランタイムを起動し、コマンド境界（[`crate::commands`]）とフロントエンド
//! （静的アセット、`../frontend`）を結線する入口。ここは実行時にシステム WebView を
//! 必要とするため、ユニット/プロパティテスト（lib）からは呼び出さない。lib の
//! テストはコマンドアダプタ・サービス・純粋ロジックを直接検証する。
//!
//! 本モジュールが担う配線:
//!
//! 1. [`crate::commands::CancelRegistry`] を `tauri::State` として `manage` する
//!    （要件 17.7）。長時間コマンドは State を通じてキャンセルを観測する。
//! 2. [`TauriProgressEmitter`]（`AppHandle` を包み `emit` でフロントエンドへ
//!    進捗イベントを送出、要件 16.7, 17.6）を提供する。
//! 3. [`crate::commands::adapters`] のコマンドを [`tauri::generate_handler!`] に
//!    登録する（要件 16.1, 16.2）。
//! 4. フォルダ選択のため tauri-plugin-dialog を初期化する（要件 1.3）。

use crate::commands::progress::ProgressEmitter;
use crate::commands::CancelRegistry;
use crate::models::Progress;

/// 進捗イベントをフロントエンドへ送出するイベント名。
///
/// フロントエンドは `window.__TAURI__.event.listen("inference://progress", ...)`
/// でこのイベントを購読する（タスク 21.2 のバッチ推論パネルで使用）。
pub const PROGRESS_EVENT: &str = "inference://progress";

/// `tauri::AppHandle` を包み、[`ProgressEmitter::emit`] で進捗をフロントエンドへ
/// 送出するエミッタ（要件 16.7, 17.6）。
///
/// 実 `AppHandle::emit` は実行中のアプリを要するため、この実装はアプリ層に置く。
/// 送出に失敗しても処理は継続させる（`emit` の戻り値は無視し、内部でエラーを
/// 握りつぶす）方針。これは進捗通知の失敗が長時間処理そのものを止めないため。
pub struct TauriProgressEmitter {
    app: tauri::AppHandle,
}

impl TauriProgressEmitter {
    /// `AppHandle` からエミッタを生成する。
    pub fn new(app: tauri::AppHandle) -> Self {
        Self { app }
    }
}

impl ProgressEmitter for TauriProgressEmitter {
    fn emit(&mut self, progress: Progress) {
        use tauri::Emitter;
        // 送出失敗は握りつぶす（進捗の欠落は処理継続を妨げない）。
        let _ = self.app.emit(PROGRESS_EVENT, progress);
    }
}

/// Tauri アプリを構築して起動する（実行時にシステム WebView を要する）。
///
/// - キャンセルレジストリを `manage` し、キャンセルコマンドから参照可能にする。
/// - フォルダ選択ダイアログプラグインを初期化する。
/// - 全コマンドアダプタを [`tauri::generate_handler!`] に登録する。
/// - `tauri.conf.json` の `frontendDist`（`../frontend`）配下の静的アセットを
///   メインウィンドウにロードする。
///
/// # パニック
///
/// アプリの起動に失敗した場合（WebView 初期化不可など）は panic する。
/// これはネイティブ起動の失敗を早期に検知するため（起動スモークはタスク 21.3）。
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(CancelRegistry::new())
        .invoke_handler(tauri::generate_handler![
            // FileService（要件 1, 2）
            crate::commands::adapters::list_images,
            crate::commands::adapters::get_thumbnail,
            crate::commands::adapters::get_preview,
            crate::commands::adapters::read_tag_file,
            crate::commands::adapters::write_tag_file,
            // TagService（要件 3, 6, 12）
            crate::commands::adapters::bulk_add_tags,
            crate::commands::adapters::bulk_remove_tags,
            crate::commands::adapters::dedup_tags,
            crate::commands::adapters::aggregate_tags,
            crate::commands::adapters::filter_images,
            // SortService（要件 7, 9, 10）
            crate::commands::adapters::sort_files,
            crate::commands::adapters::sort_by_size,
            crate::commands::adapters::sort_by_tag,
            // RenameService（要件 4, 8）
            crate::commands::adapters::rename_regex,
            crate::commands::adapters::normalize_caption_filenames,
            // CaptionService（要件 11）
            crate::commands::adapters::find_orphan_captions,
            crate::commands::adapters::delete_orphan_captions,
            // ModelService（要件 15）
            crate::commands::adapters::list_models,
            crate::commands::adapters::load_local_model,
            crate::commands::adapters::download_model,
            // PlatformService（要件 13, 16.6）
            crate::commands::adapters::capabilities,
            crate::commands::adapters::create_symlink,
            crate::commands::adapters::convert_path,
            // キャンセル（要件 17.7）
            crate::commands::adapters::cancel_operation,
        ])
        .run(tauri::generate_context!())
        .expect("error while running TagEditor application");
}
