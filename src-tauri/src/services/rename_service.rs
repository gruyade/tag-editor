//! RenameService の正規表現置換・連番改名（タスク 13.1、要件 8.1〜8.6）。
//!
//! 純粋な命名ロジック（[`crate::logic::naming`]）の連番・NUM 展開を、実ファイル
//! システム上の改名（[`std::fs::rename`]）と結線する副作用ありのサービス層。
//!
//! # 処理の流れ（[`rename_regex`]）
//!
//! 1. 正規表現 `pattern` をコンパイルする。不正なら **何も改名せず**
//!    `Err(InvalidInput)` を返す（要件 8.4）。
//! 2. 対象フォルダ直下の Image_File（jpg / jpeg / png / gif / mp4、大小無視）を
//!    ファイル名昇順に列挙する（連番の割り当て順、要件 8.2）。
//! 3. 各 Image_File について、ファイル名へ正規表現置換を適用する（要件 8.1）。
//!    `numbering` 指定時は、昇順の 0 起点 index に対応する連番
//!    （`start + index` を `width` 桁ゼロ埋め）で置換文字列中の `NUM`
//!    プレースホルダを展開する（要件 8.2、[`crate::logic::naming`]）。
//! 4. 改名後の名前が OS 禁止文字を含む場合は当該のみ改名せず記録（要件 8.5）。
//! 5. 改名後の名前が既存ファイルと衝突する場合は当該のみ改名せず記録（要件 8.6）。
//! 6. Image_File を改名する際、対応する Tag_File（`<basename>.txt`）が存在すれば
//!    同じ規則で（＝改名後の Image と同じ basename の `.txt` へ）改名し、対を
//!    維持する（要件 8.3）。
//!
//! # 対を維持する規則（要件 8.3）
//!
//! Image_File の改名後名が `new.png` になった場合、対応する Tag_File は
//! `new.txt` へ改名する。これにより「Image と Tag_File は同一 basename」という
//! 対の不変条件が改名前後で保存される（Property: タスク 13.2）。Tag_File 側の
//! 改名先が禁止文字を含む／既存と衝突する場合、その Image と Tag_File の対は
//! いずれも改名せず記録する（対を壊さないため）。

use std::path::Path;

use crate::error::{AppError, AppResult};
use crate::logic::naming;
use crate::models::OperationReport;
use crate::services::file_service::SUPPORTED_EXTENSIONS;

/// 連番付与の指定（要件 8.2）。
///
/// `start` は割り当て開始番号、`width` は NUM プレースホルダをゼロ埋めする桁数。
///
/// Tauri コマンド境界（`rename_regex`）の引数として受け取るため serde 対応する。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Numbering {
    /// 連番の開始番号。ファイル名昇順の 0 番目にこの値を割り当てる。
    pub start: u64,
    /// ゼロ埋めの桁数。
    pub width: usize,
}

/// OS（主に Windows）でファイル名に使用できない禁止文字（要件 8.5）。
///
/// Windows の予約文字集合 `< > : " / \ | ? *` を採用する。パス区切り
/// （`/` `\`）を含むため、改名後の名前がサブディレクトリを跨ぐことも防ぐ。
/// 制御文字（U+0000〜U+001F）も無効とみなす。
const FORBIDDEN_CHARS: [char; 9] = ['<', '>', ':', '"', '/', '\\', '|', '?', '*'];

/// ファイル名が OS 禁止文字を含む、または空である場合に `true`（要件 8.5）。
fn has_forbidden_chars(name: &str) -> bool {
    name.is_empty()
        || name.chars().any(|c| FORBIDDEN_CHARS.contains(&c) || c.is_control())
}

/// 拡張子（`.` 以降）が対応 Image 拡張子のいずれかか判定する（大小無視）。
fn is_image_file_name(file_name: &str) -> bool {
    match file_name.rsplit_once('.') {
        Some((_, ext)) => SUPPORTED_EXTENSIONS
            .iter()
            .any(|s| ext.eq_ignore_ascii_case(s)),
        None => false,
    }
}

