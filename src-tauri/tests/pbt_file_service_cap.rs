// Feature: tag-editor, Property 2: 列挙件数の上限適用
//
// Validates: Requirements 1.2
//
// 対象: `tag_editor_core::services::file_service::{list_images, MAX_LISTING}`
//        （MAX_LISTING = 10_000）。
//
// プロパティ本文:
//   任意の件数 N の対応拡張子ファイルを直下に持つフォルダを列挙したとき、
//     - 列挙結果の件数 items.len() は min(N, MAX_LISTING) に等しい
//     - total は上限適用前の総数 N に等しい
//     - truncated フラグは (N > MAX_LISTING) と一致する
//
// ジェネレータ制約と case 数を絞る正当化:
//   このプロパティは実ファイルシステム上で N 個の実ファイルを生成して検証する
//   統合テストである。min(N, MAX_LISTING) の上限境界（10_000）を踏むには
//   N > 10_000 の実ファイル作成が必要で、1 ケースあたり 1 万件超のファイル
//   I/O が発生し非常に高コストである。そこで:
//     - ProptestConfig::with_cases を小さく（20）設定する。
//     - N は上限境界 MAX_LISTING の前後に集中させた小さな候補集合
//       {0, 1, 9_999, 10_000, 10_001, 10_005} から生成する。
//       これにより「上限未満」「上限ちょうど」「上限超（truncated=true）」の
//       各区分と、境界（10_000 / 10_001）を最小コストで網羅する。
//   proptest の反復回数は少ないが、候補集合が有限かつ境界を明示的に含むため、
//   本プロパティの上限適用ロジックを十分に検証できる。

use proptest::prelude::*;
use tag_editor_core::services::file_service::{list_images, MAX_LISTING};
use tempfile::tempdir;

/// 検証対象の件数 N の候補集合。上限境界 MAX_LISTING の前後に集中させる。
/// MAX_LISTING は 10_000 である前提で境界値を明示的に含める。
fn count_strategy() -> impl Strategy<Value = usize> {
    prop_oneof![
        Just(0_usize),
        Just(1_usize),
        Just(MAX_LISTING - 1), // 9_999: 上限直下
        Just(MAX_LISTING),     // 10_000: 上限ちょうど（truncated=false 側の境界）
        Just(MAX_LISTING + 1), // 10_001: 上限超の最小（truncated=true 側の境界）
        Just(MAX_LISTING + 5), // 10_005: 上限超
    ]
}

/// N 個の空の対応拡張子ファイルを一時ディレクトリに作成し、list_images の
/// 上限適用（要件 1.2）を検証する共通ルーチン。
fn check_cap(n: usize) -> Result<(), TestCaseError> {
    let dir = tempdir().expect("一時ディレクトリ作成に失敗");

    // N 個の対応拡張子ファイルを作成する。ゼロ埋めした名前で一意性を確保する。
    for i in 0..n {
        // 拡張子は対応拡張子（png）。名前の一意性のみが必要。
        let name = format!("img_{i:06}.png");
        std::fs::write(dir.path().join(name), b"").expect("ファイル作成に失敗");
    }

    let listing = list_images(dir.path()).expect("列挙に失敗");

    let expected_len = n.min(MAX_LISTING);
    let expected_truncated = n > MAX_LISTING;

    // items.len() == min(N, MAX_LISTING)
    prop_assert_eq!(
        listing.items.len(),
        expected_len,
        "items.len() が min(N, MAX_LISTING) と不一致: N={}, MAX_LISTING={}",
        n,
        MAX_LISTING
    );
    // total == N（上限適用前の総数）
    prop_assert_eq!(listing.total, n, "total が N と不一致: N={}", n);
    // truncated == (N > MAX_LISTING)
    prop_assert_eq!(
        listing.truncated,
        expected_truncated,
        "truncated が (N > MAX_LISTING) と不一致: N={}, MAX_LISTING={}",
        n,
        MAX_LISTING
    );

    Ok(())
}

proptest! {
    // 実ファイル生成を伴うため case 数を最小限に絞る（ファイル冒頭コメント参照）。
    #![proptest_config(ProptestConfig::with_cases(20))]

    /// 任意の件数 N について、列挙件数の上限適用が成立することを検証する。
    #[test]
    fn listing_count_is_capped_at_max_listing(n in count_strategy()) {
        check_cap(n)?;
    }
}
