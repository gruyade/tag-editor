//! Tauri コマンドアダプタ（要件 16.1, 16.2, 16.7, 16.8, 17.6, 17.7）。
//!
//! 各サービスへ薄く委譲する `#[tauri::command]` 関数群。すべて
//! `Result<T, AppError>` を返し、`T`・[`AppError`] とも `serde::Serialize` を
//! 実装するため、Tauri は戻り値をそのまま JSON DTO としてフロントエンドへ渡す
//! （要件 16.1, 16.2）。
//!
//! # テスト境界について
//!
//! 実 `tauri::Builder::run` と `AppHandle::emit` は実行中のアプリ＋設定
//! （タスク 21 のバイナリ配線）を要する。本モジュールのコマンド関数は
//! **関数レベルで単体テスト可能**な薄いアダプタとして定義し、実 Tauri ランタイム
//! を起動せずに委譲の正しさを検証する。`#[tauri::command]` 属性はコマンド生成の
//! ためのラッパーを付与するだけで、関数本体は通常の Rust 関数として直接呼び出せる。
//!
//! アプリ層では [`tauri::generate_handler!`] にこれらのコマンドを登録し、
//! [`super::cancel::CancelRegistry`] を `tauri::State` として管理する
//! （タスク 21）。
//!
//! # 長時間処理の spawn とキャンセル・進捗
//!
//! バッチ推論・大量仕訳・ダウンロードなどの長時間処理は、UI スレッドを塞がない
//! よう別スレッド/タスクで実行する（要件 16.8）。本モジュールは Tauri 非依存で
//! テスト可能な同期コア [`run_inference_job`] を提供し、これを
//! [`spawn_inference_job`] が [`std::thread`] 上で起動する。進捗は
//! [`super::progress::ProgressEmitter`]、キャンセルは
//! [`super::cancel::CancelRegistry`] 経由で配線する（要件 16.7, 17.6, 17.7）。

use std::path::Path;
use std::sync::Arc;

use crate::app::{ModelSessionState, TauriProgressEmitter};
use crate::error::{AppError, AppResult};
use crate::models::{ImageEntry, ModelVariant, OperationReport, SortingOperation};
use crate::services::inference_service::OwnedOrtRunner;
use crate::services::{
    caption_service, file_service, model_service, platform_service, rename_service, sort_service,
    tag_file, tag_service,
};

use super::cancel::CancelRegistry;
use super::progress::ProgressEmitter;

// ---------------------------------------------------------------------------
// FileService（要件 1, 2）
// ---------------------------------------------------------------------------

/// 選択フォルダ直下の Image_File を列挙する（`file_service::list_images` へ委譲）。
#[tauri::command]
pub fn list_images(folder: String) -> AppResult<file_service::ImageListing> {
    file_service::list_images(folder)
}

/// サムネイルを生成する（`file_service::get_thumbnail` へ委譲、要件 1.3, 1.4）。
///
/// 生成失敗はエラーにせず `placeholder=true` の [`file_service::ThumbnailData`] を
/// 返すため、戻り値は `Result` ではなく成功で包む。
#[tauri::command]
pub fn get_thumbnail(path: String, size: u32) -> AppResult<file_service::ThumbnailData> {
    Ok(file_service::get_thumbnail(path, size))
}

/// 拡大プレビューを生成する（`file_service::get_preview` へ委譲、要件 1.5）。
#[tauri::command]
pub fn get_preview(path: String) -> AppResult<file_service::PreviewData> {
    Ok(file_service::get_preview(path))
}

/// サムネイルの軽量転送経路（`file_service::get_thumbnail_path` へ委譲、
/// 要件 2.10, 2.11, 2.12、タスク 3.7）。
///
/// PNG バイト列を JSON 数値配列で転送する [`get_thumbnail`] とは異なり、
/// 縮小 PNG をアプリキャッシュディレクトリへ書き出したファイルパスを返す。
/// フロントエンドはこのパスを `convertFileSrc`（asset protocol）で
/// `<img src>` に直接割り当てられる。生成失敗はエラーにせず
/// `placeholder=true` の [`file_service::ImagePathData`] を返す（保持 3.7）。
/// 既存 [`get_thumbnail`] とその DTO は削除しない（保持 3.10）。
#[tauri::command]
pub fn get_thumbnail_path(path: String, size: u32) -> AppResult<file_service::ImagePathData> {
    Ok(file_service::get_thumbnail_path(path, size))
}

/// プレビューの軽量転送経路（`file_service::get_preview_path` へ委譲、
/// 要件 2.10, 2.11, 2.12、タスク 3.7）。
///
/// 元画像が [`file_service::MAX_PREVIEW_SIZE`] 以下なら元ファイルのパスを
/// そのまま返し（縮小・再エンコード不要、保持 3.9）、超過時のみ縮小 PNG を
/// キャッシュへ書き出してそのパスを返す。既存 [`get_preview`] とその DTO は
/// 削除しない（保持 3.10）。
#[tauri::command]
pub fn get_preview_path(path: String) -> AppResult<file_service::ImagePathData> {
    Ok(file_service::get_preview_path(path))
}

/// 同名 Tag_File を読み込む（`tag_file::read_tag_file` へ委譲、要件 2.1）。
#[tauri::command]
pub fn read_tag_file(image_path: String) -> AppResult<tag_file::TagFileContent> {
    tag_file::read_tag_file(image_path)
}

/// 同名 Tag_File を書き込む（`tag_file::write_tag_file` へ委譲、要件 2.3）。
#[tauri::command]
pub fn write_tag_file(image_path: String, content: String) -> AppResult<()> {
    tag_file::write_tag_file(image_path, &content)
}

// ---------------------------------------------------------------------------
// TagService（要件 3, 6, 12）
// ---------------------------------------------------------------------------

/// タグ一括追加（`tag_service::bulk_add_tags` へ委譲、要件 3.1）。
#[tauri::command]
pub fn bulk_add_tags(targets: Vec<String>, tags: Vec<String>) -> AppResult<OperationReport> {
    Ok(tag_service::bulk_add_tags(&targets, &tags))
}

/// タグ一括削除（`tag_service::bulk_remove_tags` へ委譲、要件 3.3）。
#[tauri::command]
pub fn bulk_remove_tags(targets: Vec<String>, tags: Vec<String>) -> AppResult<OperationReport> {
    Ok(tag_service::bulk_remove_tags(&targets, &tags))
}

/// 重複タグ除去（`tag_service::dedup_tags` へ委譲、要件 3.4）。
#[tauri::command]
pub fn dedup_tags(targets: Vec<String>) -> AppResult<OperationReport> {
    Ok(tag_service::dedup_tags(&targets))
}

/// タグ集計（`tag_service::aggregate_tags` へ委譲、要件 6.1）。
#[tauri::command]
pub fn aggregate_tags(folder: String) -> AppResult<tag_service::TagAggregation> {
    tag_service::aggregate_tags(folder)
}

/// タグフィルタ（`tag_service::filter_images` へ委譲、要件 6.5〜6.8）。
#[tauri::command]
pub fn filter_images(
    folder: String,
    include: Vec<String>,
    exclude: Vec<String>,
) -> AppResult<Vec<ImageEntry>> {
    tag_service::filter_images(folder, &include, &exclude)
}

// ---------------------------------------------------------------------------
// SortService（要件 7, 9, 10）
// ---------------------------------------------------------------------------

/// 仕訳（move/copy/gather/distribute）（`sort_service::sort_files` へ委譲、要件 7）。
#[tauri::command]
pub fn sort_files(
    op: SortingOperation,
    source: String,
    dest: String,
) -> AppResult<OperationReport> {
    sort_service::sort_files(op, source, dest)
}

/// 画像サイズ振分（`sort_service::sort_by_size` へ委譲、要件 9）。
#[tauri::command]
pub fn sort_by_size(
    source: String,
    threshold: i64,
    dest_root: String,
) -> AppResult<OperationReport> {
    sort_service::sort_by_size(source, threshold, dest_root)
}

/// タグ振分（`sort_service::sort_by_tag` へ委譲、要件 10）。
#[tauri::command]
pub fn sort_by_tag(
    source: String,
    judge_tags: Vec<String>,
    contains_dest: String,
    not_contains_dest: String,
) -> AppResult<OperationReport> {
    sort_service::sort_by_tag(source, &judge_tags, contains_dest, not_contains_dest)
}

// ---------------------------------------------------------------------------
// RenameService（要件 4, 8）
// ---------------------------------------------------------------------------

/// 正規表現置換・連番改名（`rename_service::rename_regex` へ委譲、要件 8）。
#[tauri::command]
pub fn rename_regex(
    folder: String,
    pattern: String,
    replacement: String,
    numbering: Option<rename_service::Numbering>,
) -> AppResult<OperationReport> {
    rename_service::rename_regex(folder, &pattern, &replacement, numbering)
}

/// ファイル名正規化（`rename_service::normalize_caption_filenames` へ委譲、要件 4）。
#[tauri::command]
pub fn normalize_caption_filenames(folder: String) -> AppResult<OperationReport> {
    rename_service::normalize_caption_filenames(folder)
}

// ---------------------------------------------------------------------------
// CaptionService（要件 11）
// ---------------------------------------------------------------------------

/// 孤立キャプション特定（`caption_service::find_orphan_captions` へ委譲、要件 11.1）。
#[tauri::command]
pub fn find_orphan_captions(folder: String) -> AppResult<Vec<String>> {
    caption_service::find_orphan_captions(folder)
}

/// 孤立キャプション削除（`caption_service::delete_orphan_captions` へ委譲、要件 11.3）。
#[tauri::command]
pub fn delete_orphan_captions(
    paths: Vec<String>,
) -> AppResult<caption_service::DeleteOrphanResult> {
    caption_service::delete_orphan_captions(&paths)
}

// ---------------------------------------------------------------------------
// ModelService（要件 15）
// ---------------------------------------------------------------------------

/// 組み込みカタログと取得状態の一覧（要件 1.1, 1.2, 3.1, 3.5）。
///
/// [`model_service::builtin_catalog`]（検証済みの登録カタログ）と
/// [`model_service::catalog_presence`]（各バリアントの Model_Present /
/// Not_Present 判定）を結合して [`CatalogListing`] を返す。`base_dir` は
/// アプリ層が Tauri の PathResolver（[`resolve_base_dir`]）から供給する絶対
/// パス。フロントエンドは `base_dir` を渡さずに呼べる（存在判定は同一 `base_dir`
/// に対して [`spawn_variant_download`] / [`start_inference`] の保存・読込先と
/// 整合する、要件 3.5）。
///
/// `model_dir_present` は Model_Dir（`resolve_model_dir(base_dir)`）の存在で
/// 判定する（要件 3.5）。`excluded` は「必須フィールド欠落で除外した情報」
/// （要件 1.4）だが、`builtin_catalog` は [`model_service::validate_catalog`] を
/// 通した検証済みカタログで全て有効なため、実運用では空になる。動的な除外提示が
/// 必要になった場合は `model_service` に除外集合も返す API を追加する。
#[tauri::command]
pub fn list_catalog(app: tauri::AppHandle) -> AppResult<crate::models::CatalogListing> {
    let base_path = resolve_base_dir(&app);

    let catalog = model_service::builtin_catalog();
    let variants = model_service::catalog_presence(&base_path, &catalog);
    let model_dir_present = model_service::resolve_model_dir(&base_path).exists();

    Ok(crate::models::CatalogListing {
        variants,
        // builtin_catalog は validate_catalog 済みで全て有効なため除外なし。
        excluded: Vec::new(),
        model_dir_present,
    })
}

/// モデル保存フォルダ（Model_Dir）のパスを解決する（UI に表示するテキスト用）。
///
/// [`resolve_base_dir`] で得た `base_dir` から
/// [`model_service::resolve_model_dir`] で Model_Dir を求め、未作成なら
/// [`model_service::ensure_model_dir`] で作成してから絶対パス文字列を返す
/// （要件 2.2）。フロントエンドはこのパスをモデル管理タブへテキストとして
/// 表示するだけで、OS のファイルマネージャを開く操作は提供しない。作成失敗
/// （権限不足等）は `Err` を返す。
#[tauri::command]
pub fn get_model_dir_path(app: tauri::AppHandle) -> AppResult<String> {
    let base_path = resolve_base_dir(&app);
    let model_dir = model_service::resolve_model_dir(&base_path);
    model_service::ensure_model_dir(&model_dir)?;
    Ok(model_dir.to_string_lossy().into_owned())
}

