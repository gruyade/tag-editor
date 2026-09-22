// Feature: local-model-management, Property 11: Keep_Tags の無条件採用（Keep 優先）
//
// Property 11: 任意の Predicted_Tag 集合と Tag_Filter について、Replace 適用後の
// タグ名が Keep_Tags に含まれるタグは、Confidence_Threshold 未満・Exclude_Rules
// 該当のいずれであっても Adopted_Tags に採用される。
//
// Keep は Replace 書換後のタグ名の正規化キー（前後トリム＋大小無視）で判定される。
// Keep に一致するタグは Confidence / Exclude の判定より優先され無条件 Adopted。
//
// Validates: Requirements 9.3, 9.8

use proptest::prelude::*;
use tag_editor_core::logic::tag_filter::{apply_filter, compile_filter};
use tag_editor_core::models::{RawTagFilter, Tag};

/// タグ名の生成器。
///
/// Keep 判定は正規化キー（trim + lowercase）で行われ、Exclude パターンは
/// タグ全体一致・大小無視でコンパイルされる。正規表現メタ文字を含めると
/// Exclude パターンとしての全体一致が壊れるため、ここでは英数・アンダースコア・
/// 空白に限定した「素直なタグ名」を生成する。大文字・前後空白を混ぜて
/// 正規化経路も踏ませる。
fn tag_name_strategy() -> impl Strategy<Value = String> {
    prop_oneof![
        "[A-Za-z0-9_]{1,12}",
        "[A-Za-z0-9_]{1,8} [A-Za-z0-9_]{1,8}",
        " *[A-Za-z0-9_]{1,10} *",
        "[A-Z][a-z]{1,8}",
    ]
}

/// confidence の生成器。閾値近傍・境界・None を含める。
fn confidence_strategy() -> impl Strategy<Value = Option<f32>> {
    prop_oneof![
        Just(None),
        Just(Some(0.0f32)),
        Just(Some(1.0f32)),
        (0.0f32..=1.0f32).prop_map(Some),
    ]
}

