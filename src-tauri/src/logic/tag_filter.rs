//! タグフィルタの純粋ロジック（タスク 6, 要件 9, 10）。
//!
//! UI から受け取る生の設定 [`RawTagFilter`] を、正規表現をコンパイル済みの
//! [`TagFilter`] へ変換する `compile_filter` と、単一画像の Predicted_Tag へ
//! フィルタを適用する `apply_filter` を提供する。
//!
//! Exclude_Rules / Replace_Rules の検索パターンは「タグ全体一致（`^...$`）・
//! 大文字小文字無視」でコンパイルする。無効な正規表現は [`TagFilter`] に含めず、
//! [`InvalidPattern`] として通知集合へ集める（要件 9.7）。

use regex::{Regex, RegexBuilder};

use crate::logic::tag_ops::normalize_key;
use crate::models::{FilterOutcome, InvalidPattern, RawTagFilter, Tag, TagFilter};

/// 生のフィルタ設定をコンパイル済みフィルタへ変換する（要件 9.7）。
///
/// - Keep_Tags は「前後トリム＋大文字小文字無視」の正規化キー集合へ変換する
///   （[`normalize_key`] に準拠、要件 9.3）。
/// - Exclude_Rules / Replace_Rules の検索パターンをタグ全体一致（`^...$`）・
///   大文字小文字無視でコンパイルする。無効な正規表現は当該パターンを除外し、
///   [`InvalidPattern`] へ集める（要件 9.7）。有効なパターンだけを含む
///   [`TagFilter`] を返す。
/// - `additional` / `confidence_threshold` / `fraction_threshold` はそのまま移送する。
///
/// # 戻り値
///
/// コンパイル済み [`TagFilter`] と、コンパイルに失敗した [`InvalidPattern`] の列。
pub fn compile_filter(raw: RawTagFilter) -> (TagFilter, Vec<InvalidPattern>) {
    let RawTagFilter {
        keep,
        exclude,
        replace,
        additional,
        confidence_threshold,
        fraction_threshold,
    } = raw;

    let mut invalid: Vec<InvalidPattern> = Vec::new();

    // Keep_Tags → 正規化キー集合。
    let keep = keep.iter().map(|t| normalize_key(t)).collect();

    // Exclude_Rules → コンパイル済み Regex（無効は除外し invalid へ）。
    let mut compiled_exclude: Vec<Regex> = Vec::with_capacity(exclude.len());
    for pattern in exclude {
        match compile_pattern(&pattern) {
            Ok(re) => compiled_exclude.push(re),
            Err(reason) => invalid.push(InvalidPattern { pattern, reason }),
        }
    }

    // Replace_Rules → (コンパイル済み Regex, 置換文字列)（無効は除外し invalid へ）。
    let mut compiled_replace: Vec<(Regex, String)> = Vec::with_capacity(replace.len());
    for (pattern, replacement) in replace {
        match compile_pattern(&pattern) {
            Ok(re) => compiled_replace.push((re, replacement)),
            Err(reason) => invalid.push(InvalidPattern { pattern, reason }),
        }
    }

    let filter = TagFilter {
        keep,
        exclude: compiled_exclude,
        replace: compiled_replace,
        additional,
        confidence_threshold,
        fraction_threshold,
    };

    (filter, invalid)
}

/// 検索パターンをタグ全体一致・大文字小文字無視でコンパイルする。
///
/// パターンを `^...$` でラップしてタグ全体との一致にする。ただしユーザーが
/// 既にアンカー（先頭 `^` / 末尾 `$`）を含めている場合は二重付与しない。
/// 正規表現メタ文字はそのまま解釈する（wd14-tagger の compile_rex に準拠）。
///
/// # 戻り値
///
/// コンパイル成功なら [`Regex`]、失敗なら無効理由（エラーメッセージ）。
fn compile_pattern(pattern: &str) -> Result<Regex, String> {
    let anchored = anchor_pattern(pattern);
    RegexBuilder::new(&anchored)
        .case_insensitive(true)
        .build()
        .map_err(|e| e.to_string())
}

