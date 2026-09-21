//! タグ集計・フィルタ述語（タスク 3）。
//!
//! 本モジュールには集計（タスク 3.1）とフィルタ述語（タスク 3.3）を実装する。

use crate::models::TagCount;

/// タグ列群から各タグの出現回数を集計し、整列済みの [`TagCount`] 列を返す（タスク 3.1）。
///
/// - 入力は「Tag_File 1 件分のタグ列」を要素とするイテレータ。各タグ文字列は
///   前後空白をトリムし、トリム後に空になるタグは集計対象から除外する。
/// - 出現回数は全 Tag_File を横断した総出現回数（同一 Tag_File 内の重複も加算）。
/// - 並び順は出現回数の降順、出現回数が同一のタグはタグ名（トリム後の文字列）の
///   昇順（Unicode コードポイント順）。
///
/// タグ同一性はトリム後の文字列の完全一致で判定する（表示名をそのまま識別子とする）。
/// 大文字小文字を区別しない重複判定は一括操作（タスク 4）の関心事であり、集計では
/// 表示上区別されるべき別タグとして扱う。
///
/// 要件: 6.1, 6.3 / 設計 Property 11。
pub fn aggregate_tags<I, F, T>(files: I) -> Vec<TagCount>
where
    I: IntoIterator<Item = F>,
    F: IntoIterator<Item = T>,
    T: AsRef<str>,
{
    let mut counts: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for file in files {
        for tag in file {
            let trimmed = tag.as_ref().trim();
            if trimmed.is_empty() {
                continue;
            }
            *counts.entry(trimmed.to_string()).or_insert(0) += 1;
        }
    }

    let mut result: Vec<TagCount> = counts
        .into_iter()
        .map(|(tag, count)| TagCount { tag, count })
        .collect();

    // 出現回数の降順、同数はタグ名の昇順。
    result.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.tag.cmp(&b.tag)));

    result
}

/// タグの正規化キーを算出する（前後トリム＋大文字小文字無視）。
///
/// 重複判定・タグ一致判定は「前後トリム＋大文字小文字無視の完全一致」で統一する
/// （design.md「タグファイルの正規化ルール」／要件 3-2/3/4, 10-1）。
fn normalize_key(tag: &str) -> String {
    tag.trim().to_lowercase()
}

/// 画像のタグ集合が include/exclude 条件を満たすか判定する（要件 6.5/6.6/6.7/6.8）。
///
/// - `image_tags`: 対象画像が持つタグ列。
/// - `include`: 包含タグ。指定タグを *すべて* 含む場合のみ真（要件 6.5）。
/// - `exclude`: 除外タグ。指定タグの *いずれか* を含む場合は偽（要件 6.6）。
///
/// include と exclude を同時指定した場合は「include をすべて含み、かつ exclude を
/// いずれも含まない」場合のみ真（要件 6.7）。include・exclude がともに空なら全件真
/// （要件 6.8）。判定はすべて正規化キー（前後トリム＋大文字小文字無視）で行う。
pub fn matches_filter(image_tags: &[String], include: &[String], exclude: &[String]) -> bool {
    // 画像タグを正規化キー集合へ変換（重複は自然に畳まれる）。
    let tag_keys: std::collections::HashSet<String> =
        image_tags.iter().map(|t| normalize_key(t)).collect();

    // include: 指定タグをすべて含む（空なら vacuously true）。
    let all_included = include
        .iter()
        .all(|t| tag_keys.contains(&normalize_key(t)));

    // exclude: 指定タグのいずれも含まない（空なら vacuously true）。
    let none_excluded = exclude
        .iter()
        .all(|t| !tag_keys.contains(&normalize_key(t)));

    all_included && none_excluded
}

#[cfg(test)]
mod matches_filter_tests {
    use super::matches_filter;

