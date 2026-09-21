// Feature: tag-editor, Property 25: バッチ分割の保存性
//
// Validates: Requirements 17.2
//
// 対象: `tag_editor_core::logic::inference_aux::split_into_batches`。
//
// プロパティ本文:
//   任意の件数と Batch_Size について、分割された各チャンクの長さは
//   Batch_Size 以下であり、チャンクを順に連結すると元の列（順序を保存）に
//   一致し、最終チャンク以外の長さはすべて Batch_Size に等しい。
//
// ジェネレータ制約（本文が前提とする入力空間へ最小限に絞り、それぞれ正当化）:
//   - items（Vec<u32>）は長さ 0..=256 の範囲から生成する。
//       空入力（下限境界）を含み、分割が発生する十分な長さまでを網羅する。
//       要素値は u32 全域から生成し、順序保存の検証を意味あるものにする。
//   - batch_size（u32）は 1..=64 から生成する。
//       split_into_batches は 1 以上を前提とする（doc）。上限 64 は
//       items 上限 256 に対し「件数 < Batch_Size」「ちょうど倍数」「余りあり」
//       の全パターンを踏める十分な範囲。
//
// 境界カバレッジ（別テストで明示的に踏む）:
//   - 空入力         -> 空の列を返す
//   - 件数 < batch   -> 単一チャンク（全要素）
//   - ちょうど倍数   -> 全チャンクが batch_size 長、余りチャンクなし

use proptest::prelude::*;
use tag_editor_core::logic::inference_aux::split_into_batches;

/// 分割結果が Property 25 の 3 条件を満たすことを検証する共通ヘルパー。
fn check(items: &[u32], batch_size: u32) -> Result<(), TestCaseError> {
    let batches = split_into_batches(items, batch_size);
    let bs = batch_size as usize;

    // 条件 1: 各チャンクの長さは batch_size 以下。
    for chunk in &batches {
        prop_assert!(
            chunk.len() <= bs,
            "チャンク長が batch_size を超過: len={}, batch_size={}",
            chunk.len(),
            batch_size
        );
    }

    // 条件 2: チャンクを順に連結すると元の列に一致する（順序保存）。
    let flat: Vec<u32> = batches.iter().flatten().copied().collect();
    prop_assert_eq!(
        &flat,
        items,
        "連結結果が元の列と不一致: batch_size={}",
        batch_size
    );

    // 条件 3: 最終チャンク以外の長さはすべて batch_size に等しい。
    //         入力が空なら batches も空でこの検証は自明に成立。
    if !batches.is_empty() {
        for chunk in &batches[..batches.len() - 1] {
            prop_assert_eq!(
                chunk.len(),
                bs,
                "最終以外のチャンク長が batch_size と不一致: len={}, batch_size={}",
                chunk.len(),
                batch_size
            );
        }
        // 追加の内部整合: 最終チャンクは非空かつ batch_size 以下。
        let last = batches.last().unwrap();
        prop_assert!(!last.is_empty(), "最終チャンクが空になっている");
        prop_assert!(last.len() <= bs, "最終チャンク長が batch_size を超過");
    }

    // 空入力なら結果は空の列。
    if items.is_empty() {
        prop_assert!(batches.is_empty(), "空入力なのに空の列を返さない");
    }

    Ok(())
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(500))]

    /// 任意の件数・batch_size で保存性（3 条件）が成り立つ。
    #[test]
    fn split_preserves_and_bounds(
        items in prop::collection::vec(any::<u32>(), 0..=256),
        batch_size in 1_u32..=64,
    ) {
        check(&items, batch_size)?;
    }

    /// 境界: 空入力は空の列を返す（batch_size は任意）。
    #[test]
    fn empty_input_yields_empty(batch_size in 1_u32..=64) {
        let items: Vec<u32> = Vec::new();
        let batches = split_into_batches(&items, batch_size);
        prop_assert!(batches.is_empty(), "空入力が空の列にならない");
        check(&items, batch_size)?;
    }

    /// 境界: 件数 < batch_size のとき単一チャンクに全要素が入る。
    #[test]
    fn count_less_than_batch_yields_single_chunk(
        items in prop::collection::vec(any::<u32>(), 1..=63),
        extra in 0_u32..=64,
    ) {
        // batch_size を件数より大きく設定する。
        let batch_size = items.len() as u32 + 1 + extra;
        let batches = split_into_batches(&items, batch_size);
        prop_assert_eq!(
            batches.len(),
            1,
            "件数 < batch_size なのに単一チャンクでない: count={}, batch_size={}",
            items.len(),
            batch_size
        );
        prop_assert_eq!(&batches[0], &items, "単一チャンクが全要素を保持しない");
        check(&items, batch_size)?;
    }

    /// 境界: 件数がちょうど batch_size の倍数のとき、全チャンクが batch_size 長。
    #[test]
    fn exact_multiple_all_chunks_full(
        batch_size in 1_u32..=32,
        multiplier in 1_usize..=8,
    ) {
        let count = batch_size as usize * multiplier;
        let items: Vec<u32> = (0..count as u32).collect();
        let batches = split_into_batches(&items, batch_size);

        prop_assert_eq!(
            batches.len(),
            multiplier,
            "倍数入力のチャンク数が不一致: count={}, batch_size={}",
            count,
            batch_size
        );
        for chunk in &batches {
            prop_assert_eq!(
                chunk.len(),
                batch_size as usize,
                "倍数入力なのに batch_size 長でないチャンクがある"
            );
        }
        check(&items, batch_size)?;
    }
}
