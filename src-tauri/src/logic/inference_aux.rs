//! 閾値採用・動画除外・Batch_Size 解決・バッチ分割（タスク 8）。
//!
//! ファイル I/O や推論に依存しない決定的関数群。
//! - 信頼度閾値によるタグ採用（要件 14.4）
//! - 動画（mp4）ファイルの推論除外（要件 14.8）
//! - Batch_Size の解決（要件 17.3, 17.4, 17.5）
//! - バッチ分割（要件 17.2）

use crate::models::Tag;

/// Batch_Size の既定値（要件 17.4）。
pub const DEFAULT_BATCH_SIZE: u32 = 8;

/// Batch_Size の下限（要件 17.3）。
pub const MIN_BATCH_SIZE: u32 = 1;

/// Batch_Size の上限（要件 17.3）。
pub const MAX_BATCH_SIZE: u32 = 64;

/// 信頼度が閾値以上のタグのみを採用する（要件 14.4）。
///
/// 各タグについて、信頼度が `threshold` 以上（`confidence >= threshold`）のもの
/// だけを採用し、順序を保存したまま返す。
///
/// # 信頼度なしタグの扱い
///
/// 推論結果は常に信頼度を伴う想定だが、[`Tag::confidence`] が `None` のタグも
/// 安全に扱う。信頼度が無いタグは「閾値と比較できない」ため採用しない
/// （閾値を満たすことを確認できない項目は不採用とする保守的な方針）。
/// これにより Property 22（採用集合＝信頼度が閾値以上のタグ集合）が
/// 信頼度付きタグに対して厳密に成立する。
///
/// # 引数
///
/// - `tags`: 判定対象のタグ列（閾値適用前の全確信度）。
/// - `threshold`: 採用の下限信頼度（0.0〜1.0 を想定）。
///
/// # 戻り値
///
/// 採用されたタグの複製列（入力順を保存）。
pub fn adopt_by_threshold(tags: &[Tag], threshold: f32) -> Vec<Tag> {
    tags.iter()
        .filter(|tag| matches!(tag.confidence, Some(c) if c >= threshold))
        .cloned()
        .collect()
}

/// パスが動画（mp4）拡張子かどうかを判定する（要件 14.8）。
///
/// 拡張子の判定は大文字小文字を区別しない（`.mp4` / `.MP4` / `.Mp4` すべて真）。
fn is_mp4(path: &str) -> bool {
    // 最後の '.' 以降を拡張子として取り出す。区切り文字は '/' と '\\' の双方を考慮。
    let file_name = path
        .rsplit(|c| c == '/' || c == '\\')
        .next()
        .unwrap_or(path);
    match file_name.rsplit_once('.') {
        Some((_, ext)) => ext.eq_ignore_ascii_case("mp4"),
        None => false,
    }
}

/// 推論対象から動画（mp4）ファイルを除外する（要件 14.8）。
///
/// mp4 拡張子（大文字小文字を区別しない）を持つパスを取り除き、
/// 残りのパスを入力順を保存して返す。
///
/// # 戻り値
///
/// mp4 を除外したパスの複製列（入力順を保存）。
pub fn exclude_videos(paths: &[String]) -> Vec<String> {
    paths
        .iter()
        .filter(|path| !is_mp4(path))
        .cloned()
        .collect()
}

/// Batch_Size を解決する（要件 17.3, 17.4, 17.5）。
///
/// - 未指定（`None`）なら既定値 8 を用いる（要件 17.4）。
/// - 指定値は 1〜64 の範囲へ丸める（clamp）。範囲内の値はそのまま用いる
///   （要件 17.3, 17.5）。
///
/// # 戻り値
///
/// 常に 1〜64 の範囲に収まる Batch_Size。
pub fn resolve_batch_size(requested: Option<u32>) -> u32 {
    match requested {
        None => DEFAULT_BATCH_SIZE,
        Some(value) => value.clamp(MIN_BATCH_SIZE, MAX_BATCH_SIZE),
    }
}

