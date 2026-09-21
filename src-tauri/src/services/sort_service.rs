//! SortService の仕訳（gather / distribute / move / copy）（タスク 14.1、要件 7.1〜7.8）。
//!
//! 純粋な命名ロジック（[`crate::logic::naming`]）の gather/distribute 命名を、
//! 実ファイルシステム上の移動・コピー（[`std::fs`]）と結線する副作用ありの
//! サービス層。設計 `SortService::sort_files`（要件 7）に対応する。
//!
//! # 対応する Sorting_Operation（要件 7.1）
//!
//! [`SortingOperation`] は 1 種別を指定する:
//!
//! - [`SortingOperation::Move`] / [`SortingOperation::Copy`]:
//!   対象フォルダ直下の各 Image_File を宛先へ移動 / コピーする。対応する
//!   Tag_File（`<basename>.txt`）が同一フォルダに存在すれば同じ操作を適用し、
//!   対を維持する。無ければ Image_File のみ処理する（要件 7.2）。
//! - [`SortingOperation::Gather`]:
//!   対象フォルダの **直下サブフォルダ** 内の各ファイルについて、当該サブフォルダ
//!   名を接頭辞として付与（[`naming::gather_name`]）したうえで単一の宛先フォルダ
//!   へ集約する（要件 7.3）。
//! - [`SortingOperation::Distribute`]:
//!   対象フォルダ直下の各ファイル名を接頭辞部分と元ファイル名部分へ分解
//!   （[`naming::split_gathered_name`]）し、接頭辞名のサブフォルダを宛先に作成
//!   または再利用して元ファイル名で配置する（要件 7.4）。分解できないファイルは
//!   処理対象から除外し記録する（要件 7.7）。
//!
//! # move / copy の意味論（gather / distribute）
//!
//! 要件 7.3 / 7.4 は gather / distribute が移動・コピーのいずれで実行されるかを
//! 明示していない。本実装では **移動（move）を既定** とする。gather は散在した
//! ファイルを 1 箇所へ「集約」する操作であり、distribute はその逆に「配布」する
//! 操作で、いずれも元の散在状態を残さず整理し直す意図が強いためである。move / copy
//! 明示指定の操作（要件 7.2）と異なり、gather / distribute では元ファイルを宛先へ
//! 移すことで整理後の状態を一意に保つ。
//!
//! # 原子性・衝突（要件 7.5, 7.6）
//!
//! - 対象フォルダ・宛先フォルダのいずれかが存在しない / アクセスできない場合、
//!   **いかなるファイルも変更・移動せず** `Err(AppError)` を返す（要件 7.6）。
//!   検証は各ファイルの処理を始める前に行う。
//! - 宛先に同名ファイルが既に存在する場合、**上書きせず** 当該ファイルを処理対象
//!   から除外し衝突として記録する（要件 7.5）。move / copy で対の Tag_File を扱う
//!   場合、Image・Tag_File のどちらかが衝突するなら対を壊さないため両方とも処理
//!   しない。
//!
//! # 結果（要件 7.8）
//!
//! [`OperationReport`] に成功件数（`succeeded`）・衝突件数（`conflicted`）・
//! スキップ件数（`skipped`）とメッセージを記録して返す。1 つの Image_File と
//! その対 Tag_File を処理した場合、成功は 1 件として数える（対単位）。

use std::path::{Path, PathBuf};

use crate::error::{AppError, AppResult};
use crate::logic::naming;
use crate::models::{OperationReport, SortingOperation};
use crate::services::file_service::SUPPORTED_EXTENSIONS;

/// 拡張子（`.` 以降）が対応 Image 拡張子のいずれかか判定する（大小無視）。
fn is_image_file_name(file_name: &str) -> bool {
    match file_name.rsplit_once('.') {
        Some((_, ext)) => SUPPORTED_EXTENSIONS
            .iter()
            .any(|s| ext.eq_ignore_ascii_case(s)),
        None => false,
    }
}

/// ファイル名の basename（最後の `.` より前）を返す。`.` を含まなければ全体。
fn basename_of(file_name: &str) -> &str {
    match file_name.rsplit_once('.') {
        Some((base, _)) => base,
        None => file_name,
    }
}

/// パスがフォルダとして存在・アクセス可能かを検証する（要件 7.6）。
///
/// 存在しない / ディレクトリでない / 読み取れない場合は `Err` を返す。これを
/// 各ファイル処理の前に呼ぶことで、対象・宛先が利用不可なら未着手で失敗させる。
fn ensure_dir_accessible(dir: &Path, role: &str) -> AppResult<()> {
    let meta = std::fs::metadata(dir).map_err(|e| {
        AppError::from(e).with_path(dir.to_string_lossy().into_owned())
    })?;
    if !meta.is_dir() {
        return Err(AppError::invalid_input(format!(
            "{role}がフォルダではありません"
        ))
        .with_path(dir.to_string_lossy().into_owned()));
    }
    // 読み取り可能性を read_dir で確認（権限不足はここで顕在化する）。
    std::fs::read_dir(dir).map_err(|e| {
        AppError::from(e).with_path(dir.to_string_lossy().into_owned())
    })?;
    Ok(())
}

/// 対象フォルダ直下のファイル名を列挙する（サブディレクトリは除外、昇順）。
fn list_file_names(folder: &Path) -> AppResult<Vec<String>> {
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
        names.push(entry.file_name().to_string_lossy().into_owned());
    }
    names.sort();
    Ok(names)
}

/// 対象フォルダ直下のサブフォルダ名を列挙する（ファイルは除外、昇順）。
fn list_subfolder_names(folder: &Path) -> AppResult<Vec<String>> {
    let read_dir = std::fs::read_dir(folder)
        .map_err(|e| AppError::from(e).with_path(folder.to_string_lossy().into_owned()))?;
    let mut names: Vec<String> = Vec::new();
    for entry in read_dir {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };
        match entry.file_type() {
            Ok(ft) if ft.is_dir() => {
                names.push(entry.file_name().to_string_lossy().into_owned());
            }
            _ => continue,
        }
    }
    names.sort();
    Ok(names)
}

/// 1 ファイルを移動または複製する。`is_move` が true なら移動、false ならコピー。
///
/// 宛先が既存の場合は上書きしない前提（呼び出し側で衝突チェック済み）。
fn transfer(src: &Path, dst: &Path, is_move: bool) -> std::io::Result<()> {
    if is_move {
        std::fs::rename(src, dst)?;
    } else {
        std::fs::copy(src, dst)?;
    }
    Ok(())
}

/// 仕訳を実行する（要件 7.1〜7.8）。
///
/// - `op`: 仕訳種別（[`SortingOperation`]）。
/// - `source`: 対象フォルダ。
/// - `dest`: 宛先フォルダ。
///
/// 対象・宛先が利用不可なら未着手で `Err` を返す（要件 7.6）。それ以外は
/// [`OperationReport`] に成功 / 衝突 / スキップ件数を記録して返す（要件 7.8）。
pub fn sort_files(
    op: SortingOperation,
    source: impl AsRef<Path>,
    dest: impl AsRef<Path>,
) -> AppResult<OperationReport> {
    let source = source.as_ref();
    let dest = dest.as_ref();

    // PRE-VALIDATE: 対象・宛先が利用可能かを処理開始前に確認（要件 7.6、原子性）。
    ensure_dir_accessible(source, "対象フォルダ")?;
    ensure_dir_accessible(dest, "宛先フォルダ")?;

    match op {
        SortingOperation::Move => sort_move_copy(source, dest, true),
        SortingOperation::Copy => sort_move_copy(source, dest, false),
        SortingOperation::Gather => sort_gather(source, dest),
        SortingOperation::Distribute => sort_distribute(source, dest),
    }
}

