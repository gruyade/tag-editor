// Feature: tag-editor, Property 9: Booru 変換
//
// Property 9: 任意のタグ文字列について、Booru 変換後の文字列には
// アンダースコアが存在せず、エスケープされていない生の括弧 `(` `)`
// が存在しない。
//
// Validates: Requirements 5.1, 5.2

use proptest::prelude::*;
use tag_editor_core::logic::tag_format::to_booru;

/// エスケープされていない生の括弧が存在しないことを判定する。
///
/// `to_booru` は `(` / `)` を必ず直前のバックスラッシュ付き（`\(` / `\)`）
/// へ変換する。変換結果を走査し、直前が `\` でない `(` または `)` が
/// 1 つでも現れたら「生の括弧が残っている」とみなす。
fn has_unescaped_paren(s: &str) -> bool {
    let mut prev_backslash = false;
    for ch in s.chars() {
        match ch {
            '(' | ')' if !prev_backslash => return true,
            _ => {}
        }
        // 次の文字の判定用に「直前がバックスラッシュか」を更新する。
        prev_backslash = ch == '\\';
    }
    false
}

/// 入力空間を網羅的に覆う生成器。
///
/// 空文字列・全空白・アンダースコア・括弧・大文字小文字混在・
/// 非 ASCII / マルチバイトを含めるため、以下の文字集合から任意長
/// （0 文字を含む）の文字列を生成する。
fn tag_strategy() -> impl Strategy<Value = String> {
    // 変換対象・境界値・多様な文字種を意図的に混在させた文字プール。
    let chars = prop::sample::select(vec![
        '_',  // アンダースコア（→ スペース）
        '(',  // 開き括弧（→ エスケープ）
        ')',  // 閉じ括弧（→ エスケープ）
        '\\', // バックスラッシュ（エスケープ判定の交絡因子）
        ' ',  // 空白
        '\t', // タブ（全空白ケース用）
        'a', 'Z', 'M', // 大文字小文字混在の ASCII
        '0', // 数字
        ':', '.', // 信頼度トークン様の記号
        '猫', // 非 ASCII（マルチバイト）
        '髪', 'あ', '한', '🎨', // さらに多様なマルチバイト
    ]);
    prop::collection::vec(chars, 0..40).prop_map(|v| v.into_iter().collect())
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// 任意のタグ文字列について、Booru 変換後にアンダースコアが残らず、
    /// エスケープされていない生の括弧も残らない。
    #[test]
    fn booru_conversion_removes_underscore_and_escapes_parens(input in tag_strategy()) {
        let converted = to_booru(&input);

        // アンダースコアは 1 つも残らない（要件 5.1）。
        prop_assert!(
            !converted.contains('_'),
            "アンダースコアが残存: input={:?} converted={:?}",
            input,
            converted
        );

        // エスケープされていない生の括弧が残らない（要件 5.2）。
        prop_assert!(
            !has_unescaped_paren(&converted),
            "エスケープされていない括弧が残存: input={:?} converted={:?}",
            input,
            converted
        );
    }

    /// 明示的な境界ケース: 全空白入力でも上記の性質が保たれる。
    #[test]
    fn booru_conversion_holds_for_whitespace_only(
        input in prop::collection::vec(prop::sample::select(vec![' ', '\t', '_']), 0..20)
            .prop_map(|v| v.into_iter().collect::<String>())
    ) {
        let converted = to_booru(&input);
        prop_assert!(!converted.contains('_'));
        prop_assert!(!has_unescaped_paren(&converted));
    }
}