/// 列を Batch_Size 単位のチャンクへ分割する（要件 17.2）。
///
/// - 各チャンクの長さは `batch_size` 以下。
/// - チャンクを順に連結すると元の列（順序を保存）に一致する。
/// - 最終チャンク以外の長さはすべて `batch_size` に等しい。
///
/// `batch_size` は 1 以上を前提とする（[`resolve_batch_size`] で保証される）。
/// 万一 0 が渡された場合は 1 として扱い、パニックを避ける。
///
/// # 戻り値
///
/// 入力要素の複製を保持するチャンク列。入力が空なら空の列。
pub fn split_into_batches<T: Clone>(items: &[T], batch_size: u32) -> Vec<Vec<T>> {
    let chunk = (batch_size.max(1)) as usize;
    items.chunks(chunk).map(<[T]>::to_vec).collect()
}

#[cfg(test)]
mod adopt_by_threshold_tests {
    use super::*;

    #[test]
    fn adopts_tags_at_or_above_threshold() {
        let tags = vec![
            Tag::with_confidence("a", 0.9),
            Tag::with_confidence("b", 0.5),
            Tag::with_confidence("c", 0.3),
        ];
        let adopted = adopt_by_threshold(&tags, 0.5);
        let bodies: Vec<&str> = adopted.iter().map(|t| t.body.as_str()).collect();
        assert_eq!(bodies, vec!["a", "b"]);
    }

    #[test]
    fn threshold_boundary_is_inclusive() {
        // 信頼度がちょうど閾値のタグは採用する（>= 判定）。
        let tags = vec![Tag::with_confidence("a", 0.5)];
        assert_eq!(adopt_by_threshold(&tags, 0.5).len(), 1);
    }

    #[test]
    fn threshold_zero_adopts_all_with_confidence() {
        let tags = vec![
            Tag::with_confidence("a", 0.0),
            Tag::with_confidence("b", 1.0),
        ];
        assert_eq!(adopt_by_threshold(&tags, 0.0).len(), 2);
    }

    #[test]
    fn threshold_one_adopts_only_perfect() {
        let tags = vec![
            Tag::with_confidence("a", 1.0),
            Tag::with_confidence("b", 0.99),
        ];
        let adopted = adopt_by_threshold(&tags, 1.0);
        let bodies: Vec<&str> = adopted.iter().map(|t| t.body.as_str()).collect();
        assert_eq!(bodies, vec!["a"]);
    }

    #[test]
    fn tags_without_confidence_are_not_adopted() {
        // 信頼度なしのタグは閾値と比較できないため採用しない。
        let tags = vec![Tag::new("a"), Tag::with_confidence("b", 0.9)];
        let adopted = adopt_by_threshold(&tags, 0.0);
        let bodies: Vec<&str> = adopted.iter().map(|t| t.body.as_str()).collect();
        assert_eq!(bodies, vec!["b"]);
    }

    #[test]
    fn preserves_input_order() {
        let tags = vec![
            Tag::with_confidence("z", 0.9),
            Tag::with_confidence("a", 0.9),
            Tag::with_confidence("m", 0.9),
        ];
        let adopted = adopt_by_threshold(&tags, 0.5);
        let bodies: Vec<&str> = adopted.iter().map(|t| t.body.as_str()).collect();
        assert_eq!(bodies, vec!["z", "a", "m"]);
    }

    #[test]
    fn empty_input_yields_empty() {
        assert!(adopt_by_threshold(&[], 0.5).is_empty());
    }
}

#[cfg(test)]
mod exclude_videos_tests {
    use super::*;

