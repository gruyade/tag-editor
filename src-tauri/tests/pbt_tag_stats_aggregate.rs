// Feature: tag-editor, Property 11: タグ集計の正確性と整列
//
// **Property 11: タグ集計の正確性と整列**
// **Validates: Requirements 6.1, 6.3**
//
// 任意のタグ列群について、各タグの集計出現回数は実際の総出現回数（前後トリム後・
// 非空のタグを対象）に等しく、集計結果は出現回数の降順、同数のタグはタグ名（トリム後）
// の昇順に整列される。
//
// 統合テスト（`tests/` 配下）として実装し、モジュールソースは編集しない。

use std::collections::HashMap;

use proptest::prelude::*;
use tag_editor_core::logic::tag_stats::aggregate_tags;
use tag_editor_core::models::TagCount;

/// タグ 1 件を生成するストラテジ。
///
/// 集計仕様の境界を網羅するため、以下を混在させる:
/// - 空文字列
/// - 空白のみ（半角スペース/タブ/全角スペース）
/// - 前後に空白を含む本体（トリム対象）
/// - 大文字小文字混在（集計上は別タグ）
/// - マルチバイト（日本語・絵文字）
fn tag_strategy() -> impl Strategy<Value = String> {
    prop_oneof![
        // 空文字列。
        Just(String::new()),
        // 空白のみ（各種空白文字）。
        prop::collection::vec(prop_oneof![Just(' '), Just('\t'), Just('\u{3000}')], 1..4)
            .prop_map(|cs| cs.into_iter().collect()),
        // 大小混在の ASCII 本体（前後空白の有無を含む）。
        prop_oneof![
            Just("Cat".to_string()),
            Just("cat".to_string()),
            Just("CAT".to_string()),
            Just("  Dog ".to_string()),
            Just("dog".to_string()),
            Just("\tbird\t".to_string()),
            Just("apple".to_string()),
            Just("Banana".to_string()),
            Just("cherry ".to_string()),
        ],
        // マルチバイト（前後空白の有無を含む）。
        prop_oneof![
            Just("猫".to_string()),
            Just(" 猫 ".to_string()),
            Just("犬".to_string()),
            Just("空".to_string()),
            Just("😺".to_string()),
            Just("  😺".to_string()),
        ],
    ]
}

/// タグ列群（Tag_File 群）を生成するストラテジ。空のファイル群・空のファイルも許容。
fn files_strategy() -> impl Strategy<Value = Vec<Vec<String>>> {
    prop::collection::vec(prop::collection::vec(tag_strategy(), 0..8), 0..8)
}

/// 参照実装: 前後トリムし非空のタグのみを対象に、トリム後文字列で出現回数を数える。
fn independent_counts(files: &[Vec<String>]) -> HashMap<String, usize> {
    let mut counts: HashMap<String, usize> = HashMap::new();
    for file in files {
        for tag in file {
            let trimmed = tag.trim();
            if trimmed.is_empty() {
                continue;
            }
            *counts.entry(trimmed.to_string()).or_insert(0) += 1;
        }
    }
    counts
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// Property 11: 集計の正確性（各タグの count が独立算出の出現回数に等しく、
    /// 集計結果とタグ集合が過不足なく一致する）と整列（count 降順・同数はタグ名昇順）。
    #[test]
    fn aggregate_is_correct_and_sorted(files in files_strategy()) {
        let result: Vec<TagCount> = aggregate_tags(files.clone());
        let expected = independent_counts(&files);

        // (1) 正確性: 結果のタグ集合と件数が独立算出と完全一致する。
        //     - 結果に重複タグが無い（トリム後の一意性）。
        //     - 各 count が独立算出値に一致する。
        let mut seen: HashMap<&str, usize> = HashMap::new();
        for tc in &result {
            // トリム後の本体がそのままタグ名になっている（前後空白が残らない）。
            prop_assert_eq!(tc.tag.as_str(), tc.tag.trim());
            // 空タグは含まれない。
            prop_assert!(!tc.tag.is_empty());
            // 重複が無い。
            prop_assert!(seen.insert(tc.tag.as_str(), tc.count).is_none(),
                "duplicate tag in result: {}", tc.tag);
            // count が独立算出値に一致する。
            let want = expected.get(tc.tag.as_str()).copied();
            prop_assert_eq!(Some(tc.count), want,
                "count mismatch for tag {}: got {}, want {:?}", tc.tag, tc.count, want);
        }

        // (2) 完全性: 独立算出のタグがすべて結果に含まれ、件数も一致する。
        prop_assert_eq!(result.len(), expected.len(),
            "result length {} != expected distinct tags {}", result.len(), expected.len());
        for (tag, &cnt) in &expected {
            prop_assert_eq!(seen.get(tag.as_str()).copied(), Some(cnt),
                "expected tag {} (count {}) missing or mismatched in result", tag, cnt);
        }

        // (3) 整列不変条件: 隣接ペアが count 降順、同数はタグ名昇順。
        for pair in result.windows(2) {
            let (a, b) = (&pair[0], &pair[1]);
            let ordered = a.count > b.count
                || (a.count == b.count && a.tag <= b.tag);
            prop_assert!(ordered,
                "sort order violated: ({}, {}) before ({}, {})",
                a.tag, a.count, b.tag, b.count);
            // 同数隣接時は厳密に昇順（重複が無いため等号は起きない）。
            if a.count == b.count {
                prop_assert!(a.tag < b.tag,
                    "tie not sorted strictly ascending by name: {} then {}", a.tag, b.tag);
            }
        }
    }
}