/// 対象フォルダ直下の Image_File 名をファイル名昇順で列挙する（要件 8.2）。
fn list_image_names(folder: &Path) -> AppResult<Vec<String>> {
    let read_dir = std::fs::read_dir(folder)
        .map_err(|e| AppError::from(e).with_path(folder.to_string_lossy().into_owned()))?;

    let mut names: Vec<String> = Vec::new();
    for entry in read_dir {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };
        match entry.file_type() {
            Ok(ft) if ft.is_dir() => continue,
            Ok(_) => {}
            Err(_) => continue,
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        if is_image_file_name(&name) {
            names.push(name);
        }
    }
    names.sort();
    Ok(names)
}

/// ファイル名の basename（最後の `.` より前）を返す。`.` を含まなければ全体。
fn basename_of(file_name: &str) -> &str {
    match file_name.rsplit_once('.') {
        Some((base, _)) => base,
        None => file_name,
    }
}

/// 正規表現置換・連番付与による一括改名を行う（要件 8.1〜8.6）。
///
/// - `folder`: 対象フォルダ。直下の Image_File を対象とする。
/// - `pattern`: ファイル名へ適用する正規表現。不正なら `Err(InvalidInput)` を
///   返し、いかなるファイルも改名しない（要件 8.4）。
/// - `replacement`: 置換文字列。`numbering` 指定時、この中の `NUM`
///   プレースホルダが連番へ展開される（要件 8.2）。
/// - `numbering`: 連番付与の指定。`None` なら連番を付与しない（要件 8.1 のみ）。
///
/// 戻り値の [`OperationReport`] は成功 / 衝突 / スキップ件数とメッセージを持つ。
/// 禁止文字（要件 8.5）は `skipped`、既存衝突（要件 8.6）は `conflicted` に計上する。
pub fn rename_regex(
    folder: impl AsRef<Path>,
    pattern: &str,
    replacement: &str,
    numbering: Option<Numbering>,
) -> AppResult<OperationReport> {
    let folder = folder.as_ref();

    // 1. 正規表現をコンパイル。不正なら何も改名せずエラー（要件 8.4）。
    let re = regex::Regex::new(pattern)
        .map_err(|e| AppError::invalid_input(format!("正規表現が不正です: {e}")))?;

    // 2. 対象 Image_File をファイル名昇順で列挙（連番割り当て順、要件 8.2）。
    let image_names = list_image_names(folder)?;

    let mut report = OperationReport::default();

    for (index, old_image_name) in image_names.iter().enumerate() {
        // 3. ファイル名へ正規表現置換（要件 8.1）。連番指定時は NUM を展開（要件 8.2）。
        let replacement_expanded = match numbering {
            Some(Numbering { start, width }) => {
                naming::apply_numbering(replacement, start, index as u64, width)
            }
            None => replacement.to_string(),
        };
        let new_image_name = re
            .replace_all(old_image_name, replacement_expanded.as_str())
            .into_owned();

        // 変化がなければスキップ（改名不要）。
        if new_image_name == *old_image_name {
            continue;
        }

        // 4. 禁止文字チェック（要件 8.5）。
        if has_forbidden_chars(&new_image_name) {
            report.skipped += 1;
            report.messages.push(format!(
                "{old_image_name} は改名後の名前 \"{new_image_name}\" が無効な文字を含むため改名しませんでした"
            ));
            continue;
        }

        // 対応する Tag_File を同規則で改名する準備（要件 8.3）。
        // 対を維持するため、Tag_File は改名後 Image と同一 basename の `.txt` にする。
        let old_image_path = folder.join(old_image_name);
        let new_image_path = folder.join(&new_image_name);
        let old_tag_path = folder.join(format!("{}.txt", basename_of(old_image_name)));
        let new_tag_name = format!("{}.txt", basename_of(&new_image_name));
        let has_tag = old_tag_path.exists();
        let new_tag_path = folder.join(&new_tag_name);

        // 5. 衝突チェック（要件 8.6）。Image・Tag_File のどちらかが既存と衝突するなら
        //    対を壊さないため両方とも改名しない。
        //    改名前パス自身との一致（new==old）は上の early-continue で除外済み。
        let image_conflict = new_image_path.exists();
        let tag_conflict = has_tag && new_tag_name.as_str() != "" && {
            // Tag_File の改名先が、改名対象の Tag_File 自身でない既存と衝突するか。
            new_tag_path.exists() && new_tag_path != old_tag_path
        };
        if image_conflict || tag_conflict {
            report.conflicted += 1;
            report.messages.push(format!(
                "{old_image_name} は改名後の名前が既存ファイルと衝突するため改名しませんでした"
            ));
            continue;
        }

        // 6. 改名を実行。Image → Tag_File の順。
        if let Err(e) = std::fs::rename(&old_image_path, &new_image_path) {
            report.skipped += 1;
            report.messages.push(format!(
                "{old_image_name} の改名に失敗したためスキップしました: {e}"
            ));
            continue;
        }

        // 対の Tag_File も同規則で改名し対を維持（要件 8.3）。
        if has_tag && new_tag_path != old_tag_path {
            if let Err(e) = std::fs::rename(&old_tag_path, &new_tag_path) {
                // Tag_File 側の改名に失敗した場合は対が壊れる。
                // Image 側の改名を巻き戻して整合を保ち、スキップ記録する。
                let _ = std::fs::rename(&new_image_path, &old_image_path);
                report.skipped += 1;
                report.messages.push(format!(
                    "{old_image_name} の対 Tag_File の改名に失敗したため改名を取り消しました: {e}"
                ));
                continue;
            }
        }

        report.succeeded += 1;
    }

    Ok(report)
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

    fn exists(dir: &Path, name: &str) -> bool {
        dir.join(name).exists()
    }

    #[test]
    fn basic_regex_replace_renames_image_and_paired_txt() {
        // Image とその対 Tag_File が同規則で改名され対が維持される（要件 8.1, 8.3）。
        let dir = tempdir().unwrap();
        touch(dir.path(), "cat_001.png");
        touch(dir.path(), "cat_001.txt");

        // "cat" を "dog" へ置換。
        let report = rename_regex(dir.path(), "cat", "dog", None).unwrap();

        assert_eq!(report.succeeded, 1);
        assert_eq!(report.conflicted, 0);
        assert_eq!(report.skipped, 0);
        // Image・Tag_File とも改名され、対が保たれている。
        assert!(!exists(dir.path(), "cat_001.png"));
        assert!(!exists(dir.path(), "cat_001.txt"));
        assert!(exists(dir.path(), "dog_001.png"));
        assert!(exists(dir.path(), "dog_001.txt"));
    }

    #[test]
    fn regex_replace_without_pair_renames_image_only() {
        // 対の Tag_File が無い Image は Image のみ改名される（要件 8.1）。
        let dir = tempdir().unwrap();
        touch(dir.path(), "photo.jpg");

        let report = rename_regex(dir.path(), "photo", "pic", None).unwrap();

        assert_eq!(report.succeeded, 1);
        assert!(!exists(dir.path(), "photo.jpg"));
        assert!(exists(dir.path(), "pic.jpg"));
    }

    #[test]
    fn numbering_expands_num_in_ascending_order() {
        // ファイル名昇順に start から連番を割り当て、NUM をゼロ埋め展開する（要件 8.2）。
        let dir = tempdir().unwrap();
        // 昇順: a.png, b.png, c.png。
        touch(dir.path(), "c.png");
        touch(dir.path(), "a.png");
        touch(dir.path(), "b.png");

        // 各ファイル名全体を "img_NUM.png" に置換し、NUM を 3 桁・開始 1 で展開。
        let report = rename_regex(
            dir.path(),
            r"^.*\.png$",
            "img_NUM.png",
            Some(Numbering { start: 1, width: 3 }),
        )
        .unwrap();

        assert_eq!(report.succeeded, 3);
        // 昇順 a→001, b→002, c→003。
        assert!(exists(dir.path(), "img_001.png"));
        assert!(exists(dir.path(), "img_002.png"));
        assert!(exists(dir.path(), "img_003.png"));
    }

    #[test]
    fn numbering_keeps_pair_with_same_number() {
        // 連番改名でも対の Tag_File が同じ番号で改名され対を維持する（要件 8.2, 8.3）。
        let dir = tempdir().unwrap();
        touch(dir.path(), "a.png");
        touch(dir.path(), "a.txt");
        touch(dir.path(), "b.png");
        touch(dir.path(), "b.txt");

        let report = rename_regex(
            dir.path(),
            r"^.*\.png$",
            "n_NUM.png",
            Some(Numbering { start: 0, width: 2 }),
        )
        .unwrap();

        assert_eq!(report.succeeded, 2);
        assert!(exists(dir.path(), "n_00.png"));
        assert!(exists(dir.path(), "n_00.txt"));
        assert!(exists(dir.path(), "n_01.png"));
        assert!(exists(dir.path(), "n_01.txt"));
    }

    #[test]
    fn invalid_regex_returns_err_and_renames_nothing() {
        // 不正な正規表現はエラーを返し、いかなるファイルも改名しない（要件 8.4）。
        let dir = tempdir().unwrap();
        touch(dir.path(), "keep.png");

        // 閉じない繰り返し量指定子 → コンパイルエラー。
        let result = rename_regex(dir.path(), "(", "x", None);

        assert!(result.is_err());
        let err = result.unwrap_err();
        assert_eq!(err.kind, crate::error::AppErrorKind::InvalidInput);
        // 元ファイルは変更されていない。
        assert!(exists(dir.path(), "keep.png"));
    }

    #[test]
    fn collision_is_recorded_as_conflicted_and_not_renamed() {
        // 改名後の名前が既存ファイルと衝突する場合、当該のみ改名せず記録（要件 8.6）。
        let dir = tempdir().unwrap();
        touch(dir.path(), "src.png");
        // 改名先が既に存在する。
        touch(dir.path(), "dst.png");

        let report = rename_regex(dir.path(), "src", "dst", None).unwrap();

        assert_eq!(report.succeeded, 0);
        assert_eq!(report.conflicted, 1);
        assert_eq!(report.skipped, 0);
        // 両方とも元のまま。
        assert!(exists(dir.path(), "src.png"));
        assert!(exists(dir.path(), "dst.png"));
        assert!(report.messages.iter().any(|m| m.contains("src.png")));
    }

    #[test]
    fn forbidden_char_is_recorded_and_not_renamed() {
        // 改名後の名前が OS 禁止文字を含む場合、当該のみ改名せず記録（要件 8.5）。
        let dir = tempdir().unwrap();
        touch(dir.path(), "ok.png");

        // 拡張子前に禁止文字 ':' を挿入する置換。
        let report = rename_regex(dir.path(), "ok", "a:b", None).unwrap();

        assert_eq!(report.succeeded, 0);
        assert_eq!(report.skipped, 1);
        assert_eq!(report.conflicted, 0);
        // 元ファイルは変更されない。
        assert!(exists(dir.path(), "ok.png"));
        assert!(report.messages.iter().any(|m| m.contains("ok.png")));
    }

    #[test]
    fn invalid_regex_leaves_all_multiple_files_unchanged() {
        // 不正正規表現は複数ファイルがあってもすべて未改名で Err を返す（要件 8.4）。
        let dir = tempdir().unwrap();
        touch(dir.path(), "a.png");
        touch(dir.path(), "b.png");
        touch(dir.path(), "c.jpg");
        touch(dir.path(), "a.txt");

        // 不正な正規表現（閉じない量指定子）。
        let result = rename_regex(dir.path(), "a(b", "x", None);

        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err().kind,
            crate::error::AppErrorKind::InvalidInput
        );
        // すべての元ファイルがそのまま残る（1 件も改名されない）。
        assert!(exists(dir.path(), "a.png"));
        assert!(exists(dir.path(), "b.png"));
        assert!(exists(dir.path(), "c.jpg"));
        assert!(exists(dir.path(), "a.txt"));
    }

    #[test]
    fn forbidden_char_skips_only_offending_item_others_still_rename() {
        // 無効な改名先（空名）の項目のみスキップし、他の正常な項目は改名される（要件 8.5）。
        //
        // Windows では入力ファイル名に禁止文字を持てないため、「無効な名前＝空文字列」を
        // 利用する（`has_forbidden_chars` は空名も無効とみなす）。
        // パターン `(.*)\.png$` 置換 `$1`（拡張子を落とし basename を新名にする）:
        //  - ".png"     → ""（空名で無効 → スキップ）
        //  - "keep.png" → "keep"（正常 → 改名される）
        let dir = tempdir().unwrap();
        touch(dir.path(), ".png"); // basename が空。改名後は空名になり無効。
        touch(dir.path(), "keep.png"); // 正常に改名される。

        let report = rename_regex(dir.path(), r"(.*)\.png$", "$1", None).unwrap();

        assert_eq!(report.succeeded, 1, "正常な項目は改名される");
        assert_eq!(report.skipped, 1, "無効な名前になる項目のみスキップ");
        assert_eq!(report.conflicted, 0);
        // 無効側は元のまま残る。
        assert!(exists(dir.path(), ".png"));
        // 正常側は拡張子を落として改名済み。
        assert!(!exists(dir.path(), "keep.png"));
        assert!(exists(dir.path(), "keep"));
    }

    #[test]
    fn collision_skips_only_colliding_item_and_keeps_pair_together() {
        // 衝突する項目のみスキップし、非衝突項目は対の Tag_File ごと改名される（要件 8.6, 8.3）。
        //
        // パターン `src` 置換 `dst` で番号を残しつつ写像する:
        //  - "src1.png" → "dst1.png"（既存 dst1.png と衝突 → スキップ、対の txt も据え置き）
        //  - "src2.png" → "dst2.png"（非衝突 → 改名、対の txt も dst2.txt へ）
        let dir = tempdir().unwrap();
        // 衝突する対象（Tag_File 付き）。
        touch(dir.path(), "src1.png");
        touch(dir.path(), "src1.txt");
        // 非衝突の対象（Tag_File 付き）。
        touch(dir.path(), "src2.png");
        touch(dir.path(), "src2.txt");
        // src1 の改名先 dst1.png を先に用意し衝突させる。
        touch(dir.path(), "dst1.png");

        let report = rename_regex(dir.path(), "src", "dst", None).unwrap();

        assert_eq!(report.succeeded, 1, "非衝突項目は改名される");
        assert_eq!(report.conflicted, 1, "衝突項目のみ衝突として記録");
        assert_eq!(report.skipped, 0);

        // 衝突した src1 の対は元のまま（対が壊れていない）。
        assert!(exists(dir.path(), "src1.png"));
        assert!(exists(dir.path(), "src1.txt"));
        assert!(exists(dir.path(), "dst1.png")); // 衝突先の既存ファイルも残存

        // 非衝突の src2 は Image・Tag_File ともに改名され対が維持される。
        assert!(!exists(dir.path(), "src2.png"));
        assert!(!exists(dir.path(), "src2.txt"));
        assert!(exists(dir.path(), "dst2.png"));
        assert!(exists(dir.path(), "dst2.txt"));
    }
}

