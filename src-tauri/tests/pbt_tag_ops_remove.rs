// Feature: tag-editor, Property 6: タグ削除の完全除去
//
// Validates: Requirements 3.3
//
// *任意の* タグ列と削除タグ集合について、一括削除（remove_tags）後の Tag_File には、
// 削除対象タグ（正規化キー一致）が1つも残らない。加えて、削除で消えるのは削除対象に
// 正規化キーが一致するタグだけであり、非対象タグは順序・原文を保ったまま残る。
//
// 本テストは統合テスト（tests/）として tag_editor_core::logic::tag_ops::remove_tags を
// 外部から検証する。モジュール本体は編集しない。

use proptest::collection::vec;
use proptest::prelude::*;

use tag_editor_core::logic::tag_ops::remove_tags;

/// テスト側で独立に定義する正規化。実装（tag_ops::normalize_key）と同じ規則
/// 「前後トリム＋大文字小文字無視（to_lowercase）」を再実装し、実装の内部関数には
/// 依存しない。
fn ref_normalize(tag: &str) -> String {
    tag.trim().to_lowercase()
}

// --- ジェネレータ ---------------------------------------------------------

/// タグ文字列ジェネレータ。
///
/// 小さな語彙から選んだ本体に、ランダムな前後空白・大文字小文字化を施す。
/// - 語彙を小さく保つことで、タグ列と削除集合が実際に重なる確率を高め、
///   「削除される／削除されない」の両分岐を有意に踏ませる。
/// - 前後空白・大文字混在・マルチバイト・空文字（空白のみ）を混ぜることで、
///   正規化（トリム＋case-insensitive）が効いていることを検証する。
fn tag_strategy() -> impl Strategy<Value = String> {
    let body = prop_oneof![
        Just("cat"),
        Just("dog"),
        Just("long_hair"),
        Just("1girl"),
        Just("Solo"),
        Just("ネコ"),      // マルチバイト
        Just("大きい_耳"), // マルチバイト＋アンダースコア
        Just(""),          // トリム後に空になり得る（空白のみタグ相当）
    ];
    (
        body,
        // 大文字化するか / どの空白を前後に付けるか。
        any::<bool>(),
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

/// タグ列ジェネレータ（空集合を含む 0..=8 件）。重複も自然に生じ得る。
fn tag_vec_strategy() -> impl Strategy<Value = Vec<String>> {
    vec(tag_strategy(), 0..=8)
}

// --- プロパティ -----------------------------------------------------------

proptest! {
    // 最小 100 ケースを満たすよう 256 ケースを設定。
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// Property 6 本体: 一括削除後、削除対象タグ（正規化キー一致）は1つも残らない。
    ///
    /// tags / to_remove はそれぞれ空集合・重複・大文字混在・前後空白・マルチバイトを
    /// 含み得る。両者は同一語彙から生成されるため、実際に重なる（削除がヒットする）
    /// ケースも covered となる。
    #[test]
    fn removed_tags_never_remain(
        tags in tag_vec_strategy(),
        to_remove in tag_vec_strategy(),
    ) {
        let result = remove_tags(&tags, &to_remove);

        // 削除対象の正規化キー集合。
        let remove_keys: std::collections::HashSet<String> =
            to_remove.iter().map(|t| ref_normalize(t)).collect();

        // 完全除去: 残ったどのタグも、削除対象キーのいずれとも一致しない。
        for remaining in &result {
            let key = ref_normalize(remaining);
            prop_assert!(
                !remove_keys.contains(&key),
                "削除対象タグが残存: remaining={:?} (key={:?}) tags={:?} to_remove={:?}",
                remaining,
                key,
                tags,
                to_remove
            );
        }
    }

    /// 補完プロパティ: 削除で消えるのは削除対象キーに一致するタグのみ。
    ///
    /// 「削除対象キーに一致しない元タグ」は、順序・原文・件数を完全に保って残る。
    /// これにより「消しすぎない」ことを保証し、完全除去プロパティと合わせて
    /// remove_tags の振る舞いを両側から締める。
    #[test]
    fn non_target_tags_preserved_in_order(
        tags in tag_vec_strategy(),
        to_remove in tag_vec_strategy(),
    ) {
        let result = remove_tags(&tags, &to_remove);

        let remove_keys: std::collections::HashSet<String> =
            to_remove.iter().map(|t| ref_normalize(t)).collect();

        // 元タグから削除対象キー一致を除いた列（原文・順序保持）。
        let expected: Vec<String> = tags
            .iter()
            .filter(|t| !remove_keys.contains(&ref_normalize(t)))
            .cloned()
            .collect();

        prop_assert_eq!(
            &result,
            &expected,
            "非対象タグの保持が崩れた: tags={:?} to_remove={:?} result={:?}",
            tags,
            to_remove,
            result
        );
    }

    /// 冪等性: 一度削除した結果に同じ削除集合を再適用しても変化しない。
    #[test]
    fn remove_is_idempotent(
        tags in tag_vec_strategy(),
        to_remove in tag_vec_strategy(),
    ) {
        let once = remove_tags(&tags, &to_remove);
        let twice = remove_tags(&once, &to_remove);
        prop_assert_eq!(&once, &twice, "削除が冪等でない: once={:?} twice={:?}", once, twice);
    }
}
