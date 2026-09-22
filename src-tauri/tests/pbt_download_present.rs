// Feature: local-model-management, Property 7: 保存から存在判定への整合
//
// Property 7: 任意の妥当な .onnx バイト列とタグ定義について、それらを variant_dir
// へ保存する Download_Operation が成功すると、当該 Variant は Model_Present と
// 判定される。
//
// download_variant は variant.source.onnx_file / tag_files に対応するバイト列を
// トランスポート（ModelDownloader）から取得し、variant_dir へ原子的に保存する。
// 全取得・保存が成功した後は「.onnx とタグ定義（.csv/.json）の対」が variant_dir
// 直下に揃うため、is_present(variant_dir) は true を返す（要件 5.5）。
//
// ジェネレータは以下を変化させる:
// - onnx バイト列（空を含む任意長）
// - タグ定義内容（.csv / .json の妥当・任意内容）
// - source.onnx_file 名（サブディレクトリ付きのリポジトリ内パスを含む。保存名は
//   ベース名になる）
// - source.tag_files 名（.csv / .json 拡張子）
//
// download_variant が Ok を返した実行のみ Property 7 の対象とし、その場合に
// is_present が true であることを検証する。cancel は false 固定、on_progress は
// 空クロージャ。
//
// Validates: Requirements 5.5

use std::collections::HashMap;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use proptest::prelude::*;
use tag_editor_core::models::{ModelFamily, ModelSource, ModelVariant};
use tag_editor_core::services::model_service::{
    download_variant, is_present, DownloadError, DownloadPhase, ModelDownloader,
};

/// ネットワーク非依存のモックダウンローダ。
///
/// `responses` は「リポジトリ内ファイル名 → 返すバイト列」。download_variant は
/// source.onnx_file と tag_files の各候補名で fetch を呼ぶため、それらのキーへ
/// 対応バイト列を登録しておけば取得が成功する。登録の無いファイル名は
/// [`DownloadError::Other`] で失敗させる（tag_files のフォールバック検証にも使える）。
///
/// download_model_tests の MockDownloader は pub(crate) で tests からアクセス
/// できないため、本テスト内で独自に定義する。
struct MockDownloader {
    responses: HashMap<String, Vec<u8>>,
}

impl MockDownloader {
    fn new(responses: HashMap<String, Vec<u8>>) -> Self {
        Self { responses }
    }
}

impl ModelDownloader for MockDownloader {
    fn fetch(&self, _repo: &str, file: &str, _timeout: Duration) -> Result<Vec<u8>, DownloadError> {
        match self.responses.get(file) {
            Some(bytes) => Ok(bytes.clone()),
            None => Err(DownloadError::Other(format!("未登録のファイル: {file}"))),
        }
    }
}

/// 任意の onnx バイト列の生成器（空を含む）。中身は verbatim 保存されるだけなので
/// 妥当な ONNX である必要はない（is_present は拡張子の対だけを見る）。
fn onnx_bytes_strategy() -> impl Strategy<Value = Vec<u8>> {
    prop::collection::vec(any::<u8>(), 0..64)
}

/// 任意のタグ定義内容の生成器（バイト列。空を含む）。
fn tagdef_bytes_strategy() -> impl Strategy<Value = Vec<u8>> {
    prop::collection::vec(any::<u8>(), 0..64)
}

/// source.onnx_file 名の生成器。単純名とサブディレクトリ付きパスを混ぜる。
/// download_variant はベース名（拡張子 .onnx）で保存する。
fn onnx_file_strategy() -> impl Strategy<Value = String> {
    prop_oneof![
        "[a-z0-9_-]{1,16}".prop_map(|s| format!("{s}.onnx")),
        "[a-z0-9_-]{1,8}/[a-z0-9_-]{1,12}".prop_map(|s| format!("{s}.onnx")),
    ]
}

