//! CaptionService の孤立キャプション特定・削除。
//!
//! 対応する Image_File を持たない Tag_File（Orphan_Caption）をフォルダ直下から
//! 特定し、ユーザー承認後に削除するサービス層（要件 11.1〜11.4）。
//!
//! # Orphan_Caption の判定規則（要件 11.1）
//!
//! Tag_File は `<basename>.txt` 命名（[`crate::services::tag_file`] と同一規則）を
//! 用いる。フォルダ直下の各 `.txt` について、同じ basename を持つ Image_File
//! （拡張子 jpg / jpeg / png / gif / mp4、大文字小文字を区別しない）が同フォルダに
//! 1 つも存在しなければ Orphan_Caption とみなす。
//!
//! # 削除フロー（要件 11.2, 11.3, 11.4）
//!
//! 本モジュールは特定（[`find_orphan_captions`]）と削除（[`delete_orphan_captions`]）を
//! 分離する。UI は特定結果の一覧をユーザーに提示し（要件 11.2）、承認を得てから
//! [`delete_orphan_captions`] を呼ぶ（要件 11.3）。削除件数を結果として返す
//! （要件 11.4）。

use std::collections::HashSet;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::error::{AppError, AppResult};

/// 対応する Image_File として扱う拡張子（小文字表記、要件 11.1）。
///
/// [`crate::services::file_service::SUPPORTED_EXTENSIONS`] と同一の集合。
/// 拡張子の一致判定は大文字小文字を区別しない。
const IMAGE_EXTENSIONS: [&str; 5] = ["jpg", "jpeg", "png", "gif", "mp4"];

/// 孤立キャプション削除の結果（要件 11.4）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeleteOrphanResult {
    /// 実際に削除できた Orphan_Caption の件数（要件 11.4）。
    pub deleted: usize,
}

/// ファイル名が対応画像拡張子のいずれかを持つか判定する（大小無視、要件 11.1）。
fn has_image_extension(file_name: &str) -> bool {
    match file_name.rsplit_once('.') {
        Some((_, ext)) => IMAGE_EXTENSIONS.iter().any(|s| ext.eq_ignore_ascii_case(s)),
        None => false,
    }
}

/// ファイル名から拡張子を除いた basename を返す。
///
/// 末尾の `.<ext>` を1つだけ落とす。拡張子を持たない名前はそのまま返す。
/// 判定は大小無視・拡張子の種類を問わないため、`image` と `image.txt` の
/// basename はいずれも `image` になる。
fn basename(file_name: &str) -> &str {
    match file_name.rsplit_once('.') {
        Some((stem, _)) => stem,
        None => file_name,
    }
}

/// 対象フォルダ直下の Orphan_Caption を特定する（要件 11.1）。
///
/// フォルダ直下（非再帰）の `.txt` ファイルのうち、同じ basename を持つ
/// Image_File（jpg / jpeg / png / gif / mp4、大小無視）が同フォルダに存在しない
/// ものを Orphan_Caption として抽出し、そのパス文字列の一覧を返す。
///
/// - 走査結果はファイル名（フルパス）昇順で安定に整列する。
/// - 個別エントリの読み取りに失敗した場合はスキップして走査を継続する。
/// - フォルダとして読み取れない（存在しない / ファイルである / 権限不足）場合は
///   `Err(AppError)` を返す。
///
/// # 引数
///
/// - `folder`: 走査対象フォルダのパス。
pub fn find_orphan_captions(folder: impl AsRef<Path>) -> AppResult<Vec<String>> {
    let folder = folder.as_ref();

    let read_dir = std::fs::read_dir(folder)
        .map_err(|e| AppError::from(e).with_path(folder.to_string_lossy().into_owned()))?;

    // 直下のファイル名を収集し、Image_File の basename 集合と .txt の一覧を作る。
    let mut txt_files: Vec<String> = Vec::new();
    // 画像の basename は大小無視で突き合わせるため小文字化して保持する。
    let mut image_basenames: HashSet<String> = HashSet::new();

    for entry in read_dir {
        let entry = match entry {
            Ok(e) => e,
            // 個別エントリの読み取り失敗はスキップして継続。
            Err(_) => continue,
        };
        // ディレクトリは対象外。file_type 取得失敗もスキップ。
        match entry.file_type() {
            Ok(ft) if ft.is_dir() => continue,
            Ok(_) => {}
            Err(_) => continue,
        }
        let name = entry.file_name().to_string_lossy().into_owned();

        if has_image_extension(&name) {
            image_basenames.insert(basename(&name).to_ascii_lowercase());
        } else if name
            .rsplit_once('.')
            .is_some_and(|(_, ext)| ext.eq_ignore_ascii_case("txt"))
        {
            txt_files.push(name);
        }
    }

    // 対応する Image_File を持たない .txt を Orphan_Caption として抽出する。
    let mut orphans: Vec<String> = txt_files
        .into_iter()
        .filter(|txt| {
            let stem = basename(txt).to_ascii_lowercase();
            !image_basenames.contains(&stem)
        })
        .map(|txt| folder.join(txt).to_string_lossy().into_owned())
        .collect();

    // 決定的な順序のため昇順で整列。
    orphans.sort();

    Ok(orphans)
}

