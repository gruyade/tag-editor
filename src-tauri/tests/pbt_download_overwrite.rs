// Feature: local-model-management, Property 10: 上書き失敗時の既存 Assets 保持
//
// Property 10: 任意の既に Model_Present である variant_dir について、上書き用
// Download_Operation が失敗しても、上書き前の `.onnx` とタグ定義の対は保持され、
// 当該 Variant は Model_Present のまま維持される。
//
// download_variant は開始時に is_present で was_present を記録し、was_present な
// 場合の保存は save_overwrite（既存を .overwrite.bak へ退避 → 新規配置 → 成功時
// .bak 削除 / 失敗時 .bak 復元）で行う。ただし取得段（.onnx / タグ定義の fetch）が
// 失敗した場合は保存段へ到達せず、既存 Assets には一切手を触れない。本テストは
// 「取得段の失敗」で既存対が無傷のまま Model_Present を維持することを検証する。
//
// Validates: Requirements 5.8

use std::fs;
use std::path::Path;
use std::sync::atomic::AtomicBool;

use proptest::prelude::*;
use tag_editor_core::models::{ModelFamily, ModelSource, ModelVariant};
use tag_editor_core::services::model_service::{
    download_variant, is_present, DownloadError, DownloadPhase, ModelDownloader,
};

/// 取得段で失敗させるモックダウンローダ。
///
/// `fail_on` で指定したファイル名の取得を必ず失敗させ、それ以外は登録済みバイト列を
/// 返す。上書き download を「取得失敗」させ、既存 Assets へ到達させないために用いる。
struct FailingDownloader {
    /// 取得に成功させるファイル（file 名 → バイト列）。
    responses: std::collections::HashMap<String, Vec<u8>>,
    /// この file 名の取得は常に失敗させる。
    fail_on: String,
    /// 失敗の種類。
    error: DownloadError,
}

impl ModelDownloader for FailingDownloader {
    fn fetch(
        &self,
        _repo: &str,
        file: &str,
        _timeout: std::time::Duration,
    ) -> Result<Vec<u8>, DownloadError> {
        if file == self.fail_on {
            return Err(self.error.clone());
        }
        match self.responses.get(file) {
            Some(bytes) => Ok(bytes.clone()),
            None => Err(DownloadError::Other(format!("未登録のファイル: {file}"))),
        }
    }
}

