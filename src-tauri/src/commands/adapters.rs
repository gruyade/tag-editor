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

/// モデル一覧（`model_service::list_models` へ委譲、要件 15.1, 15.5）。
///
/// ローカルモデルディレクトリと組み込みリモート候補から一覧を構築する。
#[tauri::command]
pub fn list_models(local_model_dir: Option<String>) -> AppResult<model_service::ModelListing> {
    let remote = model_service::builtin_remote_variants();
    let listing = match local_model_dir {
        Some(dir) => model_service::list_models(Some(Path::new(&dir)), &remote),
        None => model_service::list_models(None, &remote),
    };
    Ok(listing)
}

/// ローカルモデル読込・セッション保持（`model_service::load_local_model` へ委譲、
/// 要件 2.4, 15.2）。
///
/// [`crate::models::LoadedModel`] は `ort::Session` を保持し serde 不可のため
/// フロントエンドへは返せない。読込に成功した `LoadedModel` は
/// [`ModelSessionState::current`] に格納し、以後の推論コマンドから参照可能にする
/// （要件 2.4）。フロントエンドへは読込成否とメタ情報のみを表す軽量 DTO
/// [`LoadedModelInfo`]（`input_size`/`label_count`）を返す。この戻り値の形状は
/// 変更しない（保持 3.4）。
#[tauri::command]
pub fn load_local_model(
    state: tauri::State<'_, ModelSessionState>,
    dir: String,
) -> AppResult<LoadedModelInfo> {
    let loaded = model_service::load_local_model(Path::new(&dir))?;
    let info = LoadedModelInfo {
        input_size: loaded.input_size,
        label_count: loaded.labels.len(),
    };
    *state.current.lock().expect("ModelSessionState mutex poisoned") = Some(loaded);
    Ok(info)
}

/// [`load_local_model`] コマンドが返す serde 可能なモデルメタ情報。
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

/// リモートモデルのダウンロード・保存・セッション保持（`model_service::download_model`
/// へ委譲、要件 15.3, 2.5）。
///
/// 実ダウンローダ（`HfHubDownloader`）を用いる。長時間かつネットワークを伴う
/// 処理のため、アプリ層では別スレッド/タスクで spawn し進捗を通知する想定
/// （要件 16.8、[`spawn_download_job`] 参照）。保存先ディレクトリのパスを返す
/// （戻り値の形状は変更しない）。
///
/// ダウンロード完了後、保存先ディレクトリを [`model_service::load_local_model`] に
/// 渡してロードし、[`load_local_model`] コマンドと同じ `ModelSessionState` に
/// 保持する（要件 2.5）。ロード失敗時はダウンロード自体は成功しているため、
/// 保存先パスの返却は妨げず、セッション保持のみ行われない。
#[tauri::command]
pub fn download_model(
    state: tauri::State<'_, ModelSessionState>,
    variant: ModelVariant,
    dest_dir: String,
) -> AppResult<String> {
    let downloader = model_service::HfHubDownloader::new();
    let saved = model_service::download_model(&downloader, &variant, Path::new(&dest_dir))?;

    if let Ok(loaded) = model_service::load_local_model(&saved) {
        *state.current.lock().expect("ModelSessionState mutex poisoned") = Some(loaded);
    }

    Ok(saved.to_string_lossy().into_owned())
}

// ---------------------------------------------------------------------------
// モデルダウンロードの spawn・進捗・キャンセル（要件 2.7, 2.8, 2.9、タスク 3.5）
// ---------------------------------------------------------------------------

