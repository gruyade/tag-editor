// Feature: tag-editor, Property 22: 信頼度閾値によるタグ採用
//
// Validates: Requirements 14.4
//
// *任意の* (タグ, 信頼度) の列と 0.0〜1.0 の閾値について、採用されるタグ集合は
// 信頼度が閾値以上（confidence >= threshold）のタグの集合とちょうど一致し、
// かつ入力順序を保存する。
//
// 本テストは統合テスト（tests/）として
// tag_editor_core::logic::inference_aux::adopt_by_threshold を外部から検証する。
// モジュール本体は編集しない。

use proptest::prelude::*;

use tag_editor_core::logic::inference_aux::adopt_by_threshold;
use tag_editor_core::Tag;

/// 0.0〜1.0 の信頼度ジェネレータ。境界 0.0 / 1.0 を明示的に混ぜる。
fn confidence_strategy() -> impl Strategy<Value = f32> {
    prop_oneof![
        // 全域一様（0.0〜1.0）。
        (0.0_f32..=1.0_f32),
        // 境界値を密にサンプリング。
        Just(0.0_f32),
        Just(1.0_f32),
    ]
}

/// タグ本体ジェネレータ。
///
/// 本プロパティは採用判定（信頼度 >= 閾値）のみを検証するため、本体は
/// 一意性や書式に依存しない。単純な英数字列で十分。
fn body_strategy() -> impl Strategy<Value = String> {
    "[a-z0-9_]{1,8}"
}

/// 信頼度付きタグのジェネレータ。
///
/// Property 22 は「信頼度付きタグ」に対して採用集合＝閾値以上集合が
/// 厳密に成立することを述べる。信頼度なしタグ（confidence == None）は
/// 閾値と比較不能で常に不採用となり本プロパティの対象外のため、ここでは
/// 生成しない（信頼度なしの扱いは inference_aux 内の単体テストで検証済み）。
fn tag_strategy() -> impl Strategy<Value = Tag> {
    (body_strategy(), confidence_strategy())
        .prop_map(|(body, conf)| Tag::with_confidence(body, conf))
}

/// 閾値ジェネレータ。0.0〜1.0 の全域と境界 0.0 / 1.0 をカバーする。
fn threshold_strategy() -> impl Strategy<Value = f32> {
    prop_oneof![
        (0.0_f32..=1.0_f32),
        Just(0.0_f32),
        Just(1.0_f32),
    ]
}

/// 参照実装。仕様「信頼度が閾値以上のタグを入力順で採用」を素直に表現する。
fn reference_adopt(tags: &[Tag], threshold: f32) -> Vec<Tag> {
    tags.iter()
        .filter(|t| matches!(t.confidence, Some(c) if c >= threshold))
        .cloned()
        .collect()
}

proptest! {
    // 最小 100 ケースを満たすよう明示（既定より十分多い）。
    #![proptest_config(ProptestConfig::with_cases(512))]

    /// Property 22 本体: 採用集合は信頼度が閾値以上のタグ集合とちょうど一致し、
    /// 入力順を保存する。
    #[test]
    fn adopted_equals_tags_at_or_above_threshold(
        tags in prop::collection::vec(tag_strategy(), 0..24),
        threshold in threshold_strategy(),
    ) {
        let adopted = adopt_by_threshold(&tags, threshold);
        let expected = reference_adopt(&tags, threshold);

        // 採用集合が参照実装（順序保存フィルタ）と完全一致する。
        prop_assert_eq!(&adopted, &expected, "threshold={}", threshold);

        // 採用された各タグは必ず閾値以上である。
        for tag in &adopted {
            let c = tag.confidence.expect("生成タグは信頼度を持つ");
            prop_assert!(c >= threshold, "採用タグの信頼度 {} < 閾値 {}", c, threshold);
        }

        // 入力中で閾値以上のタグは漏れなく採用される（採用件数の一致）。
        let expected_count = tags
            .iter()
            .filter(|t| matches!(t.confidence, Some(c) if c >= threshold))
            .count();
        prop_assert_eq!(adopted.len(), expected_count, "採用件数が不一致");

        // 採用列は入力列の部分列（順序保存）である。
        let mut it = tags.iter();
        for a in &adopted {
            let found = it.by_ref().any(|t| t == a);
            prop_assert!(found, "採用タグが入力順の部分列でない");
        }
    }
}

// 境界の信頼度/閾値を確定的に固定検証する例示テスト（プロパティを補完）。
#[test]
fn boundary_confidences_and_thresholds() {
    let tags = vec![
        Tag::with_confidence("zero", 0.0),
        Tag::with_confidence("half", 0.5),
        Tag::with_confidence("one", 1.0),
    ];

    // 採用結果の本体列を取り出すヘルパー。
    fn bodies(tags: Vec<Tag>) -> Vec<String> {
        tags.into_iter().map(|t| t.body).collect()
    }

    // 閾値 0.0: 全採用（0.0 >= 0.0 を含む）。
    assert_eq!(
        bodies(adopt_by_threshold(&tags, 0.0)),
        vec!["zero", "half", "one"]
    );

    // 閾値 1.0: 信頼度 1.0 のみ採用（境界は inclusive）。
    assert_eq!(bodies(adopt_by_threshold(&tags, 1.0)), vec!["one"]);

    // 閾値 0.5: 0.5 と 1.0 を採用。
    assert_eq!(
        bodies(adopt_by_threshold(&tags, 0.5)),
        vec!["half", "one"]
    );
}