/// 上書き対象のテスト用 WD14 バリアント（source.onnx_file = model.onnx,
/// tag_files = [selected_tags.csv]）。
fn overwrite_variant() -> ModelVariant {
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

/// 失敗させる取得段の種類。
#[derive(Debug, Clone)]
enum FailKind {
    /// `.onnx` の取得を失敗させる（この場合タグ定義取得へ進まない）。
    Onnx(DownloadError),
    /// タグ定義（selected_tags.csv）の取得を失敗させる。
    TagDefinition(DownloadError),
}

/// 失敗の種類（onnx 取得失敗 / タグ定義取得失敗）× エラー種別の生成器。
fn fail_kind_strategy() -> impl Strategy<Value = FailKind> {
    let error = prop_oneof![
        Just(DownloadError::Timeout),
        "[a-z ]{1,20}".prop_map(DownloadError::Other),
    ];
    prop_oneof![
        error.clone().prop_map(FailKind::Onnx),
        error.prop_map(FailKind::TagDefinition),
    ]
}

/// variant_dir に一時ファイルが残っていないことを確認する。
///
/// save_overwrite の退避先（`.overwrite.bak`）や atomic_write の一時ファイル
/// （`.download.tmp`）が残っていないことを検証する。取得段で失敗した場合は保存段へ
/// 到達しないため、これらの一時ファイルは一切作られない。
fn assert_no_temp_files(dir: &Path) {
    for entry in fs::read_dir(dir).unwrap().flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        assert!(
            !name.ends_with(".overwrite.bak"),
            "退避ファイルが残っている: {name}"
        );
        assert!(
            !name.ends_with(".download.tmp"),
            "一時ファイルが残っている: {name}"
        );
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    /// Property 10: 既に Model_Present な variant_dir への上書き download が取得段で
    /// 失敗しても、上書き前の .onnx / タグ定義の対は内容ごと保持され、Model_Present の
    /// まま維持される。退避・一時ファイルも残らない。
    #[test]
    fn overwrite_failure_preserves_existing_assets(
        onnx_bytes in prop::collection::vec(any::<u8>(), 1..64),
        tagdef_bytes in prop::collection::vec(any::<u8>(), 1..64),
        fail_kind in fail_kind_strategy(),
    ) {
        // (1) 既存の .onnx / タグ定義を書いて Model_Present にする（既存内容を記録）。
        let dir = tempfile::tempdir().unwrap();
        let variant = overwrite_variant();
        let variant_dir = dir.path().join(&variant.id);
        fs::create_dir_all(&variant_dir).unwrap();

        let onnx_dest = variant_dir.join("model.onnx");
        let tagdef_dest = variant_dir.join("selected_tags.csv");
        fs::write(&onnx_dest, &onnx_bytes).unwrap();
        fs::write(&tagdef_dest, &tagdef_bytes).unwrap();

        // 前提: 上書き前は Model_Present。
        prop_assert!(is_present(&variant_dir), "前提として Model_Present であるべき");

        // (2) 上書き download を「取得失敗」させるモックを組む。
        //     新規取得分は既存と異なるバイト列（置き換わっていないことの検出用）。
        let new_onnx: &[u8] = b"NEW-ONNX-SHOULD-NOT-BE-WRITTEN";
        let new_csv: &[u8] = b"tag_id,name,category\n1,newtag,0\n";
        let mut responses = std::collections::HashMap::new();
        let (fail_on, error) = match &fail_kind {
            FailKind::Onnx(e) => {
                // onnx 取得を失敗させる。タグ定義は登録しても到達しない。
                ("model.onnx".to_string(), e.clone())
            }
            FailKind::TagDefinition(e) => {
                // onnx は取得成功、タグ定義取得で失敗させる。
                responses.insert("model.onnx".to_string(), new_onnx.to_vec());
                ("selected_tags.csv".to_string(), e.clone())
            }
        };
        // 念のため両方登録しておく（fail_on が優先されるので失敗する側は返らない）。
        responses.entry("model.onnx".to_string()).or_insert_with(|| new_onnx.to_vec());
        responses.entry("selected_tags.csv".to_string()).or_insert_with(|| new_csv.to_vec());

        let downloader = FailingDownloader { responses, fail_on, error };

        // (3) download_variant を呼び Err になることを確認。
        let cancel = AtomicBool::new(false);
        let result = download_variant(&downloader, &variant, &variant_dir, &cancel, |_: DownloadPhase| {});
        prop_assert!(result.is_err(), "取得段が失敗するので Err になるべき");

        // (4) 既存対は Model_Present のまま、かつ内容が変わっていない。
        prop_assert!(
            is_present(&variant_dir),
            "上書き失敗後も Model_Present を維持すべき（要件 5.8）"
        );
        prop_assert_eq!(
            fs::read(&onnx_dest).unwrap(),
            onnx_bytes,
            ".onnx の内容が上書き失敗で変わってはならない"
        );
        prop_assert_eq!(
            fs::read(&tagdef_dest).unwrap(),
            tagdef_bytes,
            "タグ定義の内容が上書き失敗で変わってはならない"
        );

        // (5) .overwrite.bak / .download.tmp 等の一時ファイルが残っていない。
        assert_no_temp_files(&variant_dir);
    }
}

/// 代表的な固定ケース: onnx 取得失敗で既存対が完全保持される。
#[test]
fn representative_onnx_fetch_failure_keeps_pair() {
    let dir = tempfile::tempdir().unwrap();
    let variant = overwrite_variant();
    let variant_dir = dir.path().join(&variant.id);
    fs::create_dir_all(&variant_dir).unwrap();

    let onnx_dest = variant_dir.join("model.onnx");
    let tagdef_dest = variant_dir.join("selected_tags.csv");
    fs::write(&onnx_dest, b"old-onnx").unwrap();
    fs::write(&tagdef_dest, b"tag_id,name,category\n1,old,0\n").unwrap();
    assert!(is_present(&variant_dir));

    let downloader = FailingDownloader {
        responses: std::collections::HashMap::new(),
        fail_on: "model.onnx".to_string(),
        error: DownloadError::Timeout,
    };
    let cancel = AtomicBool::new(false);
    let err = download_variant(&downloader, &variant, &variant_dir, &cancel, |_| {}).unwrap_err();
    assert_eq!(err.kind, tag_editor_core::error::AppErrorKind::Download);

    assert!(is_present(&variant_dir));
    assert_eq!(fs::read(&onnx_dest).unwrap(), b"old-onnx");
    assert_eq!(
        fs::read(&tagdef_dest).unwrap(),
        b"tag_id,name,category\n1,old,0\n"
    );
    assert!(!variant_dir.join("model.onnx.overwrite.bak").exists());
    assert!(!variant_dir.join("selected_tags.csv.overwrite.bak").exists());
}

/// 代表的な固定ケース: タグ定義取得失敗（onnx は取得成功）でも既存対が保持される。
#[test]
fn representative_tagdef_fetch_failure_keeps_pair() {
    let dir = tempfile::tempdir().unwrap();
    let variant = overwrite_variant();
    let variant_dir = dir.path().join(&variant.id);
    fs::create_dir_all(&variant_dir).unwrap();

    let onnx_dest = variant_dir.join("model.onnx");
    let tagdef_dest = variant_dir.join("selected_tags.csv");
    fs::write(&onnx_dest, b"old-onnx").unwrap();
    fs::write(&tagdef_dest, b"tag_id,name,category\n1,old,0\n").unwrap();
    assert!(is_present(&variant_dir));

    let mut responses = std::collections::HashMap::new();
    responses.insert("model.onnx".to_string(), b"new-onnx".to_vec());
    let downloader = FailingDownloader {
        responses,
        fail_on: "selected_tags.csv".to_string(),
        error: DownloadError::Other("network down".to_string()),
    };
    let cancel = AtomicBool::new(false);
    let err = download_variant(&downloader, &variant, &variant_dir, &cancel, |_| {}).unwrap_err();
    assert_eq!(err.kind, tag_editor_core::error::AppErrorKind::Download);

    // onnx 取得は成功したが保存段へ到達していないため既存は無傷。
    assert!(is_present(&variant_dir));
    assert_eq!(fs::read(&onnx_dest).unwrap(), b"old-onnx");
    assert_eq!(
        fs::read(&tagdef_dest).unwrap(),
        b"tag_id,name,category\n1,old,0\n"
    );
    assert!(!variant_dir.join("model.onnx.overwrite.bak").exists());
    assert!(!variant_dir.join("model.onnx.download.tmp").exists());
}