// 旧 `load_local_model` コマンドはタスク 13.3 で撤去した。新設計では
// [`start_inference`] が遅延 DL + [`model_service::load_variant`] を内包するため、
// フロントエンドからの明示ロードコマンドは不要（design「移行方針まとめ」）。
// `LoadedModelInfo` 型自体は他テスト（`tests/pbt_wiring_preservation.rs`）が
// 形状を参照するため残す。

/// モデルメタ情報の serde 可能な軽量 DTO。
///
/// 実 `ort::Session` を含む [`crate::models::LoadedModel`] は serde 不可のため、
/// フロントエンドへはこの軽量 DTO を返す。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct LoadedModelInfo {
    /// 入力の一辺サイズ（例 448）。
    pub input_size: u32,
    /// 読み込めたラベル定義数。
    pub label_count: usize,
}

// ---------------------------------------------------------------------------
// モデルダウンロードの spawn・進捗・キャンセル（要件 5.1〜5.5、タスク 13.2）
// ---------------------------------------------------------------------------

/// [`spawn_variant_download`] の中核ロジック（`AppHandle` 非依存・単体テスト
/// 可能）。
///
/// [`model_service::download_variant`]（同期コア・不変）を [`std::thread::spawn`]
/// 上で実行し、即座に戻る（呼び出し元をブロックしない、要件 5.2）。取得段階
/// （onnx 取得 → タグ定義取得 → 保存）ごとに `make_emitter` が返すエミッタへ
/// 進捗を通知する（要件 5.3）。`registry` へ `operation_id` を登録して得た共有
/// フラグを取得段の境界で確認し、キャンセル要求があれば残りの処理を中断する
/// （要件 5.4）。処理完了（成功・失敗・キャンセル問わず）後、レジストリ登録を
/// 解放する。
///
/// `variant`（`variant_id` からカタログ解決済み）と保存先 `variant_dir` は
/// 呼び出し側（アプリ層）が [`model_service::builtin_catalog`] /
/// [`model_service::variant_dir`] を用いて解決する。`variant_dir` を単体テスト
/// 可能にするため、本コアは `variant`・`variant_dir` を引数で受ける。
///
/// ダウンロード完了後、`on_downloaded`（`Send + 'static`）を呼び保存先ディレクトリ
/// を渡す。アプリ層ではここで [`model_service::load_variant`] を呼び
/// `ModelSessionState` へ格納する（要件 5.5 の Model_Present 反映と同じ経路）。
///
/// 戻り値は起動した [`std::thread::JoinHandle`]。呼び出し元は `join` を待たずに
/// 戻ることで UI スレッドを塞がない。
fn spawn_variant_download_core<D, EM, ODL>(
    downloader: D,
    variant: ModelVariant,
    variant_dir: std::path::PathBuf,
    registry: Arc<CancelRegistry>,
    operation_id: String,
    make_emitter: EM,
    on_downloaded: ODL,
) -> std::thread::JoinHandle<AppResult<std::path::PathBuf>>
where
    D: model_service::ModelDownloader + Send + 'static,
    EM: FnOnce() -> Box<dyn ProgressEmitter + Send> + Send + 'static,
    ODL: FnOnce(&std::path::Path) + Send + 'static,
{
    std::thread::spawn(move || {
        let cancel = registry.register(&operation_id);
        let mut emitter = make_emitter();

        // 取得段階を Progress（done/total）へ写像する。3 段階固定
        // （onnx 取得 → タグ定義取得 → 保存、design 変更 4）。
        const TOTAL_PHASES: usize = 3;
        let op_id_for_progress = operation_id.clone();
        let on_phase = move |phase: model_service::DownloadPhase| {
            let done = match phase {
                model_service::DownloadPhase::OnnxFetched => 1,
                model_service::DownloadPhase::TagDefinitionFetched => 2,
                model_service::DownloadPhase::Saved => 3,
            };
            emitter.emit(crate::models::Progress {
                operation_id: op_id_for_progress.clone(),
                done,
                total: TOTAL_PHASES,
            });
        };

        let result =
            model_service::download_variant(&downloader, &variant, &variant_dir, &cancel, on_phase);

        registry.clear(&operation_id);

        if let Ok(saved) = &result {
            on_downloaded(saved);
        }

        result
    })
}

/// バリアントの明示ダウンロードを別スレッドで起動する（要件 5.1〜5.5）。
///
/// [`spawn_variant_download_core`] へ薄く委譲する。呼び出しは即座に戻り
/// （UI スレッドを塞がない、要件 5.2）、進捗は [`TauriProgressEmitter`] で
/// `inference://progress`（[`crate::app::PROGRESS_EVENT`]）へ emit する
/// （要件 5.3）。キャンセルは [`cancel_operation`] コマンド経由の
/// [`CancelRegistry`] を通じて取得段の境界で反映される（要件 5.4）。
///
/// `variant_id` を [`model_service::builtin_catalog`] から解決し、
/// [`model_service::variant_dir`] を保存先に用いる。カタログに存在しない
/// `variant_id` は [`AppError::not_found`] を返す。
///
/// ダウンロード完了後、保存先ディレクトリを [`model_service::load_variant`] に
/// 渡してロードし、同じ `ModelSessionState` に保持する（要件 5.5 の
/// Model_Present 反映と同じ経路）。ロード失敗時はダウンロード自体の成否には
/// 影響しない（セッション保持のみ行われない）。
///
/// # base_dir について（タスク 14 で確定）
///
/// 保存先の基準となる `base_dir` はアプリ層が Tauri の PathResolver
/// （[`resolve_base_dir`]）から供給する。Download_Operation のデータ書込に
/// 先立ち [`model_service::ensure_model_dir`] で Model_Dir（`variant_dir` の
/// 親）を作成する（要件 2.2）。作成失敗（権限不足など）は同期的にエラーを返し、
/// ダウンロードを開始しない（要件 2.4, 2.5）。
#[tauri::command]
pub fn spawn_variant_download(
    model_state: tauri::State<'_, ModelSessionState>,
    registry: tauri::State<'_, Arc<CancelRegistry>>,
    app: tauri::AppHandle,
    variant_id: String,
    operation_id: String,
) -> AppResult<()> {
    // variant_id をカタログから解決する（未知の id は NotFound）。
    let variant = model_service::builtin_catalog()
        .into_iter()
        .find(|v| v.id == variant_id)
        .ok_or_else(|| {
            AppError::not_found(format!("カタログに存在しないモデル: {variant_id}"))
        })?;

    // base_dir を PathResolver から供給する（要件 2.1）。
    let base_dir = resolve_base_dir(&app);
    // データ書込前に Model_Dir を作成する（要件 2.2）。作成失敗は開始前に返す。
    model_service::ensure_model_dir(&model_service::resolve_model_dir(&base_dir))?;
    let variant_dir = model_service::variant_dir(&base_dir, &variant);

    let registry_arc: Arc<CancelRegistry> = Arc::clone(&registry);
    let model_slot = model_state.current_slot();

    spawn_variant_download_core(
        model_service::HfHubDownloader::new(),
        variant,
        variant_dir,
        registry_arc,
        operation_id,
        move || -> Box<dyn ProgressEmitter + Send> { Box::new(TauriProgressEmitter::new(app)) },
        move |saved: &Path| {
            if let Ok(loaded) = model_service::load_variant(saved) {
                *model_slot
                    .lock()
                    .expect("ModelSessionState mutex poisoned") = Some(loaded);
            }
        },
    );

    Ok(())
}

/// アプリ層の `base_dir` 解決（タスク 14 で確定）。
///
/// Tauri v2 の PathResolver（[`tauri::Manager::path`]）から `base_dir` を取得し、
/// [`model_service::resolve_model_dir`] にはこの実 `base_dir` を渡す
/// （要件 2.1）。解決できない場合は空パスへフォールバックする。
///
/// # `resource_dir` ではなく `app_data_dir` を採用する理由
///
/// design は Model_Dir を「インストール基準ディレクトリからの固定相対パス」で
/// 解決する（要件 2.1）。候補は `resource_dir`（バンドル同梱の読み取り専用に
/// なり得るディレクトリ）と `app_data_dir`（ユーザーごとの書き込み可能な
/// アプリデータディレクトリ）だが、Model_Assets は Download_Operation で
/// 書き込む必要があるため、書き込み可能性を優先して `app_data_dir` を
/// `base_dir` に採用する。要件 2.1 の「固定相対パス」は、この `base_dir` に
/// [`model_service::resolve_model_dir`] が固定サブパス（`models`）を結合する
/// ことで満たす（`base_dir` 自体は環境ごとに一意な絶対パス）。
fn resolve_base_dir(app: &tauri::AppHandle) -> std::path::PathBuf {
    use tauri::Manager;
    app.path().app_data_dir().unwrap_or_default()
}

// ---------------------------------------------------------------------------
// PlatformService（要件 13, 16.6）
// ---------------------------------------------------------------------------

/// プラットフォーム機能可否（`platform_service::capabilities` へ委譲、要件 13.1, 16.6）。
#[tauri::command]
pub fn capabilities() -> AppResult<platform_service::Capabilities> {
    Ok(platform_service::capabilities())
}

/// シンボリックリンク作成（`platform_service::create_symlink` へ委譲、要件 13.3〜13.6）。
#[tauri::command]
pub fn create_symlink(link_target: String, link_path: String) -> AppResult<()> {
    platform_service::create_symlink(link_target, link_path)
}

/// パス変換（`platform_service::convert_path` へ委譲、要件 13.7〜13.9）。
#[tauri::command]
pub fn convert_path(
    path: String,
    direction: platform_service::ConvertDirection,
) -> AppResult<String> {
    platform_service::convert_path(&path, direction)
}

// ---------------------------------------------------------------------------
// キャンセルコマンド（要件 17.7）
// ---------------------------------------------------------------------------

/// 進行中処理へキャンセルを要求する（[`CancelRegistry::request_cancel`] へ委譲）。
///
/// 該当 `operation_id` が登録済みなら `true`、未登録（既に完了/未開始）なら
/// `false` を返す。アプリ層ではレジストリを `tauri::State` として渡す
/// （タスク 21）。ここでは境界の配線を明示するため、レジストリ参照を第 1 引数で
/// 受ける薄い関数として定義する。
#[tauri::command]
pub fn cancel_operation(
    registry: tauri::State<'_, Arc<CancelRegistry>>,
    operation_id: String,
) -> bool {
    registry.request_cancel(&operation_id)
}

// ---------------------------------------------------------------------------
// 長時間処理: バッチ推論の spawn とキャンセル・進捗配線（要件 16.8, 17.6, 17.7）
// ---------------------------------------------------------------------------

/// バッチ推論ジョブのパラメータ。
///
/// [`run_inference_job`]（同期コア）と [`spawn_inference_job`]（別スレッド起動）で
/// 共有する。`labels`/`input_size`/`channel_order` はモデルメタ由来。
pub struct InferenceJob {
    /// 対象 Image_File のパス列（mp4 は内部で除外、要件 14.8）。
    pub image_paths: Vec<String>,
    /// コンパイル済み Tag_Filter（採用/不採用・閾値・出現割合、要件 9, 10）。
    pub filter: crate::models::TagFilter,
    /// Batch_Size（未指定は既定 8、範囲 1〜64、要件 17.3〜17.5）。
    pub batch_size: Option<u32>,
    /// ラベル定義。
    pub labels: Vec<crate::models::LabelDef>,
    /// 入力の一辺サイズ。
    pub input_size: u32,
    /// 入力チャンネル順。
    pub channel_order: crate::models::ChannelOrder,
    /// 進捗・キャンセルに用いる処理識別子。
    pub operation_id: String,
}