    fn v(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn empty_include_and_exclude_matches_all() {
        // 要件 6.8: include/exclude 空なら全件（タグ有無に関わらず真）。
        assert!(matches_filter(&v(&["cat", "dog"]), &[], &[]));
        assert!(matches_filter(&v(&[]), &[], &[]));
    }

    #[test]
    fn include_all_present_matches() {
        // 要件 6.5: 指定 include をすべて含む場合のみ真。
        assert!(matches_filter(&v(&["cat", "dog", "sky"]), &v(&["cat", "dog"]), &[]));
    }

    #[test]
    fn include_missing_one_does_not_match() {
        // include の一部が欠けていれば偽。
        assert!(!matches_filter(&v(&["cat", "sky"]), &v(&["cat", "dog"]), &[]));
    }

    #[test]
    fn exclude_present_does_not_match() {
        // 要件 6.6: exclude のいずれかを含めば偽。
        assert!(!matches_filter(&v(&["cat", "dog"]), &[], &v(&["dog"])));
    }

    #[test]
    fn exclude_absent_matches() {
        assert!(matches_filter(&v(&["cat", "sky"]), &[], &v(&["dog"])));
    }

    #[test]
    fn include_and_exclude_combined() {
        // 要件 6.7: include 全含み かつ exclude いずれも含まない場合のみ真。
        assert!(matches_filter(
            &v(&["cat", "dog", "sky"]),
            &v(&["cat", "dog"]),
            &v(&["night"])
        ));
        // include は満たすが exclude を含む → 偽。
        assert!(!matches_filter(
            &v(&["cat", "dog", "night"]),
            &v(&["cat", "dog"]),
            &v(&["night"])
        ));
    }

    #[test]
    fn normalized_matching_trims_and_ignores_case() {
        // 前後トリム＋大文字小文字無視で一致判定。
        assert!(matches_filter(
            &v(&["  Cat ", "DOG"]),
            &v(&["cat", "dog"]),
            &[]
        ));
        assert!(!matches_filter(
            &v(&["  Cat ", "DOG"]),
            &[],
            &v(&["  dOg  "])
        ));
    }

    #[test]
    fn empty_image_tags_with_include_does_not_match() {
        assert!(!matches_filter(&v(&[]), &v(&["cat"]), &[]));
    }

    #[test]
    fn empty_image_tags_with_exclude_matches() {
        // タグ無し画像は exclude に該当しないので真。
        assert!(matches_filter(&v(&[]), &[], &v(&["cat"])));
    }

    #[test]
    fn duplicate_image_tags_do_not_affect_result() {
        assert!(matches_filter(
            &v(&["cat", "cat", "dog"]),
            &v(&["cat", "dog"]),
            &[]
        ));
    }
}

#[cfg(test)]
mod aggregate_tags_tests {
    use super::aggregate_tags;
    use crate::models::TagCount;

    #[test]
    fn counts_occurrences_across_files() {
        let files = vec![
            vec!["cat", "dog", "cat"],
            vec!["dog", "bird"],
            vec!["cat"],
        ];
        let result = aggregate_tags(files);
        // cat=3, dog=2, bird=1
        assert_eq!(
            result,
            vec![
                TagCount { tag: "cat".into(), count: 3 },
                TagCount { tag: "dog".into(), count: 2 },
                TagCount { tag: "bird".into(), count: 1 },
            ]
        );
    }

    #[test]
    fn sorts_by_count_desc_then_name_asc() {
        // すべて同数(1) の場合はタグ名昇順。
        let files = vec![vec!["banana", "apple", "cherry"]];
        let result = aggregate_tags(files);
        let names: Vec<&str> = result.iter().map(|t| t.tag.as_str()).collect();
        assert_eq!(names, vec!["apple", "banana", "cherry"]);
    }

    #[test]
    fn ties_break_by_name_ascending() {
        // count が同じ (2) のタグ間はタグ名昇順、count が大きい方が先。
        let files = vec![
            vec!["zzz", "aaa", "mmm"],
            vec!["zzz", "aaa", "mmm"],
            vec!["top"], // count=1
        ];
        let result = aggregate_tags(files);
        assert_eq!(
            result,
            vec![
                TagCount { tag: "aaa".into(), count: 2 },
                TagCount { tag: "mmm".into(), count: 2 },
                TagCount { tag: "zzz".into(), count: 2 },
                TagCount { tag: "top".into(), count: 1 },
            ]
        );
    }

    #[test]
    fn trims_whitespace_and_ignores_empty() {
        let files = vec![vec!["  cat  ", "dog", "   ", "", "\tcat"]];
        let result = aggregate_tags(files);
        // "  cat  " と "\tcat" はトリム後 "cat" で同一。空白のみ/空は無視。
        assert_eq!(
            result,
            vec![
                TagCount { tag: "cat".into(), count: 2 },
                TagCount { tag: "dog".into(), count: 1 },
            ]
        );
    }

    #[test]
    fn case_sensitive_identity_for_display() {
        // 大文字小文字は集計上は別タグ扱い（重複判定は別関心事）。
        let files = vec![vec!["Cat", "cat", "CAT"]];
        let result = aggregate_tags(files);
        // すべて count=1、タグ名昇順（大文字が先: Unicode コードポイント順）。
        assert_eq!(
            result,
            vec![
                TagCount { tag: "CAT".into(), count: 1 },
                TagCount { tag: "Cat".into(), count: 1 },
                TagCount { tag: "cat".into(), count: 1 },
            ]
        );
    }

    #[test]
    fn empty_input_yields_empty_result() {
        let files: Vec<Vec<&str>> = vec![];
        assert!(aggregate_tags(files).is_empty());
    }

    #[test]
    fn files_with_only_empty_tags_yield_empty_result() {
        let files = vec![vec!["", "   ", "\t"], vec![]];
        assert!(aggregate_tags(files).is_empty());
    }

    #[test]
    fn handles_multibyte_tags() {
        let files = vec![vec!["猫", "犬", "猫"], vec![" 猫 "]];
        let result = aggregate_tags(files);
        assert_eq!(
            result,
            vec![
                TagCount { tag: "猫".into(), count: 3 },
                TagCount { tag: "犬".into(), count: 1 },
            ]
        );
    }

    #[test]
    fn accepts_string_owned_tags() {
        // AsRef<str> により String 所有値のイテレータも受け付ける。
        let files: Vec<Vec<String>> = vec![vec!["a".to_string(), "b".to_string()]];
        let result = aggregate_tags(files);
        assert_eq!(result.len(), 2);
    }
}
