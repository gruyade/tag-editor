// Feature: inference-model-wiring (bugfix), Property 2: Preservation
//
// Bug_Condition に該当しない入力（isBugCondition == false）に対し、修正版は
// 既存版と同じ結果を返す。本テストは「未修正コードの現在の挙動を観測し、その
// ベースラインをプロパティとして固定する」観測優先方法論に基づく。
// 実装後（タスク 3.10）に同一テストを再実行し、リグレッションが無いことを確認する。
//
// 検証対象（bugfix.md 3.1〜3.10 / design.md Preservation Requirements）:
//   3.1  既存サービスコマンド群の戻り値・エラー種別（list_images / bulk_add_tags /
//        sort_files / rename_regex / find_orphan_captions / capabilities /
//        create_symlink / convert_path）
//   3.2  同期コア（run_inference_job / spawn_inference_job）の進捗単調非減少・
//        最終 done==total・キャンセル反映・部分失敗時の処理前状態保持
//   3.3  list_models のローカル/リモート一覧と ONNX 非対応の available/excluded 区分
//   3.4  LoadedModelInfo の input_size / label_count 形状
//   3.5  cancel_operation 相当（CancelRegistry）の登録済み true / 未登録 false
//   3.6  非 Windows で capabilities の windows_only == false（= cfg!(windows)）
//   3.7  破損画像で placeholder == true
//   3.8  サムネイル 64〜512 クランプとアスペクト比保持縮小（拡大なし）
//   3.9  プレビュー MAX_PREVIEW_SIZE(2048) 超のみ内接縮小（以下は元寸法）
//   3.10 PreviewData / ThumbnailData の width / height / png / placeholder 形状
//
// Validates: Requirements 3.1, 3.2, 3.3, 3.4, 3.5, 3.6, 3.7, 3.8, 3.9, 3.10
//
// 既存 PBT（tests/pbt_*）のジェネレータ・パターンを流用する:
//   - model_service 分割: pbt_model_onnx_filter.rs の variant / candidates 生成器
//   - サムネイルクランプ: pbt_file_service_clamp.rs の size_strategy
//   - バッチ分割・件数: pbt_inference_batches.rs の件数レンジ
//
// いずれのプロパティも「未修正コードで観測される現在の挙動」を主張する。実装は
// 一切変更しない（コア・DTO・コマンドの追加のみで満たされる保持条件）。

use std::path::Path;
use std::sync::Arc;

use proptest::prelude::*;

use tag_editor_core::commands::adapters::{
    self, run_inference_job, spawn_inference_job, InferenceJob, LoadedModelInfo,
};
use tag_editor_core::commands::cancel::CancelRegistry;
use tag_editor_core::commands::progress::RecordingEmitter;
use tag_editor_core::error::AppErrorKind;
use tag_editor_core::models::{ChannelOrder, LabelDef, ModelFamily, ModelLocation, ModelVariant, TagCategory};
use tag_editor_core::services::inference_service::SessionRunner;
use tag_editor_core::services::model_service::list_models;
use tag_editor_core::services::platform_service::{capabilities, convert_path, ConvertDirection};
use tag_editor_core::services::{file_service, model_service};

// ===========================================================================
// 共通ヘルパー / モック
// ===========================================================================

/// 固定の確信度ベクトルを返すモック実行器（既存 adapters/inference の MockRunner と同型）。
struct MockRunner {
    output: Vec<f32>,
}
impl SessionRunner for MockRunner {
    fn run(&self, _input: &[f32]) -> tag_editor_core::AppResult<Vec<f32>> {
        Ok(self.output.clone())
    }
}

/// ラベル定義を組み立てる。
fn labels(names: &[&str]) -> Vec<LabelDef> {
    names
        .iter()
        .map(|n| LabelDef {
            name: (*n).to_string(),
            category: TagCategory::General,
        })
        .collect()
}

/// 指定寸法の単色 PNG を書き出す。
fn write_png(dir: &Path, name: &str, w: u32, h: u32) -> std::path::PathBuf {
    let img = image::RgbaImage::from_pixel(w.max(1), h.max(1), image::Rgba([90, 150, 200, 255]));
    let path = dir.join(name);
    img.save(&path).unwrap();
    path
}