/// バッチ推論の同期コア（Tauri 非依存・単体テスト可能）。
///
/// [`crate::services::inference_service::run_inference`] へ委譲する薄いラッパー。
/// キャンセルは [`CancelRegistry`] へ `operation_id` を登録して得た共有フラグを
/// 用い、進捗は [`ProgressEmitter`] 経由で通知する（要件 17.6, 17.7）。
/// 処理完了時にレジストリ登録を解放する。
///
/// # 引数
///
/// - `runner`: セッション実行の抽象（実 ort もモックも可）。
/// - `job`: 推論パラメータ。
/// - `registry`: キャンセルレジストリ。`operation_id` を登録し、完了時に解放する。
/// - `emitter`: 進捗通知先（アプリ層では Tauri emit、テストでは蓄積）。
pub fn run_inference_job(
    runner: &dyn crate::services::inference_service::SessionRunner,
    job: &InferenceJob,
    registry: &CancelRegistry,
    emitter: &mut dyn ProgressEmitter,
) -> crate::models::InferBatchResult {
    // キャンセルフラグを登録し、共有ハンドルを処理へ渡す（要件 17.7）。
    let cancel = registry.register(&job.operation_id);

    let mut callback = |p: crate::models::Progress| emitter.emit(p);

    let result = crate::services::inference_service::run_inference(
        runner,
        &job.image_paths,
        &job.filter,
        job.batch_size,
        &job.labels,
        job.input_size,
        job.channel_order,
        &job.operation_id,
        &cancel,
        &mut callback,
    );

    // 処理完了（または中止）後、登録を解放する。
    registry.clear(&job.operation_id);
    result
}

/// バッチ推論を別スレッドで起動する（要件 16.8）。
///
/// UI スレッドを塞がないよう [`std::thread::spawn`] 上で [`run_inference_job`] を
/// 実行する。呼び出しは即座に [`std::thread::JoinHandle`] を返し、結果は
/// ハンドルの `join` で取得できる。進捗は `emitter`（`Send` 実装）を通じて
/// スレッド内から通知される。キャンセルは共有 [`CancelRegistry`] を通じて
/// 別スレッド（UI）から [`cancel_operation`] で要求できる。
///
/// `runner` は `Send` を要する（別スレッドへ移動するため）。アプリ層では
/// モデルセッションをスレッドへ move するか、`Send` なラッパーを用いる。
pub fn spawn_inference_job<R>(
    runner: R,
    job: InferenceJob,
    registry: Arc<CancelRegistry>,
    mut emitter: Box<dyn ProgressEmitter + Send>,
) -> std::thread::JoinHandle<crate::models::InferBatchResult>
where
    R: crate::services::inference_service::SessionRunner + Send + 'static,
{
    std::thread::spawn(move || run_inference_job(&runner, &job, &registry, &mut *emitter))
}

// ---------------------------------------------------------------------------
// バッチ推論起動コマンド `start_inference`（要件 2.1, 2.2, 2.3, 2.6、タスク 3.4）
// ---------------------------------------------------------------------------

/// [`start_inference_core`] が推論起動に用いるメタ情報＋runner の組。
///
/// [`crate::models::LoadedModel`] から `labels`/`input_size`/`channel_order` を
/// 読み取って [`InferenceJob`] を組み立てる部分と、runner（実 ort もモックも可）
/// を構築する部分をまとめて表す。`take_runner` クロージャがこれを返すことで、
/// `start_inference_core` は `LoadedModel` 型そのものに依存せずに済む。
struct InferenceRunnerSetup<R> {
    /// ラベル定義。
    labels: Vec<crate::models::LabelDef>,
    /// 入力の一辺サイズ。
    input_size: u32,
    /// 入力チャンネル順。
    channel_order: crate::models::ChannelOrder,
    /// 推論実行器。
    runner: R,
}

/// [`start_inference`] の中核ロジック（`AppHandle` 非依存・単体テスト可能）。
///
/// タスク 13.3 でローカルのみ実行の遅延ダウンロードに対応。runner の準備
/// （variant_dir の存在判定 → Not_Present なら `download_variant`（遅延 DL）→
/// `load_variant` → runner 構築）は時間がかかるため、`prepare_runner` として
/// バックグラウンドスレッド内で実行する。`prepare_runner` は成功すれば
/// [`InferenceRunnerSetup`] を、Assets 不在・DL 失敗・読込失敗なら `AppError`
/// （`NotFound`/`Download`/`ModelLoad`）を返す。エラー時は推論を一切実行せず
/// スレッドを終了する（要件 4.4, 4.5, 7.3, 7.5）。`restore` は推論完了後に
/// runner から取り出した状態を呼び出し元スロットへ書き戻す処理を表す。
///
/// これらを呼び出し側から注入可能にすることで、既存設計思想（実 ort 実行は
/// [`crate::services::inference_service::SessionRunner`] トレイト境界の背後に
/// 隔離し、単体テストは `MockRunner` を使う）に合わせ、`start_inference_core`
/// 自体は実 `ort::Session`／[`crate::models::LoadedModel`] に依存しない。
/// 本番では `prepare_runner` の中で is_present 判定・遅延 DL・`load_variant` を
/// 行い、得た `LoadedModel` を [`OwnedOrtRunner::new`] へ渡し、`restore` に
/// [`OwnedOrtRunner::into_inner`] を渡す（[`start_inference`] 参照）。
///
/// `filter`（compile 済み [`crate::models::TagFilter`]）は [`run_inference`] へ
/// そのまま渡す（要件 7.1）。無効パターン検査・variant 解決の同期的な事前検証は
/// 呼び出し側（[`start_inference`]）で行い、`InvalidInput`/`NotFound` を同期的に
/// 返す。本関数はスレッド起動後すぐに戻り、`JoinHandle::join` を待たない
/// （UI スレッドを塞がない、要件 4.x のローカル実行非阻害）。
///
/// [`run_inference`]: crate::services::inference_service::run_inference
///
/// 進捗は `make_emitter`（呼び出しごとに新しい `Box<dyn ProgressEmitter + Send>`
/// を生成するファクトリ）が返すエミッタへ emit する。
///
/// 同期コア（[`run_inference_job`]/[`spawn_inference_job`]/[`InferenceJob`]/
/// [`crate::services::inference_service::SessionRunner`]/[`OwnedOrtRunner`]）は
/// 変更しない（保持 3.2）。`AppHandle`/`tauri::State` に依存しないため、実
/// Tauri ランタイムなしで単体テストできる。
///
/// `on_complete` は推論完了後に [`crate::models::InferBatchResult`]（`overview`
/// を含む）を受け取るコールバック（`Send + 'static`）。アプリ層ではここで
/// [`crate::app::emit_inference_complete`] を呼び、`inference://complete`
/// イベントで Tag_Overview をフロントエンドへ届ける（要件 11.1、タスク 18.1）。
/// 準備失敗時（推論を実行しない場合）は呼ばない。
fn start_inference_core<R, PR, RS, EM, OC>(
    registry: Arc<CancelRegistry>,
    prepare_runner: PR,
    restore: RS,
    make_emitter: EM,
    on_complete: OC,
    image_paths: Vec<String>,
    filter: crate::models::TagFilter,
    batch_size: Option<u32>,
    operation_id: String,
) -> AppResult<()>
where
    R: crate::services::inference_service::SessionRunner + Send + 'static,
    PR: FnOnce() -> AppResult<InferenceRunnerSetup<R>> + Send + 'static,
    RS: FnOnce(R) + Send + 'static,
    EM: FnOnce() -> Box<dyn ProgressEmitter + Send> + Send + 'static,
    OC: FnOnce(&crate::models::InferBatchResult) + Send + 'static,
{
    // runner の準備（遅延 DL + load）と推論をバックグラウンドスレッドで実行し、
    // UI スレッドを塞がない。準備が失敗（Assets 不在・DL 失敗・読込失敗）した
    // 場合は推論を一切実行せずスレッドを終了する（要件 4.4, 4.5, 7.3, 7.5）。
    std::thread::spawn(move || {
        let setup = match prepare_runner() {
            Ok(setup) => setup,
            Err(_err) => {
                // 準備失敗時は推論しない。エラーの UI 通知経路（error イベント）は
                // 未配線のため、ここでは推論スキップに留める（Task 17 で UI 通知）。
                return;
            }
        };

        let job = InferenceJob {
            image_paths,
            filter,
            batch_size,
            labels: setup.labels,
            input_size: setup.input_size,
            channel_order: setup.channel_order,
            operation_id,
        };
        let runner = setup.runner;

        let mut emitter = make_emitter();
        let result = run_inference_job(&runner, &job, &registry, &mut *emitter);
        // 完了後に runner の状態を呼び出し元スロットへ書き戻す
        // （次回推論のためセッションを保持し続ける）。
        restore(runner);
        // 完了通知（overview を含む）をフロントエンドへ届ける（要件 11.1）。
        on_complete(&result);
    });

    Ok(())
}

