//! 遅延ダウンロード配線とローカルのみ実行の統合テスト（タスク 13.6、
//! 要件 4.1, 4.2, 4.3, 7.1, 7.2, 7.3, 7.4, 7.5, 11.6）。
//!
//! 本ファイルは Correctness Property（proptest・100 回反復）ではなく、設計の
//! Testing Strategy「統合テスト（PBT を用いない領域、各 1〜3 例）」に従う
//! 通常の `#[test]`。モック `ModelDownloader` / `MockRunner` を用い、実
//! ネットワーク・実 ONNX ランタイムを要さない範囲で以下を検証する。
//!
//! 1. **遅延 DL 順序（Not_Present）**: Not_Present な variant_dir に対し
//!    `download_variant` を呼ぶとモックダウンローダが起動し、成功後に
//!    `is_present == true`（`.onnx` + タグ定義の対がローカルに揃う）になる。
//!    これは `start_inference` が Not_Present で辿る「DL → load → 推論」の
//!    前段（DL → is_present 成立）を統合観点で確認する（要件 4.1, 4.2）。
//! 2. **Present なら download しない**: `start_inference` の遅延 DL 判定は
//!    `is_present(variant_dir)` が false のときだけ `download_variant` を呼ぶ
//!    （adapters.rs の prepare_runner）。ここでは同じ分岐ロジック（is_present
//!    を見て DL 有無を決める）をモックダウンローダの呼び出し回数で検証する。
//!    Present の場合は DL されず、Not_Present の場合のみ DL される（要件 4.3）。
//! 3. **NotFound 系（ローカルに Assets 無ければ推論しない）**: Assets 不在の
//!    variant_dir に対し `load_variant` を呼ぶと `Err(ModelLoad)` になり、
//!    `LoadedModel` は得られない = 推論は無効のまま（要件 7.3, 7.5）。
//! 4. **ローカルのみ実行（型担保）**: `run_inference` は `runner: &dyn
//!    SessionRunner` のみを取り、Model_Source / ModelVariant を引数に取らない。
//!    MockRunner だけで（Model_Source 無しで）呼べて Tag_File を書けることが
//!    その証左（要件 7.1, 7.2, 7.4。型設計上コンパイル時に担保される）。
//! 5. **再推論反映（要件 11.6）**: 同一の生 Predicted_Tag 列に対し、初期
//!    フィルタ（keep 空）では discarded だったタグが、そのタグを keep に加えた
//!    フィルタで再適用すると adopted 側へ移る。

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use tag_editor_core::logic::tag_batch::{apply_fraction_threshold, build_overview};
use tag_editor_core::logic::tag_filter::{apply_filter, compile_filter};
use tag_editor_core::models::{
    ChannelOrder, FilterOutcome, LabelDef, ModelFamily, ModelSource, ModelVariant, Progress,
    RawTagFilter, Tag, TagCategory,
};
use tag_editor_core::services::inference_service::{run_inference, SessionRunner};
use tag_editor_core::services::model_service::{
    self, DownloadError, DownloadPhase, ModelDownloader,
};
use tag_editor_core::{AppErrorKind, AppResult};

// ---------------------------------------------------------------------------
// モック実装（tests 内で独自定義。呼び出し記録・固定確信度）
// ---------------------------------------------------------------------------

/// ネットワーク非依存のモックダウンローダ。`fetch` 呼び出しを記録する。
///
/// `responses` は「ファイル名 → 返すバイト列」。登録の無いファイルは
/// [`DownloadError::Other`] を返す。呼び出し回数は `call_count` で参照でき、
/// 「Present なら download しない」検証に用いる。
struct RecordingDownloader {
    responses: Vec<(String, Vec<u8>)>,
    calls: Mutex<Vec<(String, String)>>,
}

impl RecordingDownloader {
    fn new(pairs: &[(&str, &[u8])]) -> Self {
        Self {
            responses: pairs
                .iter()
                .map(|(n, b)| ((*n).to_string(), b.to_vec()))
                .collect(),
            calls: Mutex::new(Vec::new()),
        }
    }

    fn call_count(&self) -> usize {
        self.calls.lock().unwrap().len()
    }
}

impl ModelDownloader for RecordingDownloader {
    fn fetch(&self, repo: &str, file: &str, _timeout: Duration) -> Result<Vec<u8>, DownloadError> {
        self.calls
            .lock()
            .unwrap()
            .push((repo.to_string(), file.to_string()));
        self.responses
            .iter()
            .find(|(name, _)| name == file)
            .map(|(_, bytes)| bytes.clone())
            .ok_or_else(|| DownloadError::Other(format!("未登録のファイル: {file}")))
    }
}

/// 固定の確信度ベクトルを返すモック実行器（Model_Source を一切参照しない）。
struct MockRunner {
    output: Vec<f32>,
}

