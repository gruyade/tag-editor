// Feature: local-model-management, Property 9: 取得の再試行上限
//
// Property 9: 任意の「k 回目の取得で初めて成功する（または全失敗する）」
// トランスポートについて、個々のファイル取得の呼び出し回数は 3 を超えず、
// k <= 3 なら取得は成功し、k > 3（または全失敗）なら 3 回で打ち切って
// Download 失敗として終了する。
//
// 検証の着眼点:
//
// download_variant は onnx とタグ定義を順に fetch し、各ファイルの取得を
// fetch_with_retry で最大 MAX_DOWNLOAD_ATTEMPTS（= 3）回まで再試行する。
// 「1 ファイルあたり最大 3 回」という上限に着目するため、onnx の取得回数で
// 境界を検証する（onnx が最初に fetch されるため素直）。
//
// - onnx が k 回目で成功する mock:
//   - k <= 3: onnx の fetch 回数 = k で成功し、続けてタグ定義も取得して
//     Download 成功（Ok）となる。
//   - k > 3（= 常に失敗を含む）: onnx の fetch 回数 = 3 で打ち切られ、
//     タグ定義まで到達せず Download 失敗（Err(Download)）となる。
// - いずれの結末でも、どのファイルの fetch 呼び出し回数も 3 を超えない。
//
// 境界 k = 1, 3, 4, ∞（常に失敗）を明示的にカバーする。
//
// Validates: Requirements 6.1, 6.2

use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::sync::Mutex;
use std::time::Duration;

use proptest::prelude::*;
use tag_editor_core::error::AppErrorKind;
use tag_editor_core::models::{ModelFamily, ModelSource, ModelVariant};
use tag_editor_core::services::model_service::{
    download_variant, DownloadError, DownloadPhase, ModelDownloader,
};

/// MAX_DOWNLOAD_ATTEMPTS（model_service.rs 内の非公開定数）と同じ値。
/// 「1 ファイルあたり最大 3 回試行」を検証の上限として用いる。
const MAX_ATTEMPTS: usize = 3;

/// ファイルごとに「k 回目の取得で初めて成功する」を制御するモックダウンローダ。
///
/// `succeed_on` はファイル名 → 何回目の fetch で成功するか（1 始まり）。
/// `None`（未登録）または `usize::MAX` 相当の大きな値は「常に失敗」を表す。
/// ファイルごとに fetch 呼び出し回数をカウントし、上限検証に用いる。
struct KthAttemptDownloader {
    /// ファイル名 → 成功する試行番号（1 始まり）。到達不能な回（例: 4）や
    /// 未登録ファイルは常に失敗する。
    succeed_on: Vec<(String, usize)>,
    /// 成功時に返すバイト列（内容は検証で使わないため共通で足りる）。
    bytes: Vec<u8>,
    /// ファイル名 → これまでの fetch 呼び出し回数。
    calls: Mutex<Vec<(String, usize)>>,
}

impl KthAttemptDownloader {
    fn new(succeed_on: Vec<(String, usize)>, bytes: &[u8]) -> Self {
        Self {
            succeed_on,
            bytes: bytes.to_vec(),
            calls: Mutex::new(Vec::new()),
        }
    }

    /// 指定ファイルの成功試行番号（未登録は「常に失敗」= None）。
    fn success_attempt(&self, file: &str) -> Option<usize> {
        self.succeed_on
            .iter()
            .find(|(name, _)| name == file)
            .map(|(_, k)| *k)
    }

    /// 指定ファイルの累計 fetch 呼び出し回数。
    fn calls_for(&self, file: &str) -> usize {
        self.calls
            .lock()
            .unwrap()
            .iter()
            .find(|(name, _)| name == file)
            .map(|(_, n)| *n)
            .unwrap_or(0)
    }

    /// 全ファイルにわたる最大 fetch 呼び出し回数。
    fn max_calls(&self) -> usize {
        self.calls
            .lock()
            .unwrap()
            .iter()
            .map(|(_, n)| *n)
            .max()
            .unwrap_or(0)
    }
}

