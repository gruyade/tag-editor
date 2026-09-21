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

use crate::error::AppResult;
use crate::models::{ImageEntry, ModelVariant, OperationReport, SortingOperation};
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

/// ローカルモデル読込の可否確認（`model_service::load_local_model` へ委譲、要件 15.2）。
///
/// [`crate::models::LoadedModel`] は `ort::Session` を保持し serde 不可のため
/// フロントエンドへは返せない。読込に成功したことのみを DTO（読込できたラベル数と
/// 入力サイズ）として返し、実セッションはアプリ層の状態管理に保持する想定
/// （タスク 21）。ここでは検証用に読込成否とメタ情報のみを返す。
#[tauri::command]
pub fn load_local_model(dir: String) -> AppResult<LoadedModelInfo> {
    let loaded = model_service::load_local_model(Path::new(&dir))?;
    Ok(LoadedModelInfo {
        input_size: loaded.input_size,
        label_count: loaded.labels.len(),
    })
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

/// リモートモデルのダウンロード・保存（`model_service::download_model` へ委譲、要件 15.3）。
///
/// 実ダウンローダ（`HfHubDownloader`）を用いる。長時間かつネットワークを伴う
/// 処理のため、アプリ層では別スレッド/タスクで spawn し進捗を通知する想定
/// （要件 16.8、[`spawn_download_job`] 参照）。保存先ディレクトリのパスを返す。
#[tauri::command]
pub fn download_model(variant: ModelVariant, dest_dir: String) -> AppResult<String> {
    let downloader = model_service::HfHubDownloader::new();
    let saved = model_service::download_model(&downloader, &variant, Path::new(&dest_dir))?;
    Ok(saved.to_string_lossy().into_owned())
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
pub fn cancel_operation(registry: tauri::State<'_, CancelRegistry>, operation_id: String) -> bool {
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
}
