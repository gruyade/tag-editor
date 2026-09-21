// Feature: tag-editor, Property 17: 閾値の妥当性判定
//
// Validates: Requirements 9.2
//
// *任意の* 整数値について、長辺閾値が妥当であることは `1 ≤ 値 ≤ 100000` と同値。
//
// 本テストは統合テスト（tests/）として
// tag_editor_core::logic::sorting::is_valid_long_side_threshold を外部から検証する。
// モジュール本体は編集しない。

use proptest::prelude::*;

use tag_editor_core::logic::sorting::is_valid_long_side_threshold;

/// 妥当範囲の境界値。実装（sorting モジュール内の定数）には依存せず、テスト側で
/// 独立に「1 以上 100000 以下」という仕様を再宣言する。
const MIN: i64 = 1;
const MAX: i64 = 100_000;

/// 参照述語。仕様「1 ≤ 値 ≤ 100000」を素直に表現する。
fn reference_is_valid(value: i64) -> bool {
    (MIN..=MAX).contains(&value)
}

/// i64 値ジェネレータ。
///
/// - 全域一様（any::<i64>()）で負値・巨大値も踏む。
/// - 境界近傍（0, 1, 100000, 100001 など）を意図的に密に生成し、判定が切り替わる
///   境界ちょうどのケースを高確率でカバーする。
fn value_strategy() -> impl Strategy<Value = i64> {
    prop_oneof![
        // 全域一様。
        any::<i64>(),
        // 下限・上限境界の近傍を密にサンプリング。
        (MIN - 3..=MIN + 3),
        (MAX - 3..=MAX + 3),
        // 極値と明示的な代表点。
        Just(i64::MIN),
        Just(i64::MAX),
        Just(0),
        Just(-1),
        Just(1),
        Just(100_000),
        Just(100_001),
    ]
}

proptest! {
    // 最小 100 ケースを満たすよう明示（既定より十分多い）。
    #![proptest_config(ProptestConfig::with_cases(1024))]

    /// Property 17 本体: 妥当判定は仕様 `1 ≤ 値 ≤ 100000` と常に同値。
    #[test]
    fn validity_equivalent_to_range(value in value_strategy()) {
        prop_assert_eq!(
            is_valid_long_side_threshold(value),
            reference_is_valid(value),
            "value={}",
            value
        );
    }
}

// 境界・極値を確定的に固定検証する例示テスト（プロパティを補完）。
#[test]
fn boundary_and_extreme_values() {
    // 妥当。
    assert!(is_valid_long_side_threshold(1));
    assert!(is_valid_long_side_threshold(100_000));
    assert!(is_valid_long_side_threshold(512));
    // 不妥当。
    assert!(!is_valid_long_side_threshold(0));
    assert!(!is_valid_long_side_threshold(-1));
    assert!(!is_valid_long_side_threshold(100_001));
    assert!(!is_valid_long_side_threshold(i64::MIN));
    assert!(!is_valid_long_side_threshold(i64::MAX));
}
