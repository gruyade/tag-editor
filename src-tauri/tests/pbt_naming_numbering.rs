// Feature: tag-editor, Property 15: 連番付与の連続性・桁数・一意性
//
// Validates: Requirements 8.1, 8.2
//
// 対象: `tag_editor_core::logic::naming::{sequence_number, format_number,
//        expand_num_placeholder, apply_numbering}`。
//
// プロパティ本文:
//   任意の開始番号・桁数・件数について、ファイル名昇順で割り当てた i 番目の
//   番号は `開始番号 + i` に等しく、指定桁数でゼロ埋めされ、割り当てた番号は
//   すべて一意かつ昇順である。
//
// ジェネレータ制約（本文が前提とする入力空間へ最小限に絞り、それぞれ正当化）:
//   - start（開始番号）は u64 の広い範囲から生成するが、start + (count-1) が
//     u64 を溢れないよう上限を設ける。
//       sequence_number(start, i) = start + i は算術オーバーフローで
//       パニックしうる。実運用の連番は現実的な件数・開始番号であり、
//       オーバーフローはプロパティの対象外。境界を安全側へ制約する。
//   - width（桁数）は 0..=12 の範囲から生成し、境界 0 / 1 と大きめの桁を含む。
//       format_number は width 桁未満をゼロ埋めし、超過分は切り詰めない。
//       width=0 は「ゼロ埋めなし」の境界。上限 12 は u64 の十進最大桁数
//       （20 桁）以内かつ検証に十分な範囲。
//   - count（件数）は 1..=64 の範囲から生成し、境界 1 と複数件を含む。
//       件数 0 は割り当て対象なしで検証項目が空になるため、割り当てが
//       発生する最小 1 件以上を対象とする。

use proptest::prelude::*;
use tag_editor_core::logic::naming::{
    apply_numbering, expand_num_placeholder, format_number, sequence_number,
};

/// 割り当て件数のジェネレータ。境界 1 と複数件をカバーする。
fn count_strategy() -> impl Strategy<Value = u64> {
    prop_oneof![Just(1_u64), Just(2_u64), 1_u64..=64,]
}

/// 桁数のジェネレータ。境界 0 / 1 と大きめの桁をカバーする。
fn width_strategy() -> impl Strategy<Value = usize> {
    prop_oneof![Just(0_usize), Just(1_usize), Just(12_usize), 0_usize..=12,]
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(300))]

    /// 連続性・桁数・一意性・昇順性を検証する。
    ///
    /// - 連続性: i 番目の番号は start + i に等しい。
    /// - 桁数: 整形結果は width 桁以上で、値と一致する（ゼロ埋めのみ・切り詰めなし）。
    /// - 一意性・昇順性: 割り当てた番号列は狭義単調増加（重複なし）。
    #[test]
    fn numbering_is_consecutive_padded_and_unique(
        count in count_strategy(),
        width in width_strategy(),
        // start は start + (count-1) が u64 を溢れないよう上限を制約する。
        start in 0_u64..=1_000_000_000_000,
    ) {
        let mut numbers: Vec<u64> = Vec::with_capacity(count as usize);
        let mut formatted: Vec<String> = Vec::with_capacity(count as usize);

        for i in 0..count {
            // --- 連続性: sequence_number(start, i) == start + i ---
            let n = sequence_number(start, i);
            prop_assert_eq!(
                n,
                start + i,
                "連番が start + i と不一致: start={}, i={}",
                start,
                i
            );

            // --- 桁数: ゼロ埋め幅が width 以上、かつ値と一致 ---
            let s = format_number(n, width);

            // 少なくとも width 桁ある（width 桁未満はゼロ埋めで補われる）。
            prop_assert!(
                s.len() >= width,
                "整形結果 {:?} が width={} 桁未満",
                s,
                width
            );

            // 全文字が ASCII 数字（負号や区切りが混入しない）。
            prop_assert!(
                s.bytes().all(|b| b.is_ascii_digit()),
                "整形結果 {:?} に数字以外が含まれる",
                s
            );

            // 数値へ戻すと元の値に一致（ゼロ埋めのみで値は変わらない）。
            let reparsed: u64 = s.parse().expect("数字列は u64 として解釈可能");
            prop_assert_eq!(
                reparsed,
                n,
                "整形結果 {:?} を数値化すると元の番号と不一致",
                s
            );

            // width より短い値のときの先頭ゼロ埋め桁数を確認。
            // n の十進桁数（0 は 1 桁）が width 未満なら、差分だけ先頭に '0'。
            let digits = if n == 0 { 1 } else { (n as f64).log10().floor() as usize + 1 };
            if digits < width {
                let pad = width - digits;
                prop_assert!(
                    s.chars().take(pad).all(|c| c == '0'),
                    "先頭 {} 桁がゼロ埋めされていない: {:?}",
                    pad,
                    s
                );
            }

            // --- apply_numbering は NUM を同じ整形番号へ展開する ---
            let applied = apply_numbering("img_NUM", start, i, width);
            prop_assert_eq!(
                &applied,
                &format!("img_{}", s),
                "apply_numbering の展開結果が整形番号と不一致"
            );

            // expand_num_placeholder 単体でも同じ結果になる（合成の一貫性）。
            let expanded = expand_num_placeholder("img_NUM", n, width);
            prop_assert_eq!(
                &applied,
                &expanded,
                "apply_numbering と expand_num_placeholder の結果が不一致"
            );

            numbers.push(n);
            formatted.push(s);
        }

        // --- 昇順性・一意性: 番号列は狭義単調増加 ---
        for w in numbers.windows(2) {
            prop_assert!(
                w[0] < w[1],
                "番号列が狭義単調増加でない: {} >= {}",
                w[0],
                w[1]
            );
        }

        // 一意性を集合サイズでも確認。
        let unique: std::collections::HashSet<u64> = numbers.iter().copied().collect();
        prop_assert_eq!(
            unique.len(),
            numbers.len(),
            "割り当てた番号に重複がある"
        );

        // 先頭・末尾の連続性（範囲全体の整合）。
        prop_assert_eq!(*numbers.first().unwrap(), start, "先頭番号が start と不一致");
        prop_assert_eq!(
            *numbers.last().unwrap(),
            start + (count - 1),
            "末尾番号が start + (count-1) と不一致"
        );
    }
}