impl ModelDownloader for KthAttemptDownloader {
    fn fetch(
        &self,
        _repo: &str,
        file: &str,
        _timeout: Duration,
    ) -> Result<Vec<u8>, DownloadError> {
        // このファイルの呼び出し回数をインクリメントし、今回の試行番号 n を得る。
        let n = {
            let mut calls = self.calls.lock().unwrap();
            match calls.iter_mut().find(|(name, _)| name == file) {
                Some((_, count)) => {
                    *count += 1;
                    *count
                }
                None => {
                    calls.push((file.to_string(), 1));
                    1
                }
            }
        };

        match self.success_attempt(file) {
            Some(k) if n >= k => Ok(self.bytes.clone()),
            _ => Err(DownloadError::Timeout),
        }
    }
}

/// テスト用の WD14 バリアント（onnx = model.onnx, タグ定義 = selected_tags.csv）。
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

/// 妥当なタグ定義バイト列（保存後 onnx_with_tagdef が対を検出できる形）。
const TAGDEF_CSV: &[u8] = b"tag_id,name,category\n1,solo,0\n";

/// onnx が k 回目で成功する mock を作る（タグ定義は 1 回目で必ず成功）。
///
/// k <= 3 のときは onnx 成功後にタグ定義取得へ進むため、タグ定義も成功させる。
/// k > 3 のときは onnx が 3 回で打ち切られタグ定義へ到達しないが、到達した場合に
/// 備えてタグ定義も成功させておく（到達しないことを別途 assert する）。
fn downloader_onnx_succeeds_on(k: usize) -> KthAttemptDownloader {
    KthAttemptDownloader::new(
        vec![
            ("model.onnx".to_string(), k),
            ("selected_tags.csv".to_string(), 1),
        ],
        TAGDEF_CSV,
    )
}

/// download_variant を一時ディレクトリ上で実行する共通ヘルパ。
///
/// 進捗フェーズを収集して返し、キャンセルなしで実行する。
fn run_download(
    downloader: &KthAttemptDownloader,
    variant: &ModelVariant,
    variant_dir: &Path,
) -> (
    Result<std::path::PathBuf, tag_editor_core::error::AppError>,
    Vec<DownloadPhase>,
) {
    let cancel = AtomicBool::new(false);
    let mut phases = Vec::new();
    let result = download_variant(downloader, variant, variant_dir, &cancel, |p| {
        phases.push(p)
    });
    (result, phases)
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// Property 9: onnx が k 回目で成功する mock について、
    /// - onnx の fetch 回数は 3 を超えない、
    /// - k <= 3 なら onnx fetch 回数 = k で Download 成功、
    /// - k > 3 なら onnx fetch 回数 = 3 で打ち切り、Download 失敗（タグ定義未到達）。
    #[test]
    fn onnx_retry_respects_limit_and_success_boundary(
        // k = 1..=6 を生成し、k<=3 と k>3 の両側を広くカバーする（境界 3,4 を含む）。
        k in 1usize..=6,
    ) {
        let variant = wd14_variant();
        let downloader = downloader_onnx_succeeds_on(k);

        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("wd14-vit");

        let (result, phases) = run_download(&downloader, &variant, &dest);

        // どのファイルの fetch 呼び出し回数も 3 を超えない（上限厳守）。
        prop_assert!(
            downloader.max_calls() <= MAX_ATTEMPTS,
            "fetch 呼び出しが上限 {} を超えた: max_calls={}",
            MAX_ATTEMPTS,
            downloader.max_calls()
        );

        let onnx_calls = downloader.calls_for("model.onnx");

        if k <= MAX_ATTEMPTS {
            // k 回目で初成功 → onnx fetch 回数はちょうど k。
            prop_assert_eq!(
                onnx_calls, k,
                "k<=3 では onnx fetch 回数は k と一致すべき"
            );
            // Download 成功。
            prop_assert!(
                result.is_ok(),
                "k<=3 では成功すべき: k={}, err={:?}",
                k,
                result.as_ref().err()
            );
            // 成功時は Saved まで通知される。
            prop_assert!(phases.contains(&DownloadPhase::Saved));
        } else {
            // k>3 → onnx は 3 回で打ち切り。
            prop_assert_eq!(
                onnx_calls, MAX_ATTEMPTS,
                "k>3 では onnx fetch 回数は上限 3 で打ち切られるべき"
            );
            // Download 失敗（Download 種別）。
            let err = result.expect_err("k>3 では失敗すべき");
            prop_assert_eq!(err.kind, AppErrorKind::Download);
            // onnx で打ち切られたためタグ定義まで到達しない。
            prop_assert_eq!(
                downloader.calls_for("selected_tags.csv"),
                0,
                "onnx 失敗時はタグ定義を fetch しないはず"
            );
            // Saved は通知されない。
            prop_assert!(!phases.contains(&DownloadPhase::Saved));
        }
    }
}

