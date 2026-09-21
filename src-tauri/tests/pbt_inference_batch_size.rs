// Feature: tag-editor, Property 26: Batch_Size の解決
//
// Validates: Requirements 17.3, 17.4, 17.5
//
// *任意の* Batch_Size 指定（未指定 `None` を含む）について、解決後の値は
// 1〜64 の範囲に収まり、未指定なら 8、範囲内の指定値はそのまま用いられる。
//
// 本テストは統合テスト（tests/）として
// tag_editor_core::logic::inference_aux::resolve_batch_size を外部から検証する。
// モジュール本体は編集しない。

use proptest::prelude::*;

use tag_editor_core::logic::inference_aux::resolve_batch_size;

/// 仕様上の定数。実装（inference_aux モジュール内の定数）には依存せず、
/// テスト側で独立に再宣言する。
const MIN: u32 = 1;
const MAX: u32 = 64;
const DEFAULT: u32 = 8;

/// 参照実装。仕様を素直に表現する。
/// - 未指定なら既定値 8。
/// - 指定値は 1〜64 へ丸める（clamp）。
fn reference_resolve(requested: Option<u32>) -> u32 {
    match requested {
        None => DEFAULT,
        Some(v) => v.clamp(MIN, MAX),
    }
}

/// `Option<u32>` ジェネレータ。
///
/// - `None` と `Some(_)` の両方を含む。
/// - `Some` の中身は u32 全域一様（巨大値も踏む）に加え、
///   境界近傍（0, 1, 64, 65）を密にサンプリングして判定切り替えを高確率でカバー。
fn requested_strategy() -> impl Strategy<Value = Option<u32>> {
    let some_value = prop_oneof![
        // 全域一様。
        any::<u32>(),
        // 下限・上限境界の近傍を密にサンプリング。
        (0u32..=4),
        (MAX - 3..=MAX + 3),
        // 極値と明示的な代表点。
        Just(0u32),
        Just(1u32),
        Just(64u32),
        Just(65u32),
        Just(u32::MAX),
    ];
    prop_oneof![
        1 => Just(None),
        4 => some_value.prop_map(Some),
    ]
}

proptest! {
    // 最小 100 ケースを満たすよう明示（既定より十分多い）。
    #![proptest_config(ProptestConfig::with_cases(1024))]

    /// Property 26 本体: 解決結果は仕様（参照実装）と常に一致し、
    /// かつ常に 1〜64 の範囲に収まる。
    #[test]
    fn resolve_matches_reference_and_in_range(requested in requested_strategy()) {
        let result = resolve_batch_size(requested);

        // 常に 1〜64 の範囲に収まる。
        prop_assert!(
            (MIN..=MAX).contains(&result),
            "result={} out of range for requested={:?}",
            result,
            requested
        );

        // 参照実装と一致する。
        prop_assert_eq!(
            result,
            reference_resolve(requested),
            "requested={:?}",
            requested
        );

        // 個別条件の明示的検証。
        match requested {
            None => prop_assert_eq!(result, DEFAULT),
            Some(v) if v < MIN => prop_assert_eq!(result, MIN),
            Some(v) if v > MAX => prop_assert_eq!(result, MAX),
            Some(v) => prop_assert_eq!(result, v),
        }
    }
}

// 境界・極値を確定的に固定検証する例示テスト（プロパティを補完）。
#[test]
fn boundary_and_extreme_values() {
    // 未指定なら既定値 8。
    assert_eq!(resolve_batch_size(None), 8);
    // 範囲内はそのまま。
    assert_eq!(resolve_batch_size(Some(1)), 1);
    assert_eq!(resolve_batch_size(Some(8)), 8);
    assert_eq!(resolve_batch_size(Some(64)), 64);
    // 下限未満は 1 に丸め（0 => 1）。
    assert_eq!(resolve_batch_size(Some(0)), 1);
    // 上限超過は 64 に丸め（65 => 64）。
    assert_eq!(resolve_batch_size(Some(65)), 64);
    assert_eq!(resolve_batch_size(Some(u32::MAX)), 64);
}