/// 推論用の小さな有効 PNG を書き出す。
fn write_small_png(path: &Path) {
    use image::{Rgb, RgbImage};
    let mut img = RgbImage::new(2, 2);
    for p in img.pixels_mut() {
        *p = Rgb([100, 110, 120]);
    }
    img.save(path).unwrap();
}

// ===========================================================================
// 3.3 list_models: ローカル/リモート一覧と ONNX 区分（pbt_model_onnx_filter 流用）
// ===========================================================================

fn family_strategy() -> impl Strategy<Value = ModelFamily> {
    prop_oneof![
        Just(ModelFamily::Wd14),
        Just(ModelFamily::MlDanbooru),
        Just(ModelFamily::Local),
    ]
}

fn location_strategy() -> impl Strategy<Value = ModelLocation> {
    prop_oneof![
        "[a-z0-9/_-]{1,20}".prop_map(ModelLocation::Remote),
        "[a-z0-9/_.-]{1,20}".prop_map(ModelLocation::Local),
    ]
}

fn variant_strategy() -> impl Strategy<Value = ModelVariant> {
    (
        "[a-z0-9_-]{1,16}",
        "[A-Za-z0-9 _-]{1,20}",
        family_strategy(),
        location_strategy(),
        any::<bool>(),
    )
        .prop_map(|(id, display_name, family, location, onnx_available)| ModelVariant {
            id,
            display_name,
            family,
            location,
            onnx_available,
        })
}

fn candidates_strategy() -> impl Strategy<Value = Vec<ModelVariant>> {
    prop::collection::vec(variant_strategy(), 0..24)
}

// ===========================================================================
// 3.8 サムネイルサイズ入力（pbt_file_service_clamp 流用）
// ===========================================================================

