// Feature: local-model-management, Property 19: Tag_Overview の網羅と排他
//
// Property 19: 任意のバッチのフィルタ結果について、Tag_Overview の採用タグ名集合と
// 不採用タグ名集合の和は、バッチ内に出現した全タグ名と過不足なく一致し、
// 採用集合と不採用集合は交差しない。
//
// Feature: local-model-management, Property 20: 代表確信度＝出現確信度の平均
//
// Property 20: 任意のバッチのフィルタ結果について、Tag_Overview の各タグの
// 代表確信度は、そのタグが分類された側で出現した画像における確信度（None を除く
// 母数）の平均に等しい。確信度が皆無なら 0.0。
//
// 対象: tag_editor_core::logic::tag_batch::build_overview
//
// 実装の分類規則は「1 画像でも Adopted なら Adopted 側」（build_overview /
// build_overview_from に準拠）。Discarded 出現も、そのキーが Adopted 側へ属する
// なら Adopted 側の平均・出現へ算入する。テストの期待述語もこれに厳密に合わせる。
//
// Validates: Requirements 11.1, 11.2

use std::collections::{HashMap, HashSet};

use proptest::prelude::*;
use tag_editor_core::logic::tag_batch::build_overview;
use tag_editor_core::models::{BatchOutcome, FilterOutcome, Tag, TagOverview};

/// タグ正規化キー（前後トリム＋大小無視）。実装 normalize_key と同一規則。
fn normalize(tag: &str) -> String {
    tag.trim().to_lowercase()
}

/// 小さなタグ名集合（Adopted / Discarded で衝突を起こしやすくする）。
/// 大文字・前後空白を混ぜて正規化経路を踏ませる。
fn tag_name() -> impl Strategy<Value = String> {
    prop_oneof![
        prop::sample::select(vec!["a", "b", "c", "d", "e"]).prop_map(String::from),
        prop::sample::select(vec!["A", "B", " c ", "D "]).prop_map(String::from),
    ]
}

/// 確信度（None または 0.0〜1.0）。None を混ぜて平均計算の母数除外を検証する。
fn confidence() -> impl Strategy<Value = Option<f32>> {
    prop_oneof![
        Just(None),
        (0.0f32..=1.0).prop_map(Some),
    ]
}

/// 1 タグ（名前＋任意確信度）。
fn any_tag() -> impl Strategy<Value = Tag> {
    (tag_name(), confidence()).prop_map(|(body, confidence)| Tag { body, confidence })
}

/// 1 画像分のフィルタ結果（Adopted / Discarded を独立生成）。
fn image_outcome() -> impl Strategy<Value = FilterOutcome> {
    (
        prop::collection::vec(any_tag(), 0..5),
        prop::collection::vec(any_tag(), 0..5),
    )
        .prop_map(|(adopted, discarded)| FilterOutcome { adopted, discarded })
}

/// バッチ全体（画像ごとの FilterOutcome の列）。
fn batch() -> impl Strategy<Value = BatchOutcome> {
    prop::collection::vec(image_outcome(), 0..6).prop_map(|per_image| BatchOutcome {
        per_image,
        overview: TagOverview {
            adopted: vec![],
            discarded: vec![],
        },
    })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(200))]

    /// Feature: local-model-management, Property 19: Tag_Overview の網羅と排他。
    /// 採用タグ名集合と不採用タグ名集合の和は、バッチ内出現全タグ名と過不足なく
    /// 一致し、両集合は交差しない。
    #[test]
    fn property19_overview_partition(batch in batch()) {
        let ov = build_overview(&batch);

        let adopted_names: HashSet<String> =
            ov.adopted.iter().map(|s| normalize(&s.name)).collect();
        let discarded_names: HashSet<String> =
            ov.discarded.iter().map(|s| normalize(&s.name)).collect();

        // 排他: 採用集合と不採用集合は交差しない。
        prop_assert!(adopted_names.is_disjoint(&discarded_names));

        // 網羅: 和集合 == バッチ内に出現した全タグ名（正規化キー）。
        let mut all_keys: HashSet<String> = HashSet::new();
        for o in &batch.per_image {
            for t in o.adopted.iter().chain(o.discarded.iter()) {
                all_keys.insert(normalize(&t.body));
            }
        }
        let union: HashSet<String> =
            adopted_names.union(&discarded_names).cloned().collect();
        prop_assert_eq!(&union, &all_keys);

        // 各名は一意（重複 TagStat を作らない）。
        prop_assert_eq!(ov.adopted.len(), adopted_names.len());
        prop_assert_eq!(ov.discarded.len(), discarded_names.len());
    }

    /// Feature: local-model-management, Property 20: 代表確信度＝出現確信度の平均。
    /// Adopted 側の代表確信度は Adopted 出現の確信度平均、Discarded 側は非 Adopted
    /// キーの Discarded 出現の確信度平均（いずれも None を母数から除外）。皆無なら 0.0。
    #[test]
    fn property20_representative_confidence_is_mean(batch in batch()) {
        let ov = build_overview(&batch);

        // 分類側を再計算する（1 画像でも Adopted なら Adopted 側）。
        let mut adopted_anywhere: HashSet<String> = HashSet::new();
        for o in &batch.per_image {
            for t in &o.adopted {
                adopted_anywhere.insert(normalize(&t.body));
            }
        }

        // キーごとに、分類された側で出現した確信度列を収集する（None は母数から除外）。
        let mut adopted_confs: HashMap<String, Vec<f32>> = HashMap::new();
        let mut discarded_confs: HashMap<String, Vec<f32>> = HashMap::new();

        for o in &batch.per_image {
            // Adopted 出現は必ず Adopted 側。
            for t in &o.adopted {
                let key = normalize(&t.body);
                let entry = adopted_confs.entry(key).or_default();
                if let Some(c) = t.confidence {
                    entry.push(c);
                }
            }
            // Discarded 出現は、Adopted 側キーなら統計へ算入しない
            // （Adopted 側の代表確信度は Adopted 出現のみから算出する）。
            // 非 Adopted キーの Discarded 出現のみ Discarded 側へ集計する。
            for t in &o.discarded {
                let key = normalize(&t.body);
                if adopted_anywhere.contains(&key) {
                    continue;
                }
                let entry = discarded_confs.entry(key).or_default();
                if let Some(c) = t.confidence {
                    entry.push(c);
                }
            }
        }

        let expected = |confs: &[f32]| -> f32 {
            if confs.is_empty() {
                0.0
            } else {
                confs.iter().sum::<f32>() / confs.len() as f32
            }
        };

        for stat in &ov.adopted {
            let confs = adopted_confs
                .get(&normalize(&stat.name))
                .cloned()
                .unwrap_or_default();
            prop_assert!(
                (stat.representative_confidence - expected(&confs)).abs() < 1e-4,
                "adopted name={:?} 代表={} 期待={}",
                stat.name,
                stat.representative_confidence,
                expected(&confs)
            );
        }
        for stat in &ov.discarded {
            let confs = discarded_confs
                .get(&normalize(&stat.name))
                .cloned()
                .unwrap_or_default();
            prop_assert!(
                (stat.representative_confidence - expected(&confs)).abs() < 1e-4,
                "discarded name={:?} 代表={} 期待={}",
                stat.name,
                stat.representative_confidence,
                expected(&confs)
            );
        }
    }
}
