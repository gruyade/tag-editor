// Feature: tag-editor, Property 24: モデル一覧の ONNX フィルタ
//
// Property 24: 任意のモデル候補集合について、モデル一覧に提示される集合
// （ModelListing.available）は ONNX 形式を持つ（onnx_available == true）
// 候補の集合とちょうど一致する。加えて対象外集合（excluded）は
// onnx_available == false の候補とちょうど一致し、available ∪ excluded は
// 入力候補全体を過不足なく（欠落・重複なく）覆う。
//
// Validates: Requirements 15.1, 15.5

use proptest::prelude::*;
use tag_editor_core::models::{ModelFamily, ModelLocation, ModelVariant};
use tag_editor_core::services::model_service::list_models;

/// 系統の生成器。
fn family_strategy() -> impl Strategy<Value = ModelFamily> {
    prop_oneof![
        Just(ModelFamily::Wd14),
        Just(ModelFamily::MlDanbooru),
        Just(ModelFamily::Local),
    ]
}

/// 所在の生成器。Remote / Local を混在させる。
fn location_strategy() -> impl Strategy<Value = ModelLocation> {
    prop_oneof![
        "[a-z0-9/_-]{1,20}".prop_map(ModelLocation::Remote),
        "[a-z0-9/_.-]{1,20}".prop_map(ModelLocation::Local),
    ]
}

/// 単一の ModelVariant 生成器。
///
/// `onnx_available` はランダムな bool にし、フィルタが両方向に駆動される
/// ようにする。`id` は識別のためユニーク性が保てるパターンにする（ただし
/// 生成上の重複はテスト側でも同様に扱うため厳密なユニーク性は不要）。
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

/// 候補集合の生成器。空集合も含む。
fn candidates_strategy() -> impl Strategy<Value = Vec<ModelVariant>> {
    prop::collection::vec(variant_strategy(), 0..32)
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// Property 24（available == onnx_available==true の集合）:
    /// local_model_dir を None にしてローカル走査を行わず、候補集合のみで
    /// 一覧化したとき、available は onnx_available==true の部分列と
    /// 完全一致する（順序・重複を保存）。
    #[test]
    fn available_equals_onnx_true_subset(candidates in candidates_strategy()) {
        let expected: Vec<ModelVariant> = candidates
            .iter()
            .filter(|v| v.onnx_available)
            .cloned()
            .collect();

        let listing = list_models(None, &candidates);

        prop_assert_eq!(
            &listing.available,
            &expected,
            "available が onnx_available==true の集合と一致しない: input={:?}",
            candidates
        );
    }

    /// Property 24（excluded == onnx_available==false の集合）:
    /// excluded は onnx_available==false の部分列と完全一致する。
    #[test]
    fn excluded_equals_onnx_false_subset(candidates in candidates_strategy()) {
        let expected: Vec<ModelVariant> = candidates
            .iter()
            .filter(|v| !v.onnx_available)
            .cloned()
            .collect();

        let listing = list_models(None, &candidates);

        prop_assert_eq!(
            &listing.excluded,
            &expected,
            "excluded が onnx_available==false の集合と一致しない: input={:?}",
            candidates
        );
    }

    /// Property 24（分割の網羅性・整合性）: available と excluded は
    /// 入力候補全体を過不足なく覆う。件数の和が入力件数に等しく、
    /// available は全て onnx_available==true、excluded は全て false。
    #[test]
    fn partition_covers_all_candidates(candidates in candidates_strategy()) {
        let listing = list_models(None, &candidates);

        // 件数の和が入力全体と一致（欠落・重複がない）。
        prop_assert_eq!(
            listing.available.len() + listing.excluded.len(),
            candidates.len(),
            "分割の件数和が入力と一致しない"
        );

        // available は全て ONNX を持つ。
        prop_assert!(
            listing.available.iter().all(|v| v.onnx_available),
            "available に onnx_available==false が混入"
        );
        // excluded は全て ONNX を持たない。
        prop_assert!(
            listing.excluded.iter().all(|v| !v.onnx_available),
            "excluded に onnx_available==true が混入"
        );

        // 連結（available ++ excluded）が入力を並べ替えた集合として一致する。
        // list_models は入力順を保ちつつ true を先に、false を後に振り分ける
        // ため、入力を安定分割した列と一致する。
        let mut expected: Vec<ModelVariant> =
            candidates.iter().filter(|v| v.onnx_available).cloned().collect();
        expected.extend(candidates.iter().filter(|v| !v.onnx_available).cloned());

        let mut actual = listing.available.clone();
        actual.extend(listing.excluded.clone());

        prop_assert_eq!(
            &actual,
            &expected,
            "available ++ excluded が入力の安定分割と一致しない: input={:?}",
            candidates
        );
    }

    /// Property 24（全て ONNX の場合）: 候補が全て onnx_available==true の
    /// とき、excluded は空で available は入力全体と一致する。
    #[test]
    fn all_onnx_true_yields_empty_excluded(candidates in candidates_strategy()) {
        let all_true: Vec<ModelVariant> = candidates
            .into_iter()
            .map(|mut v| {
                v.onnx_available = true;
                v
            })
            .collect();

        let listing = list_models(None, &all_true);

        prop_assert!(listing.excluded.is_empty(), "全 ONNX なのに excluded が非空");
        prop_assert_eq!(&listing.available, &all_true, "available が入力全体と一致しない");
    }

    /// Property 24（全て非 ONNX の場合）: 候補が全て onnx_available==false の
    /// とき、available は空で excluded は入力全体と一致する（要件 15.5）。
    #[test]
    fn all_onnx_false_yields_empty_available(candidates in candidates_strategy()) {
        let all_false: Vec<ModelVariant> = candidates
            .into_iter()
            .map(|mut v| {
                v.onnx_available = false;
                v
            })
            .collect();

        let listing = list_models(None, &all_false);

        prop_assert!(listing.available.is_empty(), "全非 ONNX なのに available が非空");
        prop_assert_eq!(&listing.excluded, &all_false, "excluded が入力全体と一致しない");
    }
}

/// 代表的な固定ケース: Remote/Local 混在・ONNX 有無混在で
/// 分割が仕様どおりであることを明示的に確認する。
#[test]
fn representative_fixed_case() {
    let candidates = vec![
        ModelVariant {
            id: "wd14-vit".to_string(),
            display_name: "WD14 ViT".to_string(),
            family: ModelFamily::Wd14,
            location: ModelLocation::Remote("SmilingWolf/wd-vit-tagger-v3".to_string()),
            onnx_available: true,
        },
        ModelVariant {
            id: "deepdanbooru".to_string(),
            display_name: "DeepDanbooru (TF only)".to_string(),
            family: ModelFamily::MlDanbooru,
            location: ModelLocation::Remote("repo/deepdanbooru".to_string()),
            onnx_available: false,
        },
        ModelVariant {
            id: "local-a".to_string(),
            display_name: "Local A".to_string(),
            family: ModelFamily::Local,
            location: ModelLocation::Local("/models/a/model.onnx".to_string()),
            onnx_available: true,
        },
    ];

    let listing = list_models(None, &candidates);

    let available_ids: Vec<&str> = listing.available.iter().map(|v| v.id.as_str()).collect();
    let excluded_ids: Vec<&str> = listing.excluded.iter().map(|v| v.id.as_str()).collect();

    assert_eq!(available_ids, vec!["wd14-vit", "local-a"]);
    assert_eq!(excluded_ids, vec!["deepdanbooru"]);
}
