// Feature: local-model-management, Property 6: Model_Present 判定の同値性
//
// Property 6: 任意の variant_dir 配下のファイル構成について、当該 Variant が
// Model_Present と判定されることは「`.onnx` ファイルが存在し、かつ拡張子 `.csv`
// または `.json` のタグ定義ファイルが存在する」ことと同値である（`.onnx` のみ・
// タグ定義のみ・ディレクトリ不在・空ディレクトリはいずれも Not_Present）。
//
// 大文字拡張子（`.ONNX` / `.CSV` / `.JSON`）も onnx_with_tagdef が
// to_ascii_lowercase で正規化するため present と判定される。生成器はこれらの
// エッジ（`.onnx` のみ・タグ定義のみ・両方・大文字拡張子・空ディレクトリ・
// ディレクトリ不在）をカバーする。
//
// Validates: Requirements 3.1, 3.2, 3.3, 3.4, 3.5, 3.6

use std::fs;
use std::path::{Path, PathBuf};

use proptest::prelude::*;
use tag_editor_core::models::{ModelFamily, ModelSource, ModelVariant, VariantPresence};
use tag_editor_core::services::model_service::{catalog_presence, is_present, variant_dir};

/// ファイル構成を記述する生成器の 1 要素。生成された各ファイルは variant_dir 直下に
/// 実ファイルとして作られる。拡張子は present 判定に効くもの・効かないものを混在。
#[derive(Debug, Clone)]
enum FileSpec {
    /// 小文字 `.onnx`。
    Onnx,
    /// 大文字 `.ONNX`（大小無視で onnx として扱われるべき）。
    OnnxUpper,
    /// 小文字 `.csv` タグ定義。
    Csv,
    /// 大文字 `.CSV` タグ定義。
    CsvUpper,
    /// 小文字 `.json` タグ定義。
    Json,
    /// 大文字 `.JSON` タグ定義。
    JsonUpper,
    /// present 判定に無関係な拡張子（txt / png など）。
    Irrelevant,
}

impl FileSpec {
    /// このファイルが「onnx として数えられる」か。
    fn is_onnx(&self) -> bool {
        matches!(self, FileSpec::Onnx | FileSpec::OnnxUpper)
    }

    /// このファイルが「タグ定義として数えられる」か。
    fn is_tagdef(&self) -> bool {
        matches!(
            self,
            FileSpec::Csv | FileSpec::CsvUpper | FileSpec::Json | FileSpec::JsonUpper
        )
    }

    /// variant_dir からの相対ファイル名。重複拡張子でも衝突しないよう index を付す。
    fn file_name(&self, index: usize) -> String {
        match self {
            FileSpec::Onnx => format!("model_{index}.onnx"),
            FileSpec::OnnxUpper => format!("model_{index}.ONNX"),
            FileSpec::Csv => format!("tags_{index}.csv"),
            FileSpec::CsvUpper => format!("tags_{index}.CSV"),
            FileSpec::Json => format!("tags_{index}.json"),
            FileSpec::JsonUpper => format!("tags_{index}.JSON"),
            FileSpec::Irrelevant => format!("note_{index}.txt"),
        }
    }
}

/// 単一 FileSpec の生成器。全バリアントを網羅する。
fn file_spec_strategy() -> impl Strategy<Value = FileSpec> {
    prop_oneof![
        Just(FileSpec::Onnx),
        Just(FileSpec::OnnxUpper),
        Just(FileSpec::Csv),
        Just(FileSpec::CsvUpper),
        Just(FileSpec::Json),
        Just(FileSpec::JsonUpper),
        Just(FileSpec::Irrelevant),
    ]
}

/// ファイル構成（0 件を含む）の生成器。0 件は空ディレクトリを表す。
fn file_layout_strategy() -> impl Strategy<Value = Vec<FileSpec>> {
    prop::collection::vec(file_spec_strategy(), 0..6)
}

/// 期待される present 判定: onnx が 1 つ以上、かつ タグ定義が 1 つ以上あるとき true。
fn expected_present(specs: &[FileSpec]) -> bool {
    let has_onnx = specs.iter().any(FileSpec::is_onnx);
    let has_tagdef = specs.iter().any(FileSpec::is_tagdef);
    has_onnx && has_tagdef
}

/// specs に従って dir 直下へ実ファイルを作成する。
fn materialize(dir: &Path, specs: &[FileSpec]) {
    for (i, spec) in specs.iter().enumerate() {
        let path = dir.join(spec.file_name(i));
        fs::write(&path, b"x").expect("テストファイルの作成に失敗");
    }
}

