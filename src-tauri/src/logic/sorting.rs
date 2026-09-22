//! サイズ振分決定・閾値判定・タグ振分決定（タスク 6）。
//!
//! いずれもファイル I/O に依存しない純粋関数として実装する。実際の移動・
//! コピー・Tag_File 対保存は後続の SortService（タスク 14）が本モジュールの
//! 決定結果を用いて行う。ここでは「どの宛先に振り分けるか」「閾値が妥当か」の
//! 決定ロジックのみを担う。

use std::collections::HashSet;

// ---------------------------------------------------------------------------
// タスク 6.1: 画像サイズ振分先の一意決定
// ---------------------------------------------------------------------------

/// 画像の向き（要件 9.3）。
///
/// 幅と高さの大小関係で決まる。幅 == 高さ は横長として扱う（design.md
/// SortService「幅高さから縦横（同値は横長）」／Property 16）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Orientation {
    /// 横長（幅 ≧ 高さ）。
    Landscape,
    /// 縦長（幅 < 高さ）。
    Portrait,
}

/// 長辺と閾値の大小区分（要件 9.3）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LongSideBand {
    /// 長辺が閾値以上。
    AtOrAbove,
    /// 長辺が閾値未満。
    Below,
}

/// サイズ振分先（要件 9.3）。
///
/// 「向き（横長/縦長）」と「長辺が閾値以上/未満か」の 2×2 の組み合わせにより、
/// 4 つの宛先のうち必ずちょうど 1 つに一意決定される（design.md Property 16）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SizeDestination {
    /// 向き。
    pub orientation: Orientation,
    /// 長辺と閾値の区分。
    pub band: LongSideBand,
}

/// 幅と高さから向きを決定する（要件 9.3）。
///
/// 幅 ≧ 高さ を [`Orientation::Landscape`]、幅 < 高さ を [`Orientation::Portrait`]
/// とする。幅 == 高さ は横長扱い（Property 16 の「width==height treated as
/// landscape」）。
pub fn orientation_of(width: u32, height: u32) -> Orientation {
    if width >= height {
        Orientation::Landscape
    } else {
        Orientation::Portrait
    }
}

/// 画像サイズと長辺閾値から振分先を一意決定する（要件 9.3）。
///
/// - 向き: [`orientation_of`]（幅 ≧ 高さ→横長、幅 < 高さ→縦長）。
/// - 長辺: `max(width, height)`。これが `threshold` 以上なら
///   [`LongSideBand::AtOrAbove`]、未満なら [`LongSideBand::Below`]。
///
/// 呼び出し側は [`is_valid_long_side_threshold`] で妥当性を確認済みの閾値を
/// 渡す前提。任意の入力に対し結果はちょうど 1 つに定まる（design.md Property 16）。
pub fn decide_size_destination(width: u32, height: u32, threshold: u32) -> SizeDestination {
    let orientation = orientation_of(width, height);
    let long_side = width.max(height);
    let band = if long_side >= threshold {
        LongSideBand::AtOrAbove
    } else {
        LongSideBand::Below
    };
    SizeDestination { orientation, band }
}

// ---------------------------------------------------------------------------
// タスク 6.3: 長辺閾値の妥当性判定
// ---------------------------------------------------------------------------

/// 長辺閾値の下限（含む）。
pub const LONG_SIDE_THRESHOLD_MIN: i64 = 1;
/// 長辺閾値の上限（含む）。
pub const LONG_SIDE_THRESHOLD_MAX: i64 = 100_000;

/// 長辺閾値が妥当か判定する（要件 9.2）。
///
/// 妥当であることは `1 ≤ 値 ≤ 100000` と同値（design.md Property 17）。
/// 「整数として解釈できない」入力（非数値・小数）は文字列パースの段階で弾かれる
/// ため、本関数は整数値を受け取り範囲のみを判定する。`i64` を受けることで
/// 範囲外の負値や上限超過も同一の述語で扱える。
pub fn is_valid_long_side_threshold(value: i64) -> bool {
    (LONG_SIDE_THRESHOLD_MIN..=LONG_SIDE_THRESHOLD_MAX).contains(&value)
}

// ---------------------------------------------------------------------------
// タスク 6.5: タグによる振分先の一意決定
// ---------------------------------------------------------------------------

/// タグ振分先（要件 10）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TagDestination {
    /// 判定タグのいずれかを含む。
    Contains,
    /// 判定タグのいずれも含まない（Tag_File 無しもここに分類）。
    NotContains,
}