impl SessionRunner for MockRunner {
    fn run(&self, _input: &[f32]) -> AppResult<Vec<f32>> {
        Ok(self.output.clone())
    }
}

// ---------------------------------------------------------------------------
// テスト用ヘルパー
// ---------------------------------------------------------------------------

/// 取得段が成功する WD14 バリアント（source ベース）。
fn wd14_variant() -> ModelVariant {
    ModelVariant {
        id: "wd14-vit".to_string(),
        display_name: "WD14 ViT".to_string(),
        family: ModelFamily::Wd14,
        source: ModelSource {
            repo: "owner/wd14-vit".to_string(),
            onnx_file: "model.onnx".to_string(),
            tag_files: vec!["selected_tags.csv".to_string()],
        },
    }
}

/// 取得段で成功するモック（`.onnx` + 妥当な CSV タグ定義）。
fn ok_downloader() -> RecordingDownloader {
    let csv: &[u8] = b"tag_id,name,category\n1,solo,0\n2,miku,4\n";
    RecordingDownloader::new(&[("model.onnx", b"onnx-bytes"), ("selected_tags.csv", csv)])
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

/// 単色 2x2 の PNG 画像を実ファイルとして作成する。
fn write_image(path: &Path) {
    use image::{Rgb, RgbImage};
    let mut img = RgbImage::new(2, 2);
    for p in img.pixels_mut() {
        *p = Rgb([120, 130, 140]);
    }
    img.save(path).unwrap();
}

/// Confidence_Threshold のみ設定した TagFilter を作る（keep 空・fraction 非適用）。
fn threshold_filter(threshold: f32) -> tag_editor_core::models::TagFilter {
    let (filter, invalid) = compile_filter(RawTagFilter {
        keep: Vec::new(),
        exclude: Vec::new(),
        replace: Vec::new(),
        additional: Vec::new(),
        confidence_threshold: threshold,
        fraction_threshold: 0.0,
    });
    assert!(invalid.is_empty());
    filter
}

// ---------------------------------------------------------------------------
// 1. 遅延 DL 順序（Not_Present → download_variant → is_present 成立）
//    （要件 4.1, 4.2）
// ---------------------------------------------------------------------------

/// Not_Present な variant_dir に対し `download_variant` を呼ぶと、モック
/// ダウンローダが起動（fetch 呼び出しが記録される）し、成功後に
/// `is_present == true`（`.onnx` + タグ定義の対が揃う）になることを確認する。
///
/// これは `start_inference` が Not_Present で辿る「遅延 DL → is_present 成立」
/// の前段を統合観点で確認する。以降の load_variant は実 ONNX ランタイムを要する
/// ため、対の存在（`is_present`）確認までに留める（設計の Testing Strategy）。
#[test]
fn not_present_triggers_download_then_becomes_present() {
    let dir = tempfile::tempdir().unwrap();
    let variant_dir = dir.path().join("wd14-vit");
    let variant = wd14_variant();
    let downloader = ok_downloader();
    let cancel = AtomicBool::new(false);

    // 開始時は Not_Present（Assets が無い）。
    assert!(
        !model_service::is_present(&variant_dir),
        "開始時は Not_Present であるべき"
    );

    // 遅延 DL の各段階（onnx → tagdef → saved）を順に通知する。
    let mut phases = Vec::new();
    let returned = model_service::download_variant(
        &downloader,
        &variant,
        &variant_dir,
        &cancel,
        |p: DownloadPhase| phases.push(p),
    )
    .expect("Not_Present からの遅延 DL は成功するべき");

    // ダウンロード呼び出しが行われた（モックの呼び出し記録で確認）。
    assert!(
        downloader.call_count() >= 1,
        "Not_Present では download_variant がダウンロードを起動するべき"
    );
    // 段階は onnx → tagdef → saved の順。
    assert_eq!(
        phases,
        vec![
            DownloadPhase::OnnxFetched,
            DownloadPhase::TagDefinitionFetched,
            DownloadPhase::Saved,
        ]
    );

    // 保存後は Present（次段 load_variant が読める形＝対が揃う）。
    assert_eq!(returned, variant_dir);
    assert!(
        model_service::is_present(&variant_dir),
        "DL 成功後は Present（.onnx + タグ定義の対）になるべき"
    );
}

// ---------------------------------------------------------------------------
// 2. Present なら download しない（要件 4.3）
// ---------------------------------------------------------------------------

/// `start_inference`（adapters.rs の prepare_runner）と同一の分岐ロジック
/// ——「`is_present(variant_dir)` が false のときだけ `download_variant` を呼ぶ」
/// ——をモックダウンローダの呼び出し回数で検証する。
///
/// `start_inference` 自体は `AppHandle`/`tauri::State` を要しコマンド境界のため、
/// ここでは同じ判定を実行するローカルヘルパー `prepare_if_absent` で再現する。
/// Present の場合は DL されず（call_count == 0）、Not_Present の場合のみ DL される。
#[test]
fn present_variant_is_not_downloaded_but_absent_is() {
    // prepare_runner と同じ判定: Not_Present のときだけ download する。
    fn prepare_if_absent(
        downloader: &RecordingDownloader,
        variant: &ModelVariant,
        variant_dir: &Path,
    ) -> AppResult<()> {
        if !model_service::is_present(variant_dir) {
            let cancel = AtomicBool::new(false);
            model_service::download_variant(downloader, variant, variant_dir, &cancel, |_| {})?;
        }
        Ok(())
    }

    let variant = wd14_variant();

    // ケース A: 既に Present。Assets を先に配置しておく。
    let present_root = tempfile::tempdir().unwrap();
    let present_dir = present_root.path().join("wd14-vit");
    std::fs::create_dir_all(&present_dir).unwrap();
    std::fs::write(present_dir.join("model.onnx"), b"onnx").unwrap();
    std::fs::write(
        present_dir.join("selected_tags.csv"),
        b"tag_id,name,category\n1,solo,0\n",
    )
    .unwrap();
    assert!(model_service::is_present(&present_dir));

    let dl_present = ok_downloader();
    prepare_if_absent(&dl_present, &variant, &present_dir).unwrap();
    // Present なので download_variant は呼ばれず、fetch も一度も起動しない（要件 4.3）。
    assert_eq!(
        dl_present.call_count(),
        0,
        "Present の variant はダウンロードしてはならない"
    );

    // ケース B: Not_Present。こちらは DL される（対比）。
    let absent_root = tempfile::tempdir().unwrap();
    let absent_dir = absent_root.path().join("wd14-vit");
    assert!(!model_service::is_present(&absent_dir));

    let dl_absent = ok_downloader();
    prepare_if_absent(&dl_absent, &variant, &absent_dir).unwrap();
    assert!(
        dl_absent.call_count() >= 1,
        "Not_Present の variant はダウンロードされるべき"
    );
    assert!(model_service::is_present(&absent_dir));
}

// ---------------------------------------------------------------------------
// 3. NotFound 系: ローカルに Assets 無ければ推論しない（要件 7.3, 7.5）
// ---------------------------------------------------------------------------

/// Assets 不在の variant_dir に対し `load_variant` を呼ぶと `Err(ModelLoad)` に
/// なり、`LoadedModel` を得られない = 推論は無効のまま維持されることを確認する。
///
/// 本設計では Not_Present なら遅延 DL するが、DL 不能（Assets 無し・取得失敗）の
/// ときは load 段で対が揃わず ModelLoad となる。統合観点で「ローカルに Assets が
/// 無ければ推論を開始しない」ことを 1 例で示す（要件 7.3, 7.5）。
#[test]
fn missing_local_assets_yield_model_load_error_and_no_inference() {
    let dir = tempfile::tempdir().unwrap();
    let variant_dir = dir.path().join("empty-variant");
    std::fs::create_dir_all(&variant_dir).unwrap();

    // Assets が無いので Not_Present。
    assert!(!model_service::is_present(&variant_dir));

    // load_variant は対（.onnx + タグ定義）が無く ModelLoad を返す。
    let err = model_service::load_variant(&variant_dir)
        .expect_err("Assets 不在では LoadedModel を生成してはならない");
    assert_eq!(err.kind, AppErrorKind::ModelLoad);
}

// ---------------------------------------------------------------------------
// 4. ローカルのみ実行（型担保）: run_inference は runner のみを取る
//    （要件 7.1, 7.2, 7.4）
// ---------------------------------------------------------------------------

/// `run_inference` が `runner: &dyn SessionRunner` のみを取り、Model_Source /
/// ModelVariant を引数に取らないことを、MockRunner だけで（Model_Source 無しで）
/// 呼べて Tag_File を書けることで示す。Model_Source 非参照は型設計上コンパイル時に
/// 担保され、本テストはその「runner のみで動く」性質を統合観点で明示する。
#[test]
fn run_inference_uses_runner_only_without_model_source() {
    let dir = tempfile::tempdir().unwrap();
    let png = dir.path().join("img.png");
    write_image(&png);

    // MockRunner は Model_Source を一切参照せず固定確信度を返す。
    let runner = MockRunner {
        output: vec![0.9, 0.2, 0.8],
    };
    let labels = labels(&["cat", "dog", "bird"]);
    let cancel = AtomicBool::new(false);
    let mut progress = |_p: Progress| {};
    let paths = vec![png.to_string_lossy().into_owned()];

    let result = run_inference(
        &runner,
        &paths,
        &threshold_filter(0.5),
        None,
        &labels,
        4,
        ChannelOrder::Bgr,
        "op-local-only",
        &cancel,
        &mut progress,
    );

    // runner のみで推論が完了し Tag_File が書ける。
    assert_eq!(result.succeeded, 1);
    let written = std::fs::read_to_string(dir.path().join("img.txt")).unwrap();
    // 閾値 0.5 以上の cat(0.9), bird(0.8) が採用、dog(0.2) は不採用。
    assert_eq!(written, "cat, bird");
}

/// `run_inference` のシグネチャが `&dyn SessionRunner` を受けることを、
/// `MockRunner` を dyn 参照へ強制してコンパイル時に確認する補助テスト。
/// Model_Source を渡す余地が型に無いことの明示（要件 7.1, 7.2, 7.4）。
#[test]
fn run_inference_signature_takes_only_dyn_session_runner() {
    let runner = MockRunner { output: vec![1.0] };
    let dyn_runner: &dyn SessionRunner = &runner;
    let cancel = AtomicBool::new(false);
    // キャンセル済みかつ空入力で即時完了させ、副作用を出さない。
    cancel.store(true, Ordering::SeqCst);
    let mut progress = |_p: Progress| {};
    let result = run_inference(
        dyn_runner,
        &[],
        &threshold_filter(0.5),
        None,
        &labels(&["a"]),
        4,
        ChannelOrder::Bgr,
        "op-sig",
        &cancel,
        &mut progress,
    );
    assert_eq!(result.succeeded, 0);
    assert_eq!(result.failed, 0);
}

// ---------------------------------------------------------------------------
// 5. 再推論反映（要件 11.6）: Keep_Tags 更新後の再適用でタグが Adopted へ移る
// ---------------------------------------------------------------------------

/// 同一の生 Predicted_Tag 列に対し、
/// (a) 初期フィルタ（keep 空・閾値でそのタグが落ちる）では当該タグが discarded 側、
/// (b) そのタグを keep に加えたフィルタで再適用すると adopted 側へ移る、
/// ことを `apply_filter` → `apply_fraction_threshold` → `build_overview` の
/// 再適用で確認する（要件 11.6）。
#[test]
fn rerun_after_keep_update_moves_tag_to_adopted() {
    // 1 画像分の生 Predicted_Tag。"blush" は確信度 0.3 で閾値 0.5 未満。
    let predicted = vec![
        Tag::with_confidence("solo", 0.95),
        Tag::with_confidence("blush", 0.3),
    ];

    // (a) 初期フィルタ: keep 空・閾値 0.5。blush は閾値未満で discarded。
    let initial = threshold_filter(0.5);
    let outcome_a: FilterOutcome = apply_filter(&initial, &predicted);
    let batch_a = apply_fraction_threshold(std::slice::from_ref(&outcome_a), &initial, 1);
    let overview_a = build_overview(&batch_a);

    let adopted_a: Vec<&str> = overview_a.adopted.iter().map(|s| s.name.as_str()).collect();
    let discarded_a: Vec<&str> = overview_a
        .discarded
        .iter()
        .map(|s| s.name.as_str())
        .collect();
    assert!(adopted_a.contains(&"solo"), "solo は初期でも adopted");
    assert!(
        discarded_a.contains(&"blush"),
        "blush は初期フィルタでは discarded であるべき"
    );
    assert!(
        !adopted_a.contains(&"blush"),
        "blush は初期フィルタでは adopted に無いべき"
    );

    // (b) blush を Keep_Tags に追加した更新後フィルタで再適用する。
    let (updated, invalid) = compile_filter(RawTagFilter {
        keep: vec!["blush".to_string()],
        exclude: Vec::new(),
        replace: Vec::new(),
        additional: Vec::new(),
        confidence_threshold: 0.5,
        fraction_threshold: 0.0,
    });
    assert!(invalid.is_empty());

    let outcome_b = apply_filter(&updated, &predicted);
    let batch_b = apply_fraction_threshold(std::slice::from_ref(&outcome_b), &updated, 1);
    let overview_b = build_overview(&batch_b);

    let adopted_b: Vec<&str> = overview_b.adopted.iter().map(|s| s.name.as_str()).collect();
    let discarded_b: Vec<&str> = overview_b
        .discarded
        .iter()
        .map(|s| s.name.as_str())
        .collect();

    // 再推論反映: blush が Adopted 側へ移り、Discarded 側から消える（要件 11.6）。
    assert!(
        adopted_b.contains(&"blush"),
        "Keep 追加後は blush が adopted へ移るべき"
    );
    assert!(
        !discarded_b.contains(&"blush"),
        "Keep 追加後は blush が discarded から消えるべき"
    );
    assert!(adopted_b.contains(&"solo"), "solo は引き続き adopted");
}
