//! バッチ集計と一覧（タスク 7）。
//!
//! ファイル I/O や推論に依存しない決定的関数群。
//! - Fraction_Threshold 適用（要件 10.1, 10.2, 10.3, 10.4）
//! - Tag_Overview 構築（要件 11.1, 11.2）
//! - Tag_Overview 検索絞り込み（要件 11.3）

use std::collections::{HashMap, HashSet};

use crate::models::{BatchOutcome, FilterOutcome, Tag, TagFilter, TagOverview, TagStat};

/// タグ正規化キーを算出する（design「タグ正規化ルール」）。
///
/// 前後トリム＋大文字小文字無視の完全一致で統一する。
fn normalize_key(tag: &str) -> String {
    tag.trim().to_lowercase()
}

/// バッチ内出現割合（Fraction_Threshold）を適用する（要件 10）。
///
/// 各タグの出現割合（採用候補とした画像数 / バッチ対象画像数）を算出し、
/// 割合が `filter.fraction_threshold` 未満で、かつ Keep_Tags / Additional_Tags
/// でないタグを全画像の Adopted から Discarded へ移す（要件 10.1, 10.2）。
///
/// # 非適用条件
///
/// - `filter.fraction_threshold == 0` なら移送しない（要件 10.3）。
///   割合条件（割合 >= 0）が常に成立し、誰も落ちない（Property 17）。
/// - `image_count == 1`（単一画像）なら適用せず、`per_image` をそのまま返す
///   （要件 10.4、Property 18）。
///
/// # 採用同値条件（Property 17）
///
/// あるタグが Fraction_Threshold 適用後も Adopted に残ることは
/// 「出現割合 >= fraction_threshold、または Keep_Tags に含まれる、
/// または Additional_Tags に含まれる」ことと同値。
///
/// # 戻り値
///
/// 移送後の `per_image` を保持する [`BatchOutcome`]。`overview` は移送後の
/// `per_image` から [`build_overview`] で構築して結線する（要件 11.1, 11.2）。
pub fn apply_fraction_threshold(
    per_image: &[FilterOutcome],
    filter: &TagFilter,
    image_count: usize,
) -> BatchOutcome {
    // 単一画像（要件 10.4）または閾値 0（要件 10.3）は非適用。移送しないが、
    // overview は移送後（＝入力そのまま）の per_image から構築する。
    if image_count <= 1 || filter.fraction_threshold == 0.0 {
        let per_image = per_image.to_vec();
        let overview = build_overview_from(&per_image);
        return BatchOutcome {
            per_image,
            overview,
        };
    }

    // 各タグの出現画像数（採用候補とした画像数）を正規化キーで集計する。
    // 同一画像内で同じキーが複数回現れても 1 画像として数える。
    let mut appearance: HashMap<String, usize> = HashMap::new();
    for outcome in per_image {
        let keys: HashSet<String> = outcome
            .adopted
            .iter()
            .map(|tag| normalize_key(&tag.body))
            .collect();
        for key in keys {
            *appearance.entry(key).or_insert(0) += 1;
        }
    }

    // Keep_Tags / Additional_Tags は割合による除外の対象外（要件 10.2）。
    let keep_keys: HashSet<String> = filter.keep.iter().map(|k| normalize_key(k)).collect();
    let additional_keys: HashSet<String> =
        filter.additional.iter().map(|t| normalize_key(t)).collect();

    let image_count_f = image_count as f32;

    // 割合 < fraction_threshold かつ Keep/Additional でないキーを除去対象とする。
    let dropped: HashSet<String> = appearance
        .iter()
        .filter(|(key, &count)| {
            if keep_keys.contains(*key) || additional_keys.contains(*key) {
                return false;
            }
            let ratio = count as f32 / image_count_f;
            ratio < filter.fraction_threshold
        })
        .map(|(key, _)| key.clone())
        .collect();

    // 全画像の Adopted から除去対象を Discarded へ移す（順序を保存）。
    let per_image: Vec<FilterOutcome> = per_image
        .iter()
        .map(|outcome| move_dropped(outcome, &dropped))
        .collect();

    // 移送後の per_image から一覧を構築して結線する（要件 11.1, 11.2）。
    let overview = build_overview_from(&per_image);

    BatchOutcome {
        per_image,
        overview,
    }
}

