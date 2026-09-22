// Feature: local-model-management, Property 8: 原子的保存の全か無か
//
// Property 8: 任意の Download_Operation の結末（成功／取得失敗／保存失敗／
// キャンセル）について、成功時は variant_dir に .onnx とタグ定義の対がともに存在し
// （Model_Present）、失敗またはキャンセル時は当該操作が作成した部分ファイルが残らず
// Model_Present と判定される Assets が存在しない（Not_Present へ戻る）。
//
// 前提: このテストは「新規（Not_Present 起点）」の download に限定する。variant_dir は
// 毎回空/新規から始めるため、失敗・キャンセル時は Not_Present へ戻ることを検証する。
// 上書き（既存 Present）失敗時の既存保持は Property 10（Task 9.5）で別途検証する。
//
// Validates: Requirements 2.4, 2.6, 4.4, 5.4, 5.6, 6.4, 6.5, 6.6

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use proptest::prelude::*;
use tag_editor_core::models::{ModelFamily, ModelSource, ModelVariant};
use tag_editor_core::services::model_service::{
    download_variant, is_present, DownloadError, DownloadPhase, ModelDownloader,
};

/// Download_Operation の結末を制御する生成器の 1 要素。
///
/// download_variant は「onnx 取得 → タグ定義取得 → 保存」の順で進むため、
/// この結末で MockDownloader または cancel フラグの挙動を切り替える。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Outcome {
    /// 全成功（onnx・タグ定義ともに取得成功 → 保存成功）。
    Success,
    /// onnx 取得失敗（fetch が常に失敗を返す）。
    OnnxFetchFail,
    /// タグ定義取得失敗（onnx は成功、タグ定義候補は全て失敗）。
    TagDefFetchFail,
    /// 開始前キャンセル（cancel を最初から true にする）。
    CancelUpfront,
    /// onnx 取得完了フェーズでキャンセル（on_progress 内で cancel をセット）。
    CancelAfterOnnx,
    /// タグ定義取得完了フェーズでキャンセル（on_progress 内で cancel をセット）。
    CancelAfterTagDef,
}

impl Outcome {
    /// 成功結末か。成功なら is_present==true、それ以外は is_present==false を期待。
    fn is_success(&self) -> bool {
        matches!(self, Outcome::Success)
    }
}

fn outcome_strategy() -> impl Strategy<Value = Outcome> {
    prop_oneof![
        Just(Outcome::Success),
        Just(Outcome::OnnxFetchFail),
        Just(Outcome::TagDefFetchFail),
        Just(Outcome::CancelUpfront),
        Just(Outcome::CancelAfterOnnx),
        Just(Outcome::CancelAfterTagDef),
    ]
}

/// テスト専用のダウンロードトランスポート。ネットワークを使わず、ファイル名に応じて
/// 成功/失敗を切り替える。onnx（`onnx_file` の名前）とタグ定義（`.csv`/`.json`）で
/// 別々に成否を制御できる。
struct MockDownloader {
    /// onnx ファイル名（この名前の fetch のみ onnx 扱い）。
    onnx_file: String,
    /// onnx 取得を成功させるか。
    onnx_ok: bool,
    /// タグ定義取得を成功させるか。
    tagdef_ok: bool,
}

impl ModelDownloader for MockDownloader {
    fn fetch(
        &self,
        _repo: &str,
        file: &str,
        _timeout: Duration,
    ) -> Result<Vec<u8>, DownloadError> {
        if file == self.onnx_file {
            if self.onnx_ok {
                Ok(b"onnx-bytes".to_vec())
            } else {
                Err(DownloadError::Other("mock onnx 取得失敗".to_string()))
            }
        } else {
            // タグ定義候補。
            if self.tagdef_ok {
                Ok(b"tag-def-bytes".to_vec())
            } else {
                Err(DownloadError::Other("mock タグ定義取得失敗".to_string()))
            }
        }
    }
}

/// テスト用 Variant を組み立てる。onnx は `model.onnx`、タグ定義候補は
/// `selected_tags.csv` の 1 件（download_variant の分岐を素直に通す）。
fn test_variant() -> ModelVariant {
    ModelVariant {
        id: "wd14-test".to_string(),
        display_name: "WD14 Test".to_string(),
        family: ModelFamily::Wd14,
        source: ModelSource {
            repo: "SmilingWolf/wd-vit-tagger-v3".to_string(),
            onnx_file: "model.onnx".to_string(),
            tag_files: vec!["selected_tags.csv".to_string()],
        },
    }
}