/// タグファイル名の正規化改名を行う（タスク 13.4、要件 4.1〜4.3）。
///
/// 対象フォルダ直下の `.txt` ファイルを列挙し、[`naming::normalize_caption_name`]
/// を適用する。この関数は `image.png.txt` のような二重拡張子形式に対して
/// `Some(new_name)`（例: `image.txt`）を返し、既に正規化済み（単一拡張子）や
/// `.txt` 以外には `None` を返す。
///
/// - 正規化対象（`Some`）のうち、正規化後の名前が既存ファイルと衝突する場合は
///   **改名せず** 衝突として記録する（要件 4.2）。
/// - 衝突しない場合は [`std::fs::rename`] で改名し成功件数に計上する（要件 4.1）。
/// - `None` を返すファイル（正規化不要）はそのまま残し、いずれの件数にも計上しない。
///
/// 戻り値の [`OperationReport`] は改名件数（`succeeded`）と衝突件数
/// （`conflicted`）を持ち、結果表示に用いる（要件 4.3）。
pub fn normalize_caption_filenames(
    folder: impl AsRef<Path>,
) -> AppResult<OperationReport> {
    let folder = folder.as_ref();

    let read_dir = std::fs::read_dir(folder)
        .map_err(|e| AppError::from(e).with_path(folder.to_string_lossy().into_owned()))?;

    // フォルダ直下の `.txt` ファイル名を列挙（順序を安定させるためソート）。
    let mut txt_names: Vec<String> = Vec::new();
    for entry in read_dir {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };
        match entry.file_type() {
            Ok(ft) if ft.is_dir() => continue,
            Ok(_) => {}
            Err(_) => continue,
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.to_ascii_lowercase().ends_with(".txt") {
            txt_names.push(name);
        }
    }
    txt_names.sort();

    let mut report = OperationReport::default();

    for old_name in &txt_names {
        // 正規化対象か判定。None なら正規化不要のため何もしない（件数計上なし）。
        let new_name = match naming::normalize_caption_name(old_name) {
            Some(n) => n,
            None => continue,
        };

        // 正規化後の名前が現在の名前と同一なら何もしない（保険）。
        if new_name == *old_name {
            continue;
        }

        let new_path = folder.join(&new_name);

        // 衝突チェック（要件 4.2）。改名先が既存なら改名せず記録する。
        if new_path.exists() {
            report.conflicted += 1;
            report.messages.push(format!(
                "{old_name} は正規化後の名前 \"{new_name}\" が既存ファイルと衝突するため改名しませんでした"
            ));
            continue;
        }

        // 改名を実行（要件 4.1）。
        let old_path = folder.join(old_name);
        if let Err(e) = std::fs::rename(&old_path, &new_path) {
            report.skipped += 1;
            report.messages.push(format!(
                "{old_name} の改名に失敗したためスキップしました: {e}"
            ));
            continue;
        }

        report.succeeded += 1;
    }

    Ok(report)
}