/// バッチ推論をローカルモデルで起動する（要件 4.1〜4.5, 7.1, 7.3, 7.5）。
///
/// タスク 13.3 で `variant_id` + `filter`（[`crate::models::RawTagFilter`]）を
/// 受け、旧 `threshold` を廃した。処理の流れ:
///
/// 1. `filter` を [`crate::logic::tag_filter::compile_filter`] でコンパイルする。
///    無効な正規表現パターンが 1 つでもあれば [`AppError::invalid_input`]
///    （`InvalidInput`）を同期的に返し、推論は実行しない（要件 9.7、UI に修正を
///    促す）。無効パターンは適用せず有効分だけで続行する解釈もあるが、tasks.md /
///    design のコマンド境界記述「無効パターンは `InvalidInput` として UI へ通知」
///    に従い、ここでは無効パターン検出時に推論を止めて UI へ通知する方針とする。
/// 2. `variant_id` を [`model_service::builtin_catalog`] から解決する。未知の
///    `variant_id` は [`AppError::not_found`]（`NotFound`）を同期的に返す。
/// 3. `variant_dir(base_dir, &variant)` を解決し、以降の遅延 DL + load + 推論を
///    バックグラウンドスレッド（[`start_inference_core`]）で実行する。スレッド内で:
///    - `is_present(variant_dir)` が false（Not_Present）なら
///      [`model_service::download_variant`] で遅延ダウンロードし（進捗通知、
///      要件 4.1, 4.2）、失敗すれば推論しない（要件 4.4）。DL 成功後
///      [`model_service::load_variant`] でロードする。
///    - Present ならそのまま `load_variant` でロードする（要件 4.3）。
///    - 読込失敗（`ModelLoad`）は推論しない（要件 4.5）。
///    - ローカルに Assets が無く（Not_Present）DL もできなければ、`download_variant`
///      が `Download`/`NotFound` を返し推論しない（要件 7.3, 7.5）。
///    ロードした [`crate::models::LoadedModel`] を [`OwnedOrtRunner`] にして
///    [`run_inference`] へ `filter`（[`crate::models::TagFilter`]）を渡す
///    （runner のみを用い Model_Source を参照しない、要件 7.1）。
///
/// [`run_inference`]: crate::services::inference_service::run_inference
///
/// # base_dir について（タスク 14 で確定）
///
/// 保存先/読込元の基準 `base_dir` は本来アプリ層（タスク 14）が Tauri の
/// PathResolver から供給する。本タスクでは [`resolve_base_dir`]（暫定）を用いる。
///
/// `model_state`/`registry` は内部が `Arc` 共有ハンドルのため複製してスレッドへ
/// move できる。進捗は [`TauriProgressEmitter`] で `inference://progress`
/// （[`crate::app::PROGRESS_EVENT`]）へ emit する。ロードしたセッションは次回の
/// 推論のため `model_state` へ保持する。
#[tauri::command]
pub fn start_inference(
    model_state: tauri::State<'_, ModelSessionState>,
    registry: tauri::State<'_, Arc<CancelRegistry>>,
    app: tauri::AppHandle,
    variant_id: String,
    filter: crate::models::RawTagFilter,
    image_paths: Vec<String>,
    batch_size: Option<u32>,
    operation_id: String,
) -> AppResult<()> {
    // 1) フィルタをコンパイルする。無効パターンがあれば InvalidInput を同期返却し
    //    推論しない（要件 9.7）。
    let (compiled_filter, invalid) = crate::logic::tag_filter::compile_filter(filter);
    if !invalid.is_empty() {
        let detail = invalid
            .iter()
            .map(|p| format!("{}（{}）", p.pattern, p.reason))
            .collect::<Vec<_>>()
            .join(", ");
        return Err(AppError::invalid_input(format!(
            "無効な正規表現パターン: {detail}"
        )));
    }

    // 2) variant_id をカタログから解決する（未知の id は NotFound）。
    let variant = model_service::builtin_catalog()
        .into_iter()
        .find(|v| v.id == variant_id)
        .ok_or_else(|| AppError::not_found(format!("カタログに存在しないモデル: {variant_id}")))?;

    // 3) base_dir → variant_dir を解決する（base_dir は PathResolver 供給、要件 2.1）。
    let base_dir = resolve_base_dir(&app);
    let model_dir = model_service::resolve_model_dir(&base_dir);
    let variant_dir = model_service::variant_dir(&base_dir, &variant);

    let registry_arc: Arc<CancelRegistry> = Arc::clone(&registry);
    let model_slot = model_state.current_slot();
    let restore_slot = Arc::clone(&model_slot);

    // 遅延 DL + load を行うエミッタは prepare（DL 進捗）と推論（Progress）で
    // 共通の TauriProgressEmitter を用いる。prepare 内では on_progress を
    // DownloadPhase → Progress に写像する。
    let app_for_dl = app.clone();
    let op_id_for_dl = operation_id.clone();
    let registry_for_dl = Arc::clone(&registry);
    // 完了イベント（overview を含む）の emit 用に AppHandle を複製する
    // （make_emitter 側で app が move されるため別クローンを持つ、タスク 18.1）。
    let app_for_complete = app.clone();

    start_inference_core(
        registry_arc,
        move || -> AppResult<InferenceRunnerSetup<OwnedOrtRunner>> {
            // Not_Present なら遅延ダウンロード（進捗通知）。失敗すれば推論しない。
            if !model_service::is_present(&variant_dir) {
                // データ書込前に Model_Dir を作成する（要件 2.2）。
                model_service::ensure_model_dir(&model_dir)?;
                let cancel = registry_for_dl.register(&op_id_for_dl);
                let mut emitter = TauriProgressEmitter::new(app_for_dl);
                const TOTAL_PHASES: usize = 3;
                let op_id = op_id_for_dl.clone();
                let on_phase = |phase: model_service::DownloadPhase| {
                    let done = match phase {
                        model_service::DownloadPhase::OnnxFetched => 1,
                        model_service::DownloadPhase::TagDefinitionFetched => 2,
                        model_service::DownloadPhase::Saved => 3,
                    };
                    emitter.emit(crate::models::Progress {
                        operation_id: op_id.clone(),
                        done,
                        total: TOTAL_PHASES,
                    });
                };
                let dl = model_service::download_variant(
                    &model_service::HfHubDownloader::new(),
                    &variant,
                    &variant_dir,
                    &cancel,
                    on_phase,
                );
                registry_for_dl.clear(&op_id_for_dl);
                dl?;
            }

            // Present（または DL 成功）なら load_variant でロードする。
            // 読込失敗（ModelLoad）は推論しない。
            let model = model_service::load_variant(&variant_dir)?;
            Ok(InferenceRunnerSetup {
                labels: model.labels.clone(),
                input_size: model.input_size,
                channel_order: model.channel_order,
                runner: OwnedOrtRunner::new(model),
            })
        },
        move |runner: OwnedOrtRunner| {
            // 次回推論のためロード済みセッションをスロットへ保持する。
            let restored = runner.into_inner();
            *restore_slot
                .lock()
                .expect("ModelSessionState mutex poisoned") = Some(restored);
        },
        move || -> Box<dyn ProgressEmitter + Send> { Box::new(TauriProgressEmitter::new(app)) },
        move |result: &crate::models::InferBatchResult| {
            // 推論完了後、overview を含む結果を inference://complete へ emit する
            // （要件 11.1、タスク 18.1）。
            crate::app::emit_inference_complete(&app_for_complete, result);
        },
        image_paths,
        compiled_filter,
        batch_size,
        operation_id,
    )
}

// ---------------------------------------------------------------------------
// Tag_Overview コマンド群（要件 11.3, 11.4, 11.5, 11.6、タスク 13.4）
// ---------------------------------------------------------------------------
//
// 設計判断:「フロントエンド状態保持・コマンドは純粋変換」方針を採る。
//
// design のコマンド境界では overview_search / overview_send_keep /
// overview_send_exclude / rerun_inference が Tag_Overview と Keep_Tags /
// Exclude_Rules の状態を扱う。ここではこれらを Tauri の `State` として
// バックエンドで保持せず、現在の `TagOverview` / `RawTagFilter` /
// 直近バッチの生 Predicted_Tag 列（`Vec<Vec<Tag>>`）はフロントエンドが
// 保持し、各コマンドは「引数で受けて変換結果を返す」純粋変換として実装する。
//
// この方針を採る理由:
// - `start_inference` はバックグラウンドスレッドで走るため、直近バッチ状態を
//   Tauri `State` へ書き戻す配線は複雑になり、Task 14 のコマンド登録・
//   Task 18 のフロント配線とも密結合になる。純粋変換ならこの結合を避けられる。
// - overview_search / send_keep / send_exclude は本質的に状態を持たない
//   変換であり、引数で受ける形が最も素直（テストも容易）。
// - rerun_inference（要件 11.6）は「更新後 filter で同一バッチへ再適用」する
//   が、これは `apply_filter` → `apply_fraction_threshold` → `build_overview`
//   の再適用に等しく、生 Predicted_Tag 列と filter があれば純粋に計算できる。
//
// 設計上のギャップ（Task 18 / 将来課題で解消）:
// - rerun_inference の入力 `per_image_predicted`（各画像の生 Predicted_Tag 列）
//   の供給源が現状ない。`InferBatchResult` は overview しか持たないため、
//   フロントが生 Tag 列を保持するには `start_inference` の結果に per-image の
//   生 Tag 列を含める（または別イベントで送る）必要がある。この供給経路は
//   Task 18（フロント配線）で確定する。本タスクではコマンド自体を純粋変換
//   として実装・テストし、再適用ロジックの正しさは統合テスト（Task 13.6）で
//   `apply_filter` → `build_overview` の再適用が正しく動くことを確認する。

/// Tag_Overview を検索文字列で絞り込む（要件 11.3）。
///
/// [`crate::logic::tag_batch::search_overview`] へ委譲する純粋変換。大小無視の
/// 部分一致でタグ名を絞り込み、空クエリは全件を通過させる。現在の `overview` は
/// フロントエンドが保持し、絞り込み後の `TagOverview` を返す。
#[tauri::command]
pub fn overview_search(
    overview: crate::models::TagOverview,
    query: String,
) -> AppResult<crate::models::TagOverview> {
    Ok(crate::logic::tag_batch::search_overview(&overview, &query))
}

/// 表示中タグを Keep_Tags へ送る（要件 11.4）。
///
/// 現在の `RawTagFilter` の `keep` に `tags` を追加して返す純粋変換。
/// フロントエンドが保持する `filter` を受け取り、更新後の `filter` を返す。
/// 既に含まれるタグ名は重複追加しない（正規化キーで比較。`compile_filter` が
/// トリム＋小文字化するため、`keep` は正規化前の文字列でも整合する）。
#[tauri::command]
pub fn overview_send_keep(
    filter: crate::models::RawTagFilter,
    tags: Vec<String>,
) -> AppResult<crate::models::RawTagFilter> {
    let mut filter = filter;
    append_unique(&mut filter.keep, tags);
    Ok(filter)
}

/// 表示中タグを Exclude_Rules へ送る（要件 11.5）。
///
/// 現在の `RawTagFilter` の `exclude` に `tags` を追加して返す純粋変換。
/// Exclude_Rules は正規表現パターンだが、送出するのは「そのタグ名を除外」する
/// 意図なので、タグ名をそのまま exclude パターンとして追加する。`compile_filter`
/// が `^...$` でアンカーするため、追加パターンは当該タグ名の完全一致除外になる。
///
/// 正規表現メタ文字を含むタグ名（例 `(cat)`）はパターンとして解釈される点に
/// 注意。完全一致のエスケープが必要なら Task 18 のフロント側で `regex::escape`
/// 相当を施して渡す設計余地を残す。ここでは design のコマンド境界に従い、
/// タグ名をそのままパターンとして追加する。
#[tauri::command]
pub fn overview_send_exclude(
    filter: crate::models::RawTagFilter,
    tags: Vec<String>,
) -> AppResult<crate::models::RawTagFilter> {
    let mut filter = filter;
    append_unique(&mut filter.exclude, tags);
    Ok(filter)
}

/// 更新後フィルタで同一バッチへ推論を再適用し一覧へ反映する（要件 11.6）。
///
/// 直近バッチの各画像の生 Predicted_Tag 列 `per_image_predicted` と、更新後の
/// `filter`（[`crate::models::RawTagFilter`]）を受け、次を再適用して更新後の
/// [`crate::models::TagOverview`] を返す純粋変換:
///
/// 1. [`crate::logic::tag_filter::compile_filter`] で `filter` をコンパイル。
///    無効な正規表現パターンが 1 つでもあれば [`AppError::invalid_input`]
///    （`InvalidInput`）を返し、再適用しない（要件 9.7 と同じ扱い、UI へ通知）。
/// 2. 各画像の生 Predicted_Tag に [`crate::logic::tag_filter::apply_filter`] を
///    適用して `Vec<FilterOutcome>` を得る。
/// 3. [`crate::logic::tag_batch::apply_fraction_threshold`]（画像数を渡す）で
///    Fraction_Threshold を再適用し、[`crate::models::BatchOutcome`] を得る。
/// 4. その `overview` を返す（`apply_fraction_threshold` が移送後 per_image から
///    構築済み）。これにより Keep_Tags へ送ったタグが Adopted 側へ移る等、
///    更新後フィルタの結果が一覧へ反映される。
#[tauri::command]
pub fn rerun_inference(
    per_image_predicted: Vec<Vec<crate::models::Tag>>,
    filter: crate::models::RawTagFilter,
) -> AppResult<crate::models::TagOverview> {
    // (1) フィルタをコンパイル。無効パターンは InvalidInput で通知し再適用しない。
    let (compiled, invalid) = crate::logic::tag_filter::compile_filter(filter);
    if !invalid.is_empty() {
        let detail = invalid
            .iter()
            .map(|p| format!("{}（{}）", p.pattern, p.reason))
            .collect::<Vec<_>>()
            .join(", ");
        return Err(AppError::invalid_input(format!(
            "無効な正規表現パターン: {detail}"
        )));
    }

    // (2) 各画像へ apply_filter を再適用する。
    let per_image: Vec<crate::models::FilterOutcome> = per_image_predicted
        .iter()
        .map(|predicted| crate::logic::tag_filter::apply_filter(&compiled, predicted))
        .collect();

    // (3) Fraction_Threshold を再適用し overview を構築する（要件 11.6）。
    let image_count = per_image.len();
    let batch = crate::logic::tag_batch::apply_fraction_threshold(&per_image, &compiled, image_count);

    // (4) 更新後の一覧を返す。
    Ok(batch.overview)
}

