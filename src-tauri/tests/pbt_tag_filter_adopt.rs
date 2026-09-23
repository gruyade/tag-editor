// Feature: local-model-management, Property 12: 非 Keep タグの採用同値条件
//
// Validates: Requirements 9.2, 9.4
//
// *任意の* Predicted_Tag と Tag_Filter について、Keep_Tags に含まれないタグが
// Adopted_Tags に採用されることは「Exclude_Rules のいずれにも一致せず、かつ確信度が
// Confidence_Threshold 以上である」ことと同値である。
//
// 注意: apply_filter の実装では confidence None のタグは「閾値未満」とみなし
// Discarded とする。よって同値条件の「確信度が閾値以上」は None のとき偽になる。
// テスト側の期待述語もこの実装挙動（None は閾値未満扱い）に合わせる。
//
// 本テストは統合テスト（tests/）として tag_editor_core の公開ロジックを外部から
// 検証する。モジュール本体は編集しない。Replace は空とし、書換の影響は Property 13
// で別途検証する。

use proptest::collection::vec;
use proptest::prelude::*;
use regex::RegexBuilder;

use tag_editor_core::logic::tag_filter::{apply_filter, compile_filter};
use tag_editor_core::models::{RawTagFilter, Tag};

// --- 定数 -----------------------------------------------------------------

/// 閾値ちょうどの境界値。ジェネレータで confidence にこの値を混ぜ、
/// inclusive（>=）判定が正しく採用側になることを踏ませる。
const THRESHOLD: f32 = 0.5;

// --- ジェネレータ ---------------------------------------------------------

/// タグ本体の小さな語彙。exclude パターンと重ねて一致・不一致の両分岐を踏ませる。
fn body_strategy() -> impl Strategy<Value = String> {
    prop_oneof![
        Just("cat".to_string()),
        Just("dog".to_string()),
        Just("smile".to_string()),
        Just("1girl".to_string()),
        Just("Long_Hair".to_string()), // 大文字混在（正規化・大小無視の検証）
    ]
}

/// confidence ジェネレータ。境界（0.0/1.0/閾値ちょうど）・閾値近傍・None を網羅する。
fn confidence_strategy() -> impl Strategy<Value = Option<f32>> {
    prop_oneof![
        Just(None),
        Just(Some(0.0f32)),
        Just(Some(1.0f32)),
        Just(Some(THRESHOLD)),         // 閾値ちょうど（inclusive で採用）
        Just(Some(THRESHOLD - 0.001)), // 閾値直下（Discarded）
        Just(Some(THRESHOLD + 0.001)), // 閾値直上（採用）
        (0.0f32..=1.0f32).prop_map(Some),
    ]
}

/// 非 Keep タグ 1 件のジェネレータ。keep には決して入れないタグ集合を生成する。
fn tag_strategy() -> impl Strategy<Value = Tag> {
    (body_strategy(), confidence_strategy()).prop_map(|(body, confidence)| Tag { body, confidence })
}

/// Predicted_Tag 列（空集合を含む 0..=6 件）。
///
/// 実運用では推論（[`run_labeled_inference`]）がラベル定義と 1 対 1 で
/// タグを生成するため、1 バッチ内の Predicted_Tag に同一タグ名が複数の
/// 確信度で重複出現することはない。apply_filter はこの前提（predicted 内で
/// タグ名が一意）に基づき実装されており、同名重複がある場合の Adopted/
/// Discarded 分類は未定義（dedup_by_key が Adopted 側の 2 件目以降を除去する
/// が Discarded 側には移さないため、消滅した扱いになる）。テストの入力も
/// 実運用条件に合わせ、正規化キー（大小無視）でタグ名が一意になるよう
/// フィルタする。
fn tag_vec_strategy() -> impl Strategy<Value = Vec<Tag>> {
    use tag_editor_core::logic::tag_ops::normalize_key;
    vec(tag_strategy(), 0..=6).prop_map(|tags| {
        let mut seen = std::collections::HashSet::new();
        tags.into_iter()
            .filter(|t| seen.insert(normalize_key(&t.body)))
            .collect()
    })
}

/// Exclude パターン列（0..=3 件）。語彙内・語彙外を混ぜ、一致/不一致を踏ませる。
fn exclude_strategy() -> impl Strategy<Value = Vec<String>> {
    let pattern = prop_oneof![
        Just("cat".to_string()),
        Just("smile".to_string()),
        Just("long_hair".to_string()), // 大小無視で "Long_Hair" に一致
        Just("no_such_tag".to_string()),
    ];
    vec(pattern, 0..=3)
}

