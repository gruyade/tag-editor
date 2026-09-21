//! タグ正規化・booru 変換・信頼度 parse/render（タスク 2）。

use crate::models::Tag;

/// タグ文字列を Booru_Format へ変換する（要件 5.1 / 5.2）。
///
/// - アンダースコア `_` をスペースに置換（要件 5.1）。
/// - 括弧文字 `(` および `)` をバックスラッシュでエスケープ（要件 5.2）。
pub fn to_booru(tag: &str) -> String {
    let mut out = String::with_capacity(tag.len());
    for ch in tag.chars() {
        match ch {
            '_' => out.push(' '),
            '(' => out.push_str("\\("),
            ')' => out.push_str("\\)"),
            other => out.push(other),
        }
    }
    out
}

/// 信頼度トークンを本体と信頼度へ分離する（要件 5.3, 5.4）。
///
/// `(tag:0.9)` 形式のトークンを本体（`tag`）と 0.0〜1.0 の信頼度へ分離する。
/// 信頼度部分が 0.0〜1.0 の数値として解釈できない場合、または `(…:…)`
/// 形式でない場合は、トークン全体（前後トリム済み）を信頼度なしの本体として扱う。
///
/// # 戻り値
///
/// 分離した本体と信頼度を保持する [`Tag`]。
pub fn parse_confidence(token: &str) -> Tag {
    let trimmed = token.trim();

    if let Some(tag) = try_parse_confidence(trimmed) {
        tag
    } else {
        Tag::new(trimmed)
    }
}

/// `(<body>:<value>)` 形式の解析を試みる。
///
/// 形式に一致し `value` が 0.0〜1.0 の数値として解釈できる場合のみ
/// 信頼度付きの [`Tag`] を返す。それ以外は `None`。
fn try_parse_confidence(trimmed: &str) -> Option<Tag> {
    // 外側の括弧を剥がす。
    let inner = trimmed.strip_prefix('(')?.strip_suffix(')')?;

    // 最後の ':' で本体と値を分離する。本体側に ':' を含み得るため rfind を使う。
    let sep = inner.rfind(':')?;
    let body = inner[..sep].trim();
    let value_str = inner[sep + 1..].trim();

    // 本体が空の場合は信頼度トークンとして扱わない。
    if body.is_empty() {
        return None;
    }

    let value: f32 = value_str.parse().ok()?;

    // 0.0〜1.0 の範囲外・非数値（NaN 等）は信頼度なし扱い。
    if !value.is_finite() || !(0.0..=1.0).contains(&value) {
        return None;
    }

    Some(Tag::with_confidence(body, value))
}

/// タグ列を編集領域向けの文字列へ描画する（要件 5.5, 5.6）。
///
/// `show_confidence` が真のとき、信頼度を持つタグは小数第2位までの
/// `(<body>:<confidence>)` 形式で描画する。信頼度を持たないタグは本体のみ。
/// `show_confidence` が偽のときは、すべてのタグを本体のみで描画する。
///
/// 複数タグはカンマ＋スペース（`, `）で連結する。
pub fn render_tags(tags: &[Tag], show_confidence: bool) -> String {
    tags.iter()
        .map(|tag| render_tag(tag, show_confidence))
        .collect::<Vec<_>>()
        .join(", ")
}