#[cfg(test)]
mod normalize_caption_filenames_tests {
    use super::*;
    use std::fs;
    use std::path::Path;
    use tempfile::tempdir;

    fn touch(dir: &Path, name: &str) {
        fs::write(dir.join(name), b"").unwrap();
    }

    fn exists(dir: &Path, name: &str) -> bool {
        dir.join(name).exists()
    }

    #[test]
    fn double_extension_txt_is_renamed() {
        // `image.png.txt` は `image.txt` へ改名される（要件 4.1）。
        let dir = tempdir().unwrap();
        touch(dir.path(), "image.png.txt");

        let report = normalize_caption_filenames(dir.path()).unwrap();

        assert_eq!(report.succeeded, 1);
        assert_eq!(report.conflicted, 0);
        assert_eq!(report.skipped, 0);
        assert!(!exists(dir.path(), "image.png.txt"));
        assert!(exists(dir.path(), "image.txt"));
    }

    #[test]
    fn collision_is_recorded_as_conflicted_and_not_renamed() {
        // 正規化後の名前が既存の場合、改名せず衝突として記録（要件 4.2, 4.3）。
        let dir = tempdir().unwrap();
        touch(dir.path(), "image.png.txt");
        touch(dir.path(), "image.txt");

        let report = normalize_caption_filenames(dir.path()).unwrap();

        assert_eq!(report.succeeded, 0);
        assert_eq!(report.conflicted, 1);
        // 両ファイルとも元のまま残る。
        assert!(exists(dir.path(), "image.png.txt"));
        assert!(exists(dir.path(), "image.txt"));
        assert!(report.messages.iter().any(|m| m.contains("image.png.txt")));
    }

    #[test]
    fn plain_txt_is_left_untouched_and_not_counted() {
        // 単一拡張子 `notes.txt` は正規化不要のため放置し件数に計上しない。
        let dir = tempdir().unwrap();
        touch(dir.path(), "notes.txt");

        let report = normalize_caption_filenames(dir.path()).unwrap();

        assert_eq!(report.succeeded, 0);
        assert_eq!(report.conflicted, 0);
        assert_eq!(report.skipped, 0);
        assert!(exists(dir.path(), "notes.txt"));
    }
}