/// move / copy の実処理（要件 7.2, 7.5, 7.8）。
///
/// 対象フォルダ直下の各 Image_File を宛先へ移動 / コピーする。対の Tag_File
/// （`<basename>.txt`）が存在すれば同じ操作を適用して対を維持する。Image・
/// Tag_File のどちらかが宛先で衝突する場合は対を壊さないため両方とも処理せず
/// 衝突として記録する。
fn sort_move_copy(source: &Path, dest: &Path, is_move: bool) -> AppResult<OperationReport> {
    let names = list_file_names(source)?;
    let mut report = OperationReport::default();

    for name in &names {
        if !is_image_file_name(name) {
            continue;
        }

        let src_image = source.join(name);
        let dst_image = dest.join(name);

        // 対の Tag_File を判定（要件 7.2）。
        let tag_name = format!("{}.txt", basename_of(name));
        let src_tag = source.join(&tag_name);
        let has_tag = src_tag.is_file();
        let dst_tag = dest.join(&tag_name);

        // 衝突チェック（要件 7.5）。対を壊さないため Image・Tag_File の
        // どちらかが衝突するなら両方とも処理しない。
        let image_conflict = dst_image.exists();
        let tag_conflict = has_tag && dst_tag.exists();
        if image_conflict || tag_conflict {
            report.conflicted += 1;
            report.messages.push(format!(
                "{name} は宛先に同名ファイルが存在するため処理しませんでした"
            ));
            continue;
        }

        // Image を移動 / コピー。
        if let Err(e) = transfer(&src_image, &dst_image, is_move) {
            report.skipped += 1;
            report
                .messages
                .push(format!("{name} の処理に失敗したためスキップしました: {e}"));
            continue;
        }

        // 対の Tag_File も同操作（要件 7.2）。
        if has_tag {
            if let Err(e) = transfer(&src_tag, &dst_tag, is_move) {
                // Tag_File 側の処理に失敗したら対が壊れる。move の場合は
                // Image を元へ戻して整合を保つ。copy の場合は複製済みの
                // Image を宛先から削除する。
                if is_move {
                    let _ = std::fs::rename(&dst_image, &src_image);
                } else {
                    let _ = std::fs::remove_file(&dst_image);
                }
                report.skipped += 1;
                report.messages.push(format!(
                    "{name} の対 Tag_File の処理に失敗したため処理を取り消しました: {e}"
                ));
                continue;
            }
        }

        report.succeeded += 1;
    }

    Ok(report)
}

/// gather の実処理（要件 7.3, 7.5, 7.8）。既定で移動（モジュールドキュメント参照）。
///
/// 対象フォルダの直下サブフォルダ内の各ファイルについて、サブフォルダ名を
/// 接頭辞として付与（[`naming::gather_name`]）し単一の宛先へ集約する。
fn sort_gather(source: &Path, dest: &Path) -> AppResult<OperationReport> {
    let subfolders = list_subfolder_names(source)?;
    let mut report = OperationReport::default();

    for subfolder in &subfolders {
        let sub_path = source.join(subfolder);
        let file_names = match list_file_names(&sub_path) {
            Ok(n) => n,
            Err(e) => {
                // 個別サブフォルダの読み取り失敗はスキップして継続。
                report.skipped += 1;
                report.messages.push(format!(
                    "サブフォルダ {subfolder} を読み取れないためスキップしました: {e}"
                ));
                continue;
            }
        };

        for file_name in &file_names {
            let gathered = naming::gather_name(subfolder, file_name);
            let src = sub_path.join(file_name);
            let dst = dest.join(&gathered);

            // 衝突チェック（要件 7.5）。
            if dst.exists() {
                report.conflicted += 1;
                report.messages.push(format!(
                    "{gathered} は宛先に同名ファイルが存在するため処理しませんでした"
                ));
                continue;
            }

            // gather は移動（既定）。
            if let Err(e) = transfer(&src, &dst, true) {
                report.skipped += 1;
                report.messages.push(format!(
                    "{subfolder}/{file_name} の処理に失敗したためスキップしました: {e}"
                ));
                continue;
            }

            report.succeeded += 1;
        }
    }

    Ok(report)
}