/// 単一タグを描画する。
fn render_tag(tag: &Tag, show_confidence: bool) -> String {
    match (show_confidence, tag.confidence) {
        (true, Some(confidence)) => format!("({}:{:.2})", tag.body, confidence),
        _ => tag.body.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_confidence_splits_body_and_value() {
        let tag = parse_confidence("(smile:0.9)");
        assert_eq!(tag.body, "smile");
        assert_eq!(tag.confidence, Some(0.9));
    }

    #[test]
    fn parse_confidence_trims_surrounding_whitespace() {
        let tag = parse_confidence("  (smile:0.9)  ");
        assert_eq!(tag.body, "smile");
        assert_eq!(tag.confidence, Some(0.9));
    }

    #[test]
    fn parse_confidence_accepts_boundary_values() {
        assert_eq!(parse_confidence("(a:0.0)").confidence, Some(0.0));
        assert_eq!(parse_confidence("(a:1.0)").confidence, Some(1.0));
        assert_eq!(parse_confidence("(a:0)").confidence, Some(0.0));
        assert_eq!(parse_confidence("(a:1)").confidence, Some(1.0));
    }

    #[test]
    fn parse_confidence_out_of_range_is_body_only() {
        // 範囲外の値は括弧付きトークン全体を本体として扱う。
        let tag = parse_confidence("(a:1.5)");
        assert_eq!(tag.body, "(a:1.5)");
        assert_eq!(tag.confidence, None);

        let neg = parse_confidence("(a:-0.1)");
        assert_eq!(neg.body, "(a:-0.1)");
        assert_eq!(neg.confidence, None);
    }

    #[test]
    fn parse_confidence_non_numeric_is_body_only() {
        let tag = parse_confidence("(a:high)");
        assert_eq!(tag.body, "(a:high)");
        assert_eq!(tag.confidence, None);
    }

    #[test]
    fn parse_confidence_without_paren_is_body_only() {
        let tag = parse_confidence("smile");
        assert_eq!(tag.body, "smile");
        assert_eq!(tag.confidence, None);
    }

    #[test]
    fn parse_confidence_without_colon_is_body_only() {
        let tag = parse_confidence("(smile)");
        assert_eq!(tag.body, "(smile)");
        assert_eq!(tag.confidence, None);
    }

    #[test]
    fn parse_confidence_empty_body_is_body_only() {
        // 本体が空なら信頼度トークンとして扱わない。
        let tag = parse_confidence("(:0.5)");
        assert_eq!(tag.body, "(:0.5)");
        assert_eq!(tag.confidence, None);
    }

    #[test]
    fn parse_confidence_body_may_contain_colon() {
        // 最後の ':' で分離するため本体側に ':' を含められる。
        let tag = parse_confidence("(character:name:0.75)");
        assert_eq!(tag.body, "character:name");
        assert_eq!(tag.confidence, Some(0.75));
    }

    #[test]
    fn parse_confidence_nan_is_body_only() {
        let tag = parse_confidence("(a:NaN)");
        assert_eq!(tag.body, "(a:NaN)");
        assert_eq!(tag.confidence, None);
    }

    #[test]
    fn render_tags_on_formats_confidence_two_decimals() {
        let tags = vec![Tag::with_confidence("smile", 0.9)];
        assert_eq!(render_tags(&tags, true), "(smile:0.90)");
    }

    #[test]
    fn render_tags_on_rounds_to_two_decimals() {
        let tags = vec![Tag::with_confidence("smile", 0.125)];
        assert_eq!(render_tags(&tags, true), "(smile:0.12)");

        let tags = vec![Tag::with_confidence("smile", 0.126)];
        assert_eq!(render_tags(&tags, true), "(smile:0.13)");
    }

    #[test]
    fn render_tags_on_body_only_when_no_confidence() {
        let tags = vec![Tag::new("smile")];
        assert_eq!(render_tags(&tags, true), "smile");
    }

    #[test]
    fn render_tags_off_shows_body_only() {
        let tags = vec![
            Tag::with_confidence("smile", 0.9),
            Tag::new("solo"),
        ];
        assert_eq!(render_tags(&tags, false), "smile, solo");
    }

    #[test]
    fn render_tags_joins_with_comma_space() {
        let tags = vec![
            Tag::with_confidence("smile", 0.9),
            Tag::with_confidence("solo", 0.5),
        ];
        assert_eq!(render_tags(&tags, true), "(smile:0.90), (solo:0.50)");
    }

    #[test]
    fn render_tags_empty_is_empty_string() {
        assert_eq!(render_tags(&[], true), "");
        assert_eq!(render_tags(&[], false), "");
    }

    #[test]
    fn parse_then_render_roundtrip() {
        // render(ON) した文字列を parse すると本体と丸めた信頼度が復元される。
        let original = Tag::with_confidence("smile", 0.9);
        let rendered = render_tags(std::slice::from_ref(&original), true);
        let parsed = parse_confidence(&rendered);
        assert_eq!(parsed.body, "smile");
        assert_eq!(parsed.confidence, Some(0.90));
    }
}

#[cfg(test)]
mod to_booru_tests {
    use super::*;

    #[test]
    fn underscore_becomes_space() {
        assert_eq!(to_booru("hatsune_miku"), "hatsune miku");
    }

    #[test]
    fn parentheses_are_escaped() {
        assert_eq!(to_booru("(vocaloid)"), "\\(vocaloid\\)");
    }

    #[test]
    fn combined_underscore_and_parentheses() {
        assert_eq!(
            to_booru("hatsune_miku_(vocaloid)"),
            "hatsune miku \\(vocaloid\\)"
        );
    }

    #[test]
    fn plain_tag_unchanged() {
        assert_eq!(to_booru("smile"), "smile");
    }

    #[test]
    fn empty_unchanged() {
        assert_eq!(to_booru(""), "");
    }

    #[test]
    fn non_ascii_preserved() {
        // 非 ASCII 文字はそのまま保持し、アンダースコアのみ変換する。
        assert_eq!(to_booru("笑顔_の_猫"), "笑顔 の 猫");
    }

    #[test]
    fn property9_no_underscore_or_bare_parenthesis() {
        // Property-9: 変換後はアンダースコアを含まず、
        // すべての括弧はバックスラッシュでエスケープされている（裸の括弧がない）。
        let inputs = [
            "hatsune_miku_(vocaloid)",
            "a_(b)_c_(d)",
            "no_special_here",
            "笑顔_の_猫_(かわいい)",
            "()",
            "_(_)_",
        ];

        for input in inputs {
            let converted = to_booru(input);

            // アンダースコアは残らない。
            assert!(
                !converted.contains('_'),
                "変換後にアンダースコアが残存: input={input:?} -> {converted:?}"
            );

            // すべての括弧は直前がバックスラッシュ（裸の括弧がない）。
            let chars: Vec<char> = converted.chars().collect();
            for (i, &ch) in chars.iter().enumerate() {
                if ch == '(' || ch == ')' {
                    assert!(
                        i > 0 && chars[i - 1] == '\\',
                        "裸の括弧を検出: input={input:?} -> {converted:?} at {i}"
                    );
                }
            }
        }
    }
}