/// 1 画像分の Adopted から除去対象キーのタグを Discarded へ移す。
///
/// Adopted の順序を保存し、除去対象は既存 Discarded の末尾へ順に加える。
fn move_dropped(outcome: &FilterOutcome, dropped: &HashSet<String>) -> FilterOutcome {
    let mut adopted: Vec<Tag> = Vec::with_capacity(outcome.adopted.len());
    let mut discarded: Vec<Tag> = outcome.discarded.clone();

    for tag in &outcome.adopted {
        if dropped.contains(&normalize_key(&tag.body)) {
            discarded.push(tag.clone());
        } else {
            adopted.push(tag.clone());
        }
    }

    FilterOutcome { adopted, discarded }
}

/// バッチ結果から Tag_Overview を構築する（要件 11.1, 11.2）。
///
/// [`BatchOutcome::per_image`] を走査し、タグを Adopted 集合と Discarded 集合へ
/// 分類する。各タグには代表確信度（出現画像の確信度平均、Property 20）と
/// 出現画像数 `image_count` を付与する。
///
/// # 分類規則（Property 19: 網羅と排他）
///
/// あるタグ名が一部画像で Adopted、別画像で Discarded になり得る。採用集合と
/// 不採用集合を交差させないため「1 画像でも Adopted であれば Adopted 側」とする。
/// 根拠: Adopted は少なくとも 1 画像の Tag_File へ書き込まれる（要件 9.9）ため、
/// 一覧上は採用タグとして扱うのが最も整合的（要件 11.1）。
///
/// # 代表確信度（Property 20）
///
/// そのタグが「分類された側」で出現した画像における確信度の平均。confidence が
/// `None` の出現は平均計算から除外する（母数に数えない）。すべて `None`（または
/// 出現 0）の場合は 0.0 とする。
///
/// # `image_count`
///
/// そのタグが分類された側で出現した画像数（重複キーは 1 画像として数える）。
pub fn build_overview(batch: &BatchOutcome) -> TagOverview {
    build_overview_from(&batch.per_image)
}

/// `per_image` から直接 Tag_Overview を構築する内部関数。
///
/// [`build_overview`] と [`apply_fraction_threshold`] の双方から共有する。
fn build_overview_from(per_image: &[FilterOutcome]) -> TagOverview {
    // まず全画像を走査し、タグ名（正規化キー）ごとに「1 画像でも Adopted か」を判定する。
    // Property 19: 採用集合と不採用集合を交差させないための分類基準。
    let mut adopted_anywhere: HashSet<String> = HashSet::new();
    for outcome in per_image {
        for tag in &outcome.adopted {
            adopted_anywhere.insert(normalize_key(&tag.body));
        }
    }

    // 集計器: キーごとに（表示名, 確信度合計, 確信度カウント, 出現画像数）を保持。
    // 出現画像数は同一画像内の重複キーを 1 と数えるため、画像単位で更新する。
    let mut adopted_acc: HashMap<String, StatAcc> = HashMap::new();
    let mut discarded_acc: HashMap<String, StatAcc> = HashMap::new();

    for outcome in per_image {
        // この画像で既に集計したキー（重複出現を 1 画像に丸める）。
        let mut seen_adopted: HashSet<String> = HashSet::new();
        let mut seen_discarded: HashSet<String> = HashSet::new();

        for tag in &outcome.adopted {
            let key = normalize_key(&tag.body);
            // Adopted 側は文字通り Adopted 出現なので Adopted 集合へ集計。
            accumulate(&mut adopted_acc, &mut seen_adopted, key, tag);
        }

        for tag in &outcome.discarded {
            let key = normalize_key(&tag.body);
            // 1 画像でも Adopted のキーは Adopted 側に分類する（Property 19 の排他）。
            // ただし Adopted 側の image_count / 代表確信度は「実際に採用された画像」のみ
            // から算出するため、この Discarded 出現は Adopted 側統計へ算入しない
            // （採用されていない画像を採用画像数に数えない）。
            if !adopted_anywhere.contains(&key) {
                accumulate(&mut discarded_acc, &mut seen_discarded, key, tag);
            }
        }
    }

    TagOverview {
        adopted: finalize(adopted_acc),
        discarded: finalize(discarded_acc),
    }
}