/// 与えられた結末に応じた MockDownloader を組み立てる。
fn downloader_for(outcome: Outcome, onnx_file: &str) -> MockDownloader {
    let (onnx_ok, tagdef_ok) = match outcome {
        Outcome::OnnxFetchFail => (false, true),
        Outcome::TagDefFetchFail => (true, false),
        // 成功・キャンセル系は取得自体は成功させ、キャンセルは cancel フラグで表現する。
        _ => (true, true),
    };
    MockDownloader {
        onnx_file: onnx_file.to_string(),
        onnx_ok,
        tagdef_ok,
    }
}

/// 指定結末で download_variant を実行し、new（Not_Present 起点）の variant_dir に
/// 対する Property 8 を検証する。variant_dir は毎回新規（空）から始める。
fn run_case(outcome: Outcome) -> Result<(), TestCaseError> {
    let base = tempfile::tempdir().expect("一時ディレクトリ作成に失敗");
    // 新規起点: variant_dir はまだ作らない（download_variant が作る）。
    let variant_dir_path = base.path().join("variant");

    // 開始時は必ず Not_Present（新規起点の前提）。
    prop_assert!(
        !is_present(&variant_dir_path),
        "前提: 開始時は Not_Present であるべき"
    );

    let variant = test_variant();
    let downloader = downloader_for(outcome, &variant.source.onnx_file);

    // キャンセル制御。開始前キャンセルは true で開始する。
    let cancel = AtomicBool::new(matches!(outcome, Outcome::CancelUpfront));

    // on_progress 内で境界キャンセルを表現する。次の境界確認で中断させる。
    let on_progress = |phase: DownloadPhase| match outcome {
        Outcome::CancelAfterOnnx if phase == DownloadPhase::OnnxFetched => {
            cancel.store(true, Ordering::SeqCst);
        }
        Outcome::CancelAfterTagDef if phase == DownloadPhase::TagDefinitionFetched => {
            cancel.store(true, Ordering::SeqCst);
        }
        _ => {}
    };

    let result = download_variant(
        &downloader,
        &variant,
        &variant_dir_path,
        &cancel,
        on_progress,
    );

    if outcome.is_success() {
        // 成功: Ok を返し、variant_dir は Model_Present（.onnx とタグ定義の対）。
        prop_assert!(result.is_ok(), "成功結末は Ok を返すべき: {:?}", result);
        prop_assert!(
            is_present(&variant_dir_path),
            "成功時は Model_Present であるべき"
        );
    } else {
        // 失敗・キャンセル: Err を返し、新規起点なので Not_Present へ戻る。
        prop_assert!(
            result.is_err(),
            "失敗/キャンセル結末は Err を返すべき: outcome={:?}",
            outcome
        );
        prop_assert!(
            !is_present(&variant_dir_path),
            "失敗/キャンセル時は Model_Present な Assets が残ってはならない（Not_Present へ戻る）: outcome={:?}",
            outcome
        );
    }

    Ok(())
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(120))]

    /// Property 8: 新規起点の download は、成功なら Model_Present、失敗/キャンセルなら
    /// Model_Present な Assets が残らない（Not_Present へ戻る）。取得失敗（onnx/タグ
    /// 定義）・各境界でのキャンセル・成功を結末生成器でカバーする。
    #[test]
    fn all_or_nothing_for_new_download(outcome in outcome_strategy()) {
        run_case(outcome)?;
    }
}

/// 保存失敗の固定ケース: variant_dir の親をファイルにして create_dir_all を失敗させる。
///
/// download_variant は先頭で variant_dir を create_dir_all する。親がファイルだと
/// これが失敗し、取得を待たず保存失敗として Err を返す。新規起点なので Model_Present な
/// Assets は一切残らない。
#[test]
fn save_failure_leaves_not_present() {
    let base = tempfile::tempdir().expect("一時ディレクトリ作成に失敗");
    // 親をファイルにする: base/parent をファイルとして作る。
    let parent_as_file = base.path().join("parent");
    std::fs::write(&parent_as_file, b"i am a file, not a dir").unwrap();
    // variant_dir は「ファイルの下」= 作成不能。
    let variant_dir_path = parent_as_file.join("variant");

    let variant = test_variant();
    let downloader = downloader_for(Outcome::Success, &variant.source.onnx_file);
    let cancel = AtomicBool::new(false);

    let result = download_variant(
        &downloader,
        &variant,
        &variant_dir_path,
        &cancel,
        |_phase| {},
    );

    assert!(result.is_err(), "保存先を作成できない場合は Err を返すべき");
    assert!(
        !is_present(&variant_dir_path),
        "保存失敗時は Model_Present な Assets が残ってはならない"
    );
}

