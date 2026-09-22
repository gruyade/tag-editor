// Feature: local-model-management, Property 13: Replace_Rules の採用判定前適用
//
// Validates: Requirements 9.5
//
// *任意の* 元タグ名 A・Replace ルール (A→B)・確信度について、採用判定
// （Keep / Exclude / Threshold）は書き換え後のタグ名 B に基づいて行われる。
// すなわち:
//   (1) B が Keep_Tags に一致すれば、A は確信度に関わらず（閾値未満でも）
//       Adopted に B として含まれる。
//   (2) B が Exclude_Rules に一致し、かつ Keep でないなら、A は Discarded に
//       B として含まれる。
// いずれの場合も、結果タグの body は書き換え後の B である（元の A ではない）。
//
// Replace の検索パターンはタグ全体一致（`^...$`）・大小無視でコンパイルされるため、
// A 全体が B に置換される点に注意する。
//
// 本テストは統合テスト（tests/）として tag_editor_core::logic::tag_filter の
// compile_filter / apply_filter を外部から検証する。モジュール本体は編集しない。

use proptest::prelude::*;

use tag_editor_core::logic::tag_filter::{apply_filter, compile_filter};
use tag_editor_core::models::{RawTagFilter, Tag};

/// 空の生設定を作る（各テストで必要フィールドだけ差し替える）。
fn raw() -> RawTagFilter {
    RawTagFilter {
        keep: Vec::new(),
        exclude: Vec::new(),
        replace: Vec::new(),
        additional: Vec::new(),
        confidence_threshold: 0.0,
        fraction_threshold: 0.0,
    }
}

/// 正規表現メタ文字を含まない単純なタグ名ジェネレータ。
///
/// Replace の検索パターンは A 全体一致なので、A にメタ文字が入ると意図せぬ
/// 挙動になる。ここでは英数字・アンダースコア・空白のみの語彙に限定し、
/// 「A 全体を B に置換する」書換を確実に成立させる。
fn plain_name() -> impl Strategy<Value = String> {
    prop_oneof![
        Just("cat".to_string()),
        Just("dog".to_string()),
        Just("1girl".to_string()),
        Just("long hair".to_string()),
        Just("smile".to_string()),
        Just("solo".to_string()),
        Just("feline".to_string()),
        Just("banned".to_string()),
        Just("blue_sky".to_string()),
    ]
}

/// 確信度ジェネレータ（0.0/1.0/中間・境界近傍をカバー）。
fn confidence() -> impl Strategy<Value = f32> {
    prop_oneof![
        Just(0.0f32),
        Just(1.0f32),
        Just(0.5f32),
        0.0f32..=1.0f32,
    ]
}

/// 閾値ジェネレータ。
fn threshold() -> impl Strategy<Value = f32> {
    prop_oneof![Just(0.0f32), Just(0.5f32), Just(0.9f32), Just(1.0f32)]
}

