// Feature: tag-editor, Property 23: 動画ファイルの推論除外
//
// Property 23: 任意のパス集合について、推論対象からは mp4 拡張子の
// ファイルが必ず除外される。加えて、mp4 拡張子でない入力パスはすべて
// 入力順を保存して結果に残る。
//
// Validates: Requirements 14.8

use proptest::prelude::*;
use tag_editor_core::logic::inference_aux::exclude_videos;

/// パスの拡張子が mp4（大文字小文字を区別しない）かどうかを判定する
/// テスト側の独立実装。実装（`is_mp4`）と同じ規則を、テストのために
/// 別ロジックで表現する（区切りは `/` と `\` の双方を考慮）。
fn has_mp4_extension(path: &str) -> bool {
    let file_name = path
        .rsplit(|c| c == '/' || c == '\\')
        .next()
        .unwrap_or(path);
    match file_name.rsplit_once('.') {
        Some((_, ext)) => ext.eq_ignore_ascii_case("mp4"),
        None => false,
    }
}

/// ファイル名（拡張子を含みうる末尾要素）の生成器。
///
/// 拡張子の分布を意図的に mp4 系（大文字小文字混在）へ寄せ、除外ロジックが
/// よく駆動されるようにする。また、名前本体に `mp4` を部分文字列として
/// 含むケースや、拡張子を持たないケースも生成する。
fn file_name_strategy() -> impl Strategy<Value = String> {
    // 名前本体（拡張子を除く部分）の候補。`mp4` を含む名前を混ぜる。
    let stem = prop::sample::select(vec![
        "a",
        "image",
        "photo",
        "clip",
        "video",
        "mp4",
        "mp4_thumbnail",
        "my.mp4.backup",
        "データ",
        "画像",
        "file name",
    ]);
    // 拡張子候補: mp4 系（大小混在）と非 mp4、および拡張子なし（None）。
    let ext = prop::sample::select(vec![
        Some("png"),
        Some("jpg"),
        Some("gif"),
        Some("mp4"),
        Some("MP4"),
        Some("Mp4"),
        Some("mP4"),
        None,
    ]);
    (stem, ext).prop_map(|(stem, ext)| match ext {
        Some(e) => format!("{stem}.{e}"),
        None => stem.to_string(),
    })
}

/// ディレクトリ接頭辞の生成器（`/` と `\` を混在させる）。
///
/// 拡張子判定が「最後の区切り以降のファイル名」に対して行われることを
/// 検証できるよう、Windows 形式・Linux 形式・相対形式・接頭辞なしを混ぜる。
fn dir_prefix_strategy() -> impl Strategy<Value = String> {
    prop::sample::select(vec![
        "".to_string(),
        "sub/dir/".to_string(),
        "C:\\images\\".to_string(),
        "/home/user/".to_string(),
        "a/b\\c/".to_string(),
        "mp4/".to_string(), // ディレクトリ名に mp4 を含むが拡張子ではない
    ])
}

/// 単一パス生成器: ディレクトリ接頭辞 + ファイル名。
fn path_strategy() -> impl Strategy<Value = String> {
    (dir_prefix_strategy(), file_name_strategy()).prop_map(|(dir, name)| format!("{dir}{name}"))
}

/// パス集合（列）の生成器。空列も含む。
fn paths_strategy() -> impl Strategy<Value = Vec<String>> {
    prop::collection::vec(path_strategy(), 0..24)
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    /// Property 23（除外の保証）: 結果に mp4 拡張子（大小無視）のパスは
    /// 一つも含まれない。
    #[test]
    fn result_contains_no_mp4(paths in paths_strategy()) {
        let result = exclude_videos(&paths);
        for path in &result {
            prop_assert!(
                !has_mp4_extension(path),
                "結果に mp4 拡張子のパスが残存: {path:?}"
            );
        }
    }

    /// Property 23（非 mp4 の保存・順序込み）: 入力から mp4 拡張子の
    /// パスを取り除いた列が、結果と完全一致する（順序・重複を保存）。
    #[test]
    fn non_mp4_paths_preserved_in_order(paths in paths_strategy()) {
        let expected: Vec<String> = paths
            .iter()
            .filter(|p| !has_mp4_extension(p))
            .cloned()
            .collect();
        let result = exclude_videos(&paths);
        prop_assert_eq!(
            &result,
            &expected,
            "非 mp4 パスの保存に失敗: input={:?}",
            paths
        );
    }

    /// Property 23（大文字 MP4 も除外）: すべての要素を大文字 `.MP4`
    /// 拡張子にした列は、結果が空になる。
    #[test]
    fn all_uppercase_mp4_are_all_excluded(
        dirs in prop::collection::vec(dir_prefix_strategy(), 1..12),
    ) {
        let paths: Vec<String> = dirs
            .into_iter()
            .enumerate()
            .map(|(i, dir)| format!("{dir}clip{i}.MP4"))
            .collect();
        let result = exclude_videos(&paths);
        prop_assert!(
            result.is_empty(),
            "大文字 MP4 が除外されずに残存: {result:?}"
        );
    }

    /// Property 23（部分文字列 mp4 は除外しない）: 名前に `mp4` を含むが
    /// 拡張子が mp4 でないパスは、必ず結果に残る。
    #[test]
    fn mp4_substring_but_not_extension_is_kept(
        dirs in prop::collection::vec(dir_prefix_strategy(), 1..12),
    ) {
        // `mp4` を名前本体・ディレクトリに含めつつ拡張子は png にする。
        let paths: Vec<String> = dirs
            .into_iter()
            .enumerate()
            .map(|(i, dir)| format!("{dir}mp4_thumb_{i}.png"))
            .collect();
        let result = exclude_videos(&paths);
        prop_assert_eq!(
            &result,
            &paths,
            "拡張子でない mp4 を誤って除外: {:?}",
            result
        );
    }
}

/// 代表的な固定ケース（proptest で表現しづらいパターンの明示的確認）。
#[test]
fn representative_fixed_cases() {
    let input: Vec<String> = [
        "a.png",
        "b.mp4",
        "C:\\images\\clip.MP4",
        "sub/dir/video.Mp4",
        "/home/user/pic.png",
        "mp4_thumbnail.png",
        "my.mp4.backup.png",
        "README",
        "mp4/inside.gif",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();

    let expected: Vec<String> = [
        "a.png",
        "/home/user/pic.png",
        "mp4_thumbnail.png",
        "my.mp4.backup.png",
        "README",
        "mp4/inside.gif",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();

    assert_eq!(exclude_videos(&input), expected);
}
