// Feature: tag-editor, Property 12: タグフィルタの述語一致
//
// Validates: Requirements 6.5, 6.6, 6.7, 6.8
//
// *任意の* 画像タグ集合と包含タグ・除外タグの指定について、ある画像がフィルタ結果に
// 含まれることは「包含タグをすべて含み、かつ除外タグをいずれも含まない」ことと同値で
// あり、包含・除外がともに空なら全画像が結果に含まれる。
//
// 本テストは統合テスト（tests/）として tag_editor_core::logic::tag_stats::matches_filter
// を外部から検証する。モジュール本体は編集しない。

use proptest::collection::vec;
use proptest::prelude::*;

use tag_editor_core::logic::tag_stats::matches_filter;

/// テスト側で独立に定義する正規化。実装（tag_stats::normalize_key）と同じ規則
/// 「前後トリム＋大文字小文字無視（to_lowercase）」を再実装し、実装の内部関数には
/// 依存しない。
fn ref_normalize(tag: &str) -> String {
    tag.trim().to_lowercase()
}

/// 述語の独立参照実装。
///
/// 「include をすべて含み、かつ exclude をいずれも含まない」を、画像タグの正規化キー
/// 集合に対する包含判定として素直に表現する。include/exclude が空の場合は all() が
/// vacuously true となり、両方空なら常に真になる（要件 6.8）。
fn reference_matches(image_tags: &[String], include: &[String], exclude: &[String]) -> bool {
    let keys: std::collections::HashSet<String> =
        image_tags.iter().map(|t| ref_normalize(t)).collect();

    let all_included = include.iter().all(|t| keys.contains(&ref_normalize(t)));
    let none_excluded = exclude.iter().all(|t| !keys.contains(&ref_normalize(t)));

    all_included && none_excluded
}

// --- ジェネレータ ---------------------------------------------------------

/// タグ文字列ジェネレータ。
///
/// 小さな語彙から選んだ本体に、ランダムな前後空白・大文字小文字化を施す。
/// - 語彙を小さく保つことで include/exclude と image_tags が実際に重なる確率を高め、
///   一致・不一致・除外ヒットの各分岐を有意に踏ませる。
/// - 前後空白・大文字混在を混ぜることで、正規化（トリム＋case-insensitive）が
///   効いていることを検証する。
fn tag_strategy() -> impl Strategy<Value = String> {
    let body = prop_oneof![
        Just("cat"),
        Just("dog"),
        Just("sky"),
        Just("night"),
        Just("1girl"),
        Just(""), // トリム後に空になり得る（空白のみタグ相当）
    ];
    (
        body,
        // 大文字化するか / どの空白を前後に付けるか。
        any::<bool>(),
        prop_oneof![Just(""), Just(" "), Just("  "), Just("\t"), Just(" \t ")],
        prop_oneof![Just(""), Just(" "), Just("\t"), Just("  ")],
    )
        .prop_map(|(body, upper, lead, trail)| {
            let core = if upper { body.to_uppercase() } else { body.to_string() };
            format!("{lead}{core}{trail}")
        })
}

/// タグ列ジェネレータ（空集合を含む 0..=6 件）。重複も自然に生じ得る。
fn tag_vec_strategy() -> impl Strategy<Value = Vec<String>> {
    vec(tag_strategy(), 0..=6)
}

// --- プロパティ -----------------------------------------------------------

proptest! {
    // 最小 100 ケース（既定 256 でも可だが下限を明示）。
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// Property 12 本体: matches_filter の結果は独立参照実装と常に一致する。
    ///
    /// image_tags / include / exclude はそれぞれ空集合・重複・大文字混在・前後空白を
    /// 含み得る。include と exclude は同一語彙から生成されるため、両者が重なる
    /// （同じタグを include かつ exclude 指定する）ケースも covered となる。
    #[test]
    fn matches_filter_equivalent_to_reference(
        image_tags in tag_vec_strategy(),
        include in tag_vec_strategy(),
        exclude in tag_vec_strategy(),
    ) {
        let actual = matches_filter(&image_tags, &include, &exclude);
        let expected = reference_matches(&image_tags, &include, &exclude);
        prop_assert_eq!(
            actual,
            expected,
            "image_tags={:?} include={:?} exclude={:?}",
            image_tags,
            include,
            exclude
        );
    }

    /// 要件 6.8: include・exclude がともに空なら、画像タグの内容に関わらず常に真。
    #[test]
    fn empty_include_and_exclude_always_matches(image_tags in tag_vec_strategy()) {
        prop_assert!(matches_filter(&image_tags, &[], &[]));
    }
}