/// 画像タグと判定タグから振分先を一意決定する（要件 10.1〜10.4）。
///
/// - `image_tags`: 対象画像のタグ列。Tag_File が存在しない場合は空スライスを
///   渡す（要件 10.4: Tag_File 無しは「含まない」扱い＝空集合）。
/// - `judge_tags`: 判定タグ列。
///
/// 判定は前後空白をトリムした完全一致（要件 10.1）。画像タグ集合と判定タグ集合の
/// 交差が非空なら [`TagDestination::Contains`]、そうでなければ
/// [`TagDestination::NotContains`]。各画像はちょうど 1 つの宛先に分類される
/// （design.md Property 18）。
///
/// 大文字小文字は区別する（要件 10.1 はトリムのみの完全一致を規定。集計・一括
/// 操作で用いる大小無視の正規化キーとは別ルール）。
pub fn decide_tag_destination<S1, S2>(image_tags: &[S1], judge_tags: &[S2]) -> TagDestination
where
    S1: AsRef<str>,
    S2: AsRef<str>,
{
    // 判定タグをトリム済み集合へ。空トークンは判定に寄与しないため除外する。
    let judge_set: HashSet<&str> = judge_tags
        .iter()
        .map(|t| t.as_ref().trim())
        .filter(|t| !t.is_empty())
        .collect();

    if judge_set.is_empty() {
        // 判定タグが実質空なら交差は必ず空 → 「含まない」。
        return TagDestination::NotContains;
    }

    let intersects = image_tags
        .iter()
        .map(|t| t.as_ref().trim())
        .filter(|t| !t.is_empty())
        .any(|t| judge_set.contains(t));

    if intersects {
        TagDestination::Contains
    } else {
        TagDestination::NotContains
    }
}

// ===========================================================================
// Unit tests
// ===========================================================================

#[cfg(test)]
mod size_destination_tests {
    use super::*;

    #[test]
    fn width_greater_than_height_is_landscape() {
        assert_eq!(orientation_of(200, 100), Orientation::Landscape);
    }

    #[test]
    fn width_less_than_height_is_portrait() {
        assert_eq!(orientation_of(100, 200), Orientation::Portrait);
    }

    #[test]
    fn square_is_treated_as_landscape() {
        // Property 16: width == height は横長として扱う。
        assert_eq!(orientation_of(128, 128), Orientation::Landscape);
        assert_eq!(orientation_of(0, 0), Orientation::Landscape);
    }

    #[test]
    fn long_side_at_threshold_is_at_or_above() {
        // 長辺 == 閾値 は「以上」区分（境界を含む）。
        let d = decide_size_destination(1024, 512, 1024);
        assert_eq!(d.orientation, Orientation::Landscape);
        assert_eq!(d.band, LongSideBand::AtOrAbove);
    }

    #[test]
    fn long_side_below_threshold_is_below() {
        let d = decide_size_destination(512, 300, 1024);
        assert_eq!(d.band, LongSideBand::Below);
    }

    #[test]
    fn long_side_uses_max_of_width_height_for_portrait() {
        // 縦長では高さが長辺。高さ 2000 ≧ 閾値 1024。
        let d = decide_size_destination(500, 2000, 1024);
        assert_eq!(d.orientation, Orientation::Portrait);
        assert_eq!(d.band, LongSideBand::AtOrAbove);
    }

    #[test]
    fn all_four_destinations_are_reachable_and_distinct() {
        let threshold = 1000;
        let landscape_above = decide_size_destination(2000, 1000, threshold);
        let landscape_below = decide_size_destination(800, 400, threshold);
        let portrait_above = decide_size_destination(1000, 2000, threshold);
        let portrait_below = decide_size_destination(400, 800, threshold);

        assert_eq!(
            landscape_above,
            SizeDestination {
                orientation: Orientation::Landscape,
                band: LongSideBand::AtOrAbove
            }
        );
        assert_eq!(
            landscape_below,
            SizeDestination {
                orientation: Orientation::Landscape,
                band: LongSideBand::Below
            }
        );
        assert_eq!(
            portrait_above,
            SizeDestination {
                orientation: Orientation::Portrait,
                band: LongSideBand::AtOrAbove
            }
        );
        assert_eq!(
            portrait_below,
            SizeDestination {
                orientation: Orientation::Portrait,
                band: LongSideBand::Below
            }
        );

        // 4 宛先はすべて相異なる。
        let all = [
            landscape_above,
            landscape_below,
            portrait_above,
            portrait_below,
        ];
        for i in 0..all.len() {
            for j in (i + 1)..all.len() {
                assert_ne!(all[i], all[j]);
            }
        }
    }

    #[test]
    fn decision_is_deterministic() {
        // 同一入力は常に同一結果。
        let a = decide_size_destination(1920, 1080, 1280);
        let b = decide_size_destination(1920, 1080, 1280);
        assert_eq!(a, b);
    }
}