/// パターンを `^...$` でラップする（既存アンカーは二重付与しない）。
fn anchor_pattern(pattern: &str) -> String {
    let has_start = pattern.starts_with('^');
    let has_end = pattern.ends_with('$');
    match (has_start, has_end) {
        (true, true) => pattern.to_string(),
        (true, false) => format!("{pattern}$"),
        (false, true) => format!("^{pattern}"),
        (false, false) => format!("^{pattern}$"),
    }
}

/// 単一画像の Predicted_Tag 集合へ Tag_Filter を適用する（要件 9.2〜9.6, 9.8）。
///
/// 適用順序は設計「logic::tag_filter apply_filter」に厳密に従う:
///
/// 1. **Replace**（採用判定前）: 各タグ名へ Replace_Rules を順に適用して書き換える
///    （要件 9.5）。以降の Keep / Exclude / Threshold 判定はすべて書換後の
///    タグ名に対して行う（Property 13）。
/// 2. **Keep**: 書換後タグ名の正規化キーが Keep_Tags に含まれれば、
///    Confidence_Threshold 未満・Exclude 該当を問わず無条件 Adopted
///    （要件 9.3, 9.8、Property 11）。
/// 3. **Exclude / Threshold**: 非 Keep のタグで、Exclude_Rules のいずれかに一致、
///    または確信度が Confidence_Threshold 未満なら Discarded（要件 9.2, 9.4）。
/// 4. それ以外は Adopted（Property 12）。
/// 5. **Additional**: Additional_Tags を無条件に Adopted へ加える
///    （要件 9.6、Property 14）。
///
/// # 確信度なしタグの扱い
///
/// 非 Keep タグで [`Tag::confidence`] が `None` の場合は、閾値と比較できないため
/// 「Confidence_Threshold 未満」とみなし Discarded とする（旧
/// `inference_aux::adopt_by_threshold` の保守的な挙動を踏襲）。Keep タグは確信度に
/// 関わらず無条件採用のためこの限りではない。
///
/// # 戻り値
///
/// [`FilterOutcome`]（Adopted / Discarded）。各タグは代表確信度算出のため確信度を
/// 保持する。Additional_Tags は確信度なし（`None`）の [`Tag`] として付与する。
pub fn apply_filter(filter: &TagFilter, predicted: &[Tag]) -> FilterOutcome {
    let mut adopted: Vec<Tag> = Vec::new();
    let mut discarded: Vec<Tag> = Vec::new();

    for tag in predicted {
        // (1) Replace を採用判定前に適用してタグ名を書き換える（要件 9.5）。
        let rewritten_body = apply_replace(&filter.replace, &tag.body);
        let rewritten = Tag {
            body: rewritten_body,
            confidence: tag.confidence,
        };

        // 以降の判定は書換後名に対して行う（Property 13）。
        let key = normalize_key(&rewritten.body);

        // (2) Keep_Tags なら無条件 Adopted（要件 9.3, 9.8、Property 11）。
        if filter.keep.contains(&key) {
            adopted.push(rewritten);
            continue;
        }

        // (3) 非 Keep で Exclude 一致 or 確信度が閾値未満なら Discarded
        //     （要件 9.2, 9.4）。確信度 None は閾値未満とみなす。
        let excluded = filter.exclude.iter().any(|re| re.is_match(&rewritten.body));
        let below_threshold = !matches!(
            rewritten.confidence,
            Some(c) if c >= filter.confidence_threshold
        );
        if excluded || below_threshold {
            discarded.push(rewritten);
        } else {
            // (4) それ以外は Adopted（Property 12）。
            adopted.push(rewritten);
        }
    }

    // (5) Additional_Tags を Adopted の先頭へ無条件付与する（要件 9.6、Property 14）。
    //     Replace により複数の予測タグが同名へ書き換わった場合や、Additional と
    //     予測タグが重複する場合に備え、正規化キーで重複を除去する。Additional を
    //     先頭に置き、以降に予測由来の採用タグを続けることで「先頭・先勝ち」で
    //     最初の出現のみを残す。
    let mut merged: Vec<Tag> = Vec::with_capacity(filter.additional.len() + adopted.len());
    merged.extend(
        filter
            .additional
            .iter()
            .map(|extra| Tag::new(extra.clone())),
    );
    merged.extend(adopted);
    let adopted = dedup_by_key(merged);

    FilterOutcome { adopted, discarded }
}