fn size_strategy() -> impl Strategy<Value = u32> {
    prop_oneof![
        any::<u32>(),
        0_u32..=1024_u32,
        Just(0_u32),
        Just(file_service::MIN_THUMBNAIL_SIZE - 1),
        Just(file_service::MIN_THUMBNAIL_SIZE),
        Just(file_service::MAX_THUMBNAIL_SIZE),
        Just(file_service::MAX_THUMBNAIL_SIZE + 1),
        Just(u32::MAX),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    // -----------------------------------------------------------------------
    // 3.3: list_models の available/excluded 区分が onnx_available で決まる
    // （非該当入力 = モデル一覧化。修正は結線追加のみで一覧化ロジックは不変）
    // -----------------------------------------------------------------------
    #[test]
    fn preserve_list_models_partition_by_onnx(candidates in candidates_strategy()) {
        let listing = list_models(None, &candidates);

        let expected_available: Vec<ModelVariant> =
            candidates.iter().filter(|v| v.onnx_available).cloned().collect();
        let expected_excluded: Vec<ModelVariant> =
            candidates.iter().filter(|v| !v.onnx_available).cloned().collect();

        prop_assert_eq!(&listing.available, &expected_available);
        prop_assert_eq!(&listing.excluded, &expected_excluded);
        // 分割は入力を過不足なく覆う。
        prop_assert_eq!(
            listing.available.len() + listing.excluded.len(),
            candidates.len()
        );
    }

    // -----------------------------------------------------------------------
    // 3.8: サムネイルは 64〜512 にクランプし、アスペクト比を保って内接縮小し
    // 拡大しない。placeholder=false 時 png は非空、DTO 形状（3.10）を伴う。
    // -----------------------------------------------------------------------
    #[test]
    fn preserve_thumbnail_clamp_and_no_upscale(
        req_size in size_strategy(),
        w in 1_u32..=1200,
        h in 1_u32..=1200,
    ) {
        let dir = tempfile::tempdir().unwrap();
        let path = write_png(dir.path(), "t.png", w, h);

        // コマンドアダプタ経由（Ok でラップされる）で観測する。
        let thumb = adapters::get_thumbnail(path.to_string_lossy().into_owned(), req_size).unwrap();

        prop_assert!(!thumb.placeholder, "有効画像は placeholder にならない");
        prop_assert!(!thumb.png.is_empty(), "placeholder=false のとき png は非空");

        let clamped = file_service::clamp_thumbnail_size(req_size);

        // 生成寸法はクランプ後サイズに内接する（辺がクランプ値を超えない）。
        prop_assert!(thumb.width <= clamped && thumb.height <= clamped);
        // 拡大しない: 生成寸法は元寸法を超えない。
        prop_assert!(thumb.width <= w && thumb.height <= h);

        // アスペクト比保持（縮小時）。元がクランプ後サイズ以下なら元寸法のまま。
        if w <= clamped && h <= clamped {
            prop_assert_eq!(thumb.width, w);
            prop_assert_eq!(thumb.height, h);
        } else {
            // 縮小後はいずれかの辺がクランプ値に達する（内接）。
            prop_assert!(thumb.width == clamped || thumb.height == clamped,
                "縮小後にいずれの辺もクランプ値に達していない: {}x{} clamped={}",
                thumb.width, thumb.height, clamped);
        }
    }

    // -----------------------------------------------------------------------
    // 3.2: run_inference_job の進捗は単調非減少・最終 done==total・完了で
    // レジストリ解放（同期コアは不変。任意件数・batch_size で観測）。
    // -----------------------------------------------------------------------
    #[test]
    fn preserve_inference_progress_monotonic_and_final_equals_total(
        n in 1_usize..=10,
        batch in 1_u32..=6,
    ) {
        let dir = tempfile::tempdir().unwrap();
        let mut paths = Vec::new();
        for i in 0..n {
            let p = dir.path().join(format!("img{i}.png"));
            write_small_png(&p);
            paths.push(p.to_string_lossy().into_owned());
        }

        let runner = MockRunner { output: vec![0.9] };
        let registry = CancelRegistry::new();
        let mut emitter = RecordingEmitter::new();

        let op = format!("preserve-op-{n}-{batch}");
        let job = InferenceJob {
            image_paths: paths,
            threshold: 0.5,
            batch_size: Some(batch),
            labels: labels(&["a"]),
            input_size: 4,
            channel_order: ChannelOrder::Bgr,
            operation_id: op.clone(),
        };

        let result = run_inference_job(&runner, &job, &registry, &mut emitter);

        // 全件成功（有効画像・失敗なし）。
        prop_assert_eq!(result.succeeded, n);
        prop_assert_eq!(result.failed, 0);
        prop_assert_eq!(result.cancelled, 0);

        // 進捗は n 件通知され、total は一定、done は単調非減少で最終 == total。
        prop_assert_eq!(emitter.events.len(), n);
        for ev in &emitter.events {
            prop_assert_eq!(ev.total, n);
            prop_assert_eq!(&ev.operation_id, &op);
        }
        for w in emitter.events.windows(2) {
            prop_assert!(w[1].done >= w[0].done, "done が単調非減少でない");
        }
        prop_assert_eq!(emitter.events.last().unwrap().done, n);

        // 完了後にレジストリ登録は解放される（要件 17.7）。
        prop_assert!(registry.is_empty());
    }

    // -----------------------------------------------------------------------
    // 3.5: CancelRegistry（cancel_operation の実体）は登録済み true / 未登録 false。
    // -----------------------------------------------------------------------
    #[test]
    fn preserve_cancel_registry_registered_true_unregistered_false(
        op in "[a-z0-9_-]{1,20}",
        other in "[a-z0-9_-]{1,20}",
    ) {
        prop_assume!(op != other);
        let registry = CancelRegistry::new();

        // 未登録は false。
        prop_assert!(!registry.request_cancel(&op));

        // 登録すると true を返し、以後 is_cancelled も true。
        let _flag = registry.register(&op);
        prop_assert!(registry.request_cancel(&op));
        prop_assert!(registry.is_cancelled(&op));

        // 別の未登録 id は依然 false。
        prop_assert!(!registry.request_cancel(&other));
    }

    // -----------------------------------------------------------------------
    // 3.4: LoadedModelInfo の DTO 形状（input_size / label_count）が保たれる。
    // 実 ort セッション構築は環境依存のためここでは DTO 形状のみを固定する。
    // -----------------------------------------------------------------------
    #[test]
    fn preserve_loaded_model_info_shape(
        input_size in any::<u32>(),
        label_count in any::<usize>(),
    ) {
        let info = LoadedModelInfo { input_size, label_count };
        // フィールドがそのまま保持され、Copy/Eq セマンティクスも不変。
        prop_assert_eq!(info.input_size, input_size);
        prop_assert_eq!(info.label_count, label_count);
        let copied = info;
        prop_assert_eq!(copied, info);
    }
}

proptest! {
    // プレビューは大きな PNG のエンコードを伴い 1 ケースが重いため、ケース数を
    // 絞った別ブロックにする。境界（MAX_PREVIEW_SIZE=2048）の両側を密に踏む。
    #![proptest_config(ProptestConfig::with_cases(48))]

    // -----------------------------------------------------------------------
    // 3.9: プレビューは MAX_PREVIEW_SIZE(2048) 超のみ内接縮小、以下は元寸法。
    // DTO 形状（3.10）: width/height/png/placeholder を伴う。
    // -----------------------------------------------------------------------
    #[test]
    fn preserve_preview_downscale_only_over_max(
        w in prop_oneof![1_u32..=64, 2000_u32..=2100, 2500_u32..=2600],
        h in prop_oneof![1_u32..=64, 2000_u32..=2100, 2500_u32..=2600],
    ) {
        let dir = tempfile::tempdir().unwrap();
        let path = write_png(dir.path(), "p.png", w, h);

        let preview = adapters::get_preview(path.to_string_lossy().into_owned()).unwrap();
        let max = file_service::MAX_PREVIEW_SIZE;

        prop_assert!(!preview.placeholder);
        prop_assert!(!preview.png.is_empty());

        if w <= max && h <= max {
            // MAX 以下は元寸法のまま。
            prop_assert_eq!(preview.width, w);
            prop_assert_eq!(preview.height, h);
        } else {
            // MAX 超はアスペクト比を保って内接縮小（辺は MAX 以下、いずれかが MAX）。
            prop_assert!(preview.width <= max && preview.height <= max);
            prop_assert!(preview.width == max || preview.height == max,
                "内接縮小後にいずれの辺も MAX に達していない: {}x{}",
                preview.width, preview.height);
        }
    }
}

// ===========================================================================
// 固定ケース: プロパティを補完する決定的な観測
// ===========================================================================

// -- 3.1: 既存サービスコマンドの戻り値・エラー種別 --------------------------

#[test]
fn preserve_list_images_command_lists_supported_and_errors_on_missing() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("a.png"), b"").unwrap();
    std::fs::write(dir.path().join("b.txt"), b"").unwrap();

    let listing = adapters::list_images(dir.path().to_string_lossy().into_owned()).unwrap();
    let names: Vec<&str> = listing.items.iter().map(|e| e.file_name.as_str()).collect();
    assert_eq!(names, vec!["a.png"]);
    assert!(!listing.truncated);

    // 存在しないフォルダは NotFound。
    let missing = dir.path().join("nope");
    let err = adapters::list_images(missing.to_string_lossy().into_owned()).unwrap_err();
    assert_eq!(err.kind, AppErrorKind::NotFound);
}

