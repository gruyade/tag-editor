// Feature: local-model-management, Property 21: Tag_Overview 検索絞り込みの述語一致
//
// Property 21: 絞り込み結果にタグが含まれることは、そのタグ名が検索文字列を
// 部分一致（大小無視）で含むことと同値。空クエリ（トリム後空）は全件通過。
//
// 対象: tag_editor_core::logic::tag_batch::search_overview
//
// 実装の述語（tag_batch.rs::search_overview）に厳密整合:
//   needle = query.trim().to_lowercase()
//   含まれる ⇔ needle.is_empty() || stat.name.to_lowercase().contains(&needle)
//
// Validates: Requirements 11.3

use proptest::prelude::*;
use tag_editor_core::logic::tag_batch::search_overview;
use tag_editor_core::models::{TagOverview, TagStat};

/// タグ名: ASCII 大小混在・前後空白・非 ASCII を混ぜて経路を踏ませる。
fn tag_name() -> impl Strategy<Value = String> {
    prop_oneof![
        prop::sample::select(vec!["cat", "Dog", "BIRD", " Fish ", "aBc"]).prop_map(String::from),
        prop::sample::select(vec!["猫", "犬ネコ", "café", "Straße", "ЖУК"]).prop_map(String::from),
    ]
}

/// 1 TagStat。
fn any_stat() -> impl Strategy<Value = TagStat> {
    (tag_name(), 0.0f32..=1.0, 0usize..10).prop_map(
        |(name, representative_confidence, image_count)| TagStat {
            name,
            representative_confidence,
            image_count,
        },
    )
}

/// TagOverview（adopted / discarded を独立生成）。
fn any_overview() -> impl Strategy<Value = TagOverview> {
    (
        prop::collection::vec(any_stat(), 0..6),
        prop::collection::vec(any_stat(), 0..6),
    )
        .prop_map(|(adopted, discarded)| TagOverview { adopted, discarded })
}

/// 検索クエリ: 空・大小混在・部分文字列・非 ASCII・前後空白付き。
fn query() -> impl Strategy<Value = String> {
    prop_oneof![
        Just(String::new()),
        Just("   ".to_string()),
        prop::sample::select(vec!["ca", "DO", "ish", "aBc", "  Bird  ", "z"])
            .prop_map(String::from),
        prop::sample::select(vec!["猫", "ネコ", "CAFÉ", "ß", "жук"]).prop_map(String::from),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(200))]

    /// Feature: local-model-management, Property 21: Tag_Overview 検索絞り込みの述語一致。
    /// 結果に含まれる ⇔ 期待述語（空クエリは true）。adopted/discarded 双方で検証。
    #[test]
    fn property21_search_predicate_equivalence(overview in any_overview(), query in query()) {
        let result = search_overview(&overview, &query);

        let needle = query.trim().to_lowercase();
        let expected = |stat: &TagStat| -> bool {
            if needle.is_empty() {
                true
            } else {
                stat.name.to_lowercase().contains(&needle)
            }
        };

        // adopted 側: 各入力 stat について「結果に含まれる ⇔ 期待述語」。
        for stat in &overview.adopted {
            let present = result.adopted.iter().any(|s| s == stat);
            prop_assert_eq!(
                present,
                expected(stat),
                "adopted name={:?} query={:?} present={} expected={}",
                stat.name,
                query,
                present,
                expected(stat)
            );
        }

        // discarded 側: 同上。
        for stat in &overview.discarded {
            let present = result.discarded.iter().any(|s| s == stat);
            prop_assert_eq!(
                present,
                expected(stat),
                "discarded name={:?} query={:?} present={} expected={}",
                stat.name,
                query,
                present,
                expected(stat)
            );
        }

        // 結果は入力の部分集合であり、余計な要素を作らない。
        prop_assert!(result.adopted.len() <= overview.adopted.len());
        prop_assert!(result.discarded.len() <= overview.discarded.len());
    }
}