/// タグ集計の途中状態。
struct StatAcc {
    /// 表示名（最初に出現したタグ本体を採用）。
    name: String,
    /// 確信度の合計（`None` は加算しない）。
    conf_sum: f32,
    /// 確信度が付いた出現数（平均の母数）。
    conf_count: usize,
    /// 出現画像数（重複キーは 1 画像として数える）。
    image_count: usize,
}

/// 1 タグ出現を集計器へ加える。`seen` で同一画像内の重複を 1 に丸める。
fn accumulate(
    acc: &mut HashMap<String, StatAcc>,
    seen: &mut HashSet<String>,
    key: String,
    tag: &Tag,
) {
    let first_in_image = seen.insert(key.clone());
    let entry = acc.entry(key).or_insert_with(|| StatAcc {
        name: tag.body.clone(),
        conf_sum: 0.0,
        conf_count: 0,
        image_count: 0,
    });
    if first_in_image {
        entry.image_count += 1;
    }
    // 確信度平均は出現（confidence あり）単位で算入する（Property 20）。
    // None は母数から除外する。
    if let Some(c) = tag.confidence {
        entry.conf_sum += c;
        entry.conf_count += 1;
    }
}

/// 集計器を [`TagStat`] のベクタへ確定する。名前昇順で決定的に並べる。
fn finalize(acc: HashMap<String, StatAcc>) -> Vec<TagStat> {
    let mut stats: Vec<TagStat> = acc
        .into_values()
        .map(|a| TagStat {
            name: a.name,
            // 確信度が 1 つも無ければ 0.0（Property 20 の None 全滅ケース）。
            representative_confidence: if a.conf_count == 0 {
                0.0
            } else {
                a.conf_sum / a.conf_count as f32
            },
            image_count: a.image_count,
        })
        .collect();
    // HashMap の非決定順を避け、名前昇順で安定化する。
    stats.sort_by(|a, b| a.name.cmp(&b.name));
    stats
}