    fn paths(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn excludes_mp4_files() {
        let input = paths(&["a.png", "b.mp4", "c.jpg"]);
        assert_eq!(exclude_videos(&input), paths(&["a.png", "c.jpg"]));
    }

    #[test]
    fn extension_check_is_case_insensitive() {
        let input = paths(&["a.MP4", "b.Mp4", "c.mP4", "d.mp4"]);
        assert!(exclude_videos(&input).is_empty());
    }

    #[test]
    fn preserves_order_of_remaining() {
        let input = paths(&["z.jpg", "y.mp4", "x.png", "w.gif"]);
        assert_eq!(exclude_videos(&input), paths(&["z.jpg", "x.png", "w.gif"]));
    }

    #[test]
    fn handles_paths_with_directories() {
        let input = paths(&[
            "C:\\images\\clip.MP4",
            "/home/user/pic.png",
            "sub/dir/video.mp4",
        ]);
        assert_eq!(exclude_videos(&input), paths(&["/home/user/pic.png"]));
    }

    #[test]
    fn file_without_extension_is_kept() {
        let input = paths(&["README", "video.mp4"]);
        assert_eq!(exclude_videos(&input), paths(&["README"]));
    }

    #[test]
    fn mp4_substring_in_name_is_not_excluded() {
        // 拡張子のみで判定するため、名前に mp4 を含んでも拡張子でなければ残す。
        let input = paths(&["mp4_thumbnail.png", "my.mp4.backup.png"]);
        assert_eq!(
            exclude_videos(&input),
            paths(&["mp4_thumbnail.png", "my.mp4.backup.png"])
        );
    }

    #[test]
    fn empty_input_yields_empty() {
        assert!(exclude_videos(&[]).is_empty());
    }
}

#[cfg(test)]
mod resolve_batch_size_tests {
    use super::*;

    #[test]
    fn none_uses_default_eight() {
        assert_eq!(resolve_batch_size(None), 8);
    }

    #[test]
    fn in_range_values_pass_through() {
        assert_eq!(resolve_batch_size(Some(1)), 1);
        assert_eq!(resolve_batch_size(Some(8)), 8);
        assert_eq!(resolve_batch_size(Some(32)), 32);
        assert_eq!(resolve_batch_size(Some(64)), 64);
    }

    #[test]
    fn below_min_clamps_to_one() {
        assert_eq!(resolve_batch_size(Some(0)), 1);
    }

    #[test]
    fn above_max_clamps_to_sixtyfour() {
        assert_eq!(resolve_batch_size(Some(65)), 64);
        assert_eq!(resolve_batch_size(Some(1000)), 64);
        assert_eq!(resolve_batch_size(Some(u32::MAX)), 64);
    }

    #[test]
    fn result_always_within_range() {
        for value in [0u32, 1, 5, 64, 65, 999] {
            let resolved = resolve_batch_size(Some(value));
            assert!((MIN_BATCH_SIZE..=MAX_BATCH_SIZE).contains(&resolved));
        }
    }
}

#[cfg(test)]
mod split_into_batches_tests {
    use super::*;

    #[test]
    fn splits_evenly() {
        let items: Vec<u32> = (1..=6).collect();
        let batches = split_into_batches(&items, 2);
        assert_eq!(batches, vec![vec![1, 2], vec![3, 4], vec![5, 6]]);
    }

    #[test]
    fn last_chunk_may_be_shorter() {
        let items: Vec<u32> = (1..=7).collect();
        let batches = split_into_batches(&items, 3);
        assert_eq!(batches, vec![vec![1, 2, 3], vec![4, 5, 6], vec![7]]);
    }

    #[test]
    fn batch_larger_than_input_yields_single_chunk() {
        let items = vec![1, 2, 3];
        let batches = split_into_batches(&items, 10);
        assert_eq!(batches, vec![vec![1, 2, 3]]);
    }

    #[test]
    fn concatenation_preserves_order() {
        let items: Vec<u32> = (1..=10).collect();
        let batches = split_into_batches(&items, 3);
        let flat: Vec<u32> = batches.into_iter().flatten().collect();
        assert_eq!(flat, items);
    }

    #[test]
    fn all_but_last_equal_batch_size() {
        let items: Vec<u32> = (1..=10).collect();
        let batch_size = 4u32;
        let batches = split_into_batches(&items, batch_size);
        for chunk in &batches[..batches.len() - 1] {
            assert_eq!(chunk.len(), batch_size as usize);
        }
        assert!(batches.last().unwrap().len() <= batch_size as usize);
    }

    #[test]
    fn empty_input_yields_empty() {
        let items: Vec<u32> = vec![];
        assert!(split_into_batches(&items, 4).is_empty());
    }

    #[test]
    fn zero_batch_size_treated_as_one() {
        // 0 は 1 として扱いパニックを避ける（resolve_batch_size で通常は 1 以上）。
        let items = vec![1, 2, 3];
        let batches = split_into_batches(&items, 0);
        assert_eq!(batches, vec![vec![1], vec![2], vec![3]]);
    }
}
