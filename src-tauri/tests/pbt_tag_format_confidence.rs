// Feature: tag-editor, Property 10: 信頼度トークンの parse/render ラウンドトリップ
//
// Validates: Requirements 5.3, 5.5, 5.6
//
// 対象: `tag_editor_core::logic::tag_format::{parse_confidence, render_tags}`
//       と `tag_editor_core::Tag` モデル。
//
// プロパティ本文:
//   任意のタグ本体と 0.0〜1.0 の信頼度について、`(tag:conf)` 形式へ描画
//   （小数第2位）したトークンを解析すると、元の本体と小数第2位に丸めた
//   信頼度が復元される。信頼度非表示で描画した文字列には信頼度が含まれない。
//
// ジェネレータ制約（プロパティ本文が前提とする「素朴なタグ本体」に基づき、
// 最小限に留めた上でそれぞれ正当化する）:
//   - 本体は前後空白を持たない。
//       parse_confidence は本体を trim して保持するため、前後空白付きの
//       本体は「元の本体」として復元されない。プロパティは trim 済みの
//       素朴な本体を対象とする（要件のタグ正規化ルールでも本体は
//       前後トリム済みで保持される）。
//   - 本体は空でない。
//       空本体は信頼度トークンとして扱われない設計であり、ラウンドトリップ
//       の対象外。素朴なタグ本体は非空。
//   - 本体は括弧 `(` `)` を含まない。
//       描画は外側を `( )` で囲むため、本体末尾の `)` は外側の閉じ括弧と
//       混同され、解析時の外側括弧剥がしを破壊する。素朴なタグ本体は
//       描画の区切り文字である括弧を含まない前提。
//   - 本体はカンマ `,` を含まない。
//       render_tags は複数タグをカンマ＋スペースで連結するため、本体中の
//       カンマはトークン境界と混同され得る。単一トークンのラウンドトリップ
//       を検証する本プロパティでは本体をカンマなしに限定する。
//   信頼度は 0.0〜1.0 の範囲（境界 0.0 / 1.0 を必ずカバー）。

use proptest::prelude::*;
use tag_editor_core::logic::tag_format::{parse_confidence, render_tags};
use tag_editor_core::Tag;

/// 素朴なタグ本体のジェネレータ。
///
/// - 括弧・カンマを含まない可視文字（非 ASCII / マルチバイトを含む）。
/// - 前後空白を持たず、空でない。
fn body_strategy() -> impl Strategy<Value = String> {
    // 制約文字（括弧・カンマ・制御文字相当の空白）を除外した文字集合から生成。
    // 非 ASCII/マルチバイトのエッジもカバーするため広めの Unicode 範囲を含める。
    let ch = prop_oneof![
        // ASCII 可視（括弧・カンマ・空白を除く）
        prop::char::range('a', 'z'),
        prop::char::range('A', 'Z'),
        prop::char::range('0', '9'),
        Just('_'),
        Just(':'),
        Just('-'),
        Just('.'),
        Just('\\'),
        // 非 ASCII / マルチバイト（日本語など）
        prop::char::range('\u{3040}', '\u{30ff}'),
        prop::char::range('\u{4e00}', '\u{9fff}'),
    ];
    prop::collection::vec(ch, 1..24).prop_filter_map(
        "本体は前後空白なし・非空",
        |chars| {
            let s: String = chars.into_iter().collect();
            // 生成物が偶然前後空白/空になる可能性はないが、防御的に trim 一致を確認。
            if s.is_empty() || s.trim() != s {
                None
            } else {
                Some(s)
            }
        },
    )
}

/// 0.0〜1.0 の信頼度ジェネレータ。境界 0.0 / 1.0 を明示的に混ぜる。
fn confidence_strategy() -> impl Strategy<Value = f32> {
    prop_oneof![
        Just(0.0_f32),
        Just(1.0_f32),
        (0.0_f32..=1.0_f32),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(300))]

    /// 信頼度表示ONで描画→解析すると、本体と小数第2位に丸めた信頼度が復元される。
    /// 信頼度表示OFFの描画には信頼度（`:` を含む `(…:…)` 形式）が現れない。
    #[test]
    fn confidence_parse_render_roundtrip(
        body in body_strategy(),
        confidence in confidence_strategy(),
    ) {
        let tag = Tag::with_confidence(body.clone(), confidence);

        // --- 信頼度表示ON: 描画→解析のラウンドトリップ ---
        let rendered_on = render_tags(std::slice::from_ref(&tag), true);

        // 単一タグは `(body:0.xx)` 形式で描画される。
        prop_assert_eq!(
            &rendered_on,
            &format!("({}:{:.2})", body, confidence),
            "render(ON) 形式が想定と不一致"
        );

        let parsed = parse_confidence(&rendered_on);

        // 本体が復元される。
        prop_assert_eq!(&parsed.body, &body, "本体が復元されない: rendered={:?}", rendered_on);

        // 信頼度は「小数第2位に丸めた値」が復元される。
        // 期待値は描画に用いた {:.2} 表記を再解釈した f32。
        let expected_conf: f32 = format!("{:.2}", confidence)
            .parse()
            .expect("2桁固定小数は必ず f32 として解釈可能");
        prop_assert_eq!(
            parsed.confidence,
            Some(expected_conf),
            "信頼度が小数第2位に丸めた値として復元されない: rendered={:?}",
            rendered_on
        );

        // --- 信頼度表示OFF: 信頼度が含まれない ---
        let rendered_off = render_tags(std::slice::from_ref(&tag), false);

        // OFF 描画は本体のみ。
        prop_assert_eq!(&rendered_off, &body, "OFF 描画が本体と不一致");

        // OFF 描画を解析しても信頼度は付かない（本体は括弧を含まないため
        // 信頼度トークンとして解釈されない）。
        let parsed_off = parse_confidence(&rendered_off);
        prop_assert_eq!(
            parsed_off.confidence,
            None,
            "OFF 描画から信頼度が復元されてしまう: rendered={:?}",
            rendered_off
        );
        prop_assert_eq!(&parsed_off.body, &body, "OFF 描画の本体が不一致");
    }
}