#[cfg(test)]
mod threshold_tests {
    use super::*;

    #[test]
    fn min_boundary_is_valid() {
        assert!(is_valid_long_side_threshold(1));
    }

    #[test]
    fn max_boundary_is_valid() {
        assert!(is_valid_long_side_threshold(100_000));
    }

    #[test]
    fn below_min_is_invalid() {
        assert!(!is_valid_long_side_threshold(0));
        assert!(!is_valid_long_side_threshold(-1));
        assert!(!is_valid_long_side_threshold(i64::MIN));
    }

    #[test]
    fn above_max_is_invalid() {
        assert!(!is_valid_long_side_threshold(100_001));
        assert!(!is_valid_long_side_threshold(i64::MAX));
    }

    #[test]
    fn typical_values_are_valid() {
        for v in [1, 512, 1024, 4096, 99_999, 100_000] {
            assert!(is_valid_long_side_threshold(v), "expected {v} to be valid");
        }
    }
}

#[cfg(test)]
mod tag_destination_tests {
    use super::*;

    fn v(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn intersection_non_empty_is_contains() {
        // 要件 10.1/10.2: 判定タグのいずれかを含む → 含む。
        let d = decide_tag_destination(&v(&["cat", "dog", "sky"]), &v(&["dog"]));
        assert_eq!(d, TagDestination::Contains);
    }

    #[test]
    fn no_intersection_is_not_contains() {
        // 要件 10.3: 判定タグのいずれも含まない → 含まない。
        let d = decide_tag_destination(&v(&["cat", "sky"]), &v(&["dog", "bird"]));
        assert_eq!(d, TagDestination::NotContains);
    }

    #[test]
    fn missing_tag_file_is_not_contains() {
        // 要件 10.4: Tag_File 無し（空スライス）は「含まない」。
        let empty: Vec<String> = Vec::new();
        let d = decide_tag_destination(&empty, &v(&["cat"]));
        assert_eq!(d, TagDestination::NotContains);
    }

    #[test]
    fn trim_only_exact_match() {
        // 要件 10.1: 前後トリムした完全一致。トリム後に一致すれば含む。
        let d = decide_tag_destination(&v(&["  dog  "]), &v(&["dog"]));
        assert_eq!(d, TagDestination::Contains);
        let d2 = decide_tag_destination(&v(&["dog"]), &v(&["  dog  "]));
        assert_eq!(d2, TagDestination::Contains);
    }

    #[test]
    fn match_is_case_sensitive() {
        // 要件 10.1 はトリムのみの完全一致（大小区別あり）。
        let d = decide_tag_destination(&v(&["Dog"]), &v(&["dog"]));
        assert_eq!(d, TagDestination::NotContains);
    }

    #[test]
    fn partial_substring_does_not_match() {
        // 完全一致のみ。部分一致では含むにならない。
        let d = decide_tag_destination(&v(&["dogface"]), &v(&["dog"]));
        assert_eq!(d, TagDestination::NotContains);
    }

    #[test]
    fn empty_judge_tags_is_not_contains() {
        // 判定タグが空なら交差は必ず空 → 含まない。
        let d = decide_tag_destination(&v(&["cat", "dog"]), &Vec::<String>::new());
        assert_eq!(d, TagDestination::NotContains);
    }

    #[test]
    fn whitespace_only_tags_are_ignored() {
        // 全空白の判定タグ/画像タグはトリム後空となり判定に寄与しない。
        let d = decide_tag_destination(&v(&["  ", "\t"]), &v(&["   "]));
        assert_eq!(d, TagDestination::NotContains);
        // 空白のみの判定タグは実質空 → 含まない。
        let d2 = decide_tag_destination(&v(&["dog"]), &v(&["   "]));
        assert_eq!(d2, TagDestination::NotContains);
    }

    #[test]
    fn multiple_judge_tags_any_match_is_contains() {
        // 複数判定タグのいずれか 1 つでも一致すれば含む。
        let d = decide_tag_destination(&v(&["sky", "night"]), &v(&["cat", "night", "dog"]));
        assert_eq!(d, TagDestination::Contains);
    }

    #[test]
    fn multibyte_tags_match() {
        let d = decide_tag_destination(&v(&["猫", "空"]), &v(&["猫"]));
        assert_eq!(d, TagDestination::Contains);
    }

    #[test]
    fn accepts_str_slices() {
        // AsRef<str> により &str スライスも受け付ける。
        let d = decide_tag_destination(&["cat", "dog"], &["dog"]);
        assert_eq!(d, TagDestination::Contains);
    }
}