/// [`spawn_model_download_core`] の中核ロジック（`AppHandle` 非依存・単体テスト
/// 可能）。
///
/// [`model_service::download_model_with_progress`]（同期コア・不変）を
/// [`std::thread::spawn`] 上で実行し、即座に戻る（呼び出し元をブロックしない、
/// 要件 2.7）。取得段階（onnx 取得 → タグ定義取得 → 保存）ごとに `make_emitter`
/// が返すエミッタへ進捗を通知する（要件 2.8）。`registry` へ `operation_id` を
/// 登録して得た共有フラグを取得段の境界で確認し、キャンセル要求があれば
/// 残りの処理を中断する（要件 2.9）。処理完了（成功・失敗・キャンセル問わず）後、
/// レジストリ登録を解放する。
///
/// ダウンロード完了後、`on_downloaded`（`Send + 'static`）を呼び保存先ディレクトリ
/// を渡す。アプリ層ではここで [`model_service::load_local_model`] を呼び
/// `ModelSessionState` へ格納する（要件 2.5 と同じ経路）。
///
/// 戻り値は起動した [`std::thread::JoinHandle`]。呼び出し元は `join` を待たずに
/// 戻ることで UI スレッドを塞がない。
fn spawn_model_download_core<D, EM, ODL>(
    downloader: D,
    variant: ModelVariant,
    dest_dir: std::path::PathBuf,
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
        let mut on_phase = move |phase: model_service::DownloadPhase| {
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

        let result = model_service::download_model_with_progress(
            &downloader,
            &variant,
            &dest_dir,
            &cancel,
            &mut on_phase,
        );

        registry.clear(&operation_id);

        if let Ok(saved) = &result {
            on_downloaded(saved);
        }

        result
    })
}