// ---------------------------------------------------------------------------
// 境界の明示的な固定ケース（k = 1, 3, 4, ∞）
// ---------------------------------------------------------------------------

/// k = 1: 初回で成功。onnx fetch 回数 = 1 で Download 成功。
#[test]
fn boundary_k1_succeeds_on_first_attempt() {
    let downloader = downloader_onnx_succeeds_on(1);
    let variant = wd14_variant();
    let dir = tempfile::tempdir().unwrap();
    let dest = dir.path().join("wd14-vit");

    let (result, _phases) = run_download(&downloader, &variant, &dest);

    assert!(result.is_ok(), "k=1 は成功すべき");
    assert_eq!(downloader.calls_for("model.onnx"), 1);
    assert!(downloader.max_calls() <= MAX_ATTEMPTS);
}

/// k = 3: 上限内の最終試行で成功。onnx fetch 回数 = 3 で Download 成功。
#[test]
fn boundary_k3_succeeds_on_last_allowed_attempt() {
    let downloader = downloader_onnx_succeeds_on(3);
    let variant = wd14_variant();
    let dir = tempfile::tempdir().unwrap();
    let dest = dir.path().join("wd14-vit");

    let (result, _phases) = run_download(&downloader, &variant, &dest);

    assert!(result.is_ok(), "k=3 は成功すべき（上限内の最終試行）");
    assert_eq!(downloader.calls_for("model.onnx"), 3);
    assert!(downloader.max_calls() <= MAX_ATTEMPTS);
}

/// k = 4: 上限を 1 だけ超えるため打ち切り。onnx fetch 回数 = 3 で Download 失敗。
#[test]
fn boundary_k4_stops_at_limit_and_fails() {
    let downloader = downloader_onnx_succeeds_on(4);
    let variant = wd14_variant();
    let dir = tempfile::tempdir().unwrap();
    let dest = dir.path().join("wd14-vit");

    let (result, _phases) = run_download(&downloader, &variant, &dest);

    let err = result.expect_err("k=4 は上限で打ち切られ失敗すべき");
    assert_eq!(err.kind, AppErrorKind::Download);
    assert_eq!(downloader.calls_for("model.onnx"), MAX_ATTEMPTS);
    // タグ定義には到達しない。
    assert_eq!(downloader.calls_for("selected_tags.csv"), 0);
}

/// k = ∞（常に失敗）: onnx を一度も成功させない。onnx fetch 回数 = 3 で Download 失敗。
#[test]
fn boundary_k_infinite_always_fails_stops_at_limit() {
    // 成功試行番号を登録しない = 常に失敗。
    let downloader = KthAttemptDownloader::new(
        vec![("selected_tags.csv".to_string(), 1)],
        TAGDEF_CSV,
    );
    let variant = wd14_variant();
    let dir = tempfile::tempdir().unwrap();
    let dest = dir.path().join("wd14-vit");

    let (result, phases) = run_download(&downloader, &variant, &dest);

    let err = result.expect_err("常に失敗する場合は Download 失敗すべき");
    assert_eq!(err.kind, AppErrorKind::Download);
    // onnx で 3 回打ち切り、タグ定義未到達。
    assert_eq!(downloader.calls_for("model.onnx"), MAX_ATTEMPTS);
    assert_eq!(downloader.calls_for("selected_tags.csv"), 0);
    assert!(!phases.contains(&DownloadPhase::Saved));
}
