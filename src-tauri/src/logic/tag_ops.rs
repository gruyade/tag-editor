//! タグ追加/削除/重複除去のコアロジック（タスク 4）。
//!
//! 本ファイルにはタスク 2.1 で確立するタグ正規化・トークン分割の
//! 純粋関数を先行実装する。ファイル I/O には依存しない決定的関数群。
//!
//! 設計「タグファイルの正規化ルール」に準拠:
//! - Tag_File はカンマ区切り。読込時に各トークンを前後トリム。空トークンは無視。
//! - 重複判定・タグ一致判定は「前後トリム＋大文字小文字無視の完全一致」で統一
//!   （要件 3.2 / 3.3 / 3.4 / 10.1）。

/// タグの正規化キーを算出する。
///
/// 正規化キーは「前後空白をトリムし、大文字小文字を無視した」文字列で、
/// タグの一致・重複判定に用いる（要件 3.2, 3.3, 3.4, 10.1）。
///
/// # 例
/// ```
/// use tag_editor_core::logic::tag_ops::normalize_key;
/// assert_eq!(normalize_key("  Long_Hair "), "long_hair");
/// ```
pub fn normalize_key(tag: &str) -> String {
    tag.trim().to_lowercase()
}

/// カンマ区切り文字列をタグ本体のトークン列へ分割する。
///
/// 各トークンは前後をトリムし、空トークン（トリム後に空文字列となるもの）は
/// 無視する。トークンの原文（大文字小文字・内部の記号）はそのまま保持し、
/// 正規化は行わない（正規化が必要な場合は [`normalize_key`] を用いる）。
///
/// # 例
/// ```
/// use tag_editor_core::logic::tag_ops::split_tokens;
/// assert_eq!(
///     split_tokens("1girl,  long hair , ,solo, "),
///     vec!["1girl", "long hair", "solo"]
/// );
/// ```
pub fn split_tokens(input: &str) -> Vec<String> {
    input
        .split(',')
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .map(str::to_owned)
        .collect()
}

/// 2 つのタグが正規化キー上で一致するか判定する。
///
/// 「前後トリム＋大文字小文字無視の完全一致」で比較する
/// （要件 3.2, 3.3, 3.4, 10.1）。
///
/// # 例
/// ```
/// use tag_editor_core::logic::tag_ops::tags_match;
/// assert!(tags_match("Solo", " solo "));
/// assert!(!tags_match("solo", "duo"));
/// ```
pub fn tags_match(a: &str, b: &str) -> bool {
    normalize_key(a) == normalize_key(b)
}

/// タグ列（正規化前の原文スライス）に、指定タグと正規化キーが一致する
/// タグが含まれるか判定する（要件 3.2, 10.1）。
///
/// # 例
/// ```
/// use tag_editor_core::logic::tag_ops::contains_tag;
/// let tags = ["1girl".to_string(), "Long Hair".to_string()];
/// assert!(contains_tag(&tags, " long hair "));
/// assert!(!contains_tag(&tags, "solo"));
/// ```
pub fn contains_tag<S: AsRef<str>>(tags: &[S], target: &str) -> bool {
    let key = normalize_key(target);
    tags.iter().any(|t| normalize_key(t.as_ref()) == key)
}

/// タグ列へ指定タグ群を一括追加する（要件 3.1, 3.5, 12.1, 12.2, 12.3）。
///
/// 既存タグ列 `existing` の順序と原文を保持したまま、`to_add` の各タグを
/// 末尾へ追加する。ただし正規化キー（[`normalize_key`]）が既存タグまたは
/// 既に追加済みのタグと一致するものは追加しない。これにより:
///
/// - **包含性**: `to_add` のうち、既存に未登録のタグはすべて結果に含まれる。
/// - **非重複**: 正規化キーが同一のタグを重複して増やさない
///   （`to_add` 内での重複も先勝ちで1つに畳み込む）。
/// - **冪等性**: 同じ追加をもう一度適用しても結果は変化しない。
///
/// 追加タグの原文（大文字小文字・記号）はそのまま保持する。
///
/// # 例
/// ```
/// use tag_editor_core::logic::tag_ops::add_tags;
/// let existing = ["1girl".to_string(), "Solo".to_string()];
/// let result = add_tags(&existing, &["solo", "long hair", "Long Hair"]);
/// assert_eq!(result, vec!["1girl", "Solo", "long hair"]);
/// ```
pub fn add_tags<S: AsRef<str>, T: AsRef<str>>(existing: &[S], to_add: &[T]) -> Vec<String> {
    let mut result: Vec<String> = existing.iter().map(|t| t.as_ref().to_owned()).collect();
    // 既存＋追加済みの正規化キー集合。
    let mut seen: std::collections::HashSet<String> =
        existing.iter().map(|t| normalize_key(t.as_ref())).collect();
    for tag in to_add {
        let key = normalize_key(tag.as_ref());
        // 空トークンは追加しない（設計「空トークンは無視」に準拠）。
        if key.is_empty() {
            continue;
        }
        if seen.insert(key) {
            result.push(tag.as_ref().to_owned());
        }
    }
    result
}