/// タグ列から正規化キーの重複を除去する（最初の出現＝先勝ちで残す）。
///
/// Replace 後に複数タグが同名へ書き換わった場合や、Additional_Tags と予測タグが
/// 重複する場合の重複を取り除く。順序は保存し、各キーの最初の [`Tag`]（確信度含む）
/// を残す。
fn dedup_by_key(tags: Vec<Tag>) -> Vec<Tag> {
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut out: Vec<Tag> = Vec::with_capacity(tags.len());
    for tag in tags {
        if seen.insert(normalize_key(&tag.body)) {
            out.push(tag);
        }
    }
    out
}

/// Replace_Rules を順に適用してタグ名を書き換える。
///
/// 各ルール `(検索 Regex, 置換文字列)` を先頭から順に適用する。検索は
/// [`compile_pattern`] でタグ全体一致（`^...$`）・大小無視にコンパイル済みのため、
/// 一致したタグ名全体を置換文字列で置き換える。複数ルールは連鎖適用する
/// （前段の置換結果に後段のルールが作用しうる）。
fn apply_replace(rules: &[(Regex, String)], body: &str) -> String {
    let mut current = body.to_string();
    for (re, replacement) in rules {
        current = re.replace_all(&current, replacement.as_str()).into_owned();
    }
    current
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn keep_is_normalized_to_key_set() {
        let mut r = raw();
        r.keep = vec![
            "  Long_Hair ".to_string(),
            "SOLO".to_string(),
            "solo".to_string(),
        ];
        let (filter, invalid) = compile_filter(r);

        assert!(invalid.is_empty());
        // 前後トリム＋小文字化。重複キーは畳まれる。
        assert!(filter.keep.contains("long_hair"));
        assert!(filter.keep.contains("solo"));
        assert_eq!(filter.keep.len(), 2);
    }

    #[test]
    fn exclude_matches_whole_tag_case_insensitively() {
        let mut r = raw();
        r.exclude = vec!["smile".to_string()];
        let (filter, invalid) = compile_filter(r);

        assert!(invalid.is_empty());
        assert_eq!(filter.exclude.len(), 1);
        let re = &filter.exclude[0];
        // 大小無視で全体一致。
        assert!(re.is_match("smile"));
        assert!(re.is_match("SMILE"));
        // 部分一致はしない（^...$ ラップ）。
        assert!(!re.is_match("big smile"));
        assert!(!re.is_match("smiley"));
    }

    #[test]
    fn replace_pattern_is_compiled_with_replacement_preserved() {
        let mut r = raw();
        r.replace = vec![("cat".to_string(), "feline".to_string())];
        let (filter, invalid) = compile_filter(r);

        assert!(invalid.is_empty());
        assert_eq!(filter.replace.len(), 1);
        let (re, replacement) = &filter.replace[0];
        assert!(re.is_match("Cat"));
        assert_eq!(replacement, "feline");
    }

    #[test]
    fn regex_metacharacters_are_interpreted() {
        let mut r = raw();
        // `1.*girl` はメタ文字として解釈される。
        r.exclude = vec!["1.*girl".to_string()];
        let (filter, invalid) = compile_filter(r);

        assert!(invalid.is_empty());
        let re = &filter.exclude[0];
        assert!(re.is_match("1girl"));
        assert!(re.is_match("1 anime girl"));
        assert!(!re.is_match("prefix 1girl suffix"));
    }

    #[test]
    fn user_supplied_anchors_are_not_doubled() {
        // 先頭 `^` / 末尾 `$` を含めても二重付与にならず有効にコンパイルされる。
        let mut r = raw();
        r.exclude = vec![
            "^smile$".to_string(),
            "^smile".to_string(),
            "smile$".to_string(),
        ];
        let (filter, invalid) = compile_filter(r);

        assert!(invalid.is_empty());
        assert_eq!(filter.exclude.len(), 3);
        for re in &filter.exclude {
            assert!(re.is_match("smile"));
        }
    }

    #[test]
    fn invalid_patterns_are_excluded_and_collected() {
        let mut r = raw();
        // `[` は不正な正規表現。
        r.exclude = vec!["good".to_string(), "[".to_string()];
        r.replace = vec![
            ("(".to_string(), "x".to_string()),
            ("valid".to_string(), "y".to_string()),
        ];
        let (filter, invalid) = compile_filter(r);

        // 有効なパターンのみコンパイル済み集合に含まれる。
        assert_eq!(filter.exclude.len(), 1);
        assert_eq!(filter.replace.len(), 1);
        assert_eq!(filter.replace[0].1, "y");

        // 無効パターンはすべて通知集合に含まれる。
        let patterns: Vec<&str> = invalid.iter().map(|p| p.pattern.as_str()).collect();
        assert_eq!(invalid.len(), 2);
        assert!(patterns.contains(&"["));
        assert!(patterns.contains(&"("));
        // 理由は非空。
        assert!(invalid.iter().all(|p| !p.reason.is_empty()));
    }

    #[test]
    fn scalar_fields_are_carried_over() {
        let mut r = raw();
        r.additional = vec!["extra".to_string(), "tags".to_string()];
        r.confidence_threshold = 0.35;
        r.fraction_threshold = 0.5;
        let (filter, invalid) = compile_filter(r);

        assert!(invalid.is_empty());
        assert_eq!(
            filter.additional,
            vec!["extra".to_string(), "tags".to_string()]
        );
        assert_eq!(filter.confidence_threshold, 0.35);
        assert_eq!(filter.fraction_threshold, 0.5);
    }

    #[test]
    fn empty_raw_produces_empty_filter() {
        let (filter, invalid) = compile_filter(raw());
        assert!(invalid.is_empty());
        assert!(filter.keep.is_empty());
        assert!(filter.exclude.is_empty());
        assert!(filter.replace.is_empty());
        assert!(filter.additional.is_empty());
    }

    // Property 15: 無効な正規表現の非適用と通知。
    // 任意の Exclude / Replace パターン列について、コンパイル済み TagFilter には
    // 無効なパターンが 1 つも含まれず、無効なパターンはすべて通知集合に含まれる。
    #[test]
    fn property15_invalid_never_in_filter_all_in_notification() {
        // 有効・無効を混在させたパターン群。
        // 無効側は `^...$` ラップ後（compile_pattern の anchor_pattern）も確実に無効な
        // ものだけを選ぶ。`"*"`/`"\\"` は `^*$`/`^\\$` にすると有効化されるため除外。
        // 無効パターンの網羅的検証は tests/pbt_tag_filter_invalid_regex.rs が担う。
        let valid = ["a", "b.*c", "^d$", "[abc]", "\\d+"];
        let invalid_pats = ["[", "(", "(?P<", "[z-a]"];

        let mut r = raw();
        r.exclude = valid
            .iter()
            .chain(invalid_pats.iter())
            .map(|s| s.to_string())
            .collect();
        r.replace = valid
            .iter()
            .chain(invalid_pats.iter())
            .map(|s| (s.to_string(), "x".to_string()))
            .collect();

        let (filter, invalid) = compile_filter(r);

        // TagFilter に含まれるのは有効パターンのみ（無効は 0 件）。
        assert_eq!(filter.exclude.len(), valid.len());
        assert_eq!(filter.replace.len(), valid.len());

        // 無効パターンはすべて通知集合に含まれる（exclude 側 + replace 側）。
        assert_eq!(invalid.len(), invalid_pats.len() * 2);
        for p in &invalid_pats {
            assert!(
                invalid.iter().filter(|ip| ip.pattern == *p).count() >= 1,
                "無効パターン {p:?} が通知集合に含まれない"
            );
        }
    }
}

