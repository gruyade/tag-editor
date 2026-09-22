// Feature: local-model-management, Property 5: variant_dir の一意性
//
// Property 5: 任意の base_dir と、識別子が相異なる 2 つの Model_Variant について、
// variant_dir が返す保存先パスは相異なる。
//
// variant_dir は resolve_model_dir(base_dir)/<variant.id> を返すため、base_dir が
// 同一でも id が異なれば末尾コンポーネントが異なり、保存先パスは相異なる。
//
// Validates: Requirements 2.3

use std::path::PathBuf;

use proptest::prelude::*;
use tag_editor_core::models::{ModelFamily, ModelSource, ModelVariant};
use tag_editor_core::services::model_service::variant_dir;

/// Model_Family の生成器（要件 1.3 の非空系統）。
fn family_strategy() -> impl Strategy<Value = ModelFamily> {
    prop_oneof![Just(ModelFamily::Wd14), Just(ModelFamily::MlDanbooru)]
}

/// Model_Source の生成器。variant_dir は source を参照しないが、ModelVariant の
/// 全フィールドを埋めるため妥当な値を生成する。
fn source_strategy() -> impl Strategy<Value = ModelSource> {
    (
        "[a-z0-9/_-]{1,20}",
        "[a-z0-9/_.-]{1,20}\\.onnx",
        prop::collection::vec("[a-z0-9/_.-]{1,20}\\.(csv|json)", 0..3),
    )
        .prop_map(|(repo, onnx_file, tag_files)| ModelSource {
            repo,
            onnx_file,
            tag_files,
        })
}

/// 指定した id を持つ ModelVariant を組み立てる。
///
/// id 以外のフィールドは variant_dir の結果に影響しないため、生成された値を
/// そのまま埋める（全フィールドを非空で埋める）。
fn variant_with_id(
    id: String,
    display_name: String,
    family: ModelFamily,
    source: ModelSource,
) -> ModelVariant {
    ModelVariant {
        id,
        display_name,
        family,
        source,
    }
}

/// base_dir の生成器。空・末尾区切り・非 ASCII・多階層を含める。
fn base_dir_strategy() -> impl Strategy<Value = PathBuf> {
    prop_oneof![
        Just(PathBuf::from("")),
        "[a-z0-9_-]{1,12}".prop_map(PathBuf::from),
        "[a-z0-9_/-]{1,20}".prop_map(PathBuf::from),
        Just(PathBuf::from("モデル/置き場")),
        Just(PathBuf::from("/abs/base")),
    ]
}

/// 相異なる 2 つの識別子の生成器。
///
/// path セパレータや `..` を含まない安全なカタログ識別子（既存テストと同じ
/// `[a-z0-9_-]` 系）に限定し、id1 != id2 を prop_assume! で保証する。
fn distinct_ids_strategy() -> impl Strategy<Value = (String, String)> {
    ("[a-z0-9_-]{1,24}", "[a-z0-9_-]{1,24}")
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// Property 5: 相異なる id を持つ 2 つの Variant は、同一 base_dir でも
    /// 相異なる保存先パスを持つ。
    #[test]
    fn distinct_ids_yield_distinct_variant_dirs(
        base_dir in base_dir_strategy(),
        (id1, id2) in distinct_ids_strategy(),
        display1 in "[A-Za-z0-9 _-]{1,20}",
        display2 in "[A-Za-z0-9 _-]{1,20}",
        family1 in family_strategy(),
        family2 in family_strategy(),
        source1 in source_strategy(),
        source2 in source_strategy(),
    ) {
        // 相異なる id のみを対象にする（衝突ケースは前提から除外）。
        prop_assume!(id1 != id2);

        let v1 = variant_with_id(id1, display1, family1, source1);
        let v2 = variant_with_id(id2, display2, family2, source2);

        let dir1 = variant_dir(&base_dir, &v1);
        let dir2 = variant_dir(&base_dir, &v2);

        prop_assert_ne!(
            &dir1,
            &dir2,
            "相異なる id の保存先が一致した: base_dir={:?}, id1={:?}, id2={:?}",
            base_dir,
            v1.id,
            v2.id
        );
    }

    /// Property 5（同一 id は同一パス・対比用）: id が同一なら base_dir が同じ
    /// 限り保存先も同一（決定性）。上の一意性が「id 由来」であることを補強する。
    #[test]
    fn same_id_yields_same_variant_dir(
        base_dir in base_dir_strategy(),
        id in "[a-z0-9_-]{1,24}",
        source_a in source_strategy(),
        source_b in source_strategy(),
    ) {
        // 表示名・系統・取得元が異なっても、id が同じなら保存先は一致する。
        let v_a = variant_with_id(id.clone(), "A".to_string(), ModelFamily::Wd14, source_a);
        let v_b = variant_with_id(id, "B".to_string(), ModelFamily::MlDanbooru, source_b);

        prop_assert_eq!(
            variant_dir(&base_dir, &v_a),
            variant_dir(&base_dir, &v_b),
            "id が同一なのに保存先が異なる"
        );
    }
}

/// 代表的な固定ケース: 相異なる 2 つの WD14 バリアントで保存先が異なることを
/// 明示的に確認する。
#[test]
fn representative_fixed_case() {
    let base = PathBuf::from("/app/base");
    let source = ModelSource {
        repo: "SmilingWolf/wd-vit-tagger-v3".to_string(),
        onnx_file: "model.onnx".to_string(),
        tag_files: vec!["selected_tags.csv".to_string()],
    };
    let v1 = variant_with_id(
        "wd14-vit".to_string(),
        "WD14 ViT".to_string(),
        ModelFamily::Wd14,
        source.clone(),
    );
    let v2 = variant_with_id(
        "wd14-convnext".to_string(),
        "WD14 ConvNeXT".to_string(),
        ModelFamily::Wd14,
        source,
    );

    assert_ne!(variant_dir(&base, &v1), variant_dir(&base, &v2));
    // 保存先は resolve_model_dir(base)/<id> の形。
    assert!(variant_dir(&base, &v1).ends_with("wd14-vit"));
    assert!(variant_dir(&base, &v2).ends_with("wd14-convnext"));
}
