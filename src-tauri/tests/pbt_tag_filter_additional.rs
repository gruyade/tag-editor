// Feature: local-model-management, Property 14: Additional_Tags の無条件付与
//
// 任意の Predicted_Tag 集合と Additional_Tags について、フィルタ適用後の
// Adopted_Tags は Additional_Tags のすべてのタグを含む。
//
// apply_filter は Additional_Tags を Tag::new（confidence None）で無条件付与する。
// よって adopted のタグ body 集合に additional の各文字列が含まれることを検証する。
// keep / exclude / replace / confidence_threshold も変化させ、これらの設定に依らず
// Additional が常に採用されることを確認する。
//
// Validates: Requirements 9.6

use std::collections::HashSet;

use proptest::prelude::*;
use tag_editor_core::logic::tag_filter::{apply_filter, compile_filter};
use tag_editor_core::models::{RawTagFilter, Tag};

/// タグ名の文字プール（ASCII・記号・非 ASCII を含む）。
/// 正規表現メタ文字は含めない（exclude/replace の有効パターンを保つため）。
fn tag_name_strategy() -> impl Strategy<Value = String> {
    prop::collection::vec(
        prop::sample::select(vec![
            'a', 'b', 'c', 'Z', 'M', '0', '7', '_', '-', ' ', '猫', '髪', 'あ',
        ]),
        1..12,
    )
    .prop_map(|v| v.into_iter().collect::<String>())
    // 空・空白のみは正規化キーが空になりうるので最低 1 文字の非空白を保証。
    .prop_filter("非空白を含む", |s| s.trim().chars().next().is_some())
}

/// Predicted_Tag 生成器（確信度は None / 0.0〜1.0 を混在）。
fn predicted_strategy() -> impl Strategy<Value = Vec<Tag>> {
    let one = (
        tag_name_strategy(),
        prop::option::of(0.0f32..=1.0f32),
    )
        .prop_map(|(body, conf)| match conf {
            Some(c) => Tag::with_confidence(body, c),
            None => Tag::new(body),
        });
    prop::collection::vec(one, 0..8)
}

/// Additional_Tags 生成器（0〜6 個、任意のタグ名）。
fn additional_strategy() -> impl Strategy<Value = Vec<String>> {
    prop::collection::vec(tag_name_strategy(), 0..6)
}

/// keep / exclude / replace 生成器（すべて有効パターン）。
fn keep_strategy() -> impl Strategy<Value = Vec<String>> {
    prop::collection::vec(tag_name_strategy(), 0..4)
}

fn exclude_strategy() -> impl Strategy<Value = Vec<String>> {
    // メタ文字を含まない素朴なパターン（全体一致・大小無視でコンパイルされる）。
    prop::collection::vec(tag_name_strategy(), 0..4)
}

fn replace_strategy() -> impl Strategy<Value = Vec<(String, String)>> {
    prop::collection::vec((tag_name_strategy(), tag_name_strategy()), 0..3)
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(200))]

    /// Property 14: adopted の body 集合は additional の全タグを含む。
    ///
    /// keep / exclude / replace / confidence_threshold を任意に変化させても、
    /// Additional_Tags は無条件に採用されるため adopted に必ず現れる。
    #[test]
    fn adopted_contains_all_additional(
        predicted in predicted_strategy(),
        additional in additional_strategy(),
        keep in keep_strategy(),
        exclude in exclude_strategy(),
        replace in replace_strategy(),
        threshold in 0.0f32..=1.0f32,
    ) {
        let raw = RawTagFilter {
            keep,
            exclude,
            replace,
            additional: additional.clone(),
            confidence_threshold: threshold,
            fraction_threshold: 0.0,
        };
        let (filter, invalid) = compile_filter(raw);
        // 生成器はメタ文字を含まないため無効パターンは出ない前提。
        prop_assert!(invalid.is_empty(), "想定外の無効パターン: {invalid:?}");

        let out = apply_filter(&filter, &predicted);

        let adopted_bodies: HashSet<&str> =
            out.adopted.iter().map(|t| t.body.as_str()).collect();

        // additional の各タグ（文字列そのまま）が adopted の body 集合に含まれる。
        for extra in &additional {
            prop_assert!(
                adopted_bodies.contains(extra.as_str()),
                "Additional_Tag {:?} が adopted に含まれない: adopted={:?}",
                extra,
                out.adopted
            );
        }
    }
}

/// 空の predicted・空フィルタでも additional は全て採用される（境界例）。
#[test]
fn empty_predicted_still_adopts_additional() {
    let raw = RawTagFilter {
        keep: Vec::new(),
        exclude: Vec::new(),
        replace: Vec::new(),
        additional: vec!["extra1".to_string(), "extra2".to_string()],
        confidence_threshold: 1.0,
        fraction_threshold: 0.0,
    };
    let (filter, invalid) = compile_filter(raw);
    assert!(invalid.is_empty());

    let out = apply_filter(&filter, &[]);
    let bodies: HashSet<&str> = out.adopted.iter().map(|t| t.body.as_str()).collect();
    assert!(bodies.contains("extra1"));
    assert!(bodies.contains("extra2"));
    // Additional は確信度なしで付与される。
    assert!(out.adopted.iter().all(|t| t.confidence.is_none()));
}

/// additional が exclude と一致するタグ名でも無条件採用される（境界例）。
#[test]
fn additional_adopted_even_if_matching_exclude() {
    let raw = RawTagFilter {
        keep: Vec::new(),
        exclude: vec!["banned".to_string()],
        replace: Vec::new(),
        additional: vec!["banned".to_string()],
        confidence_threshold: 0.0,
        fraction_threshold: 0.0,
    };
    let (filter, invalid) = compile_filter(raw);
    assert!(invalid.is_empty());

    let out = apply_filter(&filter, &[]);
    let bodies: HashSet<&str> = out.adopted.iter().map(|t| t.body.as_str()).collect();
    assert!(bodies.contains("banned"));
}
