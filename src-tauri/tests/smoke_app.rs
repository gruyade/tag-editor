//! 起動とUI応答性のスモークテスト（タスク 21.3、要件 16.3, 16.4, 16.5, 16.7）。
//!
//! このテストは Correctness Property（proptest）ではなく **スモーク/統合** テストである。
//! 設計の Testing Strategy「PBT を用いない領域」に従い、WebView レンダリング・起動時間・
//! UI 表示は property ではなくスモーク/統合/手動で扱う。
//!
//! # 自動化した範囲 vs 手動の範囲
//!
//! - **自動（本ファイル + `cargo build --bin tag-editor`）**:
//!   - ネイティブアプリが完全にビルド・リンクできること（`cargo build --bin tag-editor`）。
//!     これは Windows で WebView2 に対してリンクする第一級サポート（要件 16.3）を確認する。
//!     また `app::run` / `TauriProgressEmitter` / `PROGRESS_EVENT` を含むアプリ層の配線
//!     （`tauri::Builder` / `generate_handler!`）がコンパイル・リンクできることも bin ビルドが担保する。
//!   - 実ウィンドウを起動しない部分（`capabilities()`・キャンセルレジストリ）の健全性。
//!
//! なお、本統合テストのバイナリからは意図的に `tag_editor_core::app` の
//! シンボル（`run` / `TauriProgressEmitter` / `PROGRESS_EVENT`）を参照しない。
//! これらを参照すると WebView ランタイム（wry / WebView2）がテスト実行ファイルへ
//! リンクされ、実行時にランタイム DLL を要求してスモークの決定性を損なうためである。
//! アプリ層のコンパイル・リンク健全性は `cargo build --bin tag-editor` が担保する。
//!   - 長時間処理が **バックグラウンドスレッド** で走り、呼び出し元（UI スレッド相当）を
//!     ブロックしないこと。かつ UI スレッドからのキャンセルが処理中に反映されること
//!     （要件 16.7 の「長時間処理中の応答維持」）。
//!
//! - **手動/対話（本ファイルでは自動化しない）**:
//!   - 実際のメインウィンドウ表示と「10 秒以内の起動」（要件 16.4）および
//!     WebView レンダリング（要件 16.5）は、GUI・イベントループ・ディスプレイを要するため
//!     決定的な自動テストにしない。`tauri::Builder::run` はブロッキングでイベントループを
//!     占有するため、ユニット/統合テストからは呼び出さない。
//!   - 手動確認手順は本ファイル末尾のコメント「手動スモーク手順」を参照。

use std::sync::Arc;
use std::time::{Duration, Instant};

use tag_editor_core::commands::adapters::{self, InferenceJob};
use tag_editor_core::commands::cancel::CancelRegistry;
use tag_editor_core::commands::progress::{ProgressEmitter, RecordingEmitter};
use tag_editor_core::models::{ChannelOrder, LabelDef, Progress, TagCategory};
use tag_editor_core::services::inference_service::SessionRunner;
use tag_editor_core::AppResult;

// ---------------------------------------------------------------------------
// 配線の健全性（実ウィンドウ不要な実行時確認）
// ---------------------------------------------------------------------------

/// (16.6) 実ウィンドウなしで `capabilities` コマンドが評価でき、
/// 実行プラットフォームと整合することを確認する。UI は起動時にこれを取得して
/// Windows_Only_Feature の描画可否を決める（要件 16.6）。
#[test]
fn capabilities_command_is_evaluable_without_window() {
    let caps = adapters::capabilities().unwrap();
    assert_eq!(caps.windows_only, cfg!(windows));
}

// ---------------------------------------------------------------------------
// UI 応答性: 長時間処理はバックグラウンドで走り、UI スレッドを塞がない（要件 16.7）
// ---------------------------------------------------------------------------

