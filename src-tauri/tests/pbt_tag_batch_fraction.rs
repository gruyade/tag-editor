// Feature: local-model-management, Property 17: Fraction_Threshold の採用同値条件
//
// Validates: Requirements 10.1, 10.2, 10.3
//
// *任意の* バッチの画像ごとフィルタ結果・Fraction_Threshold・Keep_Tags・
// Additional_Tags について、あるタグが Fraction_Threshold 適用後も Adopted に
// 残ることは「そのタグの出現割合（採用候補とした画像数 / バッチ対象画像数）が
// Fraction_Threshold 以上、または Keep_Tags に含まれる、または Additional_Tags
// に含まれる」ことと同値である（fraction_threshold == 0 では割合条件が常に成立し
// 誰も落ちない）。
//
// Feature: local-model-management, Property 18: Fraction の単一画像非適用
//
// Validates: Requirements 10.4
//
// *任意の* 単一画像（バッチ対象画像数 = 1）のフィルタ結果について、
// Fraction_Threshold 適用後の Adopted は適用前と一致する（出現割合による除外を
// 行わない）。
//
// 本テストは統合テスト（tests/）として tag_editor_core の公開ロジックを外部から
// 検証する。design の「各 Correctness Property は単一のプロパティテストで実装」に
// 沿い、tests/ 版を正式版とする。モジュール本体は編集しない。

use std::collections::{HashMap, HashSet};

use proptest::collection::{hash_set, vec};
use proptest::prelude::*;

use tag_editor_core::logic::tag_batch::apply_fraction_threshold;
use tag_editor_core::models::{FilterOutcome, Tag, TagFilter};

// --- 定数 -----------------------------------------------------------------

/// タグ名の小さな語彙。少数に絞って画像間の出現・非出現の衝突を起こしやすくし、
/// 割合が 0.0〜1.0 の各段階を踏ませる。
const VOCAB: &[&str] = &["a", "b", "c", "d", "e"];

// --- 参照ヘルパ -----------------------------------------------------------

/// タグ正規化キー（前後トリム＋大小無視）。実装の `normalize_key` と同一規則。
fn normalize(tag: &str) -> String {
    tag.trim().to_lowercase()
}

/// 各正規化キーの出現画像数（採用候補とした画像数）を算出する。
/// 同一画像内の重複キーは 1 画像として数える。
fn appearance_counts(images: &[Vec<Tag>]) -> HashMap<String, usize> {
    let mut appearance: HashMap<String, usize> = HashMap::new();
    for adopted in images {
        let keys: HashSet<String> = adopted.iter().map(|t| normalize(&t.body)).collect();
        for key in keys {
            *appearance.entry(key).or_insert(0) += 1;
        }
    }
    appearance
}

// --- ジェネレータ ---------------------------------------------------------

/// 語彙から 1 タグ名を選ぶ。
fn tag_name() -> impl Strategy<Value = String> {
    prop::sample::select(VOCAB.to_vec()).prop_map(String::from)
}

/// 1 画像分の Adopted タグ集合。キー重複しないよう HashSet から生成し、
/// 各タグに確信度を付ける。
fn image_adopted() -> impl Strategy<Value = Vec<Tag>> {
    hash_set(tag_name(), 0..=VOCAB.len()).prop_map(|set| {
        set.into_iter()
            .map(|name| Tag::with_confidence(name, 0.9))
            .collect()
    })
}

/// Fraction_Threshold ジェネレータ。境界（0.0/1.0）・割合ちょうどになりやすい
/// 分数値・区間内一様を混ぜる。VOCAB=5・画像数 2..=6 と組み合わせ、
/// 「割合ちょうど == 閾値」（>= 採用）の分岐を踏ませる。
fn fraction_strategy() -> impl Strategy<Value = f32> {
    prop_oneof![
        Just(0.0f32),
        Just(1.0f32),
        Just(0.5f32),
        Just(1.0f32 / 3.0),
        Just(2.0f32 / 3.0),
        Just(0.25f32),
        Just(0.75f32),
        0.0f32..=1.0f32,
    ]
}

/// Keep_Tags / Additional_Tags 集合（語彙内 0..=2 件）。適用免除（要件 10.2）を踏ませる。
fn exempt_strategy() -> impl Strategy<Value = HashSet<String>> {
    hash_set(tag_name(), 0..=2)
}

