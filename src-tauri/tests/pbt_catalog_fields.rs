// Feature: local-model-management, Property 1: カタログ必須フィールドの非空
// Feature: local-model-management, Property 2: カタログ登録・除外の分割
// Feature: local-model-management, Property 3: 識別子の一意性
//
// 本ファイルは静的カタログの構築・検証（model_service::builtin_catalog /
// validate_catalog）に関する 3 つの Correctness Property を検証する。
//
// - Property 1（Validates: Requirements 1.3）:
//     任意の Model_Variant について、builtin_catalog() に登録された各 Variant の
//     識別子・表示名・Model_Family はいずれも空でない値を持つ。validate_catalog の
//     登録集合も同様に非空。family は enum のため常に有効。
// - Property 2（Validates: Requirements 1.4）:
//     任意の Variant 候補列について、登録される Variant はすべて識別子・表示名が
//     非空であり、除外される候補はいずれかのフィールドが欠落/空であり、登録集合と
//     除外集合の和は入力候補全体に一致する。
// - Property 3（Validates: Requirements 1.6, 1.7, 1.8）:
//     識別子衝突・同一リポジトリ複数 .onnx を含む任意の候補列について、
//     validate_catalog 後の全 Variant にわたり識別子は一意であり重複が残らない。

use std::collections::HashSet;

use proptest::prelude::*;
use tag_editor_core::models::{ModelFamily, ModelSource, ModelVariant};
use tag_editor_core::services::model_service::{builtin_catalog, validate_catalog};

/// Model_Family の生成器（Wd14 / MlDanbooru）。enum のため常に非空。
fn family_strategy() -> impl Strategy<Value = ModelFamily> {
    prop_oneof![Just(ModelFamily::Wd14), Just(ModelFamily::MlDanbooru)]
}

/// id の生成器。空文字（および空白のみ）を含め、必須フィールド欠落ケースを踏ませる。
fn id_strategy() -> impl Strategy<Value = String> {
    prop_oneof![
        Just(String::new()),
        Just("   ".to_string()),
        "[a-z0-9_-]{1,16}",
    ]
}

/// display_name の生成器。空文字・空白のみを含める。
fn display_name_strategy() -> impl Strategy<Value = String> {
    prop_oneof![
        Just(String::new()),
        Just("  ".to_string()),
        "[A-Za-z0-9 _-]{1,20}",
    ]
}

/// Model_Source の生成器。validate_catalog は source を参照しないが、
/// 同一リポジトリ複数 .onnx（Property 3）を踏ませるため repo/onnx_file を制御可能に
/// する生成器を別途用意する（下記 candidate_strategy 参照）。
fn source_with(repo: String, onnx_file: String) -> ModelSource {
    ModelSource {
        repo,
        onnx_file,
        tag_files: vec!["selected_tags.csv".to_string()],
    }
}

/// 任意の候補 Variant の生成器。空フィールド・任意 id を含む。
fn candidate_strategy() -> impl Strategy<Value = ModelVariant> {
    (
        id_strategy(),
        display_name_strategy(),
        family_strategy(),
        "[a-z0-9/_-]{1,20}",
        "[a-z0-9/_.-]{1,16}\\.onnx",
    )
        .prop_map(|(id, display_name, family, repo, onnx_file)| ModelVariant {
            id,
            display_name,
            family,
            source: source_with(repo, onnx_file),
        })
}