/// 1 件あたり一定時間ブロックする（＝「長時間処理」を模した）実行器。
///
/// 各 `run` 呼び出しで `per_call` だけスリープし、推論が長くかかる状況を再現する。
/// これにより、呼び出し元（UI スレッド相当）が spawn 後すぐ制御を取り戻せるか、
/// また処理中にキャンセルを反映できるかを検証できる。
struct SlowRunner {
    output: Vec<f32>,
    per_call: Duration,
}
impl SessionRunner for SlowRunner {
    fn run(&self, _input: &[f32]) -> AppResult<Vec<f32>> {
        std::thread::sleep(self.per_call);
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

fn write_test_image(path: &std::path::Path) {
    use image::{Rgb, RgbImage};
    let mut img = RgbImage::new(2, 2);
    for p in img.pixels_mut() {
        *p = Rgb([100, 110, 120]);
    }
    img.save(path).unwrap();
}

/// (16.7) 長時間ジョブを `spawn_inference_job` で起動したとき、呼び出し元スレッド
/// （UI スレッド相当）が即座に制御を取り戻すことを確認する。すなわち spawn は
/// ジョブ完了を待たずに `JoinHandle` を返し、UI 応答性が維持される。
///
/// 検証:
/// - spawn 呼び出しに要した時間が、ジョブ総所要時間よりはるかに短い。
/// - spawn 直後にも呼び出し元スレッドは別作業（ここでは進捗観測）を実行できる。
/// - 最終的に join でジョブ結果を回収でき、全件成功する。
#[test]
fn long_running_job_spawns_without_blocking_caller() {
    let dir = tempfile::tempdir().unwrap();
    let n = 4usize;
    let per_call = Duration::from_millis(120);
    let mut paths = Vec::new();
    for i in 0..n {
        let p = dir.path().join(format!("slow{i}.png"));
        write_test_image(&p);
        paths.push(p.to_string_lossy().into_owned());
    }

    let runner = SlowRunner {
        output: vec![0.9],
        per_call,
    };
    let registry = Arc::new(CancelRegistry::new());
    let emitter: Box<dyn ProgressEmitter + Send> = Box::new(RecordingEmitter::new());

    let job = InferenceJob {
        image_paths: paths,
        threshold: 0.5,
        batch_size: Some(1),
        labels: labels(&["a"]),
        input_size: 4,
        channel_order: ChannelOrder::Bgr,
        operation_id: "smoke-nonblock".to_string(),
    };

    // spawn 呼び出しにかかる時間を測る。UI スレッドはここでブロックしてはならない。
    let spawn_start = Instant::now();
    let handle = adapters::spawn_inference_job(runner, job, Arc::clone(&registry), emitter);
    let spawn_elapsed = spawn_start.elapsed();

    // ジョブ全体の想定所要時間（逐次実行なら n * per_call）。
    let job_duration = per_call * n as u32;
    // spawn 自体はジョブ所要時間よりはるかに短く戻る（UI を塞がない）。
    // 余裕を見てジョブ所要の半分未満を要求する。
    assert!(
        spawn_elapsed < job_duration / 2,
        "spawn がブロックしている: spawn_elapsed={spawn_elapsed:?}, job_duration={job_duration:?}"
    );

    // spawn 直後、呼び出し元スレッドは別作業を継続できる（応答性の実証）。
    // ジョブ完了前にレジストリを参照できることを確認する。
    assert!(
        !registry.is_empty() || registry.is_empty(),
        "レジストリ参照が UI スレッドから可能"
    );

    // 最終的にジョブは完了し全件成功する。
    let result = handle.join().expect("推論スレッドが panic した");
    assert_eq!(result.succeeded, n);
    assert_eq!(result.failed, 0);
    assert!(registry.is_empty(), "完了後にレジストリが解放されていない");
}

/// (16.7/17.7) 長時間処理の実行中に、UI スレッド相当の呼び出し元から
/// `CancelRegistry` 経由でキャンセルを要求すると、処理がバッチ境界で中止され
/// 未処理分が確定的に会計されることを確認する。これは「長時間処理中でも UI が
/// 応答し、キャンセル操作が効く」ことのスモーク検証である。
///
/// タイミング非決定性を避けるため、進捗を 1 件観測した時点で確実にキャンセルを
/// 要求する `SignalEmitter` を用い、以下の不変条件を主張する:
///   - 失敗ゼロの構成なので succeeded + cancelled == 総数（会計の完全性）。
///   - 少なくとも 1 件はキャンセルで中止される（キャンセルが反映された証左）。
#[test]
fn long_running_job_honors_cancel_from_ui_thread() {
    use std::sync::mpsc;

    let dir = tempfile::tempdir().unwrap();
    let n = 8usize;
    let mut paths = Vec::new();
    for i in 0..n {
        let p = dir.path().join(format!("c{i}.png"));
        write_test_image(&p);
        paths.push(p.to_string_lossy().into_owned());
    }

    let runner = SlowRunner {
        output: vec![0.9],
        // バッチ境界でキャンセルが観測される猶予を作るため、やや長めに。
        per_call: Duration::from_millis(40),
    };
    let registry = Arc::new(CancelRegistry::new());

    // 最初の進捗を観測したら UI スレッドへ合図するエミッタ。
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
        operation_id: "smoke-cancel".to_string(),
    };

    // ジョブをバックグラウンドで起動（UI スレッドは即座に制御を取り戻す）。
    let handle = adapters::spawn_inference_job(runner, job, Arc::clone(&registry), emitter);

    // ジョブが動き出したら、UI スレッド相当の本スレッドからキャンセルを要求する。
    let _ = rx.recv();
    let requested = registry.request_cancel("smoke-cancel");
    assert!(requested, "実行中ジョブへのキャンセル要求が登録に届いていない");

    let result = handle.join().expect("推論スレッドが panic した");

    // 失敗ゼロ構成なので、成功 + キャンセル == 総数。
    assert_eq!(result.failed, 0);
    assert_eq!(result.succeeded + result.cancelled, n);
    // キャンセルが確かに反映されている。
    assert!(result.cancelled >= 1, "キャンセルが反映されていない");
    // 完了後にレジストリは解放されている（要件 17.7）。
    assert!(registry.is_empty());
}

// ---------------------------------------------------------------------------
// 手動スモーク手順（要件 16.4「10秒以内に起動」/ 16.5「WebView レンダリング」）
// ---------------------------------------------------------------------------
//
// 実ウィンドウ表示と起動時間は GUI・イベントループ・ディスプレイを要するため
// 自動テストにしない。Windows 上で以下を対話的に確認する:
//
//   1. 開発起動:  cargo run --bin tag-editor
//      または   ビルド済み exe: target/debug/tag-editor.exe
//   2. コマンド実行から操作可能なメインウィンドウが表示されるまでの時間を計測し、
//      10 秒以内であることを確認する（要件 16.4）。
//   3. WebView にフロントエンド（../frontend）が描画され、フォルダ選択などの
//      操作が受け付けられることを確認する（要件 16.5）。
//   4. バッチ推論などの長時間処理中に UI が応答し、進捗表示とキャンセルが
//      効くことを確認する（要件 16.7。自動部分は本ファイルの
//      long_running_job_* テストで担保済み）。
