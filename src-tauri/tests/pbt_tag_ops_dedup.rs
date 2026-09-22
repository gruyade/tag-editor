// Feature: tag-editor, Property 7: 重複除去の一意性・順序保存・冪等性
//
// Validates: Requirements 3.4
//
// *任意の* タグ列について、重複除去後は正規化キーが一意であり、各キーの初出順序が
// 保持され、もう一度重複除去を適用しても結果は変化しない。
//
// 本テストは統合テスト（tests/）として tag_editor_core::logic::tag_ops::dedup_tags を
// 外部から検証する。モジュール本体は編集しない。

use proptest::collection::vec;
use proptest::prelude::*;

use tag_editor_core::logic::tag_ops::dedup_tags;

/// テスト側で独立に定義する正規化。実装（tag_ops::normalize_key）と同じ規則
/// 「前後トリム＋大文字小文字無視（to_lowercase）」を再実装し、実装の内部関数には
/// 依存しない。
fn ref_normalize(tag: &str) -> String {
    tag.trim().to_lowercase()
}

/// 初出順の重複除去の独立参照実装。
///
/// 正規化キーが空（トリム後に空文字列）となるトークンは除去し、非空キーについては
/// 最初の出現のみを原文のまま残す。HashSet の代わりに Vec を用いた素朴な線形走査で
/// 実装し、実装ロジックとは独立にする。
fn reference_dedup(tags: &[String]) -> Vec<String> {
    let mut seen: Vec<String> = Vec::new();
    let mut result: Vec<String> = Vec::new();
    for tag in tags {
        let key = ref_normalize(tag);
        if key.is_empty() {
            continue;
        }
        if !seen.contains(&key) {
            seen.push(key);
            result.push(tag.clone());
        }
    }
    result
}

// --- ジェネレータ ---------------------------------------------------------

/// タグ文字列ジェネレータ。
///
/// 小さな語彙から選んだ本体に、ランダムな前後空白・大文字小文字化を施す。
/// - 語彙を小さく保つことで正規化キーの衝突（重複）を高確率で発生させ、除去分岐を
///   有意に踏ませる（heavy duplicates）。
/// - 前後空白・大文字混在を混ぜることで、正規化（トリム＋case-insensitive）が
///   効いていることを検証する。
/// - マルチバイト本体（日本語）と空文字（空白のみタグ相当）も語彙に含める。
fn tag_strategy() -> impl Strategy<Value = String> {
    let body = prop_oneof![
        Just("cat"),
        Just("Long Hair"),
        Just("1girl"),
        Just("solo"),
        Just("笑顔"),   // マルチバイト
        Just("赤い髪"), // マルチバイト
        Just(""),       // トリム後に空になり得る（空白のみタグ相当）
    ];
    (
        body,
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

/// タグ列ジェネレータ（空集合を含む 0..=12 件）。
///
/// 上限を大きめに取り、小さな語彙と組み合わせることで重複が多発する列を生成する。
fn tag_vec_strategy() -> impl Strategy<Value = Vec<String>> {
    vec(tag_strategy(), 0..=12)
}

// --- プロパティ -----------------------------------------------------------

proptest! {
    // 最小 100 ケース（既定 256 で下限を満たす）。
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// Property 7 本体: 一意性・順序保存・冪等性を同時に検証する。
    ///
    /// 入力タグ列は空集合・重複・大文字混在・前後空白・マルチバイトを含み得る。
    #[test]
    fn dedup_unique_ordered_idempotent(tags in tag_vec_strategy()) {
        let out = dedup_tags(&tags);

        // (1) 一意性: 出力の正規化キーは重複しない。あわせて空キーが残らないことも確認。
        let mut keys: Vec<String> = Vec::new();
        for t in &out {
            let key = ref_normalize(t);
            prop_assert!(
                !key.is_empty(),
                "空の正規化キーが残った: tags={:?} out={:?}",
                tags,
                out
            );
            prop_assert!(
                !keys.contains(&key),
                "正規化キーが重複した: key={:?} tags={:?} out={:?}",
                key,
                tags,
                out
            );
            keys.push(key);
        }

        // (2) 初出順序の保存: 独立に計算した初出順デデュープと完全一致する。
        let expected = reference_dedup(&tags);
        prop_assert_eq!(
            &out,
            &expected,
            "初出順序が保存されていない: tags={:?}",
            tags
        );

        // (3) 冪等性: もう一度適用しても結果は変化しない。
        let twice = dedup_tags(&out);
        prop_assert_eq!(
            &twice,
            &out,
            "冪等でない: tags={:?} out={:?} twice={:?}",
            tags,
            out,
            twice
        );
    }
}
