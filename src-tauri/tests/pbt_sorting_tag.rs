// Feature: tag-editor, Property 18: タグによる振分先の一意決定
//
// Validates: Requirements 10.1, 10.2, 10.3, 10.4
//
// *任意の* 画像タグ集合と判定タグ集合について、その画像が「含む」振分先に分類される
// ことは、画像タグと判定タグ（前後トリムした完全一致）の交差が非空であることと同値で
// ある。Tag_File が存在しない画像は空集合として「含まない」に分類され、各画像はちょうど
// 1 つの振分先へ分類される。
//
// 本テストは統合テスト（tests/）として
// tag_editor_core::logic::sorting::{decide_tag_destination, TagDestination}
// を外部から検証する。モジュール本体は編集しない。

use std::collections::HashSet;

use proptest::collection::vec;
use proptest::prelude::*;

use tag_editor_core::logic::sorting::{decide_tag_destination, TagDestination};

/// テスト側で独立に定義するマッチ判定。
///
/// 要件 10.1 の規則「前後トリムした完全一致（大文字小文字は区別する）」を素直に
/// 再実装する。実装の内部ロジックには依存しない。
///
/// - 画像タグ・判定タグそれぞれをトリムする。
/// - トリム後に空になったトークンは判定に寄与しない（空集合相当）ため除外する。
/// - 交差が非空なら「含む」（true）、そうでなければ「含まない」（false）。
///
/// 大小区別のため `to_lowercase` 等の正規化は行わない。
fn reference_contains(image_tags: &[String], judge_tags: &[String]) -> bool {
    let judge_set: HashSet<&str> = judge_tags
        .iter()
        .map(|t| t.trim())
        .filter(|t| !t.is_empty())
        .collect();

    image_tags
        .iter()
        .map(|t| t.trim())
        .filter(|t| !t.is_empty())
        .any(|t| judge_set.contains(t))
}

// --- ジェネレータ ---------------------------------------------------------

/// タグ文字列ジェネレータ。
///
/// 小さな語彙から選んだ本体に、ランダムな前後空白・大文字小文字化を施す。
/// - 語彙を小さく保つことで image_tags と judge_tags が実際に重なる確率を高め、
///   「含む」「含まない」双方の分岐を有意に踏ませる。
/// - 前後空白を混ぜることでトリム完全一致が効いていることを検証する。
/// - 大文字化フラグにより「トリム後に本体は同じだが大小が異なる」ケースを生成し、
///   大小区別（case-sensitive）を検証する。
/// - マルチバイト語彙（猫・空）を含める。
/// - 空文字列本体を含めることでトリム後空になり得るケースを covered にする。
fn tag_strategy() -> impl Strategy<Value = String> {
    let body = prop_oneof![
        Just("cat"),
        Just("dog"),
        Just("sky"),
        Just("night"),
        Just("dogface"), // 部分一致では一致しないことを踏ませる
        Just("猫"),
        Just("空"),
        Just(""), // トリム後に空になり得る（空白のみタグ相当）
    ];
    (
        body,
        // 大文字化するか（大小差ケースを生成）。
        any::<bool>(),
        // 前後の空白パターン（トリム差ケースを生成）。
        prop_oneof![Just(""), Just(" "), Just("  "), Just("\t"), Just(" \t ")],
        prop_oneof![Just(""), Just(" "), Just("\t"), Just("  ")],
    )
        .prop_map(|(body, upper, lead, trail)| {
            let core = if upper {
                body.to_uppercase()
            } else {
                body.to_string()
            };
            format!("{lead}{core}{trail}")
        })
}

/// タグ列ジェネレータ（空集合を含む 0..=6 件）。重複も自然に生じ得る。
/// 0 件は「Tag_File 無し」＝空集合（要件 10.4）を表す。
fn tag_vec_strategy() -> impl Strategy<Value = Vec<String>> {
    vec(tag_strategy(), 0..=6)
}