#[test]
fn preserve_bulk_add_tags_command_writes_tag_file() {
    let dir = tempfile::tempdir().unwrap();
    let img = dir.path().join("i.png");
    std::fs::write(&img, b"").unwrap();

    let report = adapters::bulk_add_tags(
        vec![img.to_string_lossy().into_owned()],
        vec!["new".to_string()],
    )
    .unwrap();
    assert_eq!(report.succeeded, 1);
    assert_eq!(
        std::fs::read_to_string(dir.path().join("i.txt")).unwrap(),
        "new"
    );
}

#[test]
fn preserve_sort_files_command_moves_matching_files() {
    use tag_editor_core::models::SortingOperation;
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("src");
    let dest = dir.path().join("dst");
    std::fs::create_dir(&source).unwrap();
    std::fs::create_dir(&dest).unwrap();
    // gather 用に接頭辞付きサブフォルダを用意。
    let sub = source.join("cat");
    std::fs::create_dir(&sub).unwrap();
    std::fs::write(sub.join("a.png"), b"x").unwrap();

    let report = adapters::sort_files(
        SortingOperation::Gather,
        source.to_string_lossy().into_owned(),
        dest.to_string_lossy().into_owned(),
    )
    .unwrap();
    // gather はサブフォルダ名を接頭辞（PREFIX_DELIMITER="__"）にして単一宛先へ集約する。
    assert!(report.succeeded >= 1);
    assert!(dest.join("cat__a.png").exists());
}