/// タグ列から指定タグ群に正規化キーが一致するタグをすべて除去する
/// （要件 3.3）。
///
/// `to_remove` のいずれかと正規化キー（[`normalize_key`]）が一致するタグを
/// `tags` から全て取り除く。残ったタグの相対順序と原文は保持する。
///
/// # 例
/// ```
/// use tag_editor_core::logic::tag_ops::remove_tags;
/// let tags = ["1girl".to_string(), "Solo".to_string(), "solo".to_string()];
/// let result = remove_tags(&tags, &[" SOLO "]);
/// assert_eq!(result, vec!["1girl"]);
/// ```
pub fn remove_tags<S: AsRef<str>, T: AsRef<str>>(tags: &[S], to_remove: &[T]) -> Vec<String> {
    let remove_keys: std::collections::HashSet<String> = to_remove
        .iter()
        .map(|t| normalize_key(t.as_ref()))
        .collect();
    tags.iter()
        .filter(|t| !remove_keys.contains(&normalize_key(t.as_ref())))
        .map(|t| t.as_ref().to_owned())
        .collect()
}

/// タグ列から重複を除去する。初出を残し以降の重複を除去する（要件 3.4）。
///
/// 正規化キー（[`normalize_key`]）が同一のタグが複数存在する場合、最初の
/// 出現のみを残し以降を除去する。初出の順序と原文を保持する。空トークン
/// （正規化キーが空）は保持しない。もう一度適用しても結果は変化しない
/// （冪等）。
///
/// # 例
/// ```
/// use tag_editor_core::logic::tag_ops::dedup_tags;
/// let tags = ["Solo".to_string(), "1girl".to_string(), "solo".to_string()];
/// assert_eq!(dedup_tags(&tags), vec!["Solo", "1girl"]);
/// ```
pub fn dedup_tags<S: AsRef<str>>(tags: &[S]) -> Vec<String> {
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut result: Vec<String> = Vec::new();
    for tag in tags {
        let key = normalize_key(tag.as_ref());
        if key.is_empty() {
            continue;
        }
        if seen.insert(key) {
            result.push(tag.as_ref().to_owned());
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_key_trims_and_lowercases() {
        assert_eq!(normalize_key("  Long_Hair "), "long_hair");
        assert_eq!(normalize_key("SOLO"), "solo");
        assert_eq!(normalize_key("solo"), "solo");
        // 内部の空白はそのまま残る（トリムは前後のみ）。
        assert_eq!(normalize_key("  long hair  "), "long hair");
    }

    #[test]
    fn normalize_key_empty_and_whitespace() {
        assert_eq!(normalize_key(""), "");
        assert_eq!(normalize_key("   "), "");
    }

    #[test]
    fn split_tokens_basic() {
        assert_eq!(
            split_tokens("1girl, long hair, solo"),
            vec!["1girl", "long hair", "solo"]
        );
    }

    #[test]
    fn split_tokens_trims_each_token() {
        assert_eq!(
            split_tokens("  1girl ,long hair ,  solo  "),
            vec!["1girl", "long hair", "solo"]
        );
    }

    #[test]
    fn split_tokens_ignores_empty_tokens() {
        // 連続カンマ・末尾カンマ・空白のみのトークンはすべて無視。
        assert_eq!(
            split_tokens("1girl,,solo, ,"),
            vec!["1girl", "solo"]
        );
    }

    #[test]
    fn split_tokens_empty_input() {
        assert!(split_tokens("").is_empty());
        assert!(split_tokens("   ").is_empty());
        assert!(split_tokens(",,, ,").is_empty());
    }

    #[test]
    fn split_tokens_preserves_original_case() {
        // 分割は原文を保持し、正規化はしない。
        assert_eq!(split_tokens("Solo, LONG Hair"), vec!["Solo", "LONG Hair"]);
    }

    #[test]
    fn tags_match_case_insensitive_and_trimmed() {
        assert!(tags_match("Solo", " solo "));
        assert!(tags_match("LONG_HAIR", "long_hair"));
        assert!(tags_match("  1girl  ", "1girl"));
    }

    #[test]
    fn tags_match_rejects_different_tags() {
        assert!(!tags_match("solo", "duo"));
        // 内部空白の差異は別タグ扱い。
        assert!(!tags_match("long hair", "longhair"));
    }

    #[test]
    fn contains_tag_matches_normalized() {
        let tags = ["1girl".to_string(), "Long Hair".to_string()];
        assert!(contains_tag(&tags, " long hair "));
        assert!(contains_tag(&tags, "1GIRL"));
        assert!(!contains_tag(&tags, "solo"));
    }

    #[test]
    fn contains_tag_empty_list() {
        let tags: [String; 0] = [];
        assert!(!contains_tag(&tags, "anything"));
    }

    // ---- add_tags ----

    #[test]
    fn add_tags_appends_new_tags_in_order() {
        let existing = ["1girl".to_string(), "solo".to_string()];
        assert_eq!(
            add_tags(&existing, &["long hair", "smile"]),
            vec!["1girl", "solo", "long hair", "smile"]
        );
    }

    #[test]
    fn add_tags_skips_existing_by_normalized_key() {
        // 既存 "Solo" と正規化キーが一致する " solo " は追加しない。
        let existing = ["1girl".to_string(), "Solo".to_string()];
        assert_eq!(
            add_tags(&existing, &[" solo ", "SOLO", "long hair"]),
            vec!["1girl", "Solo", "long hair"]
        );
    }

    #[test]
    fn add_tags_dedups_within_to_add_first_wins() {
        // to_add 内の重複は先勝ちで1つに畳み込む（原文は最初のものを保持）。
        let existing: [String; 0] = [];
        assert_eq!(
            add_tags(&existing, &["Long Hair", "long hair", "LONG HAIR"]),
            vec!["Long Hair"]
        );
    }

    #[test]
    fn add_tags_ignores_empty_tokens() {
        let existing = ["1girl".to_string()];
        assert_eq!(
            add_tags(&existing, &["", "   ", "solo"]),
            vec!["1girl", "solo"]
        );
    }

    #[test]
    fn add_tags_is_idempotent() {
        let existing = ["1girl".to_string(), "solo".to_string()];
        let once = add_tags(&existing, &["long hair", "solo"]);
        let twice = add_tags(&once, &["long hair", "solo"]);
        assert_eq!(once, twice);
    }

    #[test]
    fn add_tags_empty_inputs() {
        let existing = ["1girl".to_string()];
        let none: [String; 0] = [];
        assert_eq!(add_tags(&existing, &none), vec!["1girl"]);
        assert_eq!(add_tags(&none, &["solo"]), vec!["solo"]);
    }

    // ---- remove_tags ----

    #[test]
    fn remove_tags_removes_all_matching_by_key() {
        // 正規化キー一致する全出現を除去（大小・トリム無視）。
        let tags = [
            "1girl".to_string(),
            "Solo".to_string(),
            "solo".to_string(),
            " SOLO ".to_string(),
        ];
        assert_eq!(remove_tags(&tags, &["solo"]), vec!["1girl"]);
    }

    #[test]
    fn remove_tags_preserves_remaining_order_and_case() {
        let tags = [
            "Long Hair".to_string(),
            "1girl".to_string(),
            "smile".to_string(),
        ];
        assert_eq!(
            remove_tags(&tags, &["1GIRL"]),
            vec!["Long Hair", "smile"]
        );
    }

    #[test]
    fn remove_tags_no_match_returns_all() {
        let tags = ["1girl".to_string(), "solo".to_string()];
        assert_eq!(remove_tags(&tags, &["duo"]), vec!["1girl", "solo"]);
    }

    #[test]
    fn remove_tags_empty_inputs() {
        let tags = ["1girl".to_string()];
        let none: [String; 0] = [];
        assert_eq!(remove_tags(&tags, &none), vec!["1girl"]);
        assert!(remove_tags(&none, &["solo"]).is_empty());
    }

    // ---- dedup_tags ----

    #[test]
    fn dedup_tags_keeps_first_occurrence() {
        let tags = [
            "Solo".to_string(),
            "1girl".to_string(),
            "solo".to_string(),
            " SOLO ".to_string(),
        ];
        assert_eq!(dedup_tags(&tags), vec!["Solo", "1girl"]);
    }

    #[test]
    fn dedup_tags_preserves_first_seen_order() {
        let tags = [
            "b".to_string(),
            "a".to_string(),
            "c".to_string(),
            "a".to_string(),
            "b".to_string(),
        ];
        assert_eq!(dedup_tags(&tags), vec!["b", "a", "c"]);
    }

    #[test]
    fn dedup_tags_is_idempotent() {
        let tags = [
            "Solo".to_string(),
            "solo".to_string(),
            "1girl".to_string(),
        ];
        let once = dedup_tags(&tags);
        let twice = dedup_tags(&once);
        assert_eq!(once, twice);
    }

    #[test]
    fn dedup_tags_ignores_empty_tokens() {
        let tags = [
            "1girl".to_string(),
            "".to_string(),
            "  ".to_string(),
            "1GIRL".to_string(),
        ];
        assert_eq!(dedup_tags(&tags), vec!["1girl"]);
    }

    #[test]
    fn dedup_tags_empty_list() {
        let tags: [String; 0] = [];
        assert!(dedup_tags(&tags).is_empty());
    }
}