/// 割合適用のみを設定した TagFilter を組み立てる。
/// TagFilter は serde 非導出だがフィールドを直接埋めて構築できる。
fn filter_with(fraction: f32, keep: &HashSet<String>, additional: &HashSet<String>) -> TagFilter {
    TagFilter {
        keep: keep.clone(),
        exclude: vec![],
        replace: vec![],
        additional: additional.iter().cloned().collect(),
        confidence_threshold: 0.0,
        fraction_threshold: fraction,
    }
}

/// 画像ごとの Adopted 列を FilterOutcome 列へ変換する（Discarded は空）。
fn to_per_image(images: &[Vec<Tag>]) -> Vec<FilterOutcome> {
    images
        .iter()
        .map(|adopted| FilterOutcome {
            adopted: adopted.clone(),
            discarded: vec![],
        })
        .collect()
}

// --- プロパティ -----------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// Property 17 本体: 出現キーごとに「適用後も Adopted に残る ⇔ (割合 >= 閾値)
    /// または Keep または Additional」を検証する。全画像出現/一部出現・fraction の
    /// 境界（0.0/割合ちょうど/1.0）・keep/additional 該当を語彙とジェネレータで網羅する。
    #[test]
    fn property17_fraction_adoption_equivalence(
        images in vec(image_adopted(), 2..=6),
        frac in fraction_strategy(),
        keep in exempt_strategy(),
        additional in exempt_strategy(),
    ) {
        let image_count = images.len();
        let per_image = to_per_image(&images);
        let filter = filter_with(frac, &keep, &additional);

        // 適用前の出現画像数（独立参照実装）。
        let appearance = appearance_counts(&images);
        let keep_keys: HashSet<String> = keep.iter().map(|k| normalize(k)).collect();
        let add_keys: HashSet<String> = additional.iter().map(|k| normalize(k)).collect();

        let result = apply_fraction_threshold(&per_image, &filter, image_count);

        // 適用後に残る Adopted キーの和集合。
        let remaining: HashSet<String> = result
            .per_image
            .iter()
            .flat_map(|o| o.adopted.iter().map(|t| normalize(&t.body)))
            .collect();

        // 出現した全キーについて採用同値条件を検証する。
        for (key, &count) in &appearance {
            let ratio = count as f32 / image_count as f32;
            let should_remain =
                ratio >= frac || keep_keys.contains(key) || add_keys.contains(key);
            prop_assert_eq!(
                remaining.contains(key),
                should_remain,
                "採用同値違反: key={:?} count={} ratio={} frac={} keep={} add={}",
                key, count, ratio, frac,
                keep_keys.contains(key), add_keys.contains(key)
            );
        }

        // fraction_threshold == 0 なら誰も落ちない（全出現キーが残る）。
        if frac == 0.0 {
            for key in appearance.keys() {
                prop_assert!(
                    remaining.contains(key),
                    "frac==0 で落ちた: key={:?}",
                    key
                );
            }
        }

        // 移送は Adopted → Discarded のみ。適用後の (adopted ∪ discarded) は不変。
        for (before, after) in per_image.iter().zip(result.per_image.iter()) {
            let before_all: HashSet<String> = before
                .adopted
                .iter()
                .chain(before.discarded.iter())
                .map(|t| normalize(&t.body))
                .collect();
            let after_all: HashSet<String> = after
                .adopted
                .iter()
                .chain(after.discarded.iter())
                .map(|t| normalize(&t.body))
                .collect();
            prop_assert_eq!(before_all, after_all, "移送で総タグ集合が変化した");
        }
    }

    /// Property 18 本体: image_count == 1 のとき、適用後の Adopted が適用前と一致する
    /// （fraction_threshold の値によらず出現割合による除外を行わない）。
    #[test]
    fn property18_single_image_unchanged(
        adopted in image_adopted(),
        frac in fraction_strategy(),
        keep in exempt_strategy(),
        additional in exempt_strategy(),
    ) {
        let per_image = vec![FilterOutcome {
            adopted: adopted.clone(),
            discarded: vec![],
        }];
        let filter = filter_with(frac, &keep, &additional);

        let result = apply_fraction_threshold(&per_image, &filter, 1);

        prop_assert_eq!(&result.per_image[0].adopted, &adopted);
        prop_assert!(
            result.per_image[0].discarded.is_empty(),
            "単一画像で Discarded へ移送が発生した"
        );
    }
}