#[test]
fn preserve_rename_regex_command_returns_report() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("foo1.png"), b"").unwrap();
    std::fs::write(dir.path().join("foo2.png"), b"").unwrap();

    let report = adapters::rename_regex(
        dir.path().to_string_lossy().into_owned(),
        "foo".to_string(),
        "bar".to_string(),
        None,
    )
    .unwrap();
    assert_eq!(report.succeeded, 2);
    assert!(dir.path().join("bar1.png").exists());
    assert!(dir.path().join("bar2.png").exists());
}

#[test]
fn preserve_find_orphan_captions_command_lists_orphans() {
    let dir = tempfile::tempdir().unwrap();
    // 画像を伴わない .txt は孤立キャプション。
    std::fs::write(dir.path().join("orphan.txt"), b"tags").unwrap();
    // 画像を伴う .txt は孤立ではない。
    std::fs::write(dir.path().join("paired.png"), b"").unwrap();
    std::fs::write(dir.path().join("paired.txt"), b"tags").unwrap();

    let orphans = adapters::find_orphan_captions(dir.path().to_string_lossy().into_owned()).unwrap();
    assert_eq!(orphans.len(), 1);
    assert!(orphans[0].ends_with("orphan.txt"));
}

#[test]
fn preserve_capabilities_command_matches_cfg_windows() {
    // 3.6: 非 Windows で windows_only == false（一般に cfg!(windows) と一致）。
    let caps = adapters::capabilities().unwrap();
    assert_eq!(caps.windows_only, cfg!(windows));
    // サービス層直呼びとも一致。
    assert_eq!(capabilities().windows_only, cfg!(windows));
}

#[test]
fn preserve_convert_path_command_delegates_and_errors() {
    // 正常系: Win→Linux。
    let out = adapters::convert_path(
        "C:\\foo\\bar".to_string(),
        ConvertDirection::WindowsToLinux,
    )
    .unwrap();
    assert_eq!(out, "/mnt/c/foo/bar");
    // サービス層直呼びとも一致。
    assert_eq!(
        convert_path("C:\\foo\\bar", ConvertDirection::WindowsToLinux).unwrap(),
        "/mnt/c/foo/bar"
    );

    // 異常系: 変換規則に適合しない入力は InvalidInput。
    let err = adapters::convert_path(
        "foo\\bar".to_string(),
        ConvertDirection::WindowsToLinux,
    )
    .unwrap_err();
    assert_eq!(err.kind, AppErrorKind::InvalidInput);
}

#[test]
fn preserve_create_symlink_command_missing_target_is_not_found() {
    // 3.1: create_symlink のエラー種別（リンク元不在 = NotFound）を観測。
    // 事前検査はプラットフォーム非依存で決定的。
    let dir = tempfile::tempdir().unwrap();
    let missing_target = dir.path().join("no_such_target");
    let link = dir.path().join("link");

    let err = adapters::create_symlink(
        missing_target.to_string_lossy().into_owned(),
        link.to_string_lossy().into_owned(),
    )
    .unwrap_err();
    assert_eq!(err.kind, AppErrorKind::NotFound);
    assert!(std::fs::symlink_metadata(&link).is_err());
}

// -- 3.2: spawn_inference_job（別スレッド起動）とキャンセル反映 --------------