/// 保存失敗の固定ケース 2: onnx は書けるが、タグ定義の保存先が既存ディレクトリと
/// 衝突して書込に失敗する状況を作り、部分ファイル（onnx のみ）が残らないことを検証する。
///
/// タグ定義候補名（`selected_tags.csv`）と同名のディレクトリを variant_dir 直下に
/// 先に作っておくと、atomic_write のリネームがディレクトリ相手で失敗する。新規起点の
/// 保存失敗では onnx の部分ファイルも除去され、Not_Present になる。
#[test]
fn partial_save_failure_removes_partial_files() {
    let base = tempfile::tempdir().expect("一時ディレクトリ作成に失敗");
    let variant_dir_path = base.path().join("variant");
    std::fs::create_dir_all(&variant_dir_path).unwrap();

    // タグ定義の保存先名と同名のディレクトリを作り、タグ定義の書込を失敗させる。
    let tagdef_collision = variant_dir_path.join("selected_tags.csv");
    std::fs::create_dir_all(&tagdef_collision).unwrap();

    let variant = test_variant();
    let downloader = downloader_for(Outcome::Success, &variant.source.onnx_file);
    let cancel = AtomicBool::new(false);

    let result = download_variant(
        &downloader,
        &variant,
        &variant_dir_path,
        &cancel,
        |_phase| {},
    );

    assert!(result.is_err(), "タグ定義の保存に失敗する場合は Err を返すべき");
    // onnx は書けたかもしれないが、対（タグ定義）が無いので Model_Present ではない。
    assert!(
        !is_present(&variant_dir_path),
        "保存失敗時は Model_Present な Assets が残ってはならない（部分保存は不可視）"
    );
    // 明示確認: onnx 単体が残っていても present ではないが、新規起点の除去も期待する。
    let onnx_leftover = variant_dir_path.join("model.onnx");
    assert!(
        !onnx_leftover.exists(),
        "新規起点の保存失敗では部分ファイル（onnx）が除去されるべき"
    );
}

/// 代表的な成功固定ケース: 全成功時は variant_dir に .onnx とタグ定義の対が揃う。
#[test]
fn representative_success_case() {
    let base = tempfile::tempdir().expect("一時ディレクトリ作成に失敗");
    let variant_dir_path = base.path().join("variant");

    let variant = test_variant();
    let downloader = downloader_for(Outcome::Success, &variant.source.onnx_file);
    let cancel = AtomicBool::new(false);

    let result = download_variant(
        &downloader,
        &variant,
        &variant_dir_path,
        &cancel,
        |_phase| {},
    );

    assert!(result.is_ok(), "全成功は Ok を返すべき: {result:?}");
    assert!(is_present(&variant_dir_path), "成功時は Model_Present");
    assert!(variant_dir_path.join("model.onnx").exists());
    assert!(variant_dir_path.join("selected_tags.csv").exists());
}

/// 代表的な取得失敗固定ケース: onnx 取得に失敗すると Not_Present のまま。
#[test]
fn representative_onnx_fetch_failure_case() {
    let base = tempfile::tempdir().expect("一時ディレクトリ作成に失敗");
    let variant_dir_path = base.path().join("variant");

    let variant = test_variant();
    let downloader = downloader_for(Outcome::OnnxFetchFail, &variant.source.onnx_file);
    let cancel = AtomicBool::new(false);

    let result = download_variant(
        &downloader,
        &variant,
        &variant_dir_path,
        &cancel,
        |_phase| {},
    );

    assert!(result.is_err(), "onnx 取得失敗は Err を返すべき");
    assert!(
        !is_present(&variant_dir_path),
        "onnx 取得失敗時は Not_Present であるべき"
    );
}