/// Keep に入れるタグ名の生成器（1〜5 個）。
fn keep_names_strategy() -> impl Strategy<Value = Vec<String>> {
    prop::collection::vec(tag_name_strategy(), 1..=5)
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(200))]

    /// Property 11: Keep に含まれるタグ名（正規化後）は、閾値未満でも Exclude 該当でも
    /// adopted に含まれる。
    ///
    /// 構成:
    /// - keep_names を Keep_Tags に設定する。
    /// - 同じ名前を Exclude_Rules にも入れる（Exclude 該当ケースを踏ませる）。
    /// - confidence_threshold を高め（1.0 近傍）に設定し、各 Keep タグに低〜任意の
    ///   confidence（None 含む）を与える（閾値未満ケースを踏ませる）。
    /// - Keep でないノイズタグも混ぜる。
    ///
    /// 検証: 全 Keep タグは（正規化キーで）adopted に現れる。
    #[test]
    fn keep_tags_are_unconditionally_adopted(
        keep_names in keep_names_strategy(),
        confidences in prop::collection::vec(confidence_strategy(), 1..=5),
        threshold in 0.5f32..=1.0f32,
        also_exclude in any::<bool>(),
        noise in prop::collection::vec(tag_name_strategy(), 0..4),
    ) {
        // Keep_Tags に設定するタグ名。
        let keep = keep_names.clone();

        // Exclude にも同名を入れて「Keep かつ Exclude 該当」を踏ませる。
        // Exclude パターンはタグ全体一致・大小無視でコンパイルされるため、
        // 生成器が英数・空白・アンダースコアのみである前提で全体一致する。
        let exclude = if also_exclude { keep_names.clone() } else { Vec::new() };

        let raw = RawTagFilter {
            keep,
            exclude,
            replace: Vec::new(),
            additional: Vec::new(),
            confidence_threshold: threshold,
            fraction_threshold: 0.0,
        };
        let (filter, invalid) = compile_filter(raw);
        // 素直なタグ名のみのため無効パターンは出ない前提。
        prop_assert!(invalid.is_empty(), "予期しない無効パターン: {invalid:?}");

        // Predicted_Tag: 各 Keep 名に confidence（低め/None 含む）を割り当てる。
        let mut predicted: Vec<Tag> = keep_names
            .iter()
            .enumerate()
            .map(|(i, name)| {
                let c = confidences[i % confidences.len()];
                match c {
                    Some(v) => Tag::with_confidence(name.clone(), v),
                    None => Tag::new(name.clone()),
                }
            })
            .collect();
        // Keep でないノイズタグを追加（Keep 集合に含まれない名前でも許容）。
        for n in &noise {
            predicted.push(Tag::with_confidence(n.clone(), 0.01));
        }

        let out = apply_filter(&filter, &predicted);

        // 検証: 全 Keep 名は正規化キーで adopted に現れる。
        let adopted_keys: std::collections::HashSet<String> = out
            .adopted
            .iter()
            .map(|t| t.body.trim().to_lowercase())
            .collect();

        for name in &keep_names {
            let key = name.trim().to_lowercase();
            prop_assert!(
                adopted_keys.contains(&key),
                "Keep タグ {name:?}（キー {key:?}）が adopted に含まれない。threshold={threshold}, also_exclude={also_exclude}, adopted={:?}",
                out.adopted
            );
        }

        // Keep タグは discarded に落ちていない（Keep 優先）。
        let discarded_keys: std::collections::HashSet<String> = out
            .discarded
            .iter()
            .map(|t| t.body.trim().to_lowercase())
            .collect();
        for name in &keep_names {
            let key = name.trim().to_lowercase();
            prop_assert!(
                !discarded_keys.contains(&key),
                "Keep タグ {name:?}（キー {key:?}）が discarded に混入した"
            );
        }
    }

    /// Property 11（Replace 経由）: Replace で書き換えた結果が Keep に一致すると、
    /// 元名が Keep でなく Exclude 該当・閾値未満でも adopted に採用される。
    ///
    /// 構成: 元名 src を書換先 dst へ replace し、dst を Keep に入れる。
    /// dst を Exclude にも入れ、閾値を高めに、confidence を低めに設定する。
    #[test]
    fn replace_then_keep_is_adopted(
        src in "[A-Za-z0-9_]{1,10}",
        dst in "[A-Za-z0-9_]{1,10}",
        low_conf in 0.0f32..0.4f32,
        threshold in 0.6f32..=1.0f32,
    ) {
        // src と dst の正規化キーが異なる場合のみ「書換で初めて Keep 一致」を検証できる。
        prop_assume!(src.trim().to_lowercase() != dst.trim().to_lowercase());

        let raw = RawTagFilter {
            keep: vec![dst.clone()],
            exclude: vec![dst.clone()], // 書換後名が Exclude にも該当
            replace: vec![(src.clone(), dst.clone())],
            additional: Vec::new(),
            confidence_threshold: threshold,
            fraction_threshold: 0.0,
        };
        let (filter, invalid) = compile_filter(raw);
        prop_assert!(invalid.is_empty(), "予期しない無効パターン: {invalid:?}");

        // 元名 src（Keep でない・低 confidence）を予測タグとして与える。
        let predicted = vec![Tag::with_confidence(src.clone(), low_conf)];
        let out = apply_filter(&filter, &predicted);

        let dst_key = dst.trim().to_lowercase();
        // 書換後名 dst が adopted に現れる。
        prop_assert!(
            out.adopted.iter().any(|t| t.body.trim().to_lowercase() == dst_key),
            "Replace 書換後の Keep タグ {dst:?} が adopted に含まれない。adopted={:?}",
            out.adopted
        );
        // 元名のまま discarded に残っていない。
        prop_assert!(
            out.discarded.is_empty(),
            "Keep 優先のはずが discarded に残った: {:?}",
            out.discarded
        );
    }
}

/// 代表的な固定ケース: 閾値未満かつ Exclude 該当の Keep タグが採用される。
#[test]
fn representative_keep_beats_threshold_and_exclude() {
    let raw = RawTagFilter {
        keep: vec!["solo".to_string(), "Long_Hair".to_string()],
        exclude: vec!["solo".to_string()],
        replace: Vec::new(),
        additional: Vec::new(),
        confidence_threshold: 0.9,
        fraction_threshold: 0.0,
    };
    let (filter, invalid) = compile_filter(raw);
    assert!(invalid.is_empty());

    let predicted = vec![
        Tag::with_confidence("solo", 0.05), // 閾値未満 + Exclude 該当
        Tag::new("long_hair"),              // confidence なし（大小違い）
    ];
    let out = apply_filter(&filter, &predicted);

    let adopted: Vec<String> = out.adopted.iter().map(|t| t.body.clone()).collect();
    assert!(adopted.contains(&"solo".to_string()));
    assert!(adopted.contains(&"long_hair".to_string()));
    assert!(out.discarded.is_empty());
}