#[test]
fn preserve_spawn_inference_job_runs_and_clears_registry() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("s.png");
    write_small_png(&p);

    let runner = MockRunner { output: vec![0.9] };
    let registry = Arc::new(CancelRegistry::new());
    let emitter: Box<dyn tag_editor_core::commands::progress::ProgressEmitter + Send> =
        Box::new(RecordingEmitter::new());

    let job = InferenceJob {
        image_paths: vec![p.to_string_lossy().into_owned()],
        threshold: 0.5,
        batch_size: None,
        labels: labels(&["a"]),
        input_size: 4,
        channel_order: ChannelOrder::Bgr,
        operation_id: "spawned-preserve".to_string(),
    };

    let handle = spawn_inference_job(runner, job, Arc::clone(&registry), emitter);
    let result = handle.join().expect("inference thread panicked");
    assert_eq!(result.succeeded, 1);
    assert!(registry.is_empty());
}

/// 呼び出し順に沿って推論失敗を差し込むモック（部分失敗の処理前状態保持を観測）。
struct SequencedFailRunner {
    output: Vec<f32>,
    fail_on_call: Vec<bool>,
    calls: std::sync::Mutex<usize>,
}
impl SessionRunner for SequencedFailRunner {
    fn run(&self, _input: &[f32]) -> tag_editor_core::AppResult<Vec<f32>> {
        let idx = {
            let mut c = self.calls.lock().unwrap();
            let cur = *c;
            *c += 1;
            cur
        };
        if self.fail_on_call.get(idx).copied().unwrap_or(false) {
            Err(tag_editor_core::error::AppError::model_load("mock failure"))
        } else {
            Ok(self.output.clone())
        }
    }
}

#[test]
fn preserve_partial_failure_keeps_pre_operation_state_for_failed_items() {
    // 3.2: 部分失敗時、成功項目のみ Tag_File が書かれ、失敗項目は残さない。
    let dir = tempfile::tempdir().unwrap();
    let mut paths = Vec::new();
    for i in 0..6 {
        let p = dir.path().join(format!("g{i}.png"));
        write_small_png(&p);
        paths.push(p.to_string_lossy().into_owned());
    }

    // 呼出 1 と 3（g1, g3）を失敗させる。
    let runner = SequencedFailRunner {
        output: vec![0.9],
        fail_on_call: vec![false, true, false, true, false, false],
        calls: std::sync::Mutex::new(0),
    };
    let registry = CancelRegistry::new();
    let mut emitter = RecordingEmitter::new();

    let job = InferenceJob {
        image_paths: paths,
        threshold: 0.5,
        batch_size: Some(2),
        labels: labels(&["a"]),
        input_size: 4,
        channel_order: ChannelOrder::Bgr,
        operation_id: "partial-preserve".to_string(),
    };

    let result = run_inference_job(&runner, &job, &registry, &mut emitter);
    assert_eq!(result.succeeded, 4);
    assert_eq!(result.failed, 2);
    assert_eq!(result.cancelled, 0);

    for i in [0usize, 2, 4, 5] {
        assert!(dir.path().join(format!("g{i}.txt")).exists());
    }
    for i in [1usize, 3] {
        assert!(!dir.path().join(format!("g{i}.txt")).exists());
    }
    assert_eq!(emitter.events.last().unwrap().done, 6);
    assert!(registry.is_empty());
}

#[test]
fn preserve_cancel_reflected_across_threads_in_spawned_job() {
    use std::sync::mpsc;
    use tag_editor_core::commands::progress::ProgressEmitter;
    use tag_editor_core::models::Progress;

    let dir = tempfile::tempdir().unwrap();
    let n = 12usize;
    let mut paths = Vec::new();
    for i in 0..n {
        let p = dir.path().join(format!("c{i}.png"));
        write_small_png(&p);
        paths.push(p.to_string_lossy().into_owned());
    }

    let runner = MockRunner { output: vec![0.9] };
    let registry = Arc::new(CancelRegistry::new());

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

    let job = InferenceJob {
        image_paths: paths,
        threshold: 0.5,
        batch_size: Some(1),
        labels: labels(&["a"]),
        input_size: 4,
        channel_order: ChannelOrder::Bgr,
        operation_id: "cross-thread-preserve".to_string(),
    };

    let handle = spawn_inference_job(runner, job, Arc::clone(&registry), emitter);
    let _ = rx.recv();
    registry.request_cancel("cross-thread-preserve");
    let result = handle.join().expect("inference thread panicked");

    assert_eq!(result.failed, 0);
    assert_eq!(result.succeeded + result.cancelled, n);
    assert!(result.cancelled >= 1, "キャンセルが反映されていない");
    assert!(registry.is_empty());
}