/// テスト側の独立正規化（実装 normalize_key と同規則: 前後トリム＋小文字化）。
fn ref_normalize(s: &str) -> String {
    s.trim().to_lowercase()
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// (1) B が Keep に一致するケース。
    ///
    /// Replace (A→B) を適用後、書換後名 B が Keep_Tags に含まれるので、A は
    /// 確信度が閾値未満でも無条件 Adopted となり、body は B になる。元名 A は
    /// Keep でも Exclude でもない前提（A≠B）。
    #[test]
    fn replace_then_keep_adopts_as_rewritten(
        a in plain_name(),
        b in plain_name(),
        conf in confidence(),
        thr in threshold(),
    ) {
        // A と B が正規化後同一だと「書換前後で判定が変わる」ことを検証できない。
        prop_assume!(ref_normalize(&a) != ref_normalize(&b));

        let mut r = raw();
        r.replace = vec![(a.clone(), b.clone())];
        r.keep = vec![b.clone()];
        // 閾値は高めでも Keep なら採用される（閾値未満でも採用を検証）。
        r.confidence_threshold = thr;

        let (filter, invalid) = compile_filter(r);
        prop_assert!(invalid.is_empty(), "テスト用パターンが無効: {:?}", invalid);

        let out = apply_filter(&filter, &[Tag::with_confidence(a.clone(), conf)]);

        // A は書換後 B として Adopted に含まれる。
        let adopted: Vec<&str> = out.adopted.iter().map(|t| t.body.as_str()).collect();
        prop_assert!(
            adopted.contains(&b.as_str()),
            "A={:?} → B={:?} が Keep で Adopted に含まれない: adopted={:?}",
            a, b, adopted
        );
        // 元名 A（B と異なる）は Adopted にも Discarded にも現れない。
        prop_assert!(!adopted.contains(&a.as_str()), "元名 A が残っている: {:?}", a);
        prop_assert!(
            out.discarded.iter().all(|t| t.body != a),
            "元名 A が Discarded に残っている: {:?}", a
        );
    }

    /// (2) B が Exclude に一致し Keep でないケース。
    ///
    /// Replace (A→B) を適用後、書換後名 B が Exclude_Rules に一致し Keep でない
    /// ため、A は Discarded に B として含まれる（確信度が高くても Exclude 優先）。
    #[test]
    fn replace_then_exclude_discards_as_rewritten(
        a in plain_name(),
        b in plain_name(),
        conf in confidence(),
        thr in threshold(),
    ) {
        prop_assume!(ref_normalize(&a) != ref_normalize(&b));

        let mut r = raw();
        r.replace = vec![(a.clone(), b.clone())];
        r.exclude = vec![b.clone()];
        r.confidence_threshold = thr;

        let (filter, invalid) = compile_filter(r);
        prop_assert!(invalid.is_empty(), "テスト用パターンが無効: {:?}", invalid);

        let out = apply_filter(&filter, &[Tag::with_confidence(a.clone(), conf)]);

        // A は書換後 B として Discarded に含まれる。
        let discarded: Vec<&str> = out.discarded.iter().map(|t| t.body.as_str()).collect();
        prop_assert!(
            discarded.contains(&b.as_str()),
            "A={:?} → B={:?} が Exclude で Discarded に含まれない: discarded={:?}",
            a, b, discarded
        );
        // Adopted に B は現れない（Exclude 優先）。
        prop_assert!(
            out.adopted.iter().all(|t| t.body != b),
            "Exclude 該当の B が Adopted に含まれる: {:?}", b
        );
        // 元名 A は残らない。
        prop_assert!(!discarded.contains(&a.as_str()), "元名 A が残っている: {:?}", a);
    }

    /// 書換後名の確信度が保持されることの補強。
    ///
    /// Keep も Exclude も無い純粋な Replace で、A→B 書換後の Adopted タグは
    /// body が B、confidence が元の確信度を保持する（閾値 0 で必ず採用）。
    #[test]
    fn replace_preserves_confidence_on_rewritten_body(
        a in plain_name(),
        b in plain_name(),
        conf in 0.0f32..=1.0f32,
    ) {
        prop_assume!(ref_normalize(&a) != ref_normalize(&b));

        let mut r = raw();
        r.replace = vec![(a.clone(), b.clone())];
        r.confidence_threshold = 0.0; // 確信度 0 でも採用される境界

        let (filter, invalid) = compile_filter(r);
        prop_assert!(invalid.is_empty());

        let out = apply_filter(&filter, &[Tag::with_confidence(a.clone(), conf)]);

        prop_assert_eq!(out.adopted.len(), 1);
        prop_assert_eq!(out.adopted[0].body.as_str(), b.as_str());
        prop_assert_eq!(out.adopted[0].confidence, Some(conf));
        prop_assert!(out.discarded.is_empty());
    }
}

// --- 固定の代表ケース（プロパティを補完） -----------------------------------

// Replace 後の B が大小違いで Keep に一致（正規化キー比較）。
#[test]
fn replace_then_keep_is_case_insensitive() {
    let mut r = raw();
    r.replace = vec![("cat".to_string(), "Feline".to_string())];
    r.keep = vec!["feline".to_string()]; // 小文字 Keep が大文字書換後名に一致
    r.confidence_threshold = 0.9;

    let (filter, invalid) = compile_filter(r);
    assert!(invalid.is_empty());

    // 閾値未満でも Keep で採用。
    let out = apply_filter(&filter, &[Tag::with_confidence("cat", 0.1)]);
    let adopted: Vec<&str> = out.adopted.iter().map(|t| t.body.as_str()).collect();
    assert_eq!(adopted, vec!["Feline"]);
    assert!(out.discarded.is_empty());
}

// Replace の検索は A 全体一致（^...$）: 部分一致では書換されない。
#[test]
fn replace_matches_whole_tag_only() {
    let mut r = raw();
    r.replace = vec![("cat".to_string(), "feline".to_string())];
    r.confidence_threshold = 0.0;

    let (filter, invalid) = compile_filter(r);
    assert!(invalid.is_empty());

    // "cat ears" は "cat" 全体一致でないため書換されない。
    let out = apply_filter(&filter, &[Tag::with_confidence("cat ears", 0.5)]);
    let adopted: Vec<&str> = out.adopted.iter().map(|t| t.body.as_str()).collect();
    assert_eq!(adopted, vec!["cat ears"]);
}