/// 承認済みの Orphan_Caption を削除し、削除件数を返す（要件 11.3, 11.4）。
///
/// UI は [`find_orphan_captions`] の結果一覧をユーザーに提示し（要件 11.2）、
/// 承認を得てから本関数に対象パスを渡す（要件 11.3）。
///
/// # 個別削除失敗時の挙動
///
/// 個々のパスの削除に失敗した場合はエラーで中断せず、当該ファイルをスキップして
/// 残りの削除を継続する。戻り値の `deleted` には**実際に削除できた件数のみ**を
/// 数える（要件 11.4）。これにより一部が既に手動削除されていても処理が完了する。
///
/// # 引数
///
/// - `paths`: 削除対象の Orphan_Caption のパス群（承認済み）。
pub fn delete_orphan_captions(paths: &[String]) -> AppResult<DeleteOrphanResult> {
    let mut deleted = 0usize;
    for path in paths {
        match std::fs::remove_file(path) {
            Ok(()) => deleted += 1,
            // 個別失敗（既に存在しない・権限不足など）はスキップして継続。
            Err(_) => continue,
        }
    }
    Ok(DeleteOrphanResult { deleted })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::Path;
    use tempfile::tempdir;

    /// 空ファイルを作成するヘルパー。
    fn touch(dir: &Path, name: &str) {
        fs::write(dir.join(name), b"").unwrap();
    }

    #[test]
    fn txt_with_matching_image_is_not_orphan() {
        let dir = tempdir().unwrap();
        touch(dir.path(), "image.png");
        touch(dir.path(), "image.txt");

        let orphans = find_orphan_captions(dir.path()).unwrap();
        assert!(orphans.is_empty());
    }

    #[test]
    fn txt_without_matching_image_is_orphan() {
        let dir = tempdir().unwrap();
        // 対応 Image_File を持たない .txt。
        touch(dir.path(), "orphan.txt");
        // 別の画像＋対応 .txt（こちらは孤立ではない）。
        touch(dir.path(), "kept.jpg");
        touch(dir.path(), "kept.txt");

        let orphans = find_orphan_captions(dir.path()).unwrap();
        assert_eq!(orphans.len(), 1);
        assert!(orphans[0].ends_with("orphan.txt"));
    }

    #[test]
    fn image_extension_match_is_case_insensitive() {
        let dir = tempdir().unwrap();
        // 大文字拡張子の画像でも basename が一致すれば孤立ではない。
        touch(dir.path(), "photo.JPG");
        touch(dir.path(), "photo.txt");
        // 各種対応拡張子（大小混在）でも一致する。
        touch(dir.path(), "clip.MP4");
        touch(dir.path(), "clip.txt");
        touch(dir.path(), "anim.GiF");
        touch(dir.path(), "anim.txt");

        let orphans = find_orphan_captions(dir.path()).unwrap();
        assert!(orphans.is_empty());
    }

    #[test]
    fn delete_removes_listed_orphans_and_returns_count() {
        let dir = tempdir().unwrap();
        touch(dir.path(), "a.txt");
        touch(dir.path(), "b.txt");
        // 孤立ではない .txt（対応画像あり）。削除対象に含めない。
        touch(dir.path(), "c.png");
        touch(dir.path(), "c.txt");

        let orphans = find_orphan_captions(dir.path()).unwrap();
        // a.txt と b.txt が孤立、c.txt は非孤立。
        assert_eq!(orphans.len(), 2);

        let result = delete_orphan_captions(&orphans).unwrap();
        assert_eq!(result.deleted, 2);

        // 孤立 .txt は削除され、非孤立 .txt と画像は残る。
        assert!(!dir.path().join("a.txt").exists());
        assert!(!dir.path().join("b.txt").exists());
        assert!(dir.path().join("c.txt").exists());
        assert!(dir.path().join("c.png").exists());
    }

    #[test]
    fn delete_skips_missing_paths_and_counts_only_deleted() {
        let dir = tempdir().unwrap();
        touch(dir.path(), "real.txt");
        let missing = dir.path().join("missing.txt").to_string_lossy().into_owned();
        let real = dir.path().join("real.txt").to_string_lossy().into_owned();

        // 存在しないパスを含めても中断せず、削除できた件数のみ数える。
        let result = delete_orphan_captions(&[missing, real]).unwrap();
        assert_eq!(result.deleted, 1);
        assert!(!dir.path().join("real.txt").exists());
    }

    #[test]
    fn unreadable_folder_is_error() {
        let dir = tempdir().unwrap();
        let missing = dir.path().join("does-not-exist");
        assert!(find_orphan_captions(&missing).is_err());
    }

    #[test]
    fn empty_folder_yields_no_orphans() {
        let dir = tempdir().unwrap();
        let orphans = find_orphan_captions(dir.path()).unwrap();
        assert!(orphans.is_empty());
    }
}