/// 文字列ベクタへ、既存要素と重複しないものだけを末尾へ追加する。
///
/// 重複判定は正規化キー（前後トリム＋小文字化）で行う。追加要素は元の文字列を
/// そのまま保持する（`compile_filter` 側が正規化するため）。追加順序を保存する。
fn append_unique(target: &mut Vec<String>, additions: Vec<String>) {
    let mut seen: std::collections::HashSet<String> = target
        .iter()
        .map(|s| s.trim().to_lowercase())
        .collect();
    for item in additions {
        let key = item.trim().to_lowercase();
        if seen.insert(key) {
            target.push(item);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::AppErrorKind;
    use crate::models::{ChannelOrder, LabelDef, Progress, RawTagFilter, TagCategory};
    use crate::services::inference_service::SessionRunner;
    use crate::AppResult as CoreResult;
    use std::path::Path as StdPath;
    use tempfile::tempdir;

    /// Confidence_Threshold のみを設定した TagFilter を作るテストヘルパー。
    ///
    /// 旧 `threshold: f32` フィールドと等価な採用挙動を得るため、他フィールドは
    /// 空、fraction_threshold は 0（非適用）にする。
    fn threshold_filter(threshold: f32) -> crate::models::TagFilter {
        let (filter, _invalid) = crate::logic::tag_filter::compile_filter(RawTagFilter {
            keep: Vec::new(),
            exclude: Vec::new(),
            replace: Vec::new(),
            additional: Vec::new(),
            confidence_threshold: threshold,
            fraction_threshold: 0.0,
        });
        filter
    }

    // --- 委譲の正しさ（実 Tauri ランタイム不要） ---

    #[test]
    fn convert_path_command_delegates_and_returns_string() {
        // Win→Linux 変換が platform_service へ委譲される（要件 13.7）。
        let out = convert_path(
            "C:\\foo\\bar".to_string(),
            platform_service::ConvertDirection::WindowsToLinux,
        )
        .unwrap();
        assert_eq!(out, "/mnt/c/foo/bar");
    }

    #[test]
    fn convert_path_command_propagates_invalid_input_error() {
        // 変換規則に適合しない入力は InvalidInput として伝播する（要件 13.9, 16.1）。
        let err = convert_path(
            "foo\\bar".to_string(),
            platform_service::ConvertDirection::WindowsToLinux,
        )
        .unwrap_err();
        assert_eq!(err.kind, AppErrorKind::InvalidInput);
    }

    #[test]
    fn capabilities_command_matches_cfg_windows() {
        let caps = capabilities().unwrap();
        assert_eq!(caps.windows_only, cfg!(windows));
    }

    #[test]
    fn list_images_command_delegates_to_file_service() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("a.png"), b"").unwrap();
        std::fs::write(dir.path().join("b.txt"), b"").unwrap();

        let listing = list_images(dir.path().to_string_lossy().into_owned()).unwrap();
        let names: Vec<&str> = listing.items.iter().map(|e| e.file_name.as_str()).collect();
        assert_eq!(names, vec!["a.png"]);
    }

    #[test]
    fn list_images_command_propagates_error_for_unreadable_folder() {
        let dir = tempdir().unwrap();
        let missing = dir.path().join("nope");
        let err = list_images(missing.to_string_lossy().into_owned()).unwrap_err();
        // 存在しないフォルダは NotFound（要件 1.8, 16.1）。
        assert_eq!(err.kind, AppErrorKind::NotFound);
    }

    #[test]
    fn write_then_read_tag_file_commands_roundtrip() {
        let dir = tempdir().unwrap();
        let img = dir.path().join("x.png");
        std::fs::write(&img, b"").unwrap();
        let img_str = img.to_string_lossy().into_owned();

        write_tag_file(img_str.clone(), "cat, dog".to_string()).unwrap();
        let content = read_tag_file(img_str).unwrap();
        assert!(content.exists);
        assert_eq!(content.content, "cat, dog");
    }

    #[test]
    fn bulk_add_tags_command_delegates() {
        let dir = tempdir().unwrap();
        let img = dir.path().join("i.png");
        std::fs::write(&img, b"").unwrap();
        let report = bulk_add_tags(
            vec![img.to_string_lossy().into_owned()],
            vec!["new".to_string()],
        )
        .unwrap();
        assert_eq!(report.succeeded, 1);
        assert_eq!(std::fs::read_to_string(dir.path().join("i.txt")).unwrap(), "new");
    }

    // --- 長時間処理コア: 進捗・キャンセル配線 ---

    struct MockRunner {
        output: Vec<f32>,
    }
    impl SessionRunner for MockRunner {
        fn run(&self, _input: &[f32]) -> CoreResult<Vec<f32>> {
            Ok(self.output.clone())
        }
    }

    fn labels(names: &[&str]) -> Vec<LabelDef> {
        names
            .iter()
            .map(|n| LabelDef {
                name: (*n).to_string(),
                category: TagCategory::General,
            })
            .collect()
    }

    fn write_image(path: &StdPath) {
        use image::{Rgb, RgbImage};
        let mut img = RgbImage::new(2, 2);
        for p in img.pixels_mut() {
            *p = Rgb([100, 110, 120]);
        }
        img.save(path).unwrap();
    }

    #[test]
    fn run_inference_job_emits_progress_and_clears_registry() {
        use crate::commands::progress::RecordingEmitter;

        let dir = tempdir().unwrap();
        let mut paths = Vec::new();
        for i in 0..2 {
            let p = dir.path().join(format!("img{i}.png"));
            write_image(&p);
            paths.push(p.to_string_lossy().into_owned());
        }

        let runner = MockRunner { output: vec![0.9] };
        let registry = CancelRegistry::new();
        let mut emitter = RecordingEmitter::new();

        let job = InferenceJob {
            image_paths: paths,
            filter: threshold_filter(0.5),
            batch_size: Some(1),
            labels: labels(&["a"]),
            input_size: 4,
            channel_order: ChannelOrder::Bgr,
            operation_id: "job1".to_string(),
        };

        let result = run_inference_job(&runner, &job, &registry, &mut emitter);

        assert_eq!(result.succeeded, 2);
        // 進捗が 2 件通知され done が単調増加、total は 2。
        let seq: Vec<(usize, usize)> =
            emitter.events.iter().map(|p| (p.done, p.total)).collect();
        assert_eq!(seq, vec![(1, 2), (2, 2)]);
        // 完了後にレジストリ登録は解放されている（要件 17.7）。
        assert!(!registry.is_cancelled("job1"));
        assert!(registry.is_empty());
    }

    #[test]
    fn cancel_via_registry_stops_inference_job() {
        // 進捗コールバックの中でレジストリへキャンセルを要求し、
        // バッチ境界で未処理が中止されることを確認する（要件 17.7）。
        use crate::commands::progress::ProgressEmitter;

        let dir = tempdir().unwrap();
        let mut paths = Vec::new();
        for i in 0..3 {
            let p = dir.path().join(format!("c{i}.png"));
            write_image(&p);
            paths.push(p.to_string_lossy().into_owned());
        }

        let runner = MockRunner { output: vec![0.9] };
        let registry = Arc::new(CancelRegistry::new());

        // 1 件目の進捗を受けたらキャンセル要求するエミッタ。
        struct CancelOnFirst {
            registry: Arc<CancelRegistry>,
            seen: usize,
        }
        impl ProgressEmitter for CancelOnFirst {
            fn emit(&mut self, progress: Progress) {
                self.seen += 1;
                if self.seen == 1 {
                    self.registry.request_cancel(&progress.operation_id);
                }
            }
        }

        let mut emitter = CancelOnFirst {
            registry: Arc::clone(&registry),
            seen: 0,
        };

        let job = InferenceJob {
            image_paths: paths,
            filter: threshold_filter(0.5),
            batch_size: Some(1),
            labels: labels(&["a"]),
            input_size: 4,
            channel_order: ChannelOrder::Bgr,
            operation_id: "cjob".to_string(),
        };

        let result = run_inference_job(&runner, &job, &registry, &mut emitter);
        // 1 件成功、残り 2 件はキャンセルで中止。
        assert_eq!(result.succeeded, 1);
        assert_eq!(result.cancelled, 2);
    }

    #[test]
    fn spawn_inference_job_runs_on_background_thread() {
        use crate::commands::progress::RecordingEmitter;

        let dir = tempdir().unwrap();
        let p = dir.path().join("s.png");
        write_image(&p);

        let runner = MockRunner { output: vec![0.9] };
        let registry = Arc::new(CancelRegistry::new());
        let emitter: Box<dyn ProgressEmitter + Send> = Box::new(RecordingEmitter::new());

        let job = InferenceJob {
            image_paths: vec![p.to_string_lossy().into_owned()],
            filter: threshold_filter(0.5),
            batch_size: None,
            labels: labels(&["a"]),
            input_size: 4,
            channel_order: ChannelOrder::Bgr,
            operation_id: "spawned".to_string(),
        };

        let handle = spawn_inference_job(runner, job, Arc::clone(&registry), emitter);
        let result = handle.join().expect("inference thread panicked");
        assert_eq!(result.succeeded, 1);
        assert!(registry.is_empty());
    }

    // -----------------------------------------------------------------------
    // タスク 20.2 補完テスト: 進捗の単調性と最終値 / 別スレッドキャンセル反映 /
    // 失敗時の処理前状態保持（要件 16.7, 16.8, 17.6）
    // -----------------------------------------------------------------------

    /// 呼び出し順に沿って成功/失敗を切り替えるモック実行器。
    ///
    /// `fail_on_call[n]` が true の呼び出し（0 始まり）は推論失敗を返し、
    /// それ以外は固定確信度を返す。読込成功した画像に対して入力順に逐次
    /// 呼ばれるため、成功と失敗をインターリーブして再現できる。
    struct SequencedFailRunner {
        output: Vec<f32>,
        fail_on_call: Vec<bool>,
        calls: std::sync::Mutex<usize>,
    }
    impl SessionRunner for SequencedFailRunner {
        fn run(&self, _input: &[f32]) -> CoreResult<Vec<f32>> {
            let idx = {
                let mut c = self.calls.lock().unwrap();
                let cur = *c;
                *c += 1;
                cur
            };
            if self.fail_on_call.get(idx).copied().unwrap_or(false) {
                Err(crate::error::AppError::model_load(format!(
                    "mock failure at call {idx}"
                )))
            } else {
                Ok(self.output.clone())
            }
        }
    }

    /// (16.7/17.6) 長時間ジョブを `run_inference_job` 経由で走らせ、
    /// `RecordingEmitter` に蓄積された進捗系列が単調非減少で、最終イベントの
    /// done が処理対象総数（== total）に一致することを検証する。
    ///
    /// 複数バッチ（batch_size=2, 5 枚 → 3 バッチ）にまたがっても進捗の
    /// 一貫性が保たれることを確認する。
    #[test]
    fn run_inference_job_progress_is_monotonic_and_final_equals_total() {
        use crate::commands::progress::RecordingEmitter;

        let dir = tempdir().unwrap();
        let mut paths = Vec::new();
        for i in 0..5 {
            let p = dir.path().join(format!("m{i}.png"));
            write_image(&p);
            paths.push(p.to_string_lossy().into_owned());
        }

        let runner = MockRunner { output: vec![0.9] };
        let registry = CancelRegistry::new();
        let mut emitter = RecordingEmitter::new();

        let job = InferenceJob {
            image_paths: paths,
            filter: threshold_filter(0.5),
            batch_size: Some(2),
            labels: labels(&["a"]),
            input_size: 4,
            channel_order: ChannelOrder::Bgr,
            operation_id: "monotone".to_string(),
        };

        let result = run_inference_job(&runner, &job, &registry, &mut emitter);
        assert_eq!(result.succeeded, 5);

        // 対象総数（mp4 除外なしなので 5）。全イベントの total は一定。
        let total = 5usize;
        assert_eq!(emitter.events.len(), total);
        for ev in &emitter.events {
            assert_eq!(ev.total, total);
            assert_eq!(ev.operation_id, "monotone");
            assert!(ev.done >= 1 && ev.done <= total);
        }
        // done は単調非減少（実装上は 1 ずつ増加）。
        for w in emitter.events.windows(2) {
            assert!(w[1].done >= w[0].done);
        }
        // 最終イベントの done == total（処理集合を完了）。
        assert_eq!(emitter.events.last().unwrap().done, total);
        // 完了後にレジストリ登録は解放されている。
        assert!(registry.is_empty());
    }

    /// (17.7/16.8) 別スレッドから `CancelRegistry` 経由でキャンセルを要求し、
    /// バックグラウンドで走る `spawn_inference_job` にそれが反映されることを
    /// 検証する。
    ///
    /// スレッド間のタイミングは非決定的なため、正確な分割数ではなく不変条件を
    /// 主張する: 成功件数 + キャンセル件数 == 処理対象総数（失敗ゼロの構成）。
    /// これにより、キャンセルされた分を除く全件が確定的に会計されることを保証する。
    #[test]
    fn cancel_from_another_thread_is_reflected_in_spawned_job() {
        use crate::commands::progress::{ProgressEmitter, RecordingEmitter};
        use std::sync::mpsc;

        let dir = tempdir().unwrap();
        let n = 12usize;
        let mut paths = Vec::new();
        for i in 0..n {
            let p = dir.path().join(format!("t{i}.png"));
            write_image(&p);
            paths.push(p.to_string_lossy().into_owned());
        }

        let runner = MockRunner { output: vec![0.9] };
        let registry = Arc::new(CancelRegistry::new());

        // 1 件でも進捗を観測したら、別スレッド（UI 相当）へ合図するエミッタ。
        struct SignalEmitter {
            inner: RecordingEmitter,
            tx: mpsc::Sender<()>,
            signaled: bool,
        }
        impl ProgressEmitter for SignalEmitter {
            fn emit(&mut self, progress: Progress) {
                if !self.signaled {
                    self.signaled = true;
                    let _ = self.tx.send(());
                }
                self.inner.emit(progress);
            }
        }

        let (tx, rx) = mpsc::channel::<()>();
        let emitter: Box<dyn ProgressEmitter + Send> = Box::new(SignalEmitter {
            inner: RecordingEmitter::new(),
            tx,
            signaled: false,
        });

        // batch_size を小さくして、キャンセルがバッチ境界で観測される猶予を作る。
        let job = InferenceJob {
            image_paths: paths,
            filter: threshold_filter(0.5),
            batch_size: Some(1),
            labels: labels(&["a"]),
            input_size: 4,
            channel_order: ChannelOrder::Bgr,
            operation_id: "cross-thread".to_string(),
        };

        let handle = spawn_inference_job(runner, job, Arc::clone(&registry), emitter);

        // ジョブが動き出したら別スレッド（本テストスレッド = UI 相当）から
        // レジストリ経由でキャンセルを要求する。
        let _ = rx.recv();
        registry.request_cancel("cross-thread");

        let result = handle.join().expect("inference thread panicked");

        // 失敗ゼロの構成なので、成功 + キャンセル == 総数（会計の完全性）。
        assert_eq!(result.failed, 0);
        assert_eq!(result.succeeded + result.cancelled, n);
        // 少なくとも一部はキャンセルで中止されている（別スレッド反映の確認）。
        assert!(result.cancelled >= 1, "キャンセルが反映されていない");
        // 完了後にレジストリ登録は解放されている（要件 17.7）。
        assert!(registry.is_empty());
    }

    // -----------------------------------------------------------------------
    // タスク 3.4 補完テスト: start_inference_core の分岐（未ロード時 AppError /
    // ロード済み時の推論実行・進捗送出・レジストリ解放・状態書き戻し）
    // -----------------------------------------------------------------------

    /// runner の準備（遅延 DL + load）が失敗した場合、推論を一切実行せず
    /// （`restore` を呼ばず）スレッドが終了することを検証する（要件 4.4, 4.5,
    /// 7.3, 7.5）。`prepare_runner` は `AppError` を返す。
    ///
    /// タスク 13.3 で `start_inference` 自体は即座に `Ok(())` を返し（UI 非阻害）、
    /// 準備失敗はバックグラウンドスレッド内で推論スキップとして扱う設計に変更。
    /// エラーの UI 通知経路は未配線のため、ここでは「推論が走らないこと」を
    /// `restore` 未呼び出しで検証する。
    #[test]
    fn start_inference_core_skips_inference_when_prepare_fails() {
        use crate::commands::progress::RecordingEmitter;
        use std::sync::mpsc;
        use std::time::Duration;

        let registry = Arc::new(CancelRegistry::new());
        let (restore_tx, restore_rx) = mpsc::channel::<&'static str>();
        // prepare 完了（失敗）をテスト側へ合図するチャネル。
        let (prepared_tx, prepared_rx) = mpsc::channel::<&'static str>();

        let result = start_inference_core::<MockRunner, _, _, _, _>(
            Arc::clone(&registry),
            move || -> AppResult<InferenceRunnerSetup<MockRunner>> {
                let _ = prepared_tx.send("prepared");
                Err(AppError::not_found("ローカルに Assets が無い"))
            },
            move |_runner: MockRunner| {
                // 準備失敗時は restore が呼ばれてはならない。
                let _ = restore_tx.send("restored");
            },
            || -> Box<dyn ProgressEmitter + Send> { Box::new(RecordingEmitter::new()) },
            |_result: &crate::models::InferBatchResult| {
                // 準備失敗時は on_complete も呼ばれない（推論が走らないため）。
            },
            vec!["dummy.png".to_string()],
            threshold_filter(0.5),
            None,
            "prepare-fail-job".to_string(),
        );

        // 起動自体は成功して即座に戻る（UI スレッドを塞がない）。
        assert!(result.is_ok());

        // prepare が呼ばれ（失敗し）たことを待つ。
        prepared_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("prepare_runner が時間内に呼ばれなかった");

        // restore は呼ばれない（推論が走っていない）。短い猶予後に未受信を確認。
        assert!(
            restore_rx
                .recv_timeout(Duration::from_millis(300))
                .is_err(),
            "準備失敗時に restore が呼ばれた（推論が実行されている）"
        );
    }

    /// ロード済み状態から呼び出すと、`MockRunner` 経由で推論が実行され、
    /// 進捗が通知され、完了後に `CancelRegistry` が空になり、runner の状態が
    /// `restore` を通じて書き戻されることを検証する（要件 2.1, 2.2, 2.3）。
    ///
    /// バックグラウンドスレッドで実行されるため、`restore` 内で
    /// `std::sync::mpsc` チャネルへ完了を通知し、それを待ってから
    /// アサーションする（スリープでのポーリングは避ける）。
    #[test]
    fn start_inference_core_runs_inference_and_restores_state_when_loaded() {
        use std::sync::mpsc;

        let dir = tempdir().unwrap();
        let mut paths = Vec::new();
        for i in 0..3 {
            let p = dir.path().join(format!("si{i}.png"));
            write_image(&p);
            paths.push(p.to_string_lossy().into_owned());
        }

        let registry = Arc::new(CancelRegistry::new());

        // 完了・書き戻し通知用チャネル。restore が呼ばれたことを合図する。
        let (restore_tx, restore_rx) = mpsc::channel::<&'static str>();
        // 進捗蓄積を確認するため、テスト側の Vec へ Arc<Mutex<...>> で共有する。
        // emit のたびに共有 Vec へ push する（emitter の Drop に依存しない。
        // Drop 完了は restore 合図と順序保証がなく競合の元になるため）。
        let recorded: Arc<std::sync::Mutex<Vec<crate::models::Progress>>> =
            Arc::new(std::sync::Mutex::new(Vec::new()));
        let recorded_for_emitter = Arc::clone(&recorded);

        let setup_labels = labels(&["a"]);

        let result = start_inference_core(
            Arc::clone(&registry),
            move || -> AppResult<InferenceRunnerSetup<MockRunner>> {
                Ok(InferenceRunnerSetup {
                    labels: setup_labels,
                    input_size: 4,
                    channel_order: ChannelOrder::Bgr,
                    runner: MockRunner { output: vec![0.9] },
                })
            },
            move |_runner: MockRunner| {
                // 「モデル相当の状態が再利用可能な状態に戻った」ことの代わりに、
                // restore が呼ばれたことをチャネルで確定的に合図する。
                let _ = restore_tx.send("restored");
            },
            move || -> Box<dyn ProgressEmitter + Send> {
                // emitter 自体はスレッドへ move するため、テスト側では emit の
                // たびに共有 Vec へ即時 push する（Drop 完了を待つ設計は restore
                // 合図との順序保証が無く競合の元になるため避ける）。
                struct Sharing {
                    shared: Arc<std::sync::Mutex<Vec<crate::models::Progress>>>,
                }
                impl ProgressEmitter for Sharing {
                    fn emit(&mut self, progress: crate::models::Progress) {
                        self.shared.lock().unwrap().push(progress);
                    }
                }
                Box::new(Sharing {
                    shared: recorded_for_emitter,
                })
            },
            |_result: &crate::models::InferBatchResult| {
                // 完了通知はここでは検証対象外（restore/進捗で完了を確認する）。
            },
            paths,
            threshold_filter(0.5),
            Some(1),
            "loaded-job".to_string(),
        );

        assert!(result.is_ok());

        // restore が呼ばれる（=モデル相当の状態が再利用可能な状態に戻る）まで待つ。
        let signal = restore_rx
            .recv_timeout(std::time::Duration::from_secs(5))
            .expect("restore が時間内に呼ばれなかった");
        assert_eq!(signal, "restored");

        // restore は run_inference_job 完了後に呼ばれるため、この時点で
        // レジストリは解放済み・進捗は蓄積済みのはず（emit は共有 Vec への即時
        // push のため、restore 合図の時点で確実に反映済み）。
        assert!(registry.is_empty());

        let events = recorded.lock().unwrap();
        assert_eq!(events.len(), 3, "進捗が記録されていない: {events:?}");
        assert_eq!(events.last().unwrap().done, 3);
        for ev in events.iter() {
            assert_eq!(ev.operation_id, "loaded-job");
        }
    }

    /// (16.8 失敗時の処理前状態保持) 長時間処理が途中で一部失敗しても、
    /// 成功した項目の出力（Tag_File）は書き込まれ、失敗した項目は Tag_File を
    /// 一切残さないことを検証する。すなわち中途半端にコミットされた大域状態が
    /// 存在しない。
    ///
    /// 構成（batch_size=2, 6 枚 → 3 バッチ、入力順に推論呼出）:
    ///   good0(成功), good1(推論失敗), good2(成功), good3(推論失敗),
    ///   good4(成功), good5(成功)
    /// 期待: succeeded=4, failed=2。成功分のみ Tag_File が存在する。
    #[test]
    fn partial_failure_preserves_pre_operation_state_for_failed_items() {
        let dir = tempdir().unwrap();
        let mut paths = Vec::new();
        for i in 0..6 {
            let p = dir.path().join(format!("g{i}.png"));
            write_image(&p);
            paths.push(p.to_string_lossy().into_owned());
        }

        // 入力順に推論が呼ばれる。呼出 1 と 3（g1, g3）を失敗させる。
        let runner = SequencedFailRunner {
            output: vec![0.9],
            fail_on_call: vec![false, true, false, true, false, false],
            calls: std::sync::Mutex::new(0),
        };
        let registry = CancelRegistry::new();
        let mut emitter = crate::commands::progress::RecordingEmitter::new();

        let job = InferenceJob {
            image_paths: paths,
            filter: threshold_filter(0.5),
            batch_size: Some(2),
            labels: labels(&["a"]),
            input_size: 4,
            channel_order: ChannelOrder::Bgr,
            operation_id: "partial-fail".to_string(),
        };

        let result = run_inference_job(&runner, &job, &registry, &mut emitter);

        assert_eq!(result.succeeded, 4);
        assert_eq!(result.failed, 2);
        assert_eq!(result.cancelled, 0);

        // 成功した項目は Tag_File が書き込まれている。
        for i in [0usize, 2, 4, 5] {
            assert!(
                dir.path().join(format!("g{i}.txt")).exists(),
                "成功項目 g{i} の Tag_File が存在しない"
            );
        }
        // 失敗した項目は Tag_File を一切残さない（中途コミットなし）。
        for i in [1usize, 3] {
            assert!(
                !dir.path().join(format!("g{i}.txt")).exists(),
                "失敗項目 g{i} に Tag_File が残っている（部分コミット）"
            );
        }
        // 進捗は全処理対象（6 件、mp4 除外なし）分通知され、最終 done==total。
        assert_eq!(emitter.events.len(), 6);
        assert_eq!(emitter.events.last().unwrap().done, 6);
        assert!(registry.is_empty());
    }

    // -----------------------------------------------------------------------
    // タスク 13.2 テスト: spawn_variant_download_core（別スレッド即戻り・進捗
    // 通知・キャンセル中断）と model_service::download_variant（再試行・原子的
    // 保存・部分ファイル除去のリグレッション無し）。
    // -----------------------------------------------------------------------

    use crate::services::model_service::{
        self, download_model_tests::MockDownloader, DownloadError,
    };

    /// テスト用の WD14 バリアント（取得元 `source` を持つ）。
    fn remote_wd14_variant() -> ModelVariant {
        ModelVariant {
            id: "wd14-vit".to_string(),
            display_name: "WD14 ViT".to_string(),
            family: crate::models::ModelFamily::Wd14,
            source: crate::models::ModelSource {
                repo: "owner/wd14-vit".to_string(),
                onnx_file: "model.onnx".to_string(),
                tag_files: vec!["selected_tags.csv".to_string()],
            },
        }
    }

    /// 妥当な CSV タグ定義を返すモックダウンローダを組み立てる。
    fn mock_downloader_with_valid_files() -> MockDownloader {
        MockDownloader::with_responses(&[
            ("model.onnx", b"onnx-bytes"),
            ("selected_tags.csv", b"tag_id,name,category\n1,solo,0\n"),
        ])
    }

    /// (要件 5.2) `spawn_variant_download_core` が別スレッドで実行され、
    /// 呼び出し元は `join` を待たずに即座に戻ることを検証する。
    #[test]
    fn spawn_variant_download_core_returns_immediately_without_blocking() {
        use crate::commands::progress::RecordingEmitter;
        use std::sync::mpsc;
        use std::time::Duration;

        // ダウンロード開始から完了通知まで明示的に遅延させ、`spawn` 呼び出し
        // 自体はその遅延を待たずに戻ることを確認する。
        struct SlowDownloader {
            responses: std::collections::HashMap<String, Vec<u8>>,
        }
        impl model_service::ModelDownloader for SlowDownloader {
            fn fetch(
                &self,
                _repo: &str,
                file: &str,
                _timeout: Duration,
            ) -> Result<Vec<u8>, DownloadError> {
                std::thread::sleep(Duration::from_millis(150));
                self.responses
                    .get(file)
                    .cloned()
                    .ok_or_else(|| DownloadError::Other("未登録".to_string()))
            }
        }

        let mut responses = std::collections::HashMap::new();
        responses.insert("model.onnx".to_string(), b"onnx-bytes".to_vec());
        responses.insert(
            "selected_tags.csv".to_string(),
            b"tag_id,name,category\n1,solo,0\n".to_vec(),
        );
        let downloader = SlowDownloader { responses };

        let dir = tempdir().unwrap();
        let dest = dir.path().join("wd14-vit");
        let registry = Arc::new(CancelRegistry::new());
        let (done_tx, done_rx) = mpsc::channel::<()>();

        let started = std::time::Instant::now();
        let handle = spawn_variant_download_core(
            downloader,
            remote_wd14_variant(),
            dest.clone(),
            Arc::clone(&registry),
            "spawn-immediate".to_string(),
            move || -> Box<dyn ProgressEmitter + Send> { Box::new(RecordingEmitter::new()) },
            move |_saved: &Path| {
                let _ = done_tx.send(());
            },
        );
        // spawn 呼び出し自体は遅延（150ms x 2 fetch = 300ms 超）より十分速く戻る。
        assert!(
            started.elapsed() < Duration::from_millis(100),
            "spawn_variant_download_core は即座に戻るべき"
        );

        // バックグラウンドで完了するまで待つ（テスト側の後始末のため）。
        let _ = done_rx.recv_timeout(Duration::from_secs(5));
        let result = handle.join().expect("download thread panicked");
        assert!(result.is_ok());
        assert!(registry.is_empty());
    }

    /// (要件 5.3) 取得段階（onnx 取得 → タグ定義取得 → 保存）に応じて進捗が
    /// 段階的に通知されることを検証する。
    #[test]
    fn spawn_variant_download_core_emits_progress_per_phase() {
        use crate::commands::progress::RecordingEmitter;
        use std::sync::mpsc;

        let downloader = mock_downloader_with_valid_files();
        let dir = tempdir().unwrap();
        let dest = dir.path().join("wd14-vit");
        let registry = Arc::new(CancelRegistry::new());

        let (events_tx, events_rx) = mpsc::channel::<Vec<crate::models::Progress>>();

        let handle = spawn_variant_download_core(
            downloader,
            remote_wd14_variant(),
            dest,
            Arc::clone(&registry),
            "progress-phases".to_string(),
            move || -> Box<dyn ProgressEmitter + Send> {
                struct Reporting {
                    inner: RecordingEmitter,
                    tx: mpsc::Sender<Vec<crate::models::Progress>>,
                }
                impl ProgressEmitter for Reporting {
                    fn emit(&mut self, progress: crate::models::Progress) {
                        self.inner.emit(progress);
                    }
                }
                impl Drop for Reporting {
                    fn drop(&mut self) {
                        let events = std::mem::take(&mut self.inner.events);
                        let _ = self.tx.send(events);
                    }
                }
                Box::new(Reporting {
                    inner: RecordingEmitter::new(),
                    tx: events_tx,
                })
            },
            |_saved: &Path| {},
        );

        let result = handle.join().expect("download thread panicked");
        assert!(result.is_ok());

        let events = events_rx
            .recv_timeout(std::time::Duration::from_secs(5))
            .expect("進捗イベントが記録されなかった");

        // 3 段階（onnx 取得 → タグ定義取得 → 保存）に対応する 3 件の進捗が、
        // done が単調増加する形で通知される。
        assert_eq!(events.len(), 3);
        let dones: Vec<usize> = events.iter().map(|p| p.done).collect();
        assert_eq!(dones, vec![1, 2, 3]);
        for ev in &events {
            assert_eq!(ev.total, 3);
            assert_eq!(ev.operation_id, "progress-phases");
        }
        assert!(registry.is_empty());
    }

    /// (要件 5.4) `CancelRegistry` 経由でキャンセル要求すると、取得段の境界で
    /// 処理が中断されることを検証する。
    ///
    /// onnx 取得完了直後（1 段階目の進捗通知内）でキャンセルを要求し、以降の
    /// タグ定義取得・保存が行われないことを確認する。
    #[test]
    fn spawn_variant_download_core_is_interrupted_by_cancel_registry() {
        use crate::commands::progress::RecordingEmitter;

        let downloader = mock_downloader_with_valid_files();
        let dir = tempdir().unwrap();
        let dest = dir.path().join("wd14-vit");
        let registry = Arc::new(CancelRegistry::new());
        let registry_for_cancel = Arc::clone(&registry);

        let handle = spawn_variant_download_core(
            downloader,
            remote_wd14_variant(),
            dest.clone(),
            Arc::clone(&registry),
            "cancel-on-first-phase".to_string(),
            move || -> Box<dyn ProgressEmitter + Send> {
                // 1 段階目の進捗を受けた時点でキャンセルを要求するエミッタ。
                struct CancelOnFirst {
                    inner: RecordingEmitter,
                    registry: Arc<CancelRegistry>,
                }
                impl ProgressEmitter for CancelOnFirst {
                    fn emit(&mut self, progress: crate::models::Progress) {
                        if progress.done == 1 {
                            self.registry.request_cancel(&progress.operation_id);
                        }
                        self.inner.emit(progress);
                    }
                }
                Box::new(CancelOnFirst {
                    inner: RecordingEmitter::new(),
                    registry: registry_for_cancel,
                })
            },
            |_saved: &Path| {
                panic!("キャンセルされた場合 on_downloaded は呼ばれてはならない");
            },
        );

        let result = handle.join().expect("download thread panicked");
        let err = result.unwrap_err();
        assert_eq!(err.kind, AppErrorKind::Cancelled);

        // タグ定義取得・保存段まで進んでいないため、保存物は存在しない。
        assert!(!dest.join("model.onnx").exists());
        assert!(!dest.join("selected_tags.csv").exists());
        // 完了（キャンセルによる中断含む）後にレジストリ登録は解放されている。
        assert!(registry.is_empty());
    }

    /// `model_service::download_variant` が通常経路で保存先へ `.onnx`＋タグ定義を
    /// 確定させ、取得段階（onnx → タグ定義 → 保存）の 3 段階を通知することを
    /// 確認する（spawn 経路が委譲する同期コアのリグレッション無し）。
    #[test]
    fn download_variant_saves_assets_and_emits_all_phases() {
        use std::sync::atomic::AtomicBool;

        let csv = b"tag_id,name,category\n1,solo,0\n";
        let downloader =
            MockDownloader::with_responses(&[("model.onnx", b"onnx-a"), ("selected_tags.csv", csv)]);

        let dir = tempdir().unwrap();
        let dest = dir.path().join("wd14-vit");
        let cancel = AtomicBool::new(false);
        let mut phases = Vec::new();

        let saved = model_service::download_variant(
            &downloader,
            &remote_wd14_variant(),
            &dest,
            &cancel,
            |phase| phases.push(phase),
        )
        .unwrap();

        assert_eq!(saved, dest);
        assert_eq!(std::fs::read(dest.join("model.onnx")).unwrap(), b"onnx-a");
        assert_eq!(std::fs::read(dest.join("selected_tags.csv")).unwrap(), csv);
        // 3 段階（onnx 取得 → タグ定義取得 → 保存）全てが通知される。
        assert_eq!(phases.len(), 3);
    }

    /// `download_variant` が再試行上限（最大 3 回）を尊重することを確認する。
    #[test]
    fn download_variant_preserves_retry_limit() {
        use std::sync::atomic::AtomicBool;

        let downloader = MockDownloader::always_failing(DownloadError::Timeout);
        let dir = tempdir().unwrap();
        let dest = dir.path().join("wd14-vit");
        let cancel = AtomicBool::new(false);

        let err = model_service::download_variant(
            &downloader,
            &remote_wd14_variant(),
            &dest,
            &cancel,
            |_phase| {},
        )
        .unwrap_err();

        assert_eq!(err.kind, AppErrorKind::Download);
        // .onnx の取得だけで最大 3 回まで再試行する。
        assert_eq!(downloader.call_count(), 3);
        assert!(!dest.join("model.onnx").exists());
    }

    /// 保存失敗時に部分ファイルが残らないことを確認する。
    #[test]
    fn download_variant_removes_partial_files_on_save_failure() {
        use std::sync::atomic::AtomicBool;

        let downloader = mock_downloader_with_valid_files();
        let dir = tempdir().unwrap();
        // dest 自体をファイルにして保存（ディレクトリ作成）を失敗させる。
        let dest_as_file = dir.path().join("occupied");
        std::fs::write(&dest_as_file, b"i am a file").unwrap();
        let cancel = AtomicBool::new(false);

        let err = model_service::download_variant(
            &downloader,
            &remote_wd14_variant(),
            &dest_as_file,
            &cancel,
            |_phase| {},
        )
        .unwrap_err();

        assert_ne!(err.kind, AppErrorKind::Download);
        assert!(dest_as_file.is_file());
    }

    // -----------------------------------------------------------------------
    // タスク 4 補完テスト: 起動スモークの一部として、`ModelSessionState` 相当の
    // 共有スロットを介した「一連フロー」統合テスト（要件 2.1, 2.4, 2.5, 2.7）。
    //
    // 既存テスト（`start_inference_core_*` / `spawn_variant_download_core_*`）は
    // それぞれの関数を単体で検証するのに対し、本節は複数コマンド境界をまたぐ
    // エンドツーエンドの流れ（モデル選択→推論起動→進捗更新→完了/キャンセル、
    // ダウンロード→保存先ロード→セッション保持→推論参照可能）を 1 本の
    // シナリオとして確認する。実 `ort::Session`/実ネットワークは用いず、
    // `MockRunner`/`MockDownloader` で代替する（このプロジェクトの既存テストに
    // 実 ONNX モデルを用いるものは見当たらないため）。
    // -----------------------------------------------------------------------

    /// モデル選択（セッション保持）→推論起動→進捗更新→完了→状態復元、という
    /// 一連の流れを、`ModelSessionState::current` と同じ形の共有スロット
    /// （`Arc<Mutex<Option<MockRunner>>>`）を用いて確認する（要件 2.1, 2.4）。
    ///
    /// `start_inference_core` の `take_runner`/`restore` クロージャは、本番では
    /// `ModelSessionState::current_slot()` から `LoadedModel` を take/書き戻す
    /// （[`start_inference`] 参照）。ここでは同じ「共有スロットへの take→実行→
    /// 書き戻し」構造を、実 ONNX セッション無しで検証する。
    #[test]
    fn end_to_end_select_model_then_infer_then_progress_then_complete() {
        use crate::commands::progress::RecordingEmitter;
        use std::sync::mpsc;
        use std::time::Duration;

        let dir = tempdir().unwrap();
        let mut paths = Vec::new();
        for i in 0..3 {
            let p = dir.path().join(format!("e2e{i}.png"));
            write_image(&p);
            paths.push(p.to_string_lossy().into_owned());
        }

        // 「モデル選択」に相当: セッションスロットへ runner をセットする。
        let model_slot: Arc<std::sync::Mutex<Option<MockRunner>>> =
            Arc::new(std::sync::Mutex::new(Some(MockRunner { output: vec![0.9] })));
        let take_slot = Arc::clone(&model_slot);
        let restore_slot = Arc::clone(&model_slot);
        // クロージャへ move された後もテスト末尾でスロットの状態を検査できる
        // よう、検査専用のハンドルを別途複製しておく。
        let check_slot = Arc::clone(&model_slot);

        let registry = Arc::new(CancelRegistry::new());
        let (progress_tx, progress_rx) = mpsc::channel::<Vec<crate::models::Progress>>();
        let (restore_tx, restore_rx) = mpsc::channel::<&'static str>();

        let setup_labels = labels(&["a"]);

        // 「推論起動」: start_inference_core が take_runner でスロットから
        // runner を取り出し、バックグラウンドスレッドで推論を実行する。
        let result = start_inference_core(
            Arc::clone(&registry),
            move || -> AppResult<InferenceRunnerSetup<MockRunner>> {
                let runner = take_slot
                    .lock()
                    .unwrap()
                    .take()
                    .ok_or_else(|| AppError::not_found("スロットにモデルが無い"))?;
                Ok(InferenceRunnerSetup {
                    labels: setup_labels,
                    input_size: 4,
                    channel_order: ChannelOrder::Bgr,
                    runner,
                })
            },
            move |runner: MockRunner| {
                // 「状態復元」: 推論完了後、runner をスロットへ書き戻す
                // （次回推論のためセッションを保持し続ける、要件 2.4）。
                *restore_slot.lock().unwrap() = Some(runner);
                let _ = restore_tx.send("restored");
            },
            move || -> Box<dyn ProgressEmitter + Send> {
                struct Reporting {
                    inner: RecordingEmitter,
                    tx: mpsc::Sender<Vec<crate::models::Progress>>,
                }
                impl ProgressEmitter for Reporting {
                    fn emit(&mut self, progress: crate::models::Progress) {
                        self.inner.emit(progress);
                    }
                }
                impl Drop for Reporting {
                    fn drop(&mut self) {
                        let events = std::mem::take(&mut self.inner.events);
                        let _ = self.tx.send(events);
                    }
                }
                Box::new(Reporting {
                    inner: RecordingEmitter::new(),
                    tx: progress_tx,
                })
            },
            |_result: &crate::models::InferBatchResult| {},
            paths,
            threshold_filter(0.5),
            Some(1),
            "e2e-select-infer".to_string(),
        );

        // 推論起動呼び出し自体は成功して即座に戻る（UI スレッドを塞がない）。
        assert!(result.is_ok());

        // 「完了」: 状態復元が呼ばれるまで待つ。
        let signal = restore_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("推論完了後の状態復元が時間内に行われなかった");
        assert_eq!(signal, "restored");

        // 「進捗更新」: 進捗が単調に増加し、最終的に done==total で通知される。
        let events = progress_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("進捗イベントが記録されなかった");
        assert_eq!(events.len(), 3);
        let dones: Vec<usize> = events.iter().map(|p| p.done).collect();
        assert_eq!(dones, vec![1, 2, 3]);
        assert_eq!(events.last().unwrap().total, 3);

        // 完了後にキャンセルレジストリは解放されている。
        assert!(registry.is_empty());

        // セッションが復元され、次回推論でも同じスロットから再度参照できる
        // ことを確認する（セッション保持、要件 2.4）。
        assert!(
            model_slot_has_runner(&check_slot),
            "推論完了後にセッションがスロットへ書き戻されていない"
        );
    }

    /// テスト用: 共有スロットに runner が保持されているかを確認する補助関数。
    fn model_slot_has_runner(slot: &Arc<std::sync::Mutex<Option<MockRunner>>>) -> bool {
        slot.lock().unwrap().is_some()
    }

    /// モデル選択→推論起動→進捗更新の途中でキャンセルされ、キャンセル分が
    /// 会計されつつ完了後に状態復元されることを確認する（要件 2.1, 2.3, 2.4）。
    #[test]
    fn end_to_end_select_model_then_infer_then_cancel_mid_flight() {
        use crate::commands::progress::RecordingEmitter;
        use std::sync::mpsc;
        use std::time::Duration;

        let dir = tempdir().unwrap();
        let mut paths = Vec::new();
        for i in 0..6 {
            let p = dir.path().join(format!("cancel{i}.png"));
            write_image(&p);
            paths.push(p.to_string_lossy().into_owned());
        }

        let model_slot: Arc<std::sync::Mutex<Option<MockRunner>>> =
            Arc::new(std::sync::Mutex::new(Some(MockRunner { output: vec![0.9] })));
        let take_slot = Arc::clone(&model_slot);
        let restore_slot = Arc::clone(&model_slot);
        let check_slot = Arc::clone(&model_slot);

        let registry = Arc::new(CancelRegistry::new());
        let registry_for_cancel = Arc::clone(&registry);
        let (first_progress_tx, first_progress_rx) = mpsc::channel::<()>();
        let (restore_tx, restore_rx) = mpsc::channel::<&'static str>();

        let setup_labels = labels(&["a"]);
        let operation_id = "e2e-select-infer-cancel".to_string();

        let result = start_inference_core(
            Arc::clone(&registry),
            move || -> AppResult<InferenceRunnerSetup<MockRunner>> {
                let runner = take_slot
                    .lock()
                    .unwrap()
                    .take()
                    .ok_or_else(|| AppError::not_found("スロットにモデルが無い"))?;
                Ok(InferenceRunnerSetup {
                    labels: setup_labels,
                    input_size: 4,
                    channel_order: ChannelOrder::Bgr,
                    runner,
                })
            },
            move |runner: MockRunner| {
                *restore_slot.lock().unwrap() = Some(runner);
                let _ = restore_tx.send("restored");
            },
            move || -> Box<dyn ProgressEmitter + Send> {
                // 最初の進捗通知を受けたら UI スレッド相当へ合図し、以降の
                // 進捗も記録を続ける（キャンセル要求のタイミング取得用）。
                struct SignalOnFirst {
                    inner: RecordingEmitter,
                    tx: mpsc::Sender<()>,
                    signaled: bool,
                }
                impl ProgressEmitter for SignalOnFirst {
                    fn emit(&mut self, progress: crate::models::Progress) {
                        if !self.signaled {
                            self.signaled = true;
                            let _ = self.tx.send(());
                        }
                        self.inner.emit(progress);
                    }
                }
                Box::new(SignalOnFirst {
                    inner: RecordingEmitter::new(),
                    tx: first_progress_tx,
                    signaled: false,
                })
            },
            |_result: &crate::models::InferBatchResult| {},
            paths,
            threshold_filter(0.5),
            Some(1),
            operation_id.clone(),
        );
        assert!(result.is_ok());

        // 最初の進捗（1 バッチ完了）を確認したら、UI スレッド相当から
        // キャンセルを要求する。
        first_progress_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("最初の進捗が時間内に届かなかった");
        let requested = registry_for_cancel.request_cancel(&operation_id);
        assert!(requested, "実行中ジョブへのキャンセル要求が登録に届いていない");

        let signal = restore_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("キャンセル後の状態復元が時間内に行われなかった");
        assert_eq!(signal, "restored");

        // 完了後にキャンセルレジストリは解放され、セッションは復元されている。
        assert!(registry.is_empty());
        assert!(model_slot_has_runner(&check_slot));
    }

    /// ダウンロード→保存先ロード→セッション保持→推論から参照可能、という
    /// 一連の流れを確認する（要件 2.5, 2.7）。
    ///
    /// `spawn_variant_download_core` の `on_downloaded` コールバック内で
    /// （本番の `model_service::load_variant` 相当として）モック
    /// ローダーを呼び出し、共有スロットへ格納する。その後、同じスロットを
    /// `start_inference_core` の `take_runner` から参照できることを確認し、
    /// 「ダウンロードしたモデルがそのまま推論に使える」結線を検証する。
    #[test]
    fn end_to_end_download_then_load_then_session_retained_then_inferable() {
        use crate::commands::progress::RecordingEmitter;
        use std::sync::mpsc;
        use std::time::Duration;

        let downloader = mock_downloader_with_valid_files();
        let dir = tempdir().unwrap();
        let dest = dir.path().join("wd14-vit");
        let registry = Arc::new(CancelRegistry::new());

        // ダウンロード完了後にロードされた「モデル」を保持するセッション
        // スロット（ModelSessionState::current 相当）。
        let session_slot: Arc<std::sync::Mutex<Option<MockRunner>>> =
            Arc::new(std::sync::Mutex::new(None));
        let session_slot_for_download = Arc::clone(&session_slot);

        let (done_tx, done_rx) = mpsc::channel::<()>();

        let handle = spawn_variant_download_core(
            downloader,
            remote_wd14_variant(),
            dest.clone(),
            Arc::clone(&registry),
            "e2e-download-load".to_string(),
            move || -> Box<dyn ProgressEmitter + Send> { Box::new(RecordingEmitter::new()) },
            move |saved: &Path| {
                // 「保存先ロード」: 実際は model_service::load_variant(saved)。
                // ここではダウンロードが成功し保存先が存在することを確認し、
                // 疑似ロード結果（MockRunner）をセッションスロットへ格納する
                // （セッション保持、要件 2.5）。
                assert!(saved.join("model.onnx").exists());
                assert!(saved.join("selected_tags.csv").exists());
                *session_slot_for_download.lock().unwrap() =
                    Some(MockRunner { output: vec![0.8] });
                let _ = done_tx.send(());
            },
        );

        let result = handle.join().expect("download thread panicked");
        assert!(result.is_ok());
        done_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("on_downloaded 経由のセッション保持が時間内に行われなかった");
        assert!(registry.is_empty());

        // 「推論から参照可能」: セッションスロットに保持された疑似モデルを
        // start_inference_core が take_runner 経由で取得し推論を実行できる。
        let dir2 = tempdir().unwrap();
        let mut paths = Vec::new();
        for i in 0..2 {
            let p = dir2.path().join(format!("dl{i}.png"));
            write_image(&p);
            paths.push(p.to_string_lossy().into_owned());
        }
        let registry2 = Arc::new(CancelRegistry::new());
        let (restore_tx, restore_rx) = mpsc::channel::<&'static str>();
        let setup_labels = labels(&["a"]);

        let infer_result = start_inference_core(
            registry2,
            move || -> AppResult<InferenceRunnerSetup<MockRunner>> {
                let runner = session_slot
                    .lock()
                    .unwrap()
                    .take()
                    .ok_or_else(|| AppError::not_found("セッションにモデルが無い"))?;
                Ok(InferenceRunnerSetup {
                    labels: setup_labels,
                    input_size: 4,
                    channel_order: ChannelOrder::Bgr,
                    runner,
                })
            },
            move |_runner: MockRunner| {
                let _ = restore_tx.send("restored");
            },
            move || -> Box<dyn ProgressEmitter + Send> { Box::new(RecordingEmitter::new()) },
            |_result: &crate::models::InferBatchResult| {},
            paths,
            threshold_filter(0.5),
            None,
            "e2e-download-then-infer".to_string(),
        );

        assert!(infer_result.is_ok());
        let signal = restore_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("ダウンロード済みモデルでの推論完了が時間内に行われなかった");
        assert_eq!(signal, "restored");
    }
}