#[cfg(test)]
mod apply_filter_tests {
    use super::*;
    use crate::models::Tag;

    /// テスト用: 各フィールドを指定して `RawTagFilter` を組み立て、コンパイルする。
    /// 検索/置換パターンはすべて有効な前提（無効なら panic で気付く）。
    fn filter(
        keep: &[&str],
        exclude: &[&str],
        replace: &[(&str, &str)],
        additional: &[&str],
        confidence_threshold: f32,
    ) -> TagFilter {
        let raw = RawTagFilter {
            keep: keep.iter().map(|s| s.to_string()).collect(),
            exclude: exclude.iter().map(|s| s.to_string()).collect(),
            replace: replace
                .iter()
                .map(|(p, r)| (p.to_string(), r.to_string()))
                .collect(),
            additional: additional.iter().map(|s| s.to_string()).collect(),
            confidence_threshold,
            fraction_threshold: 0.0,
        };
        let (f, invalid) = compile_filter(raw);
        assert!(invalid.is_empty(), "テスト用パターンが無効: {invalid:?}");
        f
    }

    fn bodies(tags: &[Tag]) -> Vec<&str> {
        tags.iter().map(|t| t.body.as_str()).collect()
    }

    // --- Threshold（要件 9.2） ---

    #[test]
    fn adopts_at_or_above_threshold_discards_below() {
        let f = filter(&[], &[], &[], &[], 0.5);
        let predicted = vec![
            Tag::with_confidence("a", 0.9),
            Tag::with_confidence("b", 0.5), // 境界は inclusive で採用
            Tag::with_confidence("c", 0.3),
        ];
        let out = apply_filter(&f, &predicted);
        assert_eq!(bodies(&out.adopted), vec!["a", "b"]);
        assert_eq!(bodies(&out.discarded), vec!["c"]);
    }