/// Confidence_Threshold ジェネレータ。閾値ちょうど・境界値を含む。
fn threshold_strategy() -> impl Strategy<Value = f32> {
    prop_oneof![
        Just(0.0f32),
        Just(1.0f32),
        Just(THRESHOLD),
        (0.0f32..=1.0f32),
    ]
}

// --- 参照実装 -------------------------------------------------------------

/// Exclude パターンが本体に一致するかの独立参照実装。
/// 実装（compile_pattern）と同じ規則「タグ全体一致（`^...$`）・大小無視」で再判定する。
fn ref_excluded(exclude: &[String], body: &str) -> bool {
    exclude.iter().any(|p| {
        let anchored = format!("^{p}$");
        RegexBuilder::new(&anchored)
            .case_insensitive(true)
            .build()
            .map(|re| re.is_match(body))
            .unwrap_or(false)
    })
}

/// 非 Keep タグの採用同値条件の参照述語。
/// 「Exclude いずれにも不一致」かつ「confidence が Some かつ threshold 以上」。
/// None は閾値未満扱い（実装挙動に合わせる）で偽。
fn ref_adopted(exclude: &[String], threshold: f32, tag: &Tag) -> bool {
    let not_excluded = !ref_excluded(exclude, &tag.body);
    let at_or_above = matches!(tag.confidence, Some(c) if c >= threshold);
    not_excluded && at_or_above
}

// --- プロパティ -----------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// Property 12 本体: keep が空（＝すべて非 Keep）のとき、各予測タグについて
    /// 「adopted に含まれる ⇔ (Exclude いずれにも不一致) かつ (confidence Some かつ
    /// threshold 以上)」が成り立つ。
    #[test]
    fn non_keep_adoption_equivalence(
        predicted in tag_vec_strategy(),
        exclude in exclude_strategy(),
        threshold in threshold_strategy(),
    ) {
        let raw = RawTagFilter {
            keep: Vec::new(),
            exclude: exclude.clone(),
            replace: Vec::new(),
            additional: Vec::new(),
            confidence_threshold: threshold,
            fraction_threshold: 0.0,
        };
        let (filter, invalid) = compile_filter(raw);
        prop_assert!(invalid.is_empty(), "テスト用 exclude が無効: {invalid:?}");

        let out = apply_filter(&filter, &predicted);

        // Additional は空なので adopted はすべて予測由来。各予測タグについて
        // adopted 集合内の有無と参照述語が同値であることを検証する。
        for tag in &predicted {
            let in_adopted = out.adopted.contains(tag);
            let in_discarded = out.discarded.contains(tag);
            let expected = ref_adopted(&exclude, threshold, tag);

            prop_assert_eq!(
                in_adopted,
                expected,
                "採用同値違反: tag={:?} exclude={:?} threshold={} in_adopted={} expected={}",
                tag, exclude, threshold, in_adopted, expected
            );
            // 非 Keep タグは adopted か discarded のいずれか一方に必ず分類される。
            prop_assert_eq!(
                in_adopted, !in_discarded,
                "分類の排他違反: tag={:?} adopted={} discarded={}",
                tag, in_adopted, in_discarded
            );
        }
    }

    /// 確信度 None は（Exclude 不一致でも）常に非採用（閾値未満扱い）。
    #[test]
    fn confidence_none_never_adopted_when_non_keep(
        bodies in vec(body_strategy(), 0..=6),
        threshold in threshold_strategy(),
    ) {
        let predicted: Vec<Tag> = bodies.iter().map(Tag::new).collect();
        let raw = RawTagFilter {
            keep: Vec::new(),
            exclude: Vec::new(), // Exclude 不一致に固定して None 単独の効果を見る
            replace: Vec::new(),
            additional: Vec::new(),
            confidence_threshold: threshold,
            fraction_threshold: 0.0,
        };
        let (filter, invalid) = compile_filter(raw);
        prop_assert!(invalid.is_empty());

        let out = apply_filter(&filter, &predicted);
        for tag in &predicted {
            prop_assert!(
                !out.adopted.contains(tag),
                "confidence None が採用された: tag={:?} threshold={}",
                tag, threshold
            );
        }
    }
}