/// テスト用 Variant を組み立てる。
fn variant(id: &str) -> ModelVariant {
    ModelVariant {
        id: id.to_string(),
        display_name: id.to_string(),
        family: ModelFamily::Wd14,
        source: ModelSource {
            repo: format!("repo/{id}"),
            onnx_file: "model.onnx".to_string(),
            tag_files: vec!["selected_tags.csv".to_string()],
        },
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(200))]

    /// Property 6: is_present の結果が「onnx あり かつ タグ定義あり」と同値。
    /// 空ディレクトリ（specs 空）は Not_Present、大文字拡張子は present に寄与する。
    #[test]
    fn is_present_equiv_to_onnx_and_tagdef(specs in file_layout_strategy()) {
        let dir = tempfile::tempdir().expect("一時ディレクトリ作成に失敗");
        materialize(dir.path(), &specs);

        let expected = expected_present(&specs);
        let actual = is_present(dir.path());

        prop_assert_eq!(
            actual,
            expected,
            "is_present が同値条件と一致しない: specs={:?}",
            specs
        );
    }

    /// Property 6（ディレクトリ不在）: 存在しない variant_dir は常に Not_Present。
    #[test]
    fn is_present_false_for_missing_dir(specs in file_layout_strategy()) {
        // 一時ディレクトリを作ってから削除し、確実に不在のパスを得る。
        let tmp = tempfile::tempdir().expect("一時ディレクトリ作成に失敗");
        let missing: PathBuf = tmp.path().join("does_not_exist");
        // specs は使わないが、ディレクトリ不在は構成に依らず false であることを表す。
        let _ = specs;

        prop_assert!(!is_present(&missing), "不在ディレクトリは Not_Present であるべき");
    }

    /// Property 6（catalog_presence 経由）: base_dir 配下に各 variant_dir を実体化し、
    /// catalog_presence の present が is_present および同値条件と一致する。
    #[test]
    fn catalog_presence_matches_layout(
        layouts in prop::collection::vec(file_layout_strategy(), 1..4),
    ) {
        let base = tempfile::tempdir().expect("一時ディレクトリ作成に失敗");

        // レイアウトごとに一意 id の Variant を用意し、その variant_dir を実体化する。
        let catalog: Vec<ModelVariant> =
            (0..layouts.len()).map(|i| variant(&format!("v{i}"))).collect();

        for (v, specs) in catalog.iter().zip(layouts.iter()) {
            let dir = variant_dir(base.path(), v);
            fs::create_dir_all(&dir).expect("variant_dir 作成に失敗");
            materialize(&dir, specs);
        }

        let presences: Vec<VariantPresence> = catalog_presence(base.path(), &catalog);

        prop_assert_eq!(presences.len(), catalog.len(), "件数は catalog と一致すべき");
        for ((p, v), specs) in presences.iter().zip(catalog.iter()).zip(layouts.iter()) {
            // 同順・同要素。
            prop_assert_eq!(&p.variant.id, &v.id, "戻り値の順序が catalog と不一致");
            let expected = expected_present(specs);
            prop_assert_eq!(
                p.present,
                expected,
                "catalog_presence の present が同値条件と不一致: id={:?}, specs={:?}",
                v.id,
                specs
            );
        }
    }
}

/// 代表固定ケース: `.onnx` のみ・タグ定義のみ・両方・空・大文字拡張子。
#[test]
fn representative_presence_cases() {
    // `.onnx` のみ → Not_Present（要件 3.3）。
    let d = tempfile::tempdir().unwrap();
    fs::write(d.path().join("model.onnx"), b"x").unwrap();
    assert!(!is_present(d.path()), ".onnx のみは Not_Present");

    // タグ定義のみ → Not_Present（要件 3.4）。
    let d = tempfile::tempdir().unwrap();
    fs::write(d.path().join("selected_tags.csv"), b"x").unwrap();
    assert!(!is_present(d.path()), "タグ定義のみは Not_Present");

    // 両方揃う → Model_Present（要件 3.2）。
    let d = tempfile::tempdir().unwrap();
    fs::write(d.path().join("model.onnx"), b"x").unwrap();
    fs::write(d.path().join("selected_tags.csv"), b"x").unwrap();
    assert!(is_present(d.path()), ".onnx + .csv は Model_Present");

    // 大文字拡張子でも present。
    let d = tempfile::tempdir().unwrap();
    fs::write(d.path().join("MODEL.ONNX"), b"x").unwrap();
    fs::write(d.path().join("TAGS.JSON"), b"x").unwrap();
    assert!(is_present(d.path()), "大文字拡張子でも Model_Present");

    // 空ディレクトリ → Not_Present（要件 3.6）。
    let d = tempfile::tempdir().unwrap();
    assert!(!is_present(d.path()), "空ディレクトリは Not_Present");
}

/// 要件 3.5/3.6: Model_Dir 不在なら catalog_presence は全件 present=false。
#[test]
fn catalog_presence_all_false_when_model_dir_missing() {
    // base_dir 自体は作るが、Model_Dir（resolve_model_dir(base)）は作らない。
    let base = tempfile::tempdir().unwrap();
    let catalog = vec![variant("a"), variant("b"), variant("c")];

    let presences = catalog_presence(base.path(), &catalog);
    assert_eq!(presences.len(), catalog.len());
    assert!(
        presences.iter().all(|p| !p.present),
        "Model_Dir 不在なら全件 Not_Present であるべき"
    );

    // 完全に存在しない base_dir でも同様（ディレクトリ不在）。
    let missing = base.path().join("nonexistent_base");
    let presences2 = catalog_presence(&missing, &catalog);
    assert!(presences2.iter().all(|p| !p.present));
}