/// distribute の実処理（要件 7.4, 7.5, 7.7, 7.8）。既定で移動（モジュールドキュメント参照）。
///
/// 対象フォルダ直下の各ファイル名を接頭辞部分と元ファイル名部分へ分解
/// （[`naming::split_gathered_name`]）し、接頭辞名のサブフォルダを宛先に作成
/// または再利用して元ファイル名で配置する。分解できないファイルは除外し記録する。
fn sort_distribute(source: &Path, dest: &Path) -> AppResult<OperationReport> {
    let names = list_file_names(source)?;
    let mut report = OperationReport::default();

    for name in &names {
        // 接頭辞 / 元ファイル名へ分解（要件 7.4）。分解不能は除外＋記録（要件 7.7）。
        let (prefix, original) = match naming::split_gathered_name(name) {
            Some(parts) => parts,
            None => {
                report.skipped += 1;
                report.messages.push(format!(
                    "{name} は接頭辞と元ファイル名に分解できないため処理対象から除外しました"
                ));
                continue;
            }
        };

        // 接頭辞名のサブフォルダを作成または再利用。
        let target_dir: PathBuf = dest.join(&prefix);
        if let Err(e) = std::fs::create_dir_all(&target_dir) {
            report.skipped += 1;
            report.messages.push(format!(
                "{name} の宛先サブフォルダ {prefix} を作成できないためスキップしました: {e}"
            ));
            continue;
        }

        let src = source.join(name);
        let dst = target_dir.join(&original);

        // 衝突チェック（要件 7.5）。
        if dst.exists() {
            report.conflicted += 1;
            report.messages.push(format!(
                "{prefix}/{original} は宛先に同名ファイルが存在するため処理しませんでした"
            ));
            continue;
        }

        // distribute は移動（既定）。
        if let Err(e) = transfer(&src, &dst, true) {
            report.skipped += 1;
            report
                .messages
                .push(format!("{name} の処理に失敗したためスキップしました: {e}"));
            continue;
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

    fn touch(path: &Path, content: &str) {
        fs::write(path, content.as_bytes()).unwrap();
    }

    fn exists(dir: &Path, name: &str) -> bool {
        dir.join(name).exists()
    }

    #[test]
    fn move_keeps_image_and_txt_pair() {
        // move で Image と対の Tag_File が同一宛先へ移動され対が維持される（要件 7.2）。
        let src = tempdir().unwrap();
        let dst = tempdir().unwrap();
        touch(&src.path().join("cat.png"), "img");
        touch(&src.path().join("cat.txt"), "tag");

        let report = sort_files(SortingOperation::Move, src.path(), dst.path()).unwrap();

        assert_eq!(report.succeeded, 1);
        assert_eq!(report.conflicted, 0);
        assert_eq!(report.skipped, 0);
        // 元は消え、宛先へ対で移動している。
        assert!(!exists(src.path(), "cat.png"));
        assert!(!exists(src.path(), "cat.txt"));
        assert!(exists(dst.path(), "cat.png"));
        assert!(exists(dst.path(), "cat.txt"));
    }

    #[test]
    fn move_without_pair_moves_image_only() {
        // 対の Tag_File が無ければ Image のみ移動する（要件 7.2）。
        let src = tempdir().unwrap();
        let dst = tempdir().unwrap();
        touch(&src.path().join("solo.jpg"), "img");

        let report = sort_files(SortingOperation::Move, src.path(), dst.path()).unwrap();

        assert_eq!(report.succeeded, 1);
        assert!(!exists(src.path(), "solo.jpg"));
        assert!(exists(dst.path(), "solo.jpg"));
    }

    #[test]
    fn copy_leaves_originals_in_place() {
        // copy は元ファイルを残したまま宛先へ複製する（要件 7.2）。
        let src = tempdir().unwrap();
        let dst = tempdir().unwrap();
        touch(&src.path().join("dog.png"), "img");
        touch(&src.path().join("dog.txt"), "tag");

        let report = sort_files(SortingOperation::Copy, src.path(), dst.path()).unwrap();

        assert_eq!(report.succeeded, 1);
        // 元も宛先も両方に存在する。
        assert!(exists(src.path(), "dog.png"));
        assert!(exists(src.path(), "dog.txt"));
        assert!(exists(dst.path(), "dog.png"));
        assert!(exists(dst.path(), "dog.txt"));
    }

    #[test]
    fn gather_prefixes_subfolder_names() {
        // gather は直下サブフォルダ名を接頭辞にして単一宛先へ集約する（要件 7.3）。
        let src = tempdir().unwrap();
        let dst = tempdir().unwrap();
        fs::create_dir(src.path().join("catA")).unwrap();
        fs::create_dir(src.path().join("catB")).unwrap();
        touch(&src.path().join("catA").join("001.png"), "a");
        touch(&src.path().join("catB").join("001.png"), "b");

        let report = sort_files(SortingOperation::Gather, src.path(), dst.path()).unwrap();

        assert_eq!(report.succeeded, 2);
        assert_eq!(report.conflicted, 0);
        // 接頭辞付き名で宛先に集約されている（同名 001.png でも衝突しない）。
        assert!(exists(dst.path(), "catA__001.png"));
        assert!(exists(dst.path(), "catB__001.png"));
        // 元は移動済み。
        assert!(!exists(&src.path().join("catA"), "001.png"));
    }

    #[test]
    fn distribute_splits_and_places_under_subfolder() {
        // distribute は接頭辞名サブフォルダを作成し元名で配置する（要件 7.4）。
        let src = tempdir().unwrap();
        let dst = tempdir().unwrap();
        touch(&src.path().join("catA__001.png"), "a");
        touch(&src.path().join("catB__002.png"), "b");

        let report = sort_files(SortingOperation::Distribute, src.path(), dst.path()).unwrap();

        assert_eq!(report.succeeded, 2);
        assert_eq!(report.skipped, 0);
        // 接頭辞サブフォルダ配下に元名で配置されている。
        assert!(dst.path().join("catA").join("001.png").exists());
        assert!(dst.path().join("catB").join("002.png").exists());
    }

    #[test]
    fn distribute_unsplittable_is_excluded_and_recorded() {
        // 接頭辞へ分解できないファイルは除外し記録する（要件 7.7）。
        let src = tempdir().unwrap();
        let dst = tempdir().unwrap();
        touch(&src.path().join("no_delimiter.png"), "x");

        let report = sort_files(SortingOperation::Distribute, src.path(), dst.path()).unwrap();

        assert_eq!(report.succeeded, 0);
        assert_eq!(report.skipped, 1);
        // 元ファイルは残る。
        assert!(exists(src.path(), "no_delimiter.png"));
        assert!(report.messages.iter().any(|m| m.contains("no_delimiter.png")));
    }

    #[test]
    fn collision_is_recorded_and_not_overwritten() {
        // 宛先に同名が存在する場合、上書きせず除外＋記録する（要件 7.5）。
        let src = tempdir().unwrap();
        let dst = tempdir().unwrap();
        touch(&src.path().join("img.png"), "new");
        // 宛先に既存の同名（中身が異なる）。
        touch(&dst.path().join("img.png"), "old");

        let report = sort_files(SortingOperation::Move, src.path(), dst.path()).unwrap();

        assert_eq!(report.succeeded, 0);
        assert_eq!(report.conflicted, 1);
        // 元は残り、宛先は上書きされていない。
        assert!(exists(src.path(), "img.png"));
        assert_eq!(fs::read_to_string(dst.path().join("img.png")).unwrap(), "old");
        assert!(report.messages.iter().any(|m| m.contains("img.png")));
    }

    #[test]
    fn missing_source_returns_err_and_changes_nothing() {
        // 対象フォルダが存在しない場合、未着手で Err を返す（要件 7.6）。
        let dst = tempdir().unwrap();
        let missing = dst.path().join("does_not_exist");

        let result = sort_files(SortingOperation::Move, &missing, dst.path());

        assert!(result.is_err());
        assert_eq!(result.unwrap_err().kind, crate::error::AppErrorKind::NotFound);
    }

    #[test]
    fn missing_dest_returns_err_and_changes_nothing() {
        // 宛先フォルダが存在しない場合、未着手で Err を返し対象を変更しない（要件 7.6）。
        let src = tempdir().unwrap();
        touch(&src.path().join("keep.png"), "img");
        let missing = src.path().join("no_dest");

        let result = sort_files(SortingOperation::Move, src.path(), &missing);

        assert!(result.is_err());
        // 対象ファイルは変更されていない。
        assert!(exists(src.path(), "keep.png"));
    }

    // -------------------------------------------------------------------
    // タスク 14.3 補完: 原子性（7.6）・衝突保持（7.5）の追加テスト
    // -------------------------------------------------------------------

    #[test]
    fn copy_missing_dest_returns_err_and_changes_nothing() {
        // copy でも宛先不可なら未着手で Err を返し対象を変更しない（要件 7.6）。
        let src = tempdir().unwrap();
        touch(&src.path().join("keep.png"), "img");
        touch(&src.path().join("keep.txt"), "tag");
        let missing = src.path().join("no_dest");

        let result = sort_files(SortingOperation::Copy, src.path(), &missing);

        assert!(result.is_err());
        // 対象ファイルは元の場所にそのまま残る。
        assert!(exists(src.path(), "keep.png"));
        assert!(exists(src.path(), "keep.txt"));
    }

    #[test]
    fn gather_missing_source_returns_err_and_changes_nothing() {
        // gather で対象不可なら未着手で Err を返す（要件 7.6）。
        let dst = tempdir().unwrap();
        let missing = dst.path().join("no_source");

        let result = sort_files(SortingOperation::Gather, &missing, dst.path());

        assert!(result.is_err());
        // 宛先には何も作られていない。
        assert!(std::fs::read_dir(dst.path()).unwrap().next().is_none());
    }

    #[test]
    fn gather_missing_dest_returns_err_and_changes_nothing() {
        // gather で宛先不可なら未着手で Err を返し、サブフォルダ内の元ファイルは無変更（要件 7.6）。
        let src = tempdir().unwrap();
        fs::create_dir(src.path().join("catA")).unwrap();
        touch(&src.path().join("catA").join("001.png"), "a");
        let missing = src.path().join("no_dest");

        let result = sort_files(SortingOperation::Gather, src.path(), &missing);

        assert!(result.is_err());
        // 元ファイルはサブフォルダ内にそのまま残る。
        assert!(exists(&src.path().join("catA"), "001.png"));
    }

    #[test]
    fn distribute_missing_dest_returns_err_and_changes_nothing() {
        // distribute で宛先不可なら未着手で Err を返し対象を変更しない（要件 7.6）。
        let src = tempdir().unwrap();
        touch(&src.path().join("catA__001.png"), "a");
        let missing = src.path().join("no_dest");

        let result = sort_files(SortingOperation::Distribute, src.path(), &missing);

        assert!(result.is_err());
        // 元ファイルは変更されていない。
        assert!(exists(src.path(), "catA__001.png"));
    }

    #[test]
    fn move_collision_excludes_only_conflicting_file() {
        // 同名衝突は当該ファイルのみ除外し、非衝突ファイルは移動される。
        // 既存宛先ファイルは上書きされない（要件 7.5）。
        let src = tempdir().unwrap();
        let dst = tempdir().unwrap();
        // 衝突する側。
        touch(&src.path().join("dup.png"), "new");
        touch(&dst.path().join("dup.png"), "old");
        // 衝突しない側。
        touch(&src.path().join("fresh.png"), "img");
        touch(&src.path().join("fresh.txt"), "tag");

        let report = sort_files(SortingOperation::Move, src.path(), dst.path()).unwrap();

        // 非衝突分のみ成功、衝突は 1 件記録。
        assert_eq!(report.succeeded, 1);
        assert_eq!(report.conflicted, 1);
        // 衝突ファイルは元に残り、既存宛先は上書きされていない。
        assert!(exists(src.path(), "dup.png"));
        assert_eq!(fs::read_to_string(dst.path().join("dup.png")).unwrap(), "old");
        // 非衝突ファイルは対で移動している。
        assert!(!exists(src.path(), "fresh.png"));
        assert!(exists(dst.path(), "fresh.png"));
        assert!(exists(dst.path(), "fresh.txt"));
        assert!(report.messages.iter().any(|m| m.contains("dup.png")));
    }

    #[test]
    fn move_tag_collision_excludes_pair_but_moves_others() {
        // 対 Tag_File 側だけが衝突する場合も対を壊さず両方保持し、
        // 他の非衝突ファイルは移動される（要件 7.5）。
        let src = tempdir().unwrap();
        let dst = tempdir().unwrap();
        // Image は非衝突だが Tag_File が宛先に既存 → 対で保持。
        touch(&src.path().join("pair.png"), "img");
        touch(&src.path().join("pair.txt"), "new");
        touch(&dst.path().join("pair.txt"), "old");
        // 完全に非衝突。
        touch(&src.path().join("other.png"), "img2");

        let report = sort_files(SortingOperation::Move, src.path(), dst.path()).unwrap();

        assert_eq!(report.succeeded, 1);
        assert_eq!(report.conflicted, 1);
        // 対は元に残り、既存 Tag_File は上書きされていない。
        assert!(exists(src.path(), "pair.png"));
        assert!(exists(src.path(), "pair.txt"));
        assert_eq!(fs::read_to_string(dst.path().join("pair.txt")).unwrap(), "old");
        assert!(!exists(dst.path(), "pair.png"));
        // 非衝突は移動済み。
        assert!(exists(dst.path(), "other.png"));
    }
}

// ===========================================================================
// タスク 14.4: 画像サイズによる自動仕訳（sort_by_size、要件 9.1〜9.7）
// ===========================================================================
//
// 純粋な振分決定ロジック（[`crate::logic::sorting`] の [`decide_size_destination`]
// / [`is_valid_long_side_threshold`] / [`Orientation`] / [`LongSideBand`]）を、
// 実ファイルシステム上の寸法取得・移動・Tag_File 対保存と結線する副作用あり
// サービス。設計 SortService（要件 9）に対応する。
//
// # 振分先フォルダ名（要件 9.3, 9.4）
//
// [`decide_size_destination`] が返す「向き（横長/縦長）× 長辺区分（閾値以上/未満）」
// の 2×2 をそれぞれ `dest_root` 直下の 4 サブフォルダへ一意に対応づける:
//
// - 横長 × 閾値以上 → `landscape_large`
// - 横長 × 閾値未満 → `landscape_small`
// - 縦長 × 閾値以上 → `portrait_large`
// - 縦長 × 閾値未満 → `portrait_small`
//
// （`large` = [`LongSideBand::AtOrAbove`]、`small` = [`LongSideBand::Below`]。
// 幅 == 高さ は横長扱い、長辺 == 閾値 は「以上」区分。いずれも
// [`crate::logic::sorting`] のロジックに従う。）サブフォルダは処理中に必要に
// 応じて作成する。
//
// # 閾値無効時の扱い（要件 9.2）
//
// 要件 9.2 は「処理を開始せず、閾値が無効である旨を操作結果に記録」と規定する。
// これに合わせ、[`is_valid_long_side_threshold`] が false の場合は **いかなる
// ファイルも走査・移動せず**、成功/衝突/スキップ件数がすべて 0 の
// [`OperationReport`] にその旨のメッセージを記録して `Ok` で返す（`Err` では
// なく「操作結果に記録」を選択）。
//
// # 寸法取得失敗・衝突（要件 9.6, 9.7）
//
// - 寸法を取得できない Image_File は振分から除外し元の場所に保持、記録する
//   （要件 9.7）。[`image::image_dimensions`] はヘッダのみ読むため安価。
// - 振分先に同名ファイルが既に存在する場合、Image・対 Tag_File とも移動せず
//   元の場所に保持し衝突として記録する（要件 9.6）。対を壊さないため Image・
//   Tag_File のどちらかが衝突するなら両方とも移動しない。

use crate::logic::sorting::{
    decide_size_destination, is_valid_long_side_threshold, LongSideBand, Orientation,
};

/// サイズ振分先を `dest_root` 直下のサブフォルダ名へ対応づける（要件 9.3）。
fn size_subfolder_name(orientation: Orientation, band: LongSideBand) -> &'static str {
    match (orientation, band) {
        (Orientation::Landscape, LongSideBand::AtOrAbove) => "landscape_large",
        (Orientation::Landscape, LongSideBand::Below) => "landscape_small",
        (Orientation::Portrait, LongSideBand::AtOrAbove) => "portrait_large",
        (Orientation::Portrait, LongSideBand::Below) => "portrait_small",
    }
}

/// 画像サイズによる自動仕訳を実行する（要件 9.1〜9.7）。
///
/// - `source`: 対象フォルダ。直下の各 Image_File を対象とする。
/// - `threshold`: 長辺閾値（1〜100000 の整数ピクセル）。範囲外・非整数は無効。
/// - `dest_root`: 振分先ルートフォルダ。直下に向き×長辺区分の 4 サブフォルダを作る。
///
/// # 動作
///
/// 1. 閾値が無効なら未着手で 0 件の [`OperationReport`] に記録して返す（要件 9.2）。
/// 2. 対象・振分先ルートが利用不可なら未着手で `Err` を返す（要件 9.1 の前提）。
/// 3. 各 Image_File の寸法を取得（要件 9.1）。取得失敗は除外＋保持＋記録（要件 9.7）。
/// 4. [`decide_size_destination`] で振分先を一意決定し（要件 9.3）、対応サブフォルダ
///    へ移動する（要件 9.4）。対の Tag_File があれば同じ振分先へ移動（要件 9.5）。
/// 5. 振分先に同名が存在すれば Image・Tag_File とも保持＋記録（要件 9.6）。
///
/// 成功件数（`succeeded`、対単位）・衝突件数（`conflicted`）・スキップ件数
/// （`skipped`）とメッセージを [`OperationReport`] に記録して返す。
pub fn sort_by_size(
    source: impl AsRef<Path>,
    threshold: i64,
    dest_root: impl AsRef<Path>,
) -> AppResult<OperationReport> {
    let source = source.as_ref();
    let dest_root = dest_root.as_ref();

    // 閾値の妥当性を最初に確認（要件 9.2）。無効なら未着手で記録して返す。
    if !is_valid_long_side_threshold(threshold) {
        let mut report = OperationReport::default();
        report.messages.push(format!(
            "閾値 {threshold} は無効です（1〜100000 の整数のみ有効）。処理を実行しませんでした"
        ));
        return Ok(report);
    }
    // 妥当（1..=100000）なら u32 へ収まる。
    let threshold_u32 = threshold as u32;

    // PRE-VALIDATE: 対象・振分先ルートが利用可能かを処理開始前に確認。
    ensure_dir_accessible(source, "対象フォルダ")?;
    ensure_dir_accessible(dest_root, "振り分け先ルートフォルダ")?;

    let names = list_file_names(source)?;
    let mut report = OperationReport::default();

    for name in &names {
        if !is_image_file_name(name) {
            continue;
        }

        let src_image = source.join(name);

        // 寸法取得（ヘッダのみ）。失敗は除外＋保持＋記録（要件 9.7）。
        let (width, height) = match image::image_dimensions(&src_image) {
            Ok(dim) => dim,
            Err(e) => {
                report.skipped += 1;
                report.messages.push(format!(
                    "{name} の寸法を取得できないため振り分けから除外しました: {e}"
                ));
                continue;
            }
        };

        // 振分先を一意決定（要件 9.3）してサブフォルダ名へ対応づける。
        let dest = decide_size_destination(width, height, threshold_u32);
        let subfolder = size_subfolder_name(dest.orientation, dest.band);
        let target_dir = dest_root.join(subfolder);

        // 振分先サブフォルダを作成または再利用。
        if let Err(e) = std::fs::create_dir_all(&target_dir) {
            report.skipped += 1;
            report.messages.push(format!(
                "{name} の振り分け先サブフォルダ {subfolder} を作成できないためスキップしました: {e}"
            ));
            continue;
        }

        let dst_image = target_dir.join(name);

        // 対の Tag_File を判定（要件 9.5）。
        let tag_name = format!("{}.txt", basename_of(name));
        let src_tag = source.join(&tag_name);
        let has_tag = src_tag.is_file();
        let dst_tag = target_dir.join(&tag_name);

        // 衝突チェック（要件 9.6）。対を壊さないため Image・Tag_File の
        // どちらかが衝突するなら両方とも保持する。
        let image_conflict = dst_image.exists();
        let tag_conflict = has_tag && dst_tag.exists();
        if image_conflict || tag_conflict {
            report.conflicted += 1;
            report.messages.push(format!(
                "{name} は振り分け先 {subfolder} に同名ファイルが存在するため移動しませんでした"
            ));
            continue;
        }

        // Image を移動（要件 9.4）。
        if let Err(e) = transfer(&src_image, &dst_image, true) {
            report.skipped += 1;
            report
                .messages
                .push(format!("{name} の移動に失敗したためスキップしました: {e}"));
            continue;
        }

        // 対の Tag_File も同じ振分先へ移動（要件 9.5）。
        if has_tag {
            if let Err(e) = transfer(&src_tag, &dst_tag, true) {
                // Tag_File 側が失敗したら対が壊れる。Image を元へ戻して整合を保つ。
                let _ = std::fs::rename(&dst_image, &src_image);
                report.skipped += 1;
                report.messages.push(format!(
                    "{name} の対 Tag_File の移動に失敗したため移動を取り消しました: {e}"
                ));
                continue;
            }
        }

        report.succeeded += 1;
    }

    Ok(report)
}

#[cfg(test)]
mod sort_by_size_tests {
    use super::*;
    use image::{ImageBuffer, Rgb};
    use std::fs;
    use std::path::Path;
    use tempfile::tempdir;

    /// 指定寸法の PNG を書き出す。
    fn write_png(path: &Path, width: u32, height: u32) {
        let buf: ImageBuffer<Rgb<u8>, Vec<u8>> =
            ImageBuffer::from_pixel(width, height, Rgb([120, 120, 120]));
        buf.save(path).unwrap();
    }

    fn touch(path: &Path, content: &str) {
        fs::write(path, content.as_bytes()).unwrap();
    }

    fn exists(dir: &Path, rel: &str) -> bool {
        dir.join(rel).exists()
    }

    #[test]
    fn landscape_and_portrait_route_to_distinct_folders() {
        // 横長は landscape_*、縦長は portrait_* へ振り分けられる（要件 9.3, 9.4）。
        let src = tempdir().unwrap();
        let dst = tempdir().unwrap();
        // 長辺 200、閾値 100 → いずれも large。
        write_png(&src.path().join("wide.png"), 200, 100); // 横長
        write_png(&src.path().join("tall.png"), 100, 200); // 縦長

        let report = sort_by_size(src.path(), 100, dst.path()).unwrap();

        assert_eq!(report.succeeded, 2);
        assert_eq!(report.conflicted, 0);
        assert_eq!(report.skipped, 0);
        assert!(exists(dst.path(), "landscape_large/wide.png"));
        assert!(exists(dst.path(), "portrait_large/tall.png"));
        // 元は移動済み。
        assert!(!exists(src.path(), "wide.png"));
        assert!(!exists(src.path(), "tall.png"));
    }

    #[test]
    fn long_side_equal_to_threshold_is_at_or_above() {
        // 長辺 == 閾値 は「以上（large）」区分（要件 9.3、境界）。
        let src = tempdir().unwrap();
        let dst = tempdir().unwrap();
        write_png(&src.path().join("edge.png"), 512, 300); // 長辺 512 == 閾値

        let report = sort_by_size(src.path(), 512, dst.path()).unwrap();

        assert_eq!(report.succeeded, 1);
        assert!(exists(dst.path(), "landscape_large/edge.png"));
    }

    #[test]
    fn long_side_below_threshold_is_small() {
        // 長辺 < 閾値 は small 区分。
        let src = tempdir().unwrap();
        let dst = tempdir().unwrap();
        write_png(&src.path().join("small.png"), 400, 200); // 長辺 400 < 閾値 512

        let report = sort_by_size(src.path(), 512, dst.path()).unwrap();

        assert_eq!(report.succeeded, 1);
        assert!(exists(dst.path(), "landscape_small/small.png"));
    }

    #[test]
    fn square_is_treated_as_landscape() {
        // 幅 == 高さ は横長扱い（要件 9.3）。
        let src = tempdir().unwrap();
        let dst = tempdir().unwrap();
        write_png(&src.path().join("sq.png"), 300, 300);

        let report = sort_by_size(src.path(), 100, dst.path()).unwrap();

        assert_eq!(report.succeeded, 1);
        assert!(exists(dst.path(), "landscape_large/sq.png"));
    }

    #[test]
    fn tag_file_pair_moves_together() {
        // 対の Tag_File が同じ振分先へ一緒に移動する（要件 9.5）。
        let src = tempdir().unwrap();
        let dst = tempdir().unwrap();
        write_png(&src.path().join("cat.png"), 800, 400); // 横長 large
        touch(&src.path().join("cat.txt"), "tag1, tag2");

        let report = sort_by_size(src.path(), 500, dst.path()).unwrap();

        assert_eq!(report.succeeded, 1);
        assert!(exists(dst.path(), "landscape_large/cat.png"));
        assert!(exists(dst.path(), "landscape_large/cat.txt"));
        // 元は対で消えている。
        assert!(!exists(src.path(), "cat.png"));
        assert!(!exists(src.path(), "cat.txt"));
    }

    #[test]
    fn invalid_threshold_does_nothing_and_is_recorded() {
        // 閾値が無効なら未着手で記録し 0 件（要件 9.2）。
        let src = tempdir().unwrap();
        let dst = tempdir().unwrap();
        write_png(&src.path().join("img.png"), 800, 400);

        // 範囲外（0）。
        let report = sort_by_size(src.path(), 0, dst.path()).unwrap();
        assert_eq!(report.succeeded, 0);
        assert_eq!(report.conflicted, 0);
        assert_eq!(report.skipped, 0);
        assert!(report.messages.iter().any(|m| m.contains("無効")));
        // いかなるファイルも移動していない。
        assert!(exists(src.path(), "img.png"));
        assert!(!exists(dst.path(), "landscape_large/img.png"));

        // 上限超過も無効。
        let report2 = sort_by_size(src.path(), 100_001, dst.path()).unwrap();
        assert_eq!(report2.succeeded, 0);
        assert!(report2.messages.iter().any(|m| m.contains("無効")));
        assert!(exists(src.path(), "img.png"));
    }

    #[test]
    fn collision_keeps_pair_in_place_and_records() {
        // 振分先に同名が存在する場合、Image・Tag_File とも保持＋記録（要件 9.6）。
        let src = tempdir().unwrap();
        let dst = tempdir().unwrap();
        write_png(&src.path().join("dup.png"), 800, 400); // 横長 large
        touch(&src.path().join("dup.txt"), "new");
        // 振分先に既存の同名 Image。
        let target = dst.path().join("landscape_large");
        fs::create_dir_all(&target).unwrap();
        touch(&target.join("dup.png"), "old");

        let report = sort_by_size(src.path(), 500, dst.path()).unwrap();

        assert_eq!(report.succeeded, 0);
        assert_eq!(report.conflicted, 1);
        // 元は対で残り、振分先の既存ファイルは上書きされていない。
        assert!(exists(src.path(), "dup.png"));
        assert!(exists(src.path(), "dup.txt"));
        assert_eq!(fs::read_to_string(target.join("dup.png")).unwrap(), "old");
        assert!(report.messages.iter().any(|m| m.contains("dup.png")));
    }

    #[test]
    fn unreadable_dimensions_are_excluded_kept_and_recorded() {
        // 寸法を取得できない画像は除外＋保持＋記録（要件 9.7）。
        let src = tempdir().unwrap();
        let dst = tempdir().unwrap();
        // 拡張子は対応（.png）だが中身は壊れており寸法取得に失敗する。
        touch(&src.path().join("broken.png"), "not a real png");
        // 正常な画像も 1 枚（残りは処理継続することの確認）。
        write_png(&src.path().join("ok.png"), 800, 400);

        let report = sort_by_size(src.path(), 500, dst.path()).unwrap();

        assert_eq!(report.succeeded, 1);
        assert_eq!(report.skipped, 1);
        // 壊れたファイルは元の場所に保持。
        assert!(exists(src.path(), "broken.png"));
        assert!(!exists(dst.path(), "landscape_large/broken.png"));
        // 正常画像は振り分けられている。
        assert!(exists(dst.path(), "landscape_large/ok.png"));
        assert!(report.messages.iter().any(|m| m.contains("broken.png")));
    }

    #[test]
    fn missing_source_returns_err() {
        // 対象フォルダが存在しない場合、未着手で Err を返す。
        let dst = tempdir().unwrap();
        let missing = dst.path().join("no_source");
        let result = sort_by_size(&missing, 512, dst.path());
        assert!(result.is_err());
    }

    #[test]
    fn missing_dest_root_returns_err() {
        // 振分先ルートが存在しない場合、未着手で Err を返す。
        let src = tempdir().unwrap();
        write_png(&src.path().join("keep.png"), 800, 400);
        let missing = src.path().join("no_dest");
        let result = sort_by_size(src.path(), 512, &missing);
        assert!(result.is_err());
        // 対象ファイルは変更されていない。
        assert!(exists(src.path(), "keep.png"));
    }

    // -------------------------------------------------------------------
    // タスク 14.5 補完: 衝突保持（9.6）・寸法失敗（9.7）の追加テスト
    // -------------------------------------------------------------------

    #[test]
    fn collision_keeps_conflicting_pair_but_routes_others() {
        // 衝突した画像・対 Tag_File は保持＋記録し、同時に非衝突画像は振り分けられる（要件 9.6）。
        let src = tempdir().unwrap();
        let dst = tempdir().unwrap();
        // 衝突する画像（横長 large）。対 Tag_File 付き。
        write_png(&src.path().join("dup.png"), 800, 400);
        touch(&src.path().join("dup.txt"), "new");
        let target = dst.path().join("landscape_large");
        fs::create_dir_all(&target).unwrap();
        touch(&target.join("dup.png"), "old");
        // 衝突しない画像（縦長 large）。
        write_png(&src.path().join("fresh.png"), 400, 800);
        touch(&src.path().join("fresh.txt"), "tag");

        let report = sort_by_size(src.path(), 500, dst.path()).unwrap();

        // 非衝突分のみ成功、衝突は 1 件記録。
        assert_eq!(report.succeeded, 1);
        assert_eq!(report.conflicted, 1);
        // 衝突した対は元に残り、既存ファイルは上書きされていない。
        assert!(exists(src.path(), "dup.png"));
        assert!(exists(src.path(), "dup.txt"));
        assert_eq!(fs::read_to_string(target.join("dup.png")).unwrap(), "old");
        // 非衝突は対で振り分けられている。
        assert!(exists(dst.path(), "portrait_large/fresh.png"));
        assert!(exists(dst.path(), "portrait_large/fresh.txt"));
        assert!(!exists(src.path(), "fresh.png"));
        assert!(report.messages.iter().any(|m| m.contains("dup.png")));
    }

    #[test]
    fn dimension_failure_keeps_pair_and_continues() {
        // 寸法取得に失敗した画像は対 Tag_File とともに元の場所に保持し記録、
        // 他の有効画像の処理は継続する（要件 9.7）。
        let src = tempdir().unwrap();
        let dst = tempdir().unwrap();
        // 壊れた画像（寸法取得失敗）＋対 Tag_File。
        touch(&src.path().join("broken.png"), "not a real png");
        touch(&src.path().join("broken.txt"), "keep me");
        // 有効画像 2 枚（処理継続の確認）。
        write_png(&src.path().join("ok1.png"), 800, 400); // 横長 large
        write_png(&src.path().join("ok2.png"), 300, 900); // 縦長 large

        let report = sort_by_size(src.path(), 500, dst.path()).unwrap();

        // 有効な 2 枚は成功、壊れた 1 枚はスキップ。
        assert_eq!(report.succeeded, 2);
        assert_eq!(report.skipped, 1);
        assert_eq!(report.conflicted, 0);
        // 壊れた画像と対 Tag_File は元の場所に保持。
        assert!(exists(src.path(), "broken.png"));
        assert!(exists(src.path(), "broken.txt"));
        assert!(!exists(dst.path(), "landscape_large/broken.png"));
        // 有効画像は振り分けられている。
        assert!(exists(dst.path(), "landscape_large/ok1.png"));
        assert!(exists(dst.path(), "portrait_large/ok2.png"));
        assert!(report.messages.iter().any(|m| m.contains("broken.png")));
    }
}

// ===========================================================================
// タスク 14.6: 特定タグによる画像仕訳（sort_by_tag、要件 10.1〜10.6）
// ===========================================================================
//
// 純粋な振分決定ロジック（[`crate::logic::sorting::decide_tag_destination`] /
// [`crate::logic::sorting::TagDestination`]）を、実ファイルシステム上の
// Tag_File 読み込み・トークン化・Image / Tag_File 移動と結線する副作用あり
// サービス。設計 SortService（要件 10）に対応する。
//
// # 振り分け先（要件 10.2, 10.3）
//
// 判定タグのいずれかを含むなら「含む」振り分け先（`dest_contains`）へ、いずれも
// 含まないなら「含まない」振り分け先（`dest_not_contains`）へ Image_File と対の
// Tag_File を移動する。宛先は呼び出し側が 2 つ（含む / 含まない）を指定する。
//
// # 判定方法（要件 10.1, 10.4）
//
// 各 Image_File の対 Tag_File を [`crate::services::tag_file::read_tag_file`] で
// 読み込み、[`crate::logic::tag_ops::split_tokens`] でトークン化する。Tag_File が
// 存在しない場合は空トークン集合として扱う（要件 10.4：「含まない」へ分類される）。
// トークン集合と判定タグ集合を [`decide_tag_destination`] に渡し（前後トリムの
// 完全一致、要件 10.1）、[`TagDestination::Contains`] / [`TagDestination::NotContains`]
// を得る。
//
// # 宛先の作成可否（要件 10.6）
//
// 要件 10.6 は「振り分け先が存在せず作成もできない場合、処理を開始せず利用不可を
// 通知」と規定する。これに合わせ、両振り分け先を処理開始前に `create_dir_all` で
// 作成試行する。いずれかが作成不可（例: 親がファイル）なら **いかなる Image_File も
// 走査・移動せず**、成功/衝突/スキップ件数がすべて 0 の [`OperationReport`] に
// 「振り分け先が利用不可」の旨を記録して `Ok` で返す（`Err` ではなく sort_by_size の
// 閾値無効時と同様「操作結果に記録」を選択し、UI 側で通知に用いる）。対象フォルダ
// 自体が利用不可な場合は前提が崩れるため `Err` を返す。
//
// # 衝突（要件 10.5）
//
// 振り分け先に同名ファイルが既に存在する場合、Image・対 Tag_File とも移動せず
// 元の場所に保持し衝突として記録する。対を壊さないため Image・Tag_File の
// どちらかが衝突するなら両方とも移動しない（sort_move_copy / sort_by_size と同方針）。

use crate::logic::sorting::{decide_tag_destination, TagDestination};
use crate::logic::tag_ops::split_tokens;
use crate::services::tag_file::read_tag_file;

/// 特定タグによる画像仕訳を実行する（要件 10.1〜10.6）。
///
/// - `source`: 対象フォルダ。直下の各 Image_File を対象とする。
/// - `judge_tags`: 判定タグ（1 つ以上、前後トリムの完全一致で判定、要件 10.1）。
/// - `dest_contains`: 判定タグのいずれかを含む画像の振り分け先（要件 10.2）。
/// - `dest_not_contains`: 判定タグのいずれも含まない画像の振り分け先（要件 10.3, 10.4）。
///
/// # 動作
///
/// 1. 対象フォルダが利用不可なら未着手で `Err` を返す。
/// 2. 両振り分け先を作成試行し、いずれか作成不可なら未着手で 0 件の
///    [`OperationReport`] に「利用不可」を記録して返す（要件 10.6）。
/// 3. 各 Image_File の対 Tag_File を読み込み・トークン化（無ければ空、要件 10.4）、
///    [`decide_tag_destination`] で「含む / 含まない」を判定する（要件 10.1）。
/// 4. 判定に応じた振り分け先へ Image と対 Tag_File を移動する（要件 10.2, 10.3）。
/// 5. 振り分け先に同名が存在すれば Image・Tag_File とも保持＋記録（要件 10.5）。
///
/// 成功件数（`succeeded`、対単位）・衝突件数（`conflicted`）・スキップ件数
/// （`skipped`）とメッセージを [`OperationReport`] に記録して返す。
pub fn sort_by_tag(
    source: impl AsRef<Path>,
    judge_tags: &[String],
    dest_contains: impl AsRef<Path>,
    dest_not_contains: impl AsRef<Path>,
) -> AppResult<OperationReport> {
    let source = source.as_ref();
    let dest_contains = dest_contains.as_ref();
    let dest_not_contains = dest_not_contains.as_ref();

    // PRE-VALIDATE: 対象フォルダが利用可能かを処理開始前に確認。
    ensure_dir_accessible(source, "対象フォルダ")?;

    // 両振り分け先を作成試行（要件 10.6）。作成不可なら未着手で記録して返す。
    for (dir, role) in [
        (dest_contains, "「含む」振り分け先"),
        (dest_not_contains, "「含まない」振り分け先"),
    ] {
        if let Err(e) = std::fs::create_dir_all(dir) {
            let mut report = OperationReport::default();
            report.messages.push(format!(
                "{role} {} が利用不可のため処理を実行しませんでした: {e}",
                dir.to_string_lossy()
            ));
            return Ok(report);
        }
    }

    let names = list_file_names(source)?;
    let mut report = OperationReport::default();

    for name in &names {
        if !is_image_file_name(name) {
            continue;
        }

        let src_image = source.join(name);

        // 対 Tag_File を読み込みトークン化。存在しなければ空トークン（要件 10.4）。
        // read_tag_file は未存在を exists=false + 空内容として返すため、
        // その場合 split_tokens は空 Vec を返し「含まない」へ分類される。
        let image_tags: Vec<String> = match read_tag_file(&src_image) {
            Ok(tag_content) => split_tokens(&tag_content.content),
            Err(e) => {
                // 権限不足など Tag_File 読み込みの I/O エラーは当該 Image を
                // 除外＋保持＋記録（衝突ではないためスキップ扱い）。
                report.skipped += 1;
                report.messages.push(format!(
                    "{name} の Tag_File を読み込めないため振り分けから除外しました: {e}"
                ));
                continue;
            }
        };

        // 「含む / 含まない」を判定（要件 10.1）。前後トリムの完全一致。
        let target_dir = match decide_tag_destination(&image_tags, judge_tags) {
            TagDestination::Contains => dest_contains,
            TagDestination::NotContains => dest_not_contains,
        };

        let dst_image = target_dir.join(name);

        // 対の Tag_File を判定。
        let tag_name = format!("{}.txt", basename_of(name));
        let src_tag = source.join(&tag_name);
        let has_tag = src_tag.is_file();
        let dst_tag = target_dir.join(&tag_name);

        // 衝突チェック（要件 10.5）。対を壊さないため Image・Tag_File の
        // どちらかが衝突するなら両方とも保持する。
        let image_conflict = dst_image.exists();
        let tag_conflict = has_tag && dst_tag.exists();
        if image_conflict || tag_conflict {
            report.conflicted += 1;
            report.messages.push(format!(
                "{name} は振り分け先に同名ファイルが存在するため移動しませんでした"
            ));
            continue;
        }

        // Image を移動（要件 10.2, 10.3）。
        if let Err(e) = transfer(&src_image, &dst_image, true) {
            report.skipped += 1;
            report
                .messages
                .push(format!("{name} の移動に失敗したためスキップしました: {e}"));
            continue;
        }

        // 対の Tag_File も同じ振り分け先へ移動（要件 10.2, 10.3）。
        if has_tag {
            if let Err(e) = transfer(&src_tag, &dst_tag, true) {
                // Tag_File 側が失敗したら対が壊れる。Image を元へ戻して整合を保つ。
                let _ = std::fs::rename(&dst_image, &src_image);
                report.skipped += 1;
                report.messages.push(format!(
                    "{name} の対 Tag_File の移動に失敗したため移動を取り消しました: {e}"
                ));
                continue;
            }
        }

        report.succeeded += 1;
    }

    Ok(report)
}

#[cfg(test)]
mod sort_by_tag_tests {
    use super::*;
    use std::fs;
    use std::path::Path;
    use tempfile::tempdir;

    fn touch(path: &Path, content: &str) {
        fs::write(path, content.as_bytes()).unwrap();
    }

    fn exists(dir: &Path, rel: &str) -> bool {
        dir.join(rel).exists()
    }

    fn judge(tags: &[&str]) -> Vec<String> {
        tags.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn image_with_judge_tag_moves_to_contains_with_pair() {
        // 判定タグを含む Image は対 Tag_File とともに「含む」振り分け先へ移動（要件 10.1, 10.2）。
        let src = tempdir().unwrap();
        let contains = tempdir().unwrap();
        let not_contains = tempdir().unwrap();
        touch(&src.path().join("cat.png"), "img");
        touch(&src.path().join("cat.txt"), "1girl, cat, solo");

        let report = sort_by_tag(
            src.path(),
            &judge(&["cat"]),
            contains.path(),
            not_contains.path(),
        )
        .unwrap();

        assert_eq!(report.succeeded, 1);
        assert_eq!(report.conflicted, 0);
        assert_eq!(report.skipped, 0);
        // 「含む」側へ対で移動している。
        assert!(exists(contains.path(), "cat.png"));
        assert!(exists(contains.path(), "cat.txt"));
        // 元は消え、「含まない」側には無い。
        assert!(!exists(src.path(), "cat.png"));
        assert!(!exists(not_contains.path(), "cat.png"));
    }

    #[test]
    fn image_without_judge_tag_moves_to_not_contains() {
        // 判定タグをいずれも含まない Image は「含まない」振り分け先へ移動（要件 10.3）。
        let src = tempdir().unwrap();
        let contains = tempdir().unwrap();
        let not_contains = tempdir().unwrap();
        touch(&src.path().join("dog.png"), "img");
        touch(&src.path().join("dog.txt"), "1girl, solo");

        let report = sort_by_tag(
            src.path(),
            &judge(&["cat"]),
            contains.path(),
            not_contains.path(),
        )
        .unwrap();

        assert_eq!(report.succeeded, 1);
        assert!(exists(not_contains.path(), "dog.png"));
        assert!(exists(not_contains.path(), "dog.txt"));
        assert!(!exists(contains.path(), "dog.png"));
    }

    #[test]
    fn image_without_tag_file_is_not_contains() {
        // 対 Tag_File が無い Image は「含まない」として扱う（要件 10.4）。
        let src = tempdir().unwrap();
        let contains = tempdir().unwrap();
        let not_contains = tempdir().unwrap();
        touch(&src.path().join("solo.jpg"), "img");

        let report = sort_by_tag(
            src.path(),
            &judge(&["cat"]),
            contains.path(),
            not_contains.path(),
        )
        .unwrap();

        assert_eq!(report.succeeded, 1);
        assert!(exists(not_contains.path(), "solo.jpg"));
        assert!(!exists(contains.path(), "solo.jpg"));
    }

    #[test]
    fn any_of_multiple_judge_tags_matches_contains() {
        // 複数判定タグのいずれか 1 つでも一致すれば「含む」（要件 10.1, 10.2）。
        let src = tempdir().unwrap();
        let contains = tempdir().unwrap();
        let not_contains = tempdir().unwrap();
        touch(&src.path().join("night.png"), "img");
        touch(&src.path().join("night.txt"), "sky, night, stars");

        let report = sort_by_tag(
            src.path(),
            &judge(&["cat", "night", "dog"]),
            contains.path(),
            not_contains.path(),
        )
        .unwrap();

        assert_eq!(report.succeeded, 1);
        assert!(exists(contains.path(), "night.png"));
    }

    #[test]
    fn match_is_trim_only_exact() {
        // 前後トリムの完全一致で判定する（要件 10.1）。部分一致は含むにならない。
        let src = tempdir().unwrap();
        let contains = tempdir().unwrap();
        let not_contains = tempdir().unwrap();
        // トリムすれば一致する Tag。
        touch(&src.path().join("a.png"), "img");
        touch(&src.path().join("a.txt"), "  cat  , sky");
        // 部分一致のみ（catface）。含むにならない。
        touch(&src.path().join("b.png"), "img");
        touch(&src.path().join("b.txt"), "catface");

        let report = sort_by_tag(
            src.path(),
            &judge(&["cat"]),
            contains.path(),
            not_contains.path(),
        )
        .unwrap();

        assert_eq!(report.succeeded, 2);
        assert!(exists(contains.path(), "a.png"));
        assert!(exists(not_contains.path(), "b.png"));
    }

    #[test]
    fn collision_keeps_pair_in_place_and_records() {
        // 振り分け先に同名が存在する場合、Image・Tag_File とも保持＋記録（要件 10.5）。
        let src = tempdir().unwrap();
        let contains = tempdir().unwrap();
        let not_contains = tempdir().unwrap();
        touch(&src.path().join("dup.png"), "new");
        touch(&src.path().join("dup.txt"), "cat");
        // 「含む」側に既存の同名 Image。
        touch(&contains.path().join("dup.png"), "old");

        let report = sort_by_tag(
            src.path(),
            &judge(&["cat"]),
            contains.path(),
            not_contains.path(),
        )
        .unwrap();

        assert_eq!(report.succeeded, 0);
        assert_eq!(report.conflicted, 1);
        // 元は対で残り、振り分け先の既存ファイルは上書きされていない。
        assert!(exists(src.path(), "dup.png"));
        assert!(exists(src.path(), "dup.txt"));
        assert_eq!(
            fs::read_to_string(contains.path().join("dup.png")).unwrap(),
            "old"
        );
        assert!(report.messages.iter().any(|m| m.contains("dup.png")));
    }

    #[test]
    fn uncreatable_destination_does_nothing_and_is_notified() {
        // 振り分け先が作成不可（親がファイル）なら未着手で 0 件＋利用不可を記録（要件 10.6）。
        let src = tempdir().unwrap();
        let base = tempdir().unwrap();
        touch(&src.path().join("cat.png"), "img");
        touch(&src.path().join("cat.txt"), "cat");
        // 「含む」振り分け先の親をファイルにする → create_dir_all が失敗する。
        let parent_file = base.path().join("not_a_dir");
        touch(&parent_file, "x");
        let bad_contains = parent_file.join("contains");
        let not_contains = base.path().join("not_contains");

        let report =
            sort_by_tag(src.path(), &judge(&["cat"]), &bad_contains, &not_contains).unwrap();

        // いかなるファイルも走査・移動していない。
        assert_eq!(report.succeeded, 0);
        assert_eq!(report.conflicted, 0);
        assert_eq!(report.skipped, 0);
        assert!(report.messages.iter().any(|m| m.contains("利用不可")));
        // 対象ファイルは元の場所に保持されている。
        assert!(exists(src.path(), "cat.png"));
        assert!(exists(src.path(), "cat.txt"));
    }

    #[test]
    fn missing_source_returns_err() {
        // 対象フォルダが存在しない場合、未着手で Err を返す。
        let base = tempdir().unwrap();
        let missing = base.path().join("no_source");
        let result = sort_by_tag(
            &missing,
            &judge(&["cat"]),
            base.path().join("c"),
            base.path().join("n"),
        );
        assert!(result.is_err());
    }

    // -------------------------------------------------------------------
    // タスク 14.7 補完: 衝突保持（10.5）・宛先作成不可（10.6）の追加テスト
    // -------------------------------------------------------------------

    #[test]
    fn collision_keeps_conflicting_pair_but_routes_others() {
        // 衝突した対は保持＋記録しつつ、非衝突の画像は判定に応じて振り分けられる（要件 10.5）。
        let src = tempdir().unwrap();
        let contains = tempdir().unwrap();
        let not_contains = tempdir().unwrap();
        // 「含む」側で衝突する対。
        touch(&src.path().join("dup.png"), "new");
        touch(&src.path().join("dup.txt"), "cat");
        touch(&contains.path().join("dup.png"), "old");
        // 判定タグを含まない非衝突画像 → 「含まない」へ。
        touch(&src.path().join("fresh.png"), "img");
        touch(&src.path().join("fresh.txt"), "dog");

        let report = sort_by_tag(
            src.path(),
            &judge(&["cat"]),
            contains.path(),
            not_contains.path(),
        )
        .unwrap();

        // 非衝突分のみ成功、衝突は 1 件記録。
        assert_eq!(report.succeeded, 1);
        assert_eq!(report.conflicted, 1);
        // 衝突した対は元に残り、既存宛先は上書きされていない。
        assert!(exists(src.path(), "dup.png"));
        assert!(exists(src.path(), "dup.txt"));
        assert_eq!(
            fs::read_to_string(contains.path().join("dup.png")).unwrap(),
            "old"
        );
        // 非衝突は「含まない」側へ対で移動している。
        assert!(exists(not_contains.path(), "fresh.png"));
        assert!(exists(not_contains.path(), "fresh.txt"));
        assert!(!exists(src.path(), "fresh.png"));
        assert!(report.messages.iter().any(|m| m.contains("dup.png")));
    }

    #[test]
    fn uncreatable_not_contains_destination_does_nothing_and_is_notified() {
        // 「含まない」振り分け先が作成不可（親がファイル）でも未着手で 0 件＋利用不可を記録（要件 10.6）。
        let src = tempdir().unwrap();
        let base = tempdir().unwrap();
        touch(&src.path().join("dog.png"), "img");
        touch(&src.path().join("dog.txt"), "dog");
        // 「含まない」振り分け先の親をファイルにする → create_dir_all が失敗する。
        let parent_file = base.path().join("not_a_dir");
        touch(&parent_file, "x");
        let contains = base.path().join("contains");
        let bad_not_contains = parent_file.join("not_contains");

        let report =
            sort_by_tag(src.path(), &judge(&["cat"]), &contains, &bad_not_contains).unwrap();

        // いかなるファイルも走査・移動していない。
        assert_eq!(report.succeeded, 0);
        assert_eq!(report.conflicted, 0);
        assert_eq!(report.skipped, 0);
        assert!(report.messages.iter().any(|m| m.contains("利用不可")));
        // 対象ファイルは元の場所に保持されている。
        assert!(exists(src.path(), "dog.png"));
        assert!(exists(src.path(), "dog.txt"));
    }
}
