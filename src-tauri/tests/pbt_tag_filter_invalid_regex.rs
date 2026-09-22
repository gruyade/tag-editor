// Feature: local-model-management, Property 15: 無効な正規表現の非適用と通知
//
// Validates: Requirements 9.7
//
// *任意の* Exclude_Rules / Replace_Rules パターン列（有効・無効を混在）について、
// compile_filter の結果は次を満たす:
//   (1) コンパイル済み TagFilter には無効なパターンが 1 つも含まれない
//       （filter.exclude / filter.replace の件数は有効パターン数と一致する）。
//   (2) 無効なパターンはすべて通知集合（InvalidPattern）に含まれる。
//   (3) 無効なパターンは採用判定に影響を与えない（TagFilter へ混入しない）。
//
// 検索パターンは実装（compile_pattern）と同じく「タグ全体一致（`^...$`）・
// 大文字小文字無視」でコンパイルされる。よって有効/無効の判定も同規則で行う。
//
// 本テストは統合テスト（tests/）として tag_editor_core::logic::tag_filter の
// compile_filter を外部から検証する。モジュール本体は編集しない。

use proptest::collection::vec;
use proptest::prelude::*;
use regex::RegexBuilder;

use tag_editor_core::logic::tag_filter::compile_filter;
use tag_editor_core::models::RawTagFilter;

// --- ジェネレータ ---------------------------------------------------------

/// 有効パターン語彙。英数字・アンカー・文字クラス・メタ文字など、
/// `^...$` ラップ後に確実にコンパイル成功するものだけを列挙する。
fn valid_pattern() -> impl Strategy<Value = String> {
    prop_oneof![
        Just("cat".to_string()),
        Just("dog".to_string()),
        Just("1girl".to_string()),
        Just("long_hair".to_string()),
        Just("b.*c".to_string()),
        Just("[abc]".to_string()),
        Just("\\d+".to_string()),
        Just("^solo$".to_string()),
        Just("smile".to_string()),
    ]
}

/// 無効パターン語彙。`^...$` ラップ後もコンパイルに失敗するものだけを列挙する。
///
/// 除外した候補（`^...$` ラップで有効化されるため無効語彙に含めない）:
/// - `"\\"` → `^\\$` は「バックスラッシュ1文字にマッチ」で有効。
/// - `"*"`  → `^*$` は本 regex クレートでは有効（`^` を繰り返し対象と解釈）。
fn invalid_pattern() -> impl Strategy<Value = String> {
    prop_oneof![
        Just("[".to_string()),
        Just("(".to_string()),
        Just("(?P<".to_string()),
        Just("(?<".to_string()),
        Just("[z-a]".to_string()), // 逆順レンジ
    ]
}

/// 有効・無効を混在させたパターン 1 件。
fn mixed_pattern() -> impl Strategy<Value = String> {
    prop_oneof![valid_pattern(), invalid_pattern()]
}

/// exclude 用の混在パターン列（0..=8 件、空を含む）。
fn exclude_strategy() -> impl Strategy<Value = Vec<String>> {
    vec(mixed_pattern(), 0..=8)
}

/// replace 用の (検索パターン, 置換文字列) 列（0..=8 件、空を含む）。
/// 検索パターンのみ有効/無効を分ける。置換文字列は固定で問題ない。
fn replace_strategy() -> impl Strategy<Value = Vec<(String, String)>> {
    vec(mixed_pattern().prop_map(|p| (p, "x".to_string())), 0..=8)
}

// --- 参照実装 -------------------------------------------------------------

/// パターンが有効かどうかを独立に判定する。
///
/// 実装（compile_pattern / anchor_pattern）と同じ規則で `^...$` にラップし、
/// case_insensitive でコンパイル可能かを確かめる。実装と同一挙動になるよう
/// 既存アンカーの二重付与も避ける。
fn ref_is_valid(pattern: &str) -> bool {
    let anchored = ref_anchor(pattern);
    RegexBuilder::new(&anchored)
        .case_insensitive(true)
        .build()
        .is_ok()
}

/// 実装 anchor_pattern と同規則で `^...$` ラップ（既存アンカーは二重付与しない）。
fn ref_anchor(pattern: &str) -> String {
    let has_start = pattern.starts_with('^');
    let has_end = pattern.ends_with('$');
    match (has_start, has_end) {
        (true, true) => pattern.to_string(),
        (true, false) => format!("{pattern}$"),
        (false, true) => format!("^{pattern}"),
        (false, false) => format!("^{pattern}$"),
    }
}