    #[test]
    fn threshold_boundary_is_inclusive() {
        let f = filter(&[], &[], &[], &[], 0.5);
        let out = apply_filter(&f, &[Tag::with_confidence("a", 0.5)]);
        assert_eq!(bodies(&out.adopted), vec!["a"]);
        assert!(out.discarded.is_empty());
    }

    #[test]
    fn confidence_none_is_discarded_when_non_keep() {
        // 確信度なしタグは閾値未満扱いで Discarded（adopt_by_threshold の挙動踏襲）。
        let f = filter(&[], &[], &[], &[], 0.0);
        let out = apply_filter(&f, &[Tag::new("a"), Tag::with_confidence("b", 0.0)]);
        assert_eq!(bodies(&out.adopted), vec!["b"]);
        assert_eq!(bodies(&out.discarded), vec!["a"]);
    }

    // --- Keep 優先（要件 9.3, 9.8、Property 11） ---

    #[test]
    fn keep_adopts_below_threshold_tag() {
        // Keep は Confidence_Threshold 未満でも無条件 Adopted。
        let f = filter(&["solo"], &[], &[], &[], 0.9);
        let out = apply_filter(&f, &[Tag::with_confidence("solo", 0.1)]);
        assert_eq!(bodies(&out.adopted), vec!["solo"]);
        assert!(out.discarded.is_empty());
    }

    #[test]
    fn keep_adopts_exclude_matched_tag() {
        // Keep は Exclude 該当でも無条件 Adopted。
        let f = filter(&["solo"], &["solo"], &[], &[], 0.0);
        let out = apply_filter(&f, &[Tag::with_confidence("solo", 0.99)]);
        assert_eq!(bodies(&out.adopted), vec!["solo"]);
        assert!(out.discarded.is_empty());
    }

    #[test]
    fn keep_match_is_case_insensitive_and_trimmed() {
        // Keep_Tags は正規化キーで比較する。
        let f = filter(&["Long_Hair"], &[], &[], &[], 0.9);
        let out = apply_filter(&f, &[Tag::with_confidence("long_hair", 0.1)]);
        assert_eq!(bodies(&out.adopted), vec!["long_hair"]);
    }

    // --- Exclude（要件 9.4） ---

    #[test]
    fn exclude_matched_non_keep_is_discarded() {
        let f = filter(&[], &["smile"], &[], &[], 0.0);
        let out = apply_filter(
            &f,
            &[
                Tag::with_confidence("smile", 0.99),
                Tag::with_confidence("solo", 0.99),
            ],
        );
        assert_eq!(bodies(&out.adopted), vec!["solo"]);
        assert_eq!(bodies(&out.discarded), vec!["smile"]);
    }

    // --- Replace 書換後判定（要件 9.5、Property 13） ---

    #[test]
    fn replace_applied_before_keep_judgement() {
        // 書換後名 "feline" が Keep に一致すれば採用（元名 "cat" は Keep でない）。
        let f = filter(&["feline"], &[], &[("cat", "feline")], &[], 0.9);
        let out = apply_filter(&f, &[Tag::with_confidence("cat", 0.1)]);
        assert_eq!(bodies(&out.adopted), vec!["feline"]);
    }