/// tag_files の 1 候補名の生成器（.csv / .json）。
fn tag_file_name_strategy() -> impl Strategy<Value = String> {
    prop_oneof![
        "[a-z0-9_-]{1,16}".prop_map(|s| format!("{s}.csv")),
        "[a-z0-9_-]{1,16}".prop_map(|s| format!("{s}.json")),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(200))]

    /// Property 7: 妥当な onnx バイト列・タグ定義を variant_dir へ保存する
    /// download_variant が成功すると、is_present(variant_dir) が true になる。
    #[test]
    fn successful_download_makes_variant_present(
        onnx_bytes in onnx_bytes_strategy(),
        tagdef_bytes in tagdef_bytes_strategy(),
        onnx_file in onnx_file_strategy(),
        tag_file in tag_file_name_strategy(),
    ) {
        // mock に onnx とタグ定義の対応バイト列を登録する。fetch は
        // source.onnx_file / tag_files の各名で呼ばれるため、その名前をキーにする。
        let mut responses = HashMap::new();
        responses.insert(onnx_file.clone(), onnx_bytes.clone());
        responses.insert(tag_file.clone(), tagdef_bytes.clone());
        let downloader = MockDownloader::new(responses);

        let variant = ModelVariant {
            id: "prop7-variant".to_string(),
            display_name: "Property 7 Variant".to_string(),
            family: ModelFamily::Wd14,
            source: ModelSource {
                repo: "owner/prop7".to_string(),
                onnx_file: onnx_file.clone(),
                tag_files: vec![tag_file.clone()],
            },
        };

        // 一意な variant_dir を tempdir 配下に用意する（存在しないサブパス）。
        let tmp = tempfile::tempdir().expect("一時ディレクトリ作成に失敗");
        let variant_dir = tmp.path().join(&variant.id);

        let cancel = AtomicBool::new(false);
        let mut phases: Vec<DownloadPhase> = Vec::new();

        let result =
            download_variant(&downloader, &variant, &variant_dir, &cancel, |p| phases.push(p));

        // 成功した実行のみ Property 7 の対象。
        let returned = result.expect("妥当な onnx/タグ定義の保存は成功するはず");
        prop_assert_eq!(&returned, &variant_dir, "戻り値は variant_dir と一致すべき");

        // 保存後は .onnx とタグ定義の対が揃い Model_Present と判定される（要件 5.5）。
        prop_assert!(
            is_present(&variant_dir),
            "保存成功後は Model_Present であるべき: onnx_file={:?}, tag_file={:?}",
            onnx_file,
            tag_file
        );

        // 進捗フェーズは onnx → tagdef → saved の順で通知される（補助検証）。
        prop_assert_eq!(
            phases,
            vec![
                DownloadPhase::OnnxFetched,
                DownloadPhase::TagDefinitionFetched,
                DownloadPhase::Saved,
            ]
        );
    }
}

/// 代表固定ケース: WD14 標準構成（model.onnx + selected_tags.csv）の保存成功で
/// is_present が true になる。
#[test]
fn representative_wd14_download_is_present() {
    let mut responses = HashMap::new();
    responses.insert("model.onnx".to_string(), b"onnx-bytes".to_vec());
    responses.insert(
        "selected_tags.csv".to_string(),
        b"tag_id,name,category\n1,solo,0\n".to_vec(),
    );
    let downloader = MockDownloader::new(responses);

    let variant = ModelVariant {
        id: "wd14-vit".to_string(),
        display_name: "WD14 ViT".to_string(),
        family: ModelFamily::Wd14,
        source: ModelSource {
            repo: "SmilingWolf/wd-vit-tagger-v3".to_string(),
            onnx_file: "model.onnx".to_string(),
            tag_files: vec!["selected_tags.csv".to_string()],
        },
    };

    let tmp = tempfile::tempdir().unwrap();
    let variant_dir = tmp.path().join(&variant.id);
    let cancel = AtomicBool::new(false);

    // 保存前は Not_Present。
    assert!(!is_present(&variant_dir));

    download_variant(&downloader, &variant, &variant_dir, &cancel, |_| {}).unwrap();

    // 保存後は Model_Present。
    assert!(is_present(&variant_dir));
}
