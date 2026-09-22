// Feature: tag-editor, Property 5: タグ追加の包含性・非重複・冪等性
//
// Validates: Requirements 3.1, 3.2, 3.5, 12.1, 12.2, 12.3
//
// *任意の* 既存タグ列と追加タグ集合について、一括追加後の Tag_File は
// - 追加タグ（正規化キーで非空のもの）をすべて含み（包含性）、
// - 正規化キー（前後トリム＋大文字小文字無視）が同一のタグを重複して増やさず（非重複）、
// - 同じ追加をもう一度適用しても結果は変化しない（冪等性）。
//
// 本テストは統合テスト（tests/）として tag_editor_core::logic::tag_ops::add_tags を
// 外部から検証する。モジュール本体は編集しない。

use proptest::collection::vec;
use proptest::prelude::*;

use tag_editor_core::logic::tag_ops::add_tags;

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
/// - 語彙を小さく保つことで existing と to_add の正規化キーが実際に重なる確率を
///   高め、重複スキップ分岐（要件 3.2 / 非重複）を有意に踏ませる。
/// - 前後空白・大文字混在を混ぜることで、正規化（トリム＋case-insensitive）が
///   効いていることを検証する。
/// - マルチバイト（日本語）本体を含め、非 ASCII でも正規化・一致判定が壊れない
///   ことを確認する。
/// - 空／空白のみ本体を含め、空トークンが追加されないこと（設計「空トークンは無視」）
///   を踏ませる。
fn tag_strategy() -> impl Strategy<Value = String> {
    let body = prop_oneof![
        Just("1girl"),
        Just("solo"),
        Just("long hair"),
        Just("smile"),
        Just("笑顔"), // マルチバイト
        Just("猫耳"), // マルチバイト
        Just(""),     // トリム後に空になり得る（空白のみタグ相当）
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

/// 追加タグ集合（to_add）ジェネレータ（空集合を含む 0..=6 件）。
///
/// ユーザー指定の追加集合を模す。空／空白のみ・重複・大文字混在・マルチバイトを
/// そのまま含み得る（add_tags 側で空トークン無視・重複畳み込みが行われる）。
fn to_add_vec_strategy() -> impl Strategy<Value = Vec<String>> {
    vec(tag_strategy(), 0..=6)
}

/// 既存タグ列（existing）ジェネレータ（空集合を含む 0..=6 件）。
///
/// existing は実運用では split_tokens を通した Tag_File の内容であり、空トークンを
/// 含まず正規化キーが重複しない「整形済み」の列となる。add_tags は existing を
/// 原文のまま保持する契約のため、テストでもこの前提を満たす列のみを生成する。
/// - 正規化キーが空になる本体（空／空白のみ）を除外。
/// - 同一正規化キーの重複を先勝ちで畳み込む。
/// 前後空白・大文字混在・マルチバイトは保持し、正規化が効くことは維持して検証する。
fn existing_vec_strategy() -> impl Strategy<Value = Vec<String>> {
    vec(tag_strategy(), 0..=6).prop_map(|raw| {
        let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
        raw.into_iter()
            .filter(|t| {
                let key = ref_normalize(t);
                !key.is_empty() && seen.insert(key)
            })
            .collect()
    })
}

// --- プロパティ -----------------------------------------------------------

proptest! {
    // 最小 100 ケース（既定 256 で下限を満たす）。
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// Property 5 本体: add_tags の結果が包含性・非重複・冪等性を満たす。
    ///
    /// existing / to_add はそれぞれ空集合・重複・大文字混在・前後空白・マルチバイトを
    /// 含み得る。両者は同一語彙から生成されるため、追加タグが既存に存在する
    /// （重複スキップ）ケースも covered となる。
    #[test]
    fn add_tags_inclusive_non_duplicating_idempotent(
        existing in existing_vec_strategy(),
        to_add in to_add_vec_strategy(),
    ) {
        let result = add_tags(&existing, &to_add);

        // 参照: 結果の正規化キー列。
        let result_keys: Vec<String> = result.iter().map(|t| ref_normalize(t)).collect();

        // (a) 包含性 (要件 3.1, 12.1): to_add のうち正規化キーが非空のものは、
        //     すべて結果に（正規化キーで）含まれる。
        for t in &to_add {
            let key = ref_normalize(t);
            if !key.is_empty() {
                prop_assert!(
                    result_keys.iter().any(|k| *k == key),
                    "added tag missing: {:?} (key={:?}) existing={:?} to_add={:?} result={:?}",
                    t, key, existing, to_add, result
                );
            }
        }

        // (b) 既存タグの保存 (要件 3.5, 12.3): existing の各要素は結果に
        //     （正規化キーで）そのまま存在する。追加は既存内容を維持する。
        for t in &existing {
            let key = ref_normalize(t);
            prop_assert!(
                result_keys.iter().any(|k| *k == key),
                "existing tag lost: {:?} (key={:?}) existing={:?} to_add={:?} result={:?}",
                t, key, existing, to_add, result
            );
        }

        // (c) 非重複 (要件 3.2, 12.2): 結果内に正規化キーが同一のタグは高々1つ。
        //     また空トークン（正規化キーが空）は結果に含まれない。
        let mut seen: std::collections::HashSet<&String> = std::collections::HashSet::new();
        for k in &result_keys {
            prop_assert!(!k.is_empty(), "empty-key tag in result: result={:?}", result);
            prop_assert!(
                seen.insert(k),
                "duplicate normalized key in result: {:?} result={:?}",
                k, result
            );
        }

        // (d) 冪等性 (要件 3.2 / 冪等): 同じ追加をもう一度適用しても結果は不変。
        let twice = add_tags(&result, &to_add);
        prop_assert_eq!(
            &twice,
            &result,
            "not idempotent: existing={:?} to_add={:?} once={:?} twice={:?}",
            existing, to_add, result, twice
        );
    }
}