    #[test]
    fn replace_applied_before_exclude_judgement() {
        // 書換後名 "banned" が Exclude に一致すれば Discarded。
        let f = filter(&[], &["banned"], &[("cat", "banned")], &[], 0.0);
        let out = apply_filter(&f, &[Tag::with_confidence("cat", 0.99)]);
        assert!(out.adopted.is_empty());
        assert_eq!(bodies(&out.discarded), vec!["banned"]);
    }

    #[test]
    fn replace_rewrites_body_of_adopted_tag() {
        // 採用タグの本体は書換後の名前になる。
        let f = filter(&[], &[], &[("cat", "feline")], &[], 0.0);
        let out = apply_filter(&f, &[Tag::with_confidence("Cat", 0.99)]);
        assert_eq!(bodies(&out.adopted), vec!["feline"]);
        // 確信度は保持される。
        assert_eq!(out.adopted[0].confidence, Some(0.99));
    }

    // --- Additional 付与（要件 9.6、Property 14） ---

    #[test]
    fn additional_tags_are_unconditionally_adopted() {
        let f = filter(&[], &[], &[], &["extra1", "extra2"], 0.9);
        let out = apply_filter(&f, &[Tag::with_confidence("a", 0.1)]);
        // a は閾値未満で Discarded、Additional は無条件付与。
        assert_eq!(bodies(&out.adopted), vec!["extra1", "extra2"]);
        assert_eq!(bodies(&out.discarded), vec!["a"]);
        // Additional は確信度なし。
        assert!(out.adopted.iter().all(|t| t.confidence.is_none()));
    }

    #[test]
    fn empty_predicted_yields_only_additional() {
        let f = filter(&[], &[], &[], &["extra"], 0.0);
        let out = apply_filter(&f, &[]);
        assert_eq!(bodies(&out.adopted), vec!["extra"]);
        assert!(out.discarded.is_empty());
    }

    #[test]
    fn empty_filter_adopts_all_with_confidence_at_threshold_zero() {
        let f = filter(&[], &[], &[], &[], 0.0);
        let predicted = vec![
            Tag::with_confidence("a", 0.0),
            Tag::with_confidence("b", 1.0),
        ];
        let out = apply_filter(&f, &predicted);
        assert_eq!(bodies(&out.adopted), vec!["a", "b"]);
        assert!(out.discarded.is_empty());
    }

    #[test]
    fn additional_tags_are_prepended_before_predicted() {
        // Additional_Tags は採用タグの先頭に付与される。
        let f = filter(&[], &[], &[], &["quality", "masterpiece"], 0.0);
        let predicted = vec![
            Tag::with_confidence("solo", 0.9),
            Tag::with_confidence("1girl", 0.8),
        ];
        let out = apply_filter(&f, &predicted);
        // Additional が先頭、続けて予測由来の採用タグ。
        assert_eq!(
            bodies(&out.adopted),
            vec!["quality", "masterpiece", "solo", "1girl"]
        );
    }

    #[test]
    fn replace_collision_is_deduplicated() {
        // 複数の予測タグが同名へ書き換わったら重複除去（先勝ち）。
        let f = filter(&[], &[], &[("girl", "woman"), ("1girl", "woman")], &[], 0.0);
        let predicted = vec![
            Tag::with_confidence("girl", 0.9),
            Tag::with_confidence("1girl", 0.7),
        ];
        let out = apply_filter(&f, &predicted);
        // "woman" は 1 件のみ。最初の出現（confidence 0.9）を残す。
        assert_eq!(bodies(&out.adopted), vec!["woman"]);
        assert_eq!(out.adopted[0].confidence, Some(0.9));
    }

    #[test]
    fn additional_duplicate_with_predicted_is_deduplicated_keeping_additional() {
        // Additional が予測採用タグと重複する場合、先頭の Additional を残し
        // 予測側の重複を落とす。
        let f = filter(&[], &[], &[], &["solo"], 0.0);
        let out = apply_filter(
            &f,
            &[
                Tag::with_confidence("solo", 0.9),
                Tag::with_confidence("1girl", 0.8),
            ],
        );
        // 先頭に Additional の "solo"（確信度なし）、続けて重複しない "1girl"。
        assert_eq!(bodies(&out.adopted), vec!["solo", "1girl"]);
        assert_eq!(out.adopted[0].confidence, None);
    }
}