/// 識別子衝突を積極的に踏ませるための候補列生成器。
///
/// 少数の「id プール」から id を選ばせることで重複を高頻度で発生させる。
/// 同一リポジトリで onnx_file だけ異なる候補（要件 1.6 の複数 .onnx）も含める。
fn colliding_candidates_strategy() -> impl Strategy<Value = Vec<ModelVariant>> {
    // 衝突しやすい小さな id プール（空文字含む）。
    let id_pool = prop_oneof![
        Just(String::new()),
        Just("dup".to_string()),
        Just("dup".to_string()),
        Just("dup".to_string()),
        "[a-z]{1,3}",
    ];
    prop::collection::vec(
        (
            id_pool,
            display_name_strategy(),
            family_strategy(),
            // 同一リポジトリを踏ませるため repo も小さなプールから選ぶ。
            prop_oneof![
                Just("deepghs/ml-danbooru-onnx".to_string()),
                Just("SmilingWolf/wd-vit-tagger-v3".to_string()),
            ],
            "[a-z0-9_-]{1,10}\\.onnx",
        )
            .prop_map(|(id, display_name, family, repo, onnx_file)| ModelVariant {
                id,
                display_name,
                family,
                source: source_with(repo, onnx_file),
            }),
        0..12,
    )
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(200))]

    // ------------------------------------------------------------------
    // Property 1: カタログ必須フィールドの非空
    // ------------------------------------------------------------------

    /// Property 1: builtin_catalog() の各 Variant の id / display_name は非空。
    /// family は enum で常に有効。builtin_catalog は入力に依らず静的なため、
    /// 反復ごとに毎回同一結果を返すことも併せて確認する（決定性）。
    #[test]
    fn builtin_catalog_fields_non_empty(_seed in any::<u8>()) {
        let catalog = builtin_catalog();
        prop_assert!(!catalog.is_empty(), "builtin_catalog は空であってはならない");
        for v in &catalog {
            prop_assert!(
                !v.id.trim().is_empty(),
                "id が空: {v:?}"
            );
            prop_assert!(
                !v.display_name.trim().is_empty(),
                "display_name が空: {v:?}"
            );
            // family は enum のため常に有効（Wd14 / MlDanbooru のいずれか）。
            prop_assert!(matches!(v.family, ModelFamily::Wd14 | ModelFamily::MlDanbooru));
        }
    }

    /// Property 1（validate_catalog 経由）: 任意の候補列を検証したとき、登録集合の
    /// 各 Variant の id / display_name は非空である。
    #[test]
    fn validate_catalog_registered_fields_non_empty(
        candidates in prop::collection::vec(candidate_strategy(), 0..16),
    ) {
        let (registered, _excluded) = validate_catalog(candidates);
        for v in &registered {
            prop_assert!(!v.id.trim().is_empty(), "登録集合に空 id: {v:?}");
            prop_assert!(!v.display_name.trim().is_empty(), "登録集合に空 display_name: {v:?}");
        }
    }

    // ------------------------------------------------------------------
    // Property 2: カタログ登録・除外の分割
    // ------------------------------------------------------------------

    /// Property 2: 空フィールド候補・欠落候補を含む任意の候補列について、
    /// - 登録集合は id / display_name がすべて非空
    /// - 除外集合はいずれかが空（欠落）
    /// - 登録集合 ∪ 除外集合 の件数 = 入力候補全体の件数
    #[test]
    fn validate_catalog_partitions_input(
        candidates in prop::collection::vec(candidate_strategy(), 0..16),
    ) {
        let total = candidates.len();
        let (registered, excluded) = validate_catalog(candidates);

        // 登録集合はすべて非空。
        for v in &registered {
            prop_assert!(!v.id.trim().is_empty() && !v.display_name.trim().is_empty());
        }
        // 除外集合はいずれかが空（除外理由）。
        for v in &excluded {
            prop_assert!(
                v.id.trim().is_empty() || v.display_name.trim().is_empty(),
                "除外集合に必須フィールド非空の候補が混入: {v:?}"
            );
        }
        // 登録 ∪ 除外 = 入力全体（件数一致）。
        prop_assert_eq!(
            registered.len() + excluded.len(),
            total,
            "登録集合と除外集合の和が入力全体に一致しない"
        );
    }

    // ------------------------------------------------------------------
    // Property 3: 識別子の一意性
    // ------------------------------------------------------------------

    /// Property 3: 識別子衝突・同一リポジトリ複数 .onnx を含む任意の候補列について、
    /// validate_catalog 後の登録集合の id は一意であり、重複が 1 つも残らない。
    #[test]
    fn validate_catalog_ids_unique(
        candidates in colliding_candidates_strategy(),
    ) {
        let (registered, _excluded) = validate_catalog(candidates);

        let ids: Vec<&str> = registered.iter().map(|v| v.id.as_str()).collect();
        let unique: HashSet<&str> = ids.iter().copied().collect();
        prop_assert_eq!(
            unique.len(),
            ids.len(),
            "一意化後に重複 id が残った: {:?}",
            ids
        );

        // 同一リポジトリで複数 .onnx から生成された Variant も相異なる id を持つ
        // （上の一意性検証に含意されるが、意図を明示するため再確認）。
        let mut seen: HashSet<&str> = HashSet::new();
        for v in &registered {
            prop_assert!(seen.insert(v.id.as_str()), "重複 id: {:?}", v.id);
        }
    }

    /// Property 3（builtin_catalog）: 静的カタログの id も全体で一意。
    #[test]
    fn builtin_catalog_ids_unique(_seed in any::<u8>()) {
        let catalog = builtin_catalog();
        let ids: Vec<&str> = catalog.iter().map(|v| v.id.as_str()).collect();
        let unique: HashSet<&str> = ids.iter().copied().collect();
        prop_assert_eq!(unique.len(), ids.len(), "builtin_catalog の id に重複");
    }
}

/// 代表的な固定ケース: 空 id・空 display・id 衝突を混在させた候補列の分割と一意化。
#[test]
fn representative_partition_and_dedup() {
    fn cand(id: &str, display: &str, family: ModelFamily, onnx: &str) -> ModelVariant {
        ModelVariant {
            id: id.to_string(),
            display_name: display.to_string(),
            family,
            source: source_with("deepghs/ml-danbooru-onnx".to_string(), onnx.to_string()),
        }
    }

    let candidates = vec![
        cand("a", "A", ModelFamily::Wd14, "a.onnx"),
        cand("", "空 id", ModelFamily::Wd14, "b.onnx"),
        cand(
            "dup",
            "同一リポジトリ .onnx 1",
            ModelFamily::MlDanbooru,
            "c.onnx",
        ),
        cand(
            "dup",
            "同一リポジトリ .onnx 2",
            ModelFamily::MlDanbooru,
            "d.onnx",
        ),
        cand("empty-name", "   ", ModelFamily::Wd14, "e.onnx"),
    ];
    let total = candidates.len();
    let (registered, excluded) = validate_catalog(candidates);

    // 登録 ∪ 除外 = 入力全体。
    assert_eq!(registered.len() + excluded.len(), total);
    // 除外は「空 id」「空 display」の 2 件。
    assert_eq!(excluded.len(), 2);
    // 登録集合の id は一意（dup は一意化される）。
    let ids: HashSet<&str> = registered.iter().map(|v| v.id.as_str()).collect();
    assert_eq!(ids.len(), registered.len());
    // 登録集合の必須フィールドは非空。
    assert!(registered
        .iter()
        .all(|v| !v.id.trim().is_empty() && !v.display_name.trim().is_empty()));
}