// -- 3.3: list_models のローカル検出（固定ケース） --------------------------

#[test]
fn preserve_list_models_detects_local_and_keeps_remote() {
    let dir = tempfile::tempdir().unwrap();
    let model = dir.path().join("local-a");
    std::fs::create_dir(&model).unwrap();
    std::fs::write(model.join("m.onnx"), b"onnx").unwrap();
    std::fs::write(model.join("m.csv"), b"tag").unwrap();

    let remote = ModelVariant {
        id: "wd14-vit".to_string(),
        display_name: "WD14 ViT".to_string(),
        family: ModelFamily::Wd14,
        location: ModelLocation::Remote("owner/wd14-vit".to_string()),
        onnx_available: true,
    };

    let listing = list_models(Some(model.parent().unwrap()), std::slice::from_ref(&remote));
    let mut ids: Vec<String> = listing.available.iter().map(|v| v.id.clone()).collect();
    ids.sort();
    assert_eq!(ids, vec!["local-a".to_string(), "wd14-vit".to_string()]);
    assert!(listing.excluded.is_empty());
    assert!(listing.available.iter().all(|v| v.onnx_available));
}

// -- 3.7: 破損画像は placeholder=true（サムネイル・プレビュー両面） -----------

#[test]
fn preserve_corrupt_image_yields_placeholder_thumbnail_and_preview() {
    let dir = tempfile::tempdir().unwrap();
    let broken = dir.path().join("broken.png");
    std::fs::write(&broken, b"this is definitely not an image").unwrap();

    let thumb =
        adapters::get_thumbnail(broken.to_string_lossy().into_owned(), 128).unwrap();
    assert!(thumb.placeholder);
    assert!(thumb.png.is_empty());
    assert_eq!(thumb.width, 0);
    assert_eq!(thumb.height, 0);

    let preview = adapters::get_preview(broken.to_string_lossy().into_owned()).unwrap();
    assert!(preview.placeholder);
    assert!(preview.png.is_empty());
    assert_eq!(preview.width, 0);
    assert_eq!(preview.height, 0);
}

// -- 3.7: 破損項目があっても隣接する有効画像のサムネイルには波及しない --------

#[test]
fn preserve_broken_item_does_not_affect_sibling_valid_image() {
    let dir = tempfile::tempdir().unwrap();
    let broken = dir.path().join("broken.png");
    std::fs::write(&broken, b"not an image").unwrap();
    let valid = write_png(dir.path(), "valid.png", 300, 150);

    let broken_thumb =
        adapters::get_thumbnail(broken.to_string_lossy().into_owned(), 128).unwrap();
    assert!(broken_thumb.placeholder);

    let valid_thumb =
        adapters::get_thumbnail(valid.to_string_lossy().into_owned(), 128).unwrap();
    assert!(!valid_thumb.placeholder);
    assert!(!valid_thumb.png.is_empty());
    // 300x150 を 128 内接 → 128x64。
    assert_eq!(valid_thumb.width, 128);
    assert_eq!(valid_thumb.height, 64);
}

// -- 3.4: load_local_model コマンドの失敗経路と DTO 形状（欠落時 Err(ModelLoad)） --

#[test]
fn preserve_load_local_model_missing_pair_is_model_load_error() {
    // .onnx はあるがタグ定義が無い → ModelLoad。DTO は返らない（推論無効維持）。
    //
    // `adapters::load_local_model` はタスク 3.2 で `State<ModelSessionState>` を
    // 受け取る形へ拡張された（要件 2.4）。実 `tauri::State` はアプリ実行中でしか
    // 構築できないため、コマンドが薄く委譲する先の `model_service::load_local_model`
    // を直接検証し、エラー種別（保持 3.4 の失敗経路）を確認する。
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("model.onnx"), b"onnx").unwrap();

    let err = model_service::load_local_model(dir.path()).unwrap_err();
    assert_eq!(err.kind, AppErrorKind::ModelLoad);
}
