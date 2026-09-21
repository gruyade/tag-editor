// Feature: tag-editor, Property 3: サムネイルサイズのクランプ
//
// Validates: Requirements 1.4
//
// *任意の* 整数サイズ入力について、適用される表示サイズは
// [MIN_THUMBNAIL_SIZE, MAX_THUMBNAIL_SIZE] = [64, 512] の範囲内に収まり、
// 入力が既に範囲内であればその値は変化しない。
//
// 本テストは統合テスト（tests/）として
// tag_editor_core::services::file_service::clamp_thumbnail_size を外部から
// 検証する。モジュール本体は編集しない。

use proptest::prelude::*;

use tag_editor_core::services::file_service::{
    clamp_thumbnail_size, MAX_THUMBNAIL_SIZE, MIN_THUMBNAIL_SIZE,
};

/// サイズ入力ジェネレータ。全域 (0..=u32::MAX) を一様にサンプリングしつつ、
/// 境界値 (0, 63, 64, 512, 513, u32::MAX) を明示的に密に混ぜる。
fn size_strategy() -> impl Strategy<Value = u32> {
    prop_oneof![
        // 全域一様。
        any::<u32>(),
        // 範囲付近を密にサンプリング（境界検出を確実にする）。
        (0_u32..=1024_u32),
        // 境界値そのものを確定的に混ぜる。
        Just(0_u32),
        Just(MIN_THUMBNAIL_SIZE - 1), // 63
        Just(MIN_THUMBNAIL_SIZE),     // 64
        Just(MAX_THUMBNAIL_SIZE),     // 512
        Just(MAX_THUMBNAIL_SIZE + 1), // 513
        Just(u32::MAX),
    ]
}

proptest! {
    // 最小 100 ケースを満たすよう明示（既定より十分多い）。
    #![proptest_config(ProptestConfig::with_cases(512))]

    /// Property 3 本体: 結果は必ず [64, 512] に収まり、
    /// - 入力が範囲内ならそのまま、
    /// - 下限未満なら 64、
    /// - 上限超なら 512、
    /// に一致する。
    #[test]
    fn clamp_stays_in_range_and_preserves_in_range_values(size in size_strategy()) {
        let result = clamp_thumbnail_size(size);

        // 結果は常に許容範囲内。
        prop_assert!(
            (MIN_THUMBNAIL_SIZE..=MAX_THUMBNAIL_SIZE).contains(&result),
            "結果 {} が範囲 [{}, {}] 外 (入力 {})",
            result,
            MIN_THUMBNAIL_SIZE,
            MAX_THUMBNAIL_SIZE,
            size
        );

        if (MIN_THUMBNAIL_SIZE..=MAX_THUMBNAIL_SIZE).contains(&size) {
            // 範囲内の入力は不変。
            prop_assert_eq!(result, size, "範囲内入力 {} が変化した", size);
        } else if size < MIN_THUMBNAIL_SIZE {
            // 下限未満は下限にクランプ。
            prop_assert_eq!(result, MIN_THUMBNAIL_SIZE, "下限未満入力 {} が下限に丸められない", size);
        } else {
            // 上限超は上限にクランプ。
            prop_assert_eq!(result, MAX_THUMBNAIL_SIZE, "上限超入力 {} が上限に丸められない", size);
        }
    }
}

// 境界値を確定的に固定検証する例示テスト（プロパティを補完）。
#[test]
fn boundary_sizes_clamp_as_expected() {
    // 下限未満。
    assert_eq!(clamp_thumbnail_size(0), MIN_THUMBNAIL_SIZE);
    assert_eq!(clamp_thumbnail_size(63), MIN_THUMBNAIL_SIZE);
    // 下限ちょうど。
    assert_eq!(clamp_thumbnail_size(64), 64);
    // 範囲内。
    assert_eq!(clamp_thumbnail_size(256), 256);
    // 上限ちょうど。
    assert_eq!(clamp_thumbnail_size(512), 512);
    // 上限超。
    assert_eq!(clamp_thumbnail_size(513), MAX_THUMBNAIL_SIZE);
    assert_eq!(clamp_thumbnail_size(u32::MAX), MAX_THUMBNAIL_SIZE);
}