// --- プロパティ -----------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// Property 15 本体: 無効パターンは TagFilter に残らず、すべて通知集合に入る。
    #[test]
    fn invalid_never_in_filter_all_in_notification(
        exclude in exclude_strategy(),
        replace in replace_strategy(),
    ) {
        // 参照実装で期待件数を算出する。
        let valid_exclude = exclude.iter().filter(|p| ref_is_valid(p)).count();
        let invalid_exclude = exclude.len() - valid_exclude;
        let valid_replace = replace.iter().filter(|(p, _)| ref_is_valid(p)).count();
        let invalid_replace = replace.len() - valid_replace;

        let raw = RawTagFilter {
            keep: Vec::new(),
            exclude: exclude.clone(),
            replace: replace.clone(),
            additional: Vec::new(),
            confidence_threshold: 0.0,
            fraction_threshold: 0.0,
        };

        let (filter, invalid) = compile_filter(raw);

        // (1) TagFilter に含まれるのは有効パターンのみ（件数一致）。
        prop_assert_eq!(
            filter.exclude.len(), valid_exclude,
            "exclude: 有効件数不一致 filter={} expected={} (exclude={:?})",
            filter.exclude.len(), valid_exclude, exclude
        );
        prop_assert_eq!(
            filter.replace.len(), valid_replace,
            "replace: 有効件数不一致 filter={} expected={} (replace={:?})",
            filter.replace.len(), valid_replace, replace
        );

        // (2) 通知集合の総数は無効パターン数（exclude 側 + replace 側）と一致。
        prop_assert_eq!(
            invalid.len(), invalid_exclude + invalid_replace,
            "通知集合の件数不一致 invalid={} expected={}",
            invalid.len(), invalid_exclude + invalid_replace
        );

        // (3) 通知集合の各要素は「実際に無効」なパターンで、理由は非空。
        for ip in &invalid {
            prop_assert!(
                !ref_is_valid(&ip.pattern),
                "有効なパターンが通知集合に混入: {:?}", ip.pattern
            );
            prop_assert!(!ip.reason.is_empty(), "無効理由が空: {:?}", ip.pattern);
        }

        // (4) 入力中のすべての無効パターンが通知集合に少なくとも 1 回現れる。
        for p in exclude.iter().chain(replace.iter().map(|(p, _)| p)) {
            if !ref_is_valid(p) {
                prop_assert!(
                    invalid.iter().any(|ip| &ip.pattern == p),
                    "無効パターン {:?} が通知集合に含まれない", p
                );
            }
        }
    }

    /// すべて有効なパターンなら通知集合は空で、全件がフィルタに残る。
    #[test]
    fn all_valid_yields_no_notification(
        exclude in vec(valid_pattern(), 0..=6),
        replace in vec(valid_pattern().prop_map(|p| (p, "x".to_string())), 0..=6),
    ) {
        let raw = RawTagFilter {
            keep: Vec::new(),
            exclude: exclude.clone(),
            replace: replace.clone(),
            additional: Vec::new(),
            confidence_threshold: 0.0,
            fraction_threshold: 0.0,
        };
        let (filter, invalid) = compile_filter(raw);

        prop_assert!(invalid.is_empty(), "有効のみなのに通知が発生: {:?}", invalid);
        prop_assert_eq!(filter.exclude.len(), exclude.len());
        prop_assert_eq!(filter.replace.len(), replace.len());
    }

    /// すべて無効なパターンなら、フィルタは空・通知集合は入力全件と一致する。
    #[test]
    fn all_invalid_yields_empty_filter(
        exclude in vec(invalid_pattern(), 0..=6),
        replace in vec(invalid_pattern().prop_map(|p| (p, "x".to_string())), 0..=6),
    ) {
        let total = exclude.len() + replace.len();
        let raw = RawTagFilter {
            keep: Vec::new(),
            exclude,
            replace,
            additional: Vec::new(),
            confidence_threshold: 0.0,
            fraction_threshold: 0.0,
        };
        let (filter, invalid) = compile_filter(raw);

        prop_assert!(filter.exclude.is_empty(), "無効のみなのに exclude が残る");
        prop_assert!(filter.replace.is_empty(), "無効のみなのに replace が残る");
        prop_assert_eq!(invalid.len(), total, "通知集合が入力全件と不一致");
    }
}

// --- 固定の代表ケース（プロパティを補完） -----------------------------------

/// 有効・無効混在の代表例で、有効のみが残り無効はすべて通知される。
#[test]
fn mixed_example_keeps_valid_notifies_invalid() {
    let raw = RawTagFilter {
        keep: Vec::new(),
        exclude: vec!["good".to_string(), "[".to_string(), "b.*c".to_string()],
        replace: vec![
            ("(".to_string(), "x".to_string()),
            ("valid".to_string(), "y".to_string()),
        ],
        additional: Vec::new(),
        confidence_threshold: 0.0,
        fraction_threshold: 0.0,
    };
    let (filter, invalid) = compile_filter(raw);

    // 有効: exclude 2 件（"good", "b.*c"）、replace 1 件（"valid"）。
    assert_eq!(filter.exclude.len(), 2);
    assert_eq!(filter.replace.len(), 1);
    assert_eq!(filter.replace[0].1, "y");

    // 無効: "[" と "(" の 2 件がすべて通知される。
    assert_eq!(invalid.len(), 2);
    let patterns: Vec<&str> = invalid.iter().map(|p| p.pattern.as_str()).collect();
    assert!(patterns.contains(&"["));
    assert!(patterns.contains(&"("));
    assert!(invalid.iter().all(|p| !p.reason.is_empty()));
}