/// Tag_Overview を検索文字列で絞り込む（要件 11.3, Property 21）。
///
/// 大文字小文字を区別しない部分一致でタグ名を絞り込む。空クエリ（トリム後空）は
/// 全件を通過させる。
pub fn search_overview(overview: &TagOverview, query: &str) -> TagOverview {
    let needle = query.trim().to_lowercase();

    let matches = |stat: &TagStat| -> bool {
        // 空クエリは全件通過。
        if needle.is_empty() {
            return true;
        }
        stat.name.to_lowercase().contains(&needle)
    };

    TagOverview {
        adopted: overview
            .adopted
            .iter()
            .filter(|s| matches(s))
            .cloned()
            .collect(),
        discarded: overview
            .discarded
            .iter()
            .filter(|s| matches(s))
            .cloned()
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    /// Fraction_Threshold のみを設定した TagFilter を組み立てる。
    fn filter_with(fraction: f32, keep: &[&str], additional: &[&str]) -> TagFilter {
        TagFilter {
            keep: keep.iter().map(|s| s.to_string()).collect(),
            exclude: vec![],
            replace: vec![],
            additional: additional.iter().map(|s| s.to_string()).collect(),
            confidence_threshold: 0.0,
            fraction_threshold: fraction,
        }
    }

    fn outcome(adopted: &[Tag], discarded: &[Tag]) -> FilterOutcome {
        FilterOutcome {
            adopted: adopted.to_vec(),
            discarded: discarded.to_vec(),
        }
    }

    fn adopted_keys(outcome: &FilterOutcome) -> HashSet<String> {
        outcome
            .adopted
            .iter()
            .map(|t| normalize_key(&t.body))
            .collect()
    }

    #[test]
    fn single_image_is_not_applied() {
        // image_count == 1 なら適用せず入力をそのまま返す（要件 10.4）。
        let per_image = vec![outcome(&[Tag::with_confidence("rare", 0.9)], &[])];
        let filter = filter_with(0.5, &[], &[]);
        let result = apply_fraction_threshold(&per_image, &filter, 1);
        assert_eq!(result.per_image, per_image);
    }

    #[test]
    fn zero_threshold_moves_nothing() {
        // fraction_threshold == 0 なら移送しない（要件 10.3）。
        let per_image = vec![
            outcome(&[Tag::with_confidence("a", 0.9)], &[]),
            outcome(&[Tag::with_confidence("b", 0.9)], &[]),
        ];
        let filter = filter_with(0.0, &[], &[]);
        let result = apply_fraction_threshold(&per_image, &filter, 2);
        assert_eq!(result.per_image, per_image);
    }

    #[test]
    fn drops_tags_below_fraction() {
        // 2 画像中 1 画像のみ出現（割合 0.5）< 閾値 0.6 → 除外。
        let per_image = vec![
            outcome(
                &[
                    Tag::with_confidence("common", 0.9),
                    Tag::with_confidence("rare", 0.9),
                ],
                &[],
            ),
            outcome(&[Tag::with_confidence("common", 0.8)], &[]),
        ];
        let filter = filter_with(0.6, &[], &[]);
        let result = apply_fraction_threshold(&per_image, &filter, 2);

        // common は全画像に残る。
        assert!(adopted_keys(&result.per_image[0]).contains("common"));
        assert!(adopted_keys(&result.per_image[1]).contains("common"));
        // rare は 1 画像目の Adopted から消え Discarded へ移る。
        assert!(!adopted_keys(&result.per_image[0]).contains("rare"));
        assert_eq!(result.per_image[0].discarded.len(), 1);
        assert_eq!(result.per_image[0].discarded[0].body, "rare");
    }

    #[test]
    fn ratio_at_threshold_is_kept() {
        // 割合ちょうど == 閾値は残る（>= 判定、Property 17）。
        let per_image = vec![
            outcome(&[Tag::with_confidence("a", 0.9)], &[]),
            outcome(&[], &[]),
        ];
        let filter = filter_with(0.5, &[], &[]);
        let result = apply_fraction_threshold(&per_image, &filter, 2);
        assert!(adopted_keys(&result.per_image[0]).contains("a"));
    }

    #[test]
    fn keep_tags_are_exempt() {
        // Keep_Tags は割合未満でも残る（要件 10.2）。
        let per_image = vec![
            outcome(&[Tag::with_confidence("kept", 0.9)], &[]),
            outcome(&[], &[]),
        ];
        let filter = filter_with(0.9, &["kept"], &[]);
        let result = apply_fraction_threshold(&per_image, &filter, 2);
        assert!(adopted_keys(&result.per_image[0]).contains("kept"));
    }

    #[test]
    fn additional_tags_are_exempt() {
        // Additional_Tags は割合未満でも残る（要件 10.2）。
        let per_image = vec![
            outcome(&[Tag::with_confidence("added", 0.9)], &[]),
            outcome(&[], &[]),
        ];
        let filter = filter_with(0.9, &[], &["added"]);
        let result = apply_fraction_threshold(&per_image, &filter, 2);
        assert!(adopted_keys(&result.per_image[0]).contains("added"));
    }

    #[test]
    fn exempt_check_is_case_insensitive_and_trimmed() {
        // 正規化（トリム＋大小無視）で Keep 判定する。
        let per_image = vec![
            outcome(&[Tag::with_confidence("Kept", 0.9)], &[]),
            outcome(&[], &[]),
        ];
        let filter = filter_with(0.9, &["  KEPT  "], &[]);
        let result = apply_fraction_threshold(&per_image, &filter, 2);
        assert!(adopted_keys(&result.per_image[0]).contains("kept"));
    }

    #[test]
    fn overview_is_wired_from_per_image() {
        // apply_fraction_threshold の overview は移送後 per_image から構築される。
        let per_image = vec![
            outcome(&[Tag::with_confidence("a", 0.9)], &[]),
            outcome(&[Tag::with_confidence("a", 0.8)], &[]),
        ];
        let filter = filter_with(0.5, &[], &[]);
        let result = apply_fraction_threshold(&per_image, &filter, 2);
        assert_eq!(result.overview.adopted.len(), 1);
        assert!(result.overview.discarded.is_empty());
        let a = &result.overview.adopted[0];
        assert_eq!(a.name, "a");
        assert_eq!(a.image_count, 2);
        // 代表確信度 = (0.9 + 0.8) / 2 = 0.85。
        assert!((a.representative_confidence - 0.85).abs() < 1e-6);
    }

    #[test]
    fn build_overview_separates_adopted_and_discarded() {
        // 採用のみ / 不採用のみのタグをそれぞれの側へ分類する。
        let batch = BatchOutcome {
            per_image: vec![
                outcome(
                    &[Tag::with_confidence("keep", 0.8)],
                    &[Tag::with_confidence("drop", 0.2)],
                ),
                outcome(
                    &[Tag::with_confidence("keep", 0.6)],
                    &[Tag::with_confidence("drop", 0.4)],
                ),
            ],
            overview: TagOverview {
                adopted: vec![],
                discarded: vec![],
            },
        };
        let ov = build_overview(&batch);
        assert_eq!(ov.adopted.len(), 1);
        assert_eq!(ov.discarded.len(), 1);
        assert_eq!(ov.adopted[0].name, "keep");
        assert_eq!(ov.adopted[0].image_count, 2);
        assert!((ov.adopted[0].representative_confidence - 0.7).abs() < 1e-6);
        assert_eq!(ov.discarded[0].name, "drop");
        assert!((ov.discarded[0].representative_confidence - 0.3).abs() < 1e-6);
    }

    #[test]
    fn adopted_anywhere_wins_over_discarded() {
        // 1 画像でも Adopted なら Adopted 側（Property 19）。
        // ただし Adopted 側の image_count / 代表確信度は「実際に採用された画像」のみ
        // から算出する。別画像の Discarded 出現は Adopted 側統計へ算入しない。
        let batch = BatchOutcome {
            per_image: vec![
                outcome(&[Tag::with_confidence("t", 0.9)], &[]),
                outcome(&[], &[Tag::with_confidence("t", 0.5)]),
            ],
            overview: TagOverview {
                adopted: vec![],
                discarded: vec![],
            },
        };
        let ov = build_overview(&batch);
        assert_eq!(ov.adopted.len(), 1);
        assert!(ov.discarded.is_empty());
        // Adopted 出現は 0.9 の 1 枚のみ。Discarded 出現（0.5）は算入しない。
        assert_eq!(ov.adopted[0].image_count, 1);
        assert!((ov.adopted[0].representative_confidence - 0.9).abs() < 1e-6);
    }

    #[test]
    fn none_confidence_excluded_from_average() {
        // confidence None は平均の母数から除外する。
        let batch = BatchOutcome {
            per_image: vec![
                outcome(&[Tag::with_confidence("t", 0.8)], &[]),
                outcome(&[Tag::new("t")], &[]),
            ],
            overview: TagOverview {
                adopted: vec![],
                discarded: vec![],
            },
        };
        let ov = build_overview(&batch);
        assert_eq!(ov.adopted[0].image_count, 2);
        // 確信度は 0.8 の 1 件のみが母数 → 平均 0.8。
        assert!((ov.adopted[0].representative_confidence - 0.8).abs() < 1e-6);
    }

    #[test]
    fn all_none_confidence_is_zero() {
        // すべて None なら代表確信度 0.0。
        let batch = BatchOutcome {
            per_image: vec![outcome(&[Tag::new("t")], &[])],
            overview: TagOverview {
                adopted: vec![],
                discarded: vec![],
            },
        };
        let ov = build_overview(&batch);
        assert_eq!(ov.adopted[0].representative_confidence, 0.0);
    }

    #[test]
    fn duplicate_key_in_same_image_counts_once() {
        // 同一画像内の同名（正規化）キーは 1 画像として数える。
        let batch = BatchOutcome {
            per_image: vec![outcome(
                &[
                    Tag::with_confidence("Tag", 0.6),
                    Tag::with_confidence(" tag ", 0.8),
                ],
                &[],
            )],
            overview: TagOverview {
                adopted: vec![],
                discarded: vec![],
            },
        };
        let ov = build_overview(&batch);
        assert_eq!(ov.adopted.len(), 1);
        assert_eq!(ov.adopted[0].image_count, 1);
        // 確信度は 2 出現分を算入 → (0.6 + 0.8) / 2 = 0.7。
        assert!((ov.adopted[0].representative_confidence - 0.7).abs() < 1e-6);
    }

    #[test]
    fn search_empty_query_passes_all() {
        // 空クエリ（トリム後空含む）は全件通過。
        let ov = TagOverview {
            adopted: vec![TagStat {
                name: "cat".into(),
                representative_confidence: 0.5,
                image_count: 1,
            }],
            discarded: vec![TagStat {
                name: "dog".into(),
                representative_confidence: 0.5,
                image_count: 1,
            }],
        };
        let filtered = search_overview(&ov, "   ");
        assert_eq!(filtered.adopted.len(), 1);
        assert_eq!(filtered.discarded.len(), 1);
    }

    #[test]
    fn search_is_case_insensitive_substring() {
        // 大小無視の部分一致（要件 11.3）。
        let ov = TagOverview {
            adopted: vec![
                TagStat {
                    name: "Cat".into(),
                    representative_confidence: 0.5,
                    image_count: 1,
                },
                TagStat {
                    name: "dog".into(),
                    representative_confidence: 0.5,
                    image_count: 1,
                },
            ],
            discarded: vec![TagStat {
                name: "scatter".into(),
                representative_confidence: 0.5,
                image_count: 1,
            }],
        };
        let filtered = search_overview(&ov, "CAT");
        // "Cat"（採用）と "scatter"（不採用）が部分一致でヒット。
        assert_eq!(filtered.adopted.len(), 1);
        assert_eq!(filtered.adopted[0].name, "Cat");
        assert_eq!(filtered.discarded.len(), 1);
        assert_eq!(filtered.discarded[0].name, "scatter");
    }
}

#[cfg(test)]
mod property_tests {
    use super::*;
    use proptest::prelude::*;
    use std::collections::HashSet;

    fn normalize(tag: &str) -> String {
        tag.trim().to_lowercase()
    }

    /// 小さなタグ名集合（衝突を起こしやすくする）。
    fn tag_name() -> impl Strategy<Value = String> {
        prop::sample::select(vec!["a", "b", "c", "d", "e"]).prop_map(String::from)
    }

    /// 1 画像分の Adopted タグ集合（重複しないキー）。
    fn image_adopted() -> impl Strategy<Value = Vec<Tag>> {
        prop::collection::hash_set(tag_name(), 0..5).prop_map(|set| {
            set.into_iter()
                .map(|name| Tag::with_confidence(name, 0.9))
                .collect()
        })
    }

    /// 確信度（None または 0.0〜1.0）。None も混ぜて平均計算の除外を検証する。
    fn confidence() -> impl Strategy<Value = Option<f32>> {
        prop_oneof![Just(None), (0.0f32..=1.0).prop_map(Some),]
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

    /// 単一 TagStat の生成戦略。
    fn tag_stat() -> impl Strategy<Value = TagStat> {
        (tag_name(), 0.0f32..=1.0, 1usize..5).prop_map(|(name, c, n)| TagStat {
            name,
            representative_confidence: c,
            image_count: n,
        })
    }

    /// 検索対象の Tag_Overview（TagStat の集合）。
    fn overview() -> impl Strategy<Value = TagOverview> {
        (
            prop::collection::vec(tag_stat(), 0..6),
            prop::collection::vec(tag_stat(), 0..6),
        )
            .prop_map(|(adopted, discarded)| TagOverview { adopted, discarded })
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(200))]

        /// Feature: local-model-management, Property 17: Fraction_Threshold の採用同値条件。
        /// あるタグが適用後も Adopted に残ることは「出現割合 >= fraction_threshold、
        /// または Keep、または Additional」と同値。fraction_threshold==0 では誰も落ちない。
        #[test]
        fn property17_fraction_adoption_equivalence(
            images in prop::collection::vec(image_adopted(), 2..8),
            frac in 0.0f32..=1.0,
            keep in prop::collection::hash_set(tag_name(), 0..3),
            additional in prop::collection::hash_set(tag_name(), 0..3),
        ) {
            let image_count = images.len();
            let per_image: Vec<FilterOutcome> = images
                .iter()
                .map(|adopted| FilterOutcome { adopted: adopted.clone(), discarded: vec![] })
                .collect();

            let filter = TagFilter {
                keep: keep.iter().map(|k| k.to_string()).collect(),
                exclude: vec![],
                replace: vec![],
                additional: additional.iter().cloned().collect(),
                confidence_threshold: 0.0,
                fraction_threshold: frac,
            };

            // 適用前の出現画像数を算出。
            let mut appearance: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
            for adopted in &images {
                let keys: HashSet<String> = adopted.iter().map(|t| normalize(&t.body)).collect();
                for k in keys {
                    *appearance.entry(k).or_insert(0) += 1;
                }
            }

            let keep_keys: HashSet<String> = keep.iter().map(|k| normalize(k)).collect();
            let add_keys: HashSet<String> = additional.iter().map(|k| normalize(k)).collect();

            let result = apply_fraction_threshold(&per_image, &filter, image_count);

            // 適用後に残る Adopted キーの和集合。
            let remaining: HashSet<String> = result
                .per_image
                .iter()
                .flat_map(|o| o.adopted.iter().map(|t| normalize(&t.body)))
                .collect();

            // すべての出現キーについて同値条件を検証。
            for (key, &count) in &appearance {
                let ratio = count as f32 / image_count as f32;
                let should_remain = ratio >= frac
                    || keep_keys.contains(key)
                    || add_keys.contains(key);
                prop_assert_eq!(
                    remaining.contains(key),
                    should_remain,
                    "key={:?} count={} ratio={} frac={}",
                    key, count, ratio, frac
                );
            }
        }

        /// Feature: local-model-management, Property 18: Fraction の単一画像非適用。
        /// 単一画像では適用後の Adopted が適用前と一致する。
        #[test]
        fn property18_single_image_unchanged(
            adopted in image_adopted(),
            frac in 0.0f32..=1.0,
        ) {
            let per_image = vec![FilterOutcome { adopted: adopted.clone(), discarded: vec![] }];
            let filter = TagFilter {
                keep: HashSet::new(),
                exclude: vec![],
                replace: vec![],
                additional: vec![],
                confidence_threshold: 0.0,
                fraction_threshold: frac,
            };
            let result = apply_fraction_threshold(&per_image, &filter, 1);
            prop_assert_eq!(&result.per_image[0].adopted, &adopted);
            prop_assert!(result.per_image[0].discarded.is_empty());
        }

        /// Feature: local-model-management, Property 19: Tag_Overview の網羅と排他。
        /// 採用タグ名集合と不採用タグ名集合の和は、バッチ内に出現した全タグ名と
        /// 過不足なく一致し、採用集合と不採用集合は交差しない。
        #[test]
        fn property19_overview_partition(batch in batch()) {
            let ov = build_overview(&batch);

            let adopted_names: HashSet<String> =
                ov.adopted.iter().map(|s| normalize(&s.name)).collect();
            let discarded_names: HashSet<String> =
                ov.discarded.iter().map(|s| normalize(&s.name)).collect();

            // 排他: 交差なし。
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
        /// 各タグの代表確信度は、そのタグが分類された側で出現した画像における
        /// 確信度（None を除く）の平均に等しい。確信度が皆無なら 0.0。
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

            // キーごとに、分類側の出現確信度を収集する。
            let mut adopted_confs: std::collections::HashMap<String, Vec<f32>> =
                std::collections::HashMap::new();
            let mut discarded_confs: std::collections::HashMap<String, Vec<f32>> =
                std::collections::HashMap::new();

            // 分類側ごとに確信度列を作る（None は列へ加えない ＝ 母数から除外）。
            for o in &batch.per_image {
                for t in &o.adopted {
                    let key = normalize(&t.body);
                    if let Some(c) = t.confidence {
                        adopted_confs.entry(key).or_default().push(c);
                    } else {
                        adopted_confs.entry(key).or_default();
                    }
                }
                for t in &o.discarded {
                    let key = normalize(&t.body);
                    // Adopted 側の代表確信度は Adopted 出現のみから算出するため、
                    // Adopted キーの Discarded 出現は確信度列へ加えない。
                    if adopted_anywhere.contains(&key) {
                        continue;
                    }
                    if let Some(c) = t.confidence {
                        discarded_confs.entry(key).or_default().push(c);
                    } else {
                        discarded_confs.entry(key).or_default();
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
                let confs = adopted_confs.get(&normalize(&stat.name)).cloned().unwrap_or_default();
                prop_assert!((stat.representative_confidence - expected(&confs)).abs() < 1e-4);
            }
            for stat in &ov.discarded {
                let confs = discarded_confs.get(&normalize(&stat.name)).cloned().unwrap_or_default();
                prop_assert!((stat.representative_confidence - expected(&confs)).abs() < 1e-4);
            }
        }

        /// Feature: local-model-management, Property 21: Tag_Overview 検索絞り込みの述語一致。
        /// 絞り込み結果にタグが含まれることは、そのタグ名が検索文字列を部分一致
        /// （大小無視）で含むことと同値。空クエリは全件通過。
        #[test]
        fn property21_search_predicate(overview in overview(), query in "[a-eA-E ]{0,4}") {
            let filtered = search_overview(&overview, &query);
            let needle = query.trim().to_lowercase();

            let predicate = |name: &str| -> bool {
                needle.is_empty() || name.to_lowercase().contains(&needle)
            };

            // 採用側: 全 TagStat について「述語成立 ⇔ 結果に含まれる」を検証。
            for stat in &overview.adopted {
                let present = filtered.adopted.iter().any(|s| s == stat);
                prop_assert_eq!(present, predicate(&stat.name), "adopted name={:?} query={:?}", stat.name, query);
            }
            for stat in &overview.discarded {
                let present = filtered.discarded.iter().any(|s| s == stat);
                prop_assert_eq!(present, predicate(&stat.name), "discarded name={:?} query={:?}", stat.name, query);
            }
        }
    }
}