/// モデルダウンロードを別スレッドで起動する（要件 2.7, 2.8, 2.9）。
///
/// [`spawn_model_download_core`] へ薄く委譲する。呼び出しは即座に戻り
/// （UI スレッドを塞がない、要件 2.7）、進捗は [`TauriProgressEmitter`] で
/// `inference://progress`（[`crate::app::PROGRESS_EVENT`]）へ emit する
/// （要件 2.8）。キャンセルは [`cancel_operation`] コマンド経由の
/// [`CancelRegistry`] を通じて取得段の境界で反映される（要件 2.9）。
///
/// ダウンロード完了後、保存先ディレクトリを [`model_service::load_local_model`]
/// に渡してロードし、[`load_local_model`]/[`download_model`] コマンドと同じ
/// `ModelSessionState` に保持する（要件 2.5 と同じ経路）。ロード失敗時は
/// ダウンロード自体の成否には影響しない（セッション保持のみ行われない）。
///
/// 既存同期コマンド [`download_model`] は互換のため残し、同期コア
/// [`model_service::download_model`] の再試行・原子的保存・部分ファイル除去の
/// 挙動は変更しない（保持 3.2, 3.5）。
#[tauri::command]
pub fn spawn_model_download(
    model_state: tauri::State<'_, ModelSessionState>,
    registry: tauri::State<'_, Arc<CancelRegistry>>,
    app: tauri::AppHandle,
    variant: ModelVariant,
    dest_dir: String,
    operation_id: String,
) -> AppResult<()> {
    let registry_arc: Arc<CancelRegistry> = Arc::clone(&registry);
    let model_slot = model_state.current_slot();

    spawn_model_download_core(
        model_service::HfHubDownloader::new(),
        variant,
        std::path::PathBuf::from(dest_dir),
        registry_arc,
        operation_id,
        move || -> Box<dyn ProgressEmitter + Send> { Box::new(TauriProgressEmitter::new(app)) },
        move |saved: &Path| {
            if let Ok(loaded) = model_service::load_local_model(saved) {
                *model_slot
                    .lock()
                    .expect("ModelSessionState mutex poisoned") = Some(loaded);
            }
        },
    );

    Ok(())
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
    /// 採用の下限信頼度（0.0〜1.0、要件 14.4）。
    pub threshold: f32,
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
) -> crate::services::inference_service::InferResult {
    // キャンセルフラグを登録し、共有ハンドルを処理へ渡す（要件 17.7）。
    let cancel = registry.register(&job.operation_id);

    let mut callback = |p: crate::models::Progress| emitter.emit(p);

    let result = crate::services::inference_service::run_inference(
        runner,
        &job.image_paths,
        job.threshold,
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
) -> std::thread::JoinHandle<crate::services::inference_service::InferResult>
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
/// `take_runner` は「`model_slot`（[`ModelSessionState::current`] と同じ形の
/// 共有スロット）からロード済みモデルを `take` し、runner を構築して
/// [`InferenceRunnerSetup`] を返す」処理を表す。セッション未ロードの場合は
/// `None` を返す。`restore` は推論完了後に runner から取り出した状態を
/// 呼び出し元スロットへ書き戻す処理を表す。
///
/// この 2 つを呼び出し側から注入可能にすることで、既存設計思想（実 ort 実行は
/// [`crate::services::inference_service::SessionRunner`] トレイト境界の背後に
/// 隔離し、単体テストは `MockRunner` を使う）に合わせ、`start_inference_core`
/// 自体は実 `ort::Session`／[`crate::models::LoadedModel`] に依存しない。
/// 本番では `take_runner` の中で `model_slot` から `LoadedModel` を `take` して
/// [`OwnedOrtRunner::new`] へ渡し、`restore` に [`OwnedOrtRunner::into_inner`]
/// を渡す（[`start_inference`] 参照）。
///
/// セッション未ロード（`take_runner` が `None` を返す）の場合は `AppError`
/// （[`crate::error::AppErrorKind::ModelLoad`]）を返し、クラッシュしない
/// （要件 2.6）。
///
/// 進捗は `make_emitter`（呼び出しごとに新しい `Box<dyn ProgressEmitter + Send>`
/// を生成するファクトリ）が返すエミッタへ emit する（要件 2.2）。本関数は
/// スレッド起動後すぐに戻り、`JoinHandle::join` を待たない（UI スレッドを
/// 塞がない、要件 2.1）。
///
/// 同期コア（[`run_inference_job`]/[`spawn_inference_job`]/[`InferenceJob`]/
/// [`crate::services::inference_service::SessionRunner`]/[`OwnedOrtRunner`]）は
/// 変更しない（保持 3.2）。`AppHandle`/`tauri::State` に依存しないため、実
/// Tauri ランタイムなしで単体テストできる。
fn start_inference_core<R, TR, RS, EM>(
    registry: Arc<CancelRegistry>,
    take_runner: TR,
    restore: RS,
    make_emitter: EM,
    image_paths: Vec<String>,
    threshold: f32,
    batch_size: Option<u32>,
    operation_id: String,
) -> AppResult<()>
where
    R: crate::services::inference_service::SessionRunner + Send + 'static,
    TR: FnOnce() -> Option<InferenceRunnerSetup<R>>,
    RS: FnOnce(R) + Send + 'static,
    EM: FnOnce() -> Box<dyn ProgressEmitter + Send> + Send + 'static,
{
    // セッション未ロードなら明確な AppError を返す（要件 2.6）。クラッシュしない。
    let setup = take_runner().ok_or_else(|| AppError::model_load("モデルが選択/ロードされていない"))?;

    let job = InferenceJob {
        image_paths,
        threshold,
        batch_size,
        labels: setup.labels,
        input_size: setup.input_size,
        channel_order: setup.channel_order,
        operation_id,
    };
    let runner = setup.runner;

    // バックグラウンドスレッドで同期コアを実行し、完了後に runner の状態を
    // 呼び出し元スロットへ書き戻す（次回推論のためセッションを保持し続ける）。
    std::thread::spawn(move || {
        let mut emitter = make_emitter();
        let _result = run_inference_job(&runner, &job, &registry, &mut *emitter);
        restore(runner);
    });

    Ok(())
}

/// バッチ推論を起動する（要件 2.1, 2.2, 2.3, 2.6）。
///
/// [`start_inference_core`] へ薄く委譲する。`model_state`/`registry` は
/// いずれも内部が `Arc` 共有ハンドル（[`ModelSessionState::current_slot`] /
/// `Arc<CancelRegistry>` の `State`）のため、複製してバックグラウンドスレッドへ
/// move できる（`tauri::State<'_, T>` 自体はライフタイム付きで move 不可）。
/// 進捗は [`TauriProgressEmitter`] で `inference://progress`
/// （[`crate::app::PROGRESS_EVENT`]）へ emit する。
#[tauri::command]
pub fn start_inference(
    model_state: tauri::State<'_, ModelSessionState>,
    registry: tauri::State<'_, Arc<CancelRegistry>>,
    app: tauri::AppHandle,
    image_paths: Vec<String>,
    threshold: f32,
    batch_size: Option<u32>,
    operation_id: String,
) -> AppResult<()> {
    let model_slot = model_state.current_slot();
    let registry_arc: Arc<CancelRegistry> = Arc::clone(&registry);
    let restore_slot = Arc::clone(&model_slot);

    start_inference_core(
        registry_arc,
        move || -> Option<InferenceRunnerSetup<OwnedOrtRunner>> {
            let model = model_slot
                .lock()
                .expect("ModelSessionState mutex poisoned")
                .take()?;
            Some(InferenceRunnerSetup {
                labels: model.labels.clone(),
                input_size: model.input_size,
                channel_order: model.channel_order,
                runner: OwnedOrtRunner::new(model),
            })
        },
        move |runner: OwnedOrtRunner| {
            let restored = runner.into_inner();
            *restore_slot
                .lock()
                .expect("ModelSessionState mutex poisoned") = Some(restored);
        },
        move || -> Box<dyn ProgressEmitter + Send> { Box::new(TauriProgressEmitter::new(app)) },
        image_paths,
        threshold,
        batch_size,
        operation_id,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::AppErrorKind;
    use crate::models::{ChannelOrder, LabelDef, Progress, TagCategory};
    use crate::services::inference_service::SessionRunner;
    use crate::AppResult as CoreResult;
    use std::path::Path as StdPath;
    use tempfile::tempdir;

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
            threshold: 0.5,
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
            threshold: 0.5,
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
            threshold: 0.5,
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
            threshold: 0.5,
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
            threshold: 0.5,
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

    /// セッション未ロード（`take_runner` が `None` を返す）場合に
    /// `AppError`（`AppErrorKind::ModelLoad`）を返し、パニックしないことを
    /// 検証する（要件 2.6）。
    #[test]
    fn start_inference_core_returns_model_load_error_when_unloaded() {
        use crate::commands::progress::RecordingEmitter;

        let registry = Arc::new(CancelRegistry::new());

        let result = start_inference_core::<MockRunner, _, _, _>(
            Arc::clone(&registry),
            || None,
            |_runner: MockRunner| {
                panic!("未ロード時は restore が呼ばれてはならない");
            },
            || -> Box<dyn ProgressEmitter + Send> { Box::new(RecordingEmitter::new()) },
            vec!["dummy.png".to_string()],
            0.5,
            None,
            "unloaded-job".to_string(),
        );

        let err = result.unwrap_err();
        assert_eq!(err.kind, AppErrorKind::ModelLoad);
        // 未登録のためレジストリは空のまま。
        assert!(registry.is_empty());
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
        use crate::commands::progress::RecordingEmitter;
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
        let recorded: Arc<std::sync::Mutex<Option<RecordingEmitter>>> =
            Arc::new(std::sync::Mutex::new(None));
        let recorded_for_emitter = Arc::clone(&recorded);

        let setup_labels = labels(&["a"]);

        let result = start_inference_core(
            Arc::clone(&registry),
            move || {
                Some(InferenceRunnerSetup {
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
                let emitter = RecordingEmitter::new();
                // emitter 自体はスレッドへ move するため、テスト側では
                // 完了後に別途 events を検証できないので、ここでは
                // 完了合図だけを担う軽量ラッパーへ差し替える。
                struct Sharing {
                    inner: RecordingEmitter,
                    shared: Arc<std::sync::Mutex<Option<RecordingEmitter>>>,
                }
                impl ProgressEmitter for Sharing {
                    fn emit(&mut self, progress: crate::models::Progress) {
                        self.inner.emit(progress);
                    }
                }
                impl Drop for Sharing {
                    fn drop(&mut self) {
                        let events = std::mem::take(&mut self.inner);
                        *self.shared.lock().unwrap() = Some(events);
                    }
                }
                Box::new(Sharing {
                    inner: emitter,
                    shared: recorded_for_emitter,
                })
            },
            paths,
            0.5,
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
        // レジストリは解放済み・進捗は蓄積済みのはず。
        assert!(registry.is_empty());

        let events = recorded.lock().unwrap().take().expect("進捗が記録されていない");
        assert_eq!(events.events.len(), 3);
        assert_eq!(events.events.last().unwrap().done, 3);
        for ev in &events.events {
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
            threshold: 0.5,
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
    // タスク 3.5 テスト: spawn_model_download_core（別スレッド即戻り・進捗通知・
    // キャンセル中断）と model_service::download_model_with_progress（既存
    // download_model の再試行・原子的保存・部分ファイル除去のリグレッション無し）。
    // -----------------------------------------------------------------------

    use crate::services::model_service::{
        self, download_model_tests::MockDownloader, DownloadError,
    };

    /// テスト用のリモート WD14 バリアント。
    fn remote_wd14_variant() -> ModelVariant {
        ModelVariant {
            id: "wd14-vit".to_string(),
            display_name: "WD14 ViT".to_string(),
            family: crate::models::ModelFamily::Wd14,
            location: crate::models::ModelLocation::Remote("owner/wd14-vit".to_string()),
            onnx_available: true,
        }
    }

    /// 妥当な CSV タグ定義を返すモックダウンローダを組み立てる。
    fn mock_downloader_with_valid_files() -> MockDownloader {
        MockDownloader::with_responses(&[
            ("model.onnx", b"onnx-bytes"),
            ("selected_tags.csv", b"tag_id,name,category\n1,solo,0\n"),
        ])
    }

    /// (要件 2.7) `spawn_model_download_core` が別スレッドで実行され、
    /// 呼び出し元は `join` を待たずに即座に戻ることを検証する。
    #[test]
    fn spawn_model_download_core_returns_immediately_without_blocking() {
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
        let handle = spawn_model_download_core(
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
            "spawn_model_download_core は即座に戻るべき"
        );

        // バックグラウンドで完了するまで待つ（テスト側の後始末のため）。
        let _ = done_rx.recv_timeout(Duration::from_secs(5));
        let result = handle.join().expect("download thread panicked");
        assert!(result.is_ok());
        assert!(registry.is_empty());
    }

    /// (要件 2.8) 取得段階（onnx 取得 → タグ定義取得 → 保存）に応じて進捗が
    /// 段階的に通知されることを検証する。
    #[test]
    fn spawn_model_download_core_emits_progress_per_phase() {
        use crate::commands::progress::RecordingEmitter;
        use std::sync::mpsc;

        let downloader = mock_downloader_with_valid_files();
        let dir = tempdir().unwrap();
        let dest = dir.path().join("wd14-vit");
        let registry = Arc::new(CancelRegistry::new());

        let (events_tx, events_rx) = mpsc::channel::<Vec<crate::models::Progress>>();

        let handle = spawn_model_download_core(
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

    /// (要件 2.9) `CancelRegistry` 経由でキャンセル要求すると、取得段の境界で
    /// 処理が中断されることを検証する。
    ///
    /// onnx 取得完了直後（1 段階目の進捗通知内）でキャンセルを要求し、以降の
    /// タグ定義取得・保存が行われないことを確認する。
    #[test]
    fn spawn_model_download_core_is_interrupted_by_cancel_registry() {
        use crate::commands::progress::RecordingEmitter;

        let downloader = mock_downloader_with_valid_files();
        let dir = tempdir().unwrap();
        let dest = dir.path().join("wd14-vit");
        let registry = Arc::new(CancelRegistry::new());
        let registry_for_cancel = Arc::clone(&registry);

        let handle = spawn_model_download_core(
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

    /// (保持 3.2, 3.5) `model_service::download_model_with_progress` が
    /// キャンセルされない通常経路で既存 `download_model` と同じ結果
    /// （保存先パス・ファイル内容）を返すことを確認する（リグレッション無し）。
    #[test]
    fn download_model_with_progress_matches_existing_download_model_on_success() {
        use std::sync::atomic::AtomicBool;

        let csv = b"tag_id,name,category\n1,solo,0\n";
        let downloader_a =
            MockDownloader::with_responses(&[("model.onnx", b"onnx-a"), ("selected_tags.csv", csv)]);
        let downloader_b =
            MockDownloader::with_responses(&[("model.onnx", b"onnx-a"), ("selected_tags.csv", csv)]);

        let dir = tempdir().unwrap();
        let dest_sync = dir.path().join("sync");
        let dest_progress = dir.path().join("progress");

        let sync_result =
            model_service::download_model(&downloader_a, &remote_wd14_variant(), &dest_sync)
                .unwrap();

        let cancel = AtomicBool::new(false);
        let mut phases = Vec::new();
        let progress_result = model_service::download_model_with_progress(
            &downloader_b,
            &remote_wd14_variant(),
            &dest_progress,
            &cancel,
            |phase| phases.push(phase),
        )
        .unwrap();

        assert_eq!(sync_result, dest_sync);
        assert_eq!(progress_result, dest_progress);
        assert_eq!(
            std::fs::read(sync_result.join("model.onnx")).unwrap(),
            std::fs::read(progress_result.join("model.onnx")).unwrap()
        );
        assert_eq!(
            std::fs::read(sync_result.join("selected_tags.csv")).unwrap(),
            std::fs::read(progress_result.join("selected_tags.csv")).unwrap()
        );
        // 3 段階（onnx 取得 → タグ定義取得 → 保存）全てが通知される。
        assert_eq!(phases.len(), 3);
    }

    /// (保持 3.2, 3.5) `download_model_with_progress` も既存の再試行上限
    /// （最大 3 回）を尊重することを確認する。
    #[test]
    fn download_model_with_progress_preserves_retry_limit() {
        use std::sync::atomic::AtomicBool;

        let downloader = MockDownloader::always_failing(DownloadError::Timeout);
        let dir = tempdir().unwrap();
        let dest = dir.path().join("wd14-vit");
        let cancel = AtomicBool::new(false);

        let err = model_service::download_model_with_progress(
            &downloader,
            &remote_wd14_variant(),
            &dest,
            &cancel,
            |_phase| {},
        )
        .unwrap_err();

        assert_eq!(err.kind, AppErrorKind::Download);
        // 既存 download_model と同じ再試行回数（3 回）。
        assert_eq!(downloader.call_count(), 3);
        assert!(!dest.join("model.onnx").exists());
    }

    /// (保持 3.2, 3.5) 保存失敗時に部分ファイルが残らないこと（既存
    /// `download_model` と同じ挙動）を確認する。
    #[test]
    fn download_model_with_progress_removes_partial_files_on_save_failure() {
        use std::sync::atomic::AtomicBool;

        let downloader = mock_downloader_with_valid_files();
        let dir = tempdir().unwrap();
        // dest 自体をファイルにして保存（ディレクトリ作成）を失敗させる。
        let dest_as_file = dir.path().join("occupied");
        std::fs::write(&dest_as_file, b"i am a file").unwrap();
        let cancel = AtomicBool::new(false);

        let err = model_service::download_model_with_progress(
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
    // 既存テスト（`start_inference_core_*` / `spawn_model_download_core_*`）は
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
            move || {
                let runner = take_slot.lock().unwrap().take()?;
                Some(InferenceRunnerSetup {
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
            paths,
            0.5,
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
            move || {
                let runner = take_slot.lock().unwrap().take()?;
                Some(InferenceRunnerSetup {
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
            paths,
            0.5,
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
    /// `spawn_model_download_core` の `on_downloaded` コールバック内で
    /// （本番の `model_service::load_local_model` 相当として）モック
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

        let handle = spawn_model_download_core(
            downloader,
            remote_wd14_variant(),
            dest.clone(),
            Arc::clone(&registry),
            "e2e-download-load".to_string(),
            move || -> Box<dyn ProgressEmitter + Send> { Box::new(RecordingEmitter::new()) },
            move |saved: &Path| {
                // 「保存先ロード」: 実際は model_service::load_local_model(saved)。
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
            move || {
                let runner = session_slot.lock().unwrap().take()?;
                Some(InferenceRunnerSetup {
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
            paths,
            0.5,
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