// --- プロパティ -----------------------------------------------------------

proptest! {
    // 最小 100 ケース（下限を明示。既定 256 で実行）。
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// Property 18 本体: decide_tag_destination の結果は独立参照実装と常に一致する。
    ///
    /// 「Contains であること」⇔「トリム完全一致の交差が非空であること」を検証する。
    /// image_tags は空（Tag_File 無し）・重複・前後空白・大小混在・マルチバイトを
    /// 含み得る。judge_tags も同語彙から生成されるため実際に重なるケースが covered。
    #[test]
    fn tag_destination_equivalent_to_reference(
        image_tags in tag_vec_strategy(),
        judge_tags in tag_vec_strategy(),
    ) {
        let actual = decide_tag_destination(&image_tags, &judge_tags);
        let expected = if reference_contains(&image_tags, &judge_tags) {
            TagDestination::Contains
        } else {
            TagDestination::NotContains
        };
        prop_assert_eq!(
            actual,
            expected,
            "image_tags={:?} judge_tags={:?}",
            image_tags,
            judge_tags
        );
    }

    /// 各画像はちょうど 1 つの振分先へ分類される（要件 10 / Property 18）。
    ///
    /// TagDestination は Contains / NotContains の二値であり、結果は必ずそのいずれか
    /// ちょうど一方に一致する（両方には一致しない）。
    #[test]
    fn classification_is_exactly_one_destination(
        image_tags in tag_vec_strategy(),
        judge_tags in tag_vec_strategy(),
    ) {
        let d = decide_tag_destination(&image_tags, &judge_tags);
        let is_contains = d == TagDestination::Contains;
        let is_not_contains = d == TagDestination::NotContains;
        // ちょうど 1 つ（排他的論理和が真）。
        prop_assert!(is_contains ^ is_not_contains);
    }

    /// 要件 10.4: Tag_File 無し（空スライス）は常に「含まない」。
    ///
    /// 判定タグの内容に関わらず、画像タグが空集合なら交差は必ず空 → NotContains。
    #[test]
    fn missing_tag_file_is_always_not_contains(judge_tags in tag_vec_strategy()) {
        let empty: Vec<String> = Vec::new();
        prop_assert_eq!(
            decide_tag_destination(&empty, &judge_tags),
            TagDestination::NotContains
        );
    }

    /// 判定タグが実質空（空 or 全て空白のみ）なら常に「含まない」。
    ///
    /// 交差の相手がいないため、画像タグの内容に関わらず NotContains。
    #[test]
    fn empty_judge_tags_is_always_not_contains(image_tags in tag_vec_strategy()) {
        // 空スライス。
        prop_assert_eq!(
            decide_tag_destination(&image_tags, &Vec::<String>::new()),
            TagDestination::NotContains
        );
        // 空白のみの判定タグ（トリム後空）。
        let blanks = vec!["  ".to_string(), "\t".to_string(), " \t ".to_string()];
        prop_assert_eq!(
            decide_tag_destination(&image_tags, &blanks),
            TagDestination::NotContains
        );
    }

    /// 大小区別（case-sensitive）: 本体が同じでも大小が異なれば一致しない（要件 10.1）。
    ///
    /// judge にトリム済み小文字語彙、image に同語彙の大文字版のみを与えると、
    /// 大小無視なら一致するが、トリムのみ完全一致では交差せず NotContains となる。
    #[test]
    fn case_difference_does_not_match(
        // 大文字にしても意味のある英字語彙のみ。
        bodies in vec(prop_oneof![Just("cat"), Just("dog"), Just("sky"), Just("night")], 1..=4),
    ) {
        let judge: Vec<String> = bodies.iter().map(|b| b.to_string()).collect();
        let image: Vec<String> = bodies.iter().map(|b| b.to_uppercase()).collect();
        prop_assert_eq!(
            decide_tag_destination(&image, &judge),
            TagDestination::NotContains
        );
    }
}
