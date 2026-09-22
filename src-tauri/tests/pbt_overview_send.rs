// Feature: local-model-management, Property 22: keep/exclude 送出後の包含
//
// Property 22: 任意の Keep_Tags（または Exclude_Rules）集合と送出タグ集合について、
// Tag_Overview から送出する操作（overview_send_keep / overview_send_exclude）の後、
// 更新後の集合は送出したタグをすべて含む。
//
// 送出は adapters::append_unique 経由で行われ、重複判定は正規化キー
// （前後トリム＋小文字化）で行う。よって「送出タグをすべて含む」は正規化キーの
// 包含として厳密に成立する（送出タグと同一キーの要素が更新後の集合に必ず存在する）。
// 加えて、元の集合の要素はすべて保持される。
//
// Validates: Requirements 11.4, 11.5

use proptest::prelude::*;
use tag_editor_core::commands::adapters::{overview_send_exclude, overview_send_keep};
use tag_editor_core::models::RawTagFilter;

/// タグ名の生成器。大文字・前後空白を混ぜて正規化経路（trim + lowercase）を
/// 踏ませる。素直な英数・アンダースコア・空白に限定する。
fn tag_name_strategy() -> impl Strategy<Value = String> {
    prop_oneof![
        "[A-Za-z0-9_]{1,12}",
        "[A-Za-z0-9_]{1,8} [A-Za-z0-9_]{1,8}",
        " *[A-Za-z0-9_]{1,10} *",
        "[A-Z][a-z]{1,8}",
    ]
}

/// タグ名ベクタの生成器（0〜6 個）。
fn tag_vec_strategy() -> impl Strategy<Value = Vec<String>> {
    prop::collection::vec(tag_name_strategy(), 0..=6)
}

/// 任意の初期 RawTagFilter を組み立てる（keep / exclude を任意に含む）。
fn raw_filter_strategy() -> impl Strategy<Value = RawTagFilter> {
    (
        tag_vec_strategy(),
        tag_vec_strategy(),
        0.0f32..=1.0f32,
        0.0f32..=1.0f32,
    )
        .prop_map(|(keep, exclude, ct, ft)| RawTagFilter {
            keep,
            exclude,
            replace: Vec::new(),
            additional: Vec::new(),
            confidence_threshold: ct,
            fraction_threshold: ft,
        })
}

/// 正規化キー（前後トリム＋小文字化）。append_unique と同じ規則。
fn norm_key(s: &str) -> String {
    s.trim().to_lowercase()
}

/// 集合を正規化キーの HashSet へ写像する。
fn key_set(items: &[String]) -> std::collections::HashSet<String> {
    items.iter().map(|s| norm_key(s)).collect()
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(200))]

    /// Property 22 (keep): overview_send_keep 後の filter.keep は送出タグをすべて含み
    /// （正規化キーで）、元の keep 要素もすべて保持する。exclude は不変。
    #[test]
    fn send_keep_contains_sent_tags(
        filter in raw_filter_strategy(),
        sent in tag_vec_strategy(),
    ) {
        let original_keep = filter.keep.clone();
        let original_exclude = filter.exclude.clone();

        let updated = overview_send_keep(filter, sent.clone()).expect("送出は失敗しない");
        let updated_keys = key_set(&updated.keep);

        // 送出タグはすべて（正規化キーで）更新後の keep に含まれる。
        for tag in &sent {
            prop_assert!(
                updated_keys.contains(&norm_key(tag)),
                "送出タグ {tag:?}（キー {:?}）が更新後 keep に含まれない。keep={:?}",
                norm_key(tag),
                updated.keep
            );
        }

        // 元の keep 要素はすべて保持される（正規化キーで包含）。
        for tag in &original_keep {
            prop_assert!(
                updated_keys.contains(&norm_key(tag)),
                "元の keep タグ {tag:?} が更新後 keep から失われた。keep={:?}",
                updated.keep
            );
        }

        // exclude は変化しない。
        prop_assert_eq!(&updated.exclude, &original_exclude, "exclude が変化した");
    }

    /// Property 22 (exclude): overview_send_exclude 後の filter.exclude は送出タグを
    /// すべて含み（正規化キーで）、元の exclude 要素もすべて保持する。keep は不変。
    #[test]
    fn send_exclude_contains_sent_tags(
        filter in raw_filter_strategy(),
        sent in tag_vec_strategy(),
    ) {
        let original_keep = filter.keep.clone();
        let original_exclude = filter.exclude.clone();

        let updated = overview_send_exclude(filter, sent.clone()).expect("送出は失敗しない");
        let updated_keys = key_set(&updated.exclude);

        // 送出タグはすべて（正規化キーで）更新後の exclude に含まれる。
        for tag in &sent {
            prop_assert!(
                updated_keys.contains(&norm_key(tag)),
                "送出タグ {tag:?}（キー {:?}）が更新後 exclude に含まれない。exclude={:?}",
                norm_key(tag),
                updated.exclude
            );
        }

        // 元の exclude 要素はすべて保持される（正規化キーで包含）。
        for tag in &original_exclude {
            prop_assert!(
                updated_keys.contains(&norm_key(tag)),
                "元の exclude タグ {tag:?} が更新後 exclude から失われた。exclude={:?}",
                updated.exclude
            );
        }

        // keep は変化しない。
        prop_assert_eq!(&updated.keep, &original_keep, "keep が変化した");
    }
}

/// 代表的な固定ケース: 大小・空白違いの送出でも正規化キーで包含が成立する。
#[test]
fn representative_send_keep_and_exclude_inclusion() {
    let filter = RawTagFilter {
        keep: vec!["solo".to_string()],
        exclude: vec!["blurry".to_string()],
        replace: Vec::new(),
        additional: Vec::new(),
        confidence_threshold: 0.5,
        fraction_threshold: 0.0,
    };

    // keep へ送出（既存 "solo" と大小違い + 新規 " Long_Hair "）。
    let after_keep = overview_send_keep(filter.clone(), vec![" SOLO ".to_string(), " Long_Hair ".to_string()])
        .expect("送出は失敗しない");
    let keep_keys = key_set(&after_keep.keep);
    assert!(keep_keys.contains("solo"));
    assert!(keep_keys.contains("long_hair"));
    // exclude 不変。
    assert_eq!(after_keep.exclude, filter.exclude);

    // exclude へ送出。
    let after_exclude = overview_send_exclude(filter.clone(), vec!["watermark".to_string(), "blurry".to_string()])
        .expect("送出は失敗しない");
    let exclude_keys = key_set(&after_exclude.exclude);
    assert!(exclude_keys.contains("watermark"));
    assert!(exclude_keys.contains("blurry"));
    // keep 不変。
    assert_eq!(after_exclude.keep, filter.keep);
}
