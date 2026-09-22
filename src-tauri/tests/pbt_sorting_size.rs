// Feature: tag-editor, Property 16: 画像サイズ振分先の一意決定
//
// Validates: Requirements 9.3
//
// 対象: `tag_editor_core::logic::sorting::{decide_size_destination,
//        orientation_of, Orientation, LongSideBand, SizeDestination}`。
//
// プロパティ本文:
//   任意の幅・高さ・閾値について、振分先は「向き（幅 ≧ 高さを横長、
//   幅 < 高さを縦長）」と「長辺(= max(幅, 高さ))が閾値以上か未満か」の
//   組み合わせにより、必ずちょうど 1 つに一意決定される。
//
// ジェネレータ制約（本文が前提とする入力空間へ最小限に絞り、それぞれ正当化）:
//   - width / height（u32）は 0 と大小逆転の両方を含む広い範囲から生成する。
//       向きの境界は width == height（横長扱い）。この境界を確実に踏むため、
//       等値ケースを prop_oneof で明示的に混ぜる。0 は画像サイズの下限境界。
//   - threshold（u32）は 1..=100_000 から生成する。
//       is_valid_long_side_threshold の妥当範囲（要件 9.2）に一致させ、
//       境界 1 / 100_000 を明示的に含める。呼び出し側は妥当な閾値を渡す前提
//       （decide_size_destination の doc）のため、この範囲に絞ることが正当。
//       長辺 == 閾値（AtOrAbove 側の境界）を確実に踏むため、閾値を長辺に
//       一致させるケースも別途混ぜる。

use proptest::prelude::*;
use tag_editor_core::logic::sorting::{
    decide_size_destination, orientation_of, LongSideBand, Orientation, SizeDestination,
};

/// 幅・高さのジェネレータ。0 と等値（正方形）境界を明示的にカバーする。
fn dim_strategy() -> impl Strategy<Value = u32> {
    prop_oneof![
        Just(0_u32),
        Just(1_u32),
        0_u32..=10_000,
        // u32 の広い範囲も一部含める（長辺計算のオーバーフロー非依存を確認）。
        0_u32..=u32::MAX,
    ]
}

/// 閾値のジェネレータ。妥当範囲 1..=100_000 と境界をカバーする。
fn threshold_strategy() -> impl Strategy<Value = u32> {
    prop_oneof![Just(1_u32), Just(100_000_u32), 1_u32..=100_000,]
}

/// 4 つの宛先を列挙する（結果がこのちょうど 1 つに一致することの確認に用いる）。
fn all_destinations() -> [SizeDestination; 4] {
    [
        SizeDestination {
            orientation: Orientation::Landscape,
            band: LongSideBand::AtOrAbove,
        },
        SizeDestination {
            orientation: Orientation::Landscape,
            band: LongSideBand::Below,
        },
        SizeDestination {
            orientation: Orientation::Portrait,
            band: LongSideBand::AtOrAbove,
        },
        SizeDestination {
            orientation: Orientation::Portrait,
            band: LongSideBand::Below,
        },
    ]
}

/// 幅・高さ・閾値から期待される向きと区分を独立に計算し、
/// decide_size_destination の結果と照合する共通検証。
fn check(width: u32, height: u32, threshold: u32) -> Result<(), TestCaseError> {
    let result = decide_size_destination(width, height, threshold);

    // --- 期待値を実装とは独立に計算する ---
    // 向き: width >= height なら横長、そうでなければ縦長。
    let expected_orientation = if width >= height {
        Orientation::Landscape
    } else {
        Orientation::Portrait
    };
    // 長辺 = max(width, height)。閾値以上なら AtOrAbove、未満なら Below。
    let long_side = if width >= height { width } else { height };
    let expected_band = if long_side >= threshold {
        LongSideBand::AtOrAbove
    } else {
        LongSideBand::Below
    };

    // orientation_of 単体も同じ向きを返す（合成の一貫性）。
    prop_assert_eq!(
        orientation_of(width, height),
        expected_orientation,
        "orientation_of が期待向きと不一致: w={}, h={}",
        width,
        height
    );

    // 向きの一致。
    prop_assert_eq!(
        result.orientation,
        expected_orientation,
        "向きが不一致: w={}, h={}",
        width,
        height
    );
    // 区分の一致。
    prop_assert_eq!(
        result.band,
        expected_band,
        "長辺区分が不一致: w={}, h={}, threshold={}, long_side={}",
        width,
        height,
        threshold,
        long_side
    );

    // --- 一意性: 結果は 4 宛先のちょうど 1 つに一致する ---
    let matches = all_destinations().iter().filter(|d| **d == result).count();
    prop_assert_eq!(
        matches,
        1,
        "結果が 4 宛先のちょうど 1 つに一致しない (matches={}): {:?}",
        matches,
        result
    );

    // --- 決定性: 同一入力の再呼び出しは同一結果 ---
    let again = decide_size_destination(width, height, threshold);
    prop_assert_eq!(
        result,
        again,
        "再呼び出しで結果が変化した: w={}, h={}, threshold={}",
        width,
        height,
        threshold
    );

    Ok(())
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(500))]

    /// 任意入力での一意決定・向き・区分・決定性を検証する。
    #[test]
    fn size_destination_is_uniquely_determined(
        width in dim_strategy(),
        height in dim_strategy(),
        threshold in threshold_strategy(),
    ) {
        check(width, height, threshold)?;
    }

    /// 正方形（width == height）を確実に踏むケース。向きは横長になる。
    #[test]
    fn square_is_landscape(
        side in dim_strategy(),
        threshold in threshold_strategy(),
    ) {
        let result = decide_size_destination(side, side, threshold);
        prop_assert_eq!(
            result.orientation,
            Orientation::Landscape,
            "正方形が横長にならない: side={}",
            side
        );
        check(side, side, threshold)?;
    }

    /// 長辺 == 閾値の境界を確実に踏むケース。AtOrAbove 側に分類される。
    #[test]
    fn long_side_equal_to_threshold_is_at_or_above(
        threshold in threshold_strategy(),
        portrait in any::<bool>(),
        short in 0_u32..=100_000,
    ) {
        // 長辺を threshold に一致させる。短辺は長辺以下に丸める。
        let long = threshold;
        let short = short.min(long);
        let (w, h) = if portrait { (short, long) } else { (long, short) };

        let result = decide_size_destination(w, h, threshold);
        prop_assert_eq!(
            result.band,
            LongSideBand::AtOrAbove,
            "長辺 == 閾値 が AtOrAbove にならない: w={}, h={}, threshold={}",
            w,
            h,
            threshold
        );
        check(w, h, threshold)?;
    }
}
