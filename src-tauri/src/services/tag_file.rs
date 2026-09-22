//! Tag_File の読み込み・書き込み。
//!
//! Image_File と同名の `.txt`（`<basename>.txt`）を Tag_File として扱い、
//! その内容を UTF-8 として読み書きするサービス層（要件 2.1〜2.6）。
//!
//! 本モジュールは Tag_File の**生の内容**（文字列）の入出力のみを担う。
//! カンマ区切りタグへのパースや正規化は上位の TagService が担当する。
//!
//! # 書き込みの原子性（要件 2.3, 2.4, 2.6）
//!
//! 書き込みは「同一ディレクトリ内の一時ファイルへ書き込み → 目的パスへリネーム」
//! の手順で行う。これにより、
//!
//! - 途中でクラッシュ・エラーが起きても目的ファイルは部分書込されず、
//!   元の内容がそのまま保持される（要件 2.6）。
//! - リネームは同一ボリューム上ではアトミックに近い操作となり、
//!   読み手が中途半端な内容を観測しない。

use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{AppError, AppResult};

/// Tag_File の読み込み結果（設計 `read_tag_file` の戻り値形状）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TagFileContent {
    /// 対応する Tag_File が存在したか（要件 2.1, 2.2）。
    pub exists: bool,
    /// Tag_File の内容（UTF-8 解釈済み）。存在しない場合は空文字列（要件 2.2）。
    pub content: String,
}

/// Image_File のパスから同名 Tag_File（`<basename>.txt`）のパスを導出する。
///
/// 拡張子部分を `txt` へ置換する。`file_service` の列挙結果（`has_tag_file`）
/// と同一の導出規則を用いる。
fn tag_file_path(image_path: &Path) -> PathBuf {
    image_path.with_extension("txt")
}

/// Image_File に対応する Tag_File を読み込む（要件 2.1, 2.2）。
///
/// - 同名 `.txt`（`<basename>.txt`）が存在すればその内容を UTF-8 として解釈し、
///   `exists=true` と内容を返す（要件 2.1）。
/// - 存在しなければ `exists=false`・`content` 空文字列を返す（新規作成可能な状態、
///   要件 2.2）。
///
/// # UTF-8 解釈の方針
///
/// 要件 2.1 は「UTF-8 として解釈」と定めるが、不正バイト列を含むファイルでも
/// 表示・編集を継続できるよう、厳密デコードで失敗させず**ロスあり変換**
/// （不正バイトを U+FFFD へ置換）で解釈する。これにより破損した Tag_File でも
/// 読み込みが局所的に失敗せず編集画面へ載せられる。
///
/// # 引数
///
/// - `image_path`: 対象 Image_File のパス。対応 Tag_File はこのパスの拡張子を
///   `txt` へ置換して導出する。
pub fn read_tag_file(image_path: impl AsRef<Path>) -> AppResult<TagFileContent> {
    let tag_path = tag_file_path(image_path.as_ref());

    match std::fs::read(&tag_path) {
        Ok(bytes) => Ok(TagFileContent {
            exists: true,
            // UTF-8 として解釈（不正バイトは U+FFFD へ置換するロスあり変換）。
            content: String::from_utf8_lossy(&bytes).into_owned(),
        }),
        // 未存在は「存在しない」＋空内容として正常扱い（要件 2.2）。
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(TagFileContent {
            exists: false,
            content: String::new(),
        }),
        // 権限不足などその他の I/O エラーはそのまま伝播。
        Err(e) => Err(AppError::from(e).with_path(tag_path.to_string_lossy().into_owned())),
    }
}

/// Image_File に対応する Tag_File へ内容を書き込む（要件 2.3, 2.4, 2.6）。
///
/// - 内容を UTF-8 で書き込む（要件 2.3）。
/// - 対応 Tag_File が存在しなければ新規作成する（要件 2.4）。
/// - 「同一ディレクトリの一時ファイルへ書込 → 目的パスへリネーム」により
///   部分書込を防ぐ。書き込みに失敗した場合、目的ファイルの元内容は
///   変更されないまま保持される（要件 2.6）。
///
/// # 引数
///
/// - `image_path`: 対象 Image_File のパス。対応 Tag_File はこのパスの拡張子を
///   `txt` へ置換して導出する。
/// - `content`: 書き込む内容（UTF-8）。
pub fn write_tag_file(image_path: impl AsRef<Path>, content: &str) -> AppResult<()> {
    let tag_path = tag_file_path(image_path.as_ref());

    // 一時ファイルは目的ファイルと同一ディレクトリに置く。
    // 別ボリュームだとリネームがコピー＋削除にフォールバックし原子性を失うため、
    // 必ず同一ディレクトリを用いる。
    let dir = tag_path.parent().ok_or_else(|| {
        AppError::invalid_input("Tag_File の親ディレクトリを解決できません")
            .with_path(tag_path.to_string_lossy().into_owned())
    })?;

    // 一時ファイル名は目的ファイル名に一意な接尾辞を付す。
    // プロセス ID と目的ファイル名を組み合わせ、同時実行時の衝突を避ける。
    let file_name = tag_path
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "tagfile".to_string());
    let tmp_name = format!(".{}.{}.tmp", file_name, std::process::id());
    let tmp_path = dir.join(tmp_name);

    // 一時ファイルへ書き込む。ここで失敗しても目的ファイルは無変更。
    let write_tmp = || -> std::io::Result<()> {
        let mut f = std::fs::File::create(&tmp_path)?;
        f.write_all(content.as_bytes())?;
        // ディスクへ確実に反映してからリネームする。
        f.flush()?;
        f.sync_all()?;
        Ok(())
    };

    if let Err(e) = write_tmp() {
        // 後始末（best-effort）。目的ファイルは元のまま（要件 2.6）。
        let _ = std::fs::remove_file(&tmp_path);
        return Err(AppError::from(e).with_path(tmp_path.to_string_lossy().into_owned()));
    }

    // 目的パスへリネーム。既存があれば置換される（新規なら作成、要件 2.3, 2.4）。
    if let Err(e) = std::fs::rename(&tmp_path, &tag_path) {
        // リネーム失敗時も一時ファイルを掃除し、目的ファイルは元のまま保持。
        let _ = std::fs::remove_file(&tmp_path);
        return Err(AppError::from(e).with_path(tag_path.to_string_lossy().into_owned()));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::AppErrorKind;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn read_missing_tag_file_reports_not_exists() {
        let dir = tempdir().unwrap();
        let image = dir.path().join("image.png");
        // Tag_File を作成しない。
        let result = read_tag_file(&image).unwrap();
        assert!(!result.exists);
        assert_eq!(result.content, "");
    }

    #[test]
    fn write_then_read_roundtrip() {
        let dir = tempdir().unwrap();
        let image = dir.path().join("image.png");
        let content = "tag1, tag2, マルチバイト タグ";

        write_tag_file(&image, content).unwrap();

        let result = read_tag_file(&image).unwrap();
        assert!(result.exists);
        assert_eq!(result.content, content);

        // 実ファイルが <basename>.txt として作成されている。
        assert!(dir.path().join("image.txt").exists());
    }

    #[test]
    fn write_creates_new_file_when_absent() {
        let dir = tempdir().unwrap();
        let image = dir.path().join("new.png");
        let tag_path = dir.path().join("new.txt");
        assert!(!tag_path.exists());

        write_tag_file(&image, "hello").unwrap();

        assert!(tag_path.exists());
        assert_eq!(fs::read_to_string(&tag_path).unwrap(), "hello");
    }

    #[test]
    fn overwrite_replaces_existing_content() {
        let dir = tempdir().unwrap();
        let image = dir.path().join("image.png");
        let tag_path = dir.path().join("image.txt");

        // 既存の内容を事前に用意する。
        fs::write(&tag_path, "old content").unwrap();

        // 上書きすると新しい内容のみが残り、旧内容は含まれない。
        write_tag_file(&image, "brand new").unwrap();

        let result = read_tag_file(&image).unwrap();
        assert!(result.exists);
        assert_eq!(result.content, "brand new");
        assert!(!result.content.contains("old"));
    }

    #[test]
    fn write_empty_content_is_valid() {
        let dir = tempdir().unwrap();
        let image = dir.path().join("image.png");

        write_tag_file(&image, "").unwrap();

        let result = read_tag_file(&image).unwrap();
        assert!(result.exists);
        assert_eq!(result.content, "");
    }

    /// テスト対象の一時ファイルパスを再構成する。
    ///
    /// 実装（`write_tag_file`）が用いる一時ファイル名 `.{目的ファイル名}.{pid}.tmp`
    /// と同一の規則で目的パスの親ディレクトリ上の一時パスを導出する。
    /// `std::process::id()` はプロセス内で不変のため、同一テストプロセス内であれば
    /// 実装が生成するのと同じパスになる。
    fn expected_tmp_path(image_path: &Path) -> PathBuf {
        let tag_path = tag_file_path(image_path);
        let dir = tag_path.parent().unwrap();
        let file_name = tag_path.file_name().unwrap().to_string_lossy().into_owned();
        dir.join(format!(".{}.{}.tmp", file_name, std::process::id()))
    }

    /// 要件 2.6: 書き込みに失敗しても既存 Tag_File の内容は変更されない。
    ///
    /// # 失敗の誘発方法（プラットフォーム非依存）
    ///
    /// `write_tag_file` は「同一ディレクトリの一時ファイル `.{name}.{pid}.tmp` へ
    /// `File::create` で書込 → 目的パスへ `rename`」の順で処理する。
    /// ここで、実装が使う一時ファイルパスと同一の位置に**ディレクトリ**を
    /// 事前作成しておくと、`File::create(&tmp_path)` はファイルを作成できず失敗する。
    /// この挙動は Windows / Unix いずれでも成立する（既存ディレクトリと同名の
    /// ファイルは作成できない）。これにより、原本を残したまま書込失敗を確実に誘発できる。
    #[test]
    fn write_failure_preserves_existing_content() {
        let dir = tempdir().unwrap();
        let image = dir.path().join("image.png");
        let tag_path = dir.path().join("image.txt");

        // 既存 Tag_File を用意（マルチバイト含む）。
        let original = "original tag1, original tag2, 既存タグ";
        fs::write(&tag_path, original).unwrap();

        // 実装が使う一時ファイルパスと同じ場所にディレクトリを作り、
        // 一時ファイルの `File::create` を失敗させる。
        let tmp_path = expected_tmp_path(&image);
        fs::create_dir(&tmp_path).unwrap();

        // 書き込みは失敗する（一時ファイル作成に失敗するため）。
        let err = write_tag_file(&image, "attempted new content").unwrap_err();
        // 一時ファイル作成失敗は I/O 系エラーとして伝播する。
        assert!(
            matches!(
                err.kind,
                AppErrorKind::Io | AppErrorKind::AlreadyExists | AppErrorKind::AccessDenied
            ),
            "unexpected error kind: {:?}",
            err.kind
        );

        // 原本はバイト単位で無変更（要件 2.6）。
        let after = fs::read_to_string(&tag_path).unwrap();
        assert_eq!(after, original);

        // 後始末で作った block 用ディレクトリを除去し、目的ファイル以外の
        // 余計な `.txt` 相当のゴミが残っていないことを確認する。
        fs::remove_dir(&tmp_path).unwrap();
    }

    /// 要件 2.6: 別ファイルへの書込が失敗しても、無関係の既存 Tag_File は無変更で維持される。
    ///
    /// 存在しないネストした親ディレクトリ配下の Image_File に対して書込を試みると、
    /// 一時ファイル作成（親ディレクトリ不在）で失敗し `Err` になる。この失敗が
    /// 別の場所にある既存 Tag_File に影響しないこと、および一時ファイルの
    /// 後始末が行われることを検証する。
    #[test]
    fn write_failure_into_missing_parent_leaves_other_files_untouched() {
        let dir = tempdir().unwrap();

        // 無関係の既存 Tag_File。
        let other_tag = dir.path().join("other.txt");
        let other_original = "keep me, 変更しない";
        fs::write(&other_tag, other_original).unwrap();

        // 存在しないネストした親配下への書込は失敗する。
        let missing = dir.path().join("does_not_exist").join("image.png");
        let err = write_tag_file(&missing, "new content").unwrap_err();
        assert!(
            matches!(err.kind, AppErrorKind::NotFound | AppErrorKind::Io),
            "unexpected error kind: {:?}",
            err.kind
        );

        // 無関係ファイルは無変更。
        assert_eq!(fs::read_to_string(&other_tag).unwrap(), other_original);

        // 一時ファイルの後始末: ディレクトリ直下に `.tmp` のゴミが残っていない。
        for entry in fs::read_dir(dir.path()).unwrap() {
            let name = entry.unwrap().file_name().to_string_lossy().into_owned();
            assert!(
                !name.ends_with(".tmp"),
                "leftover temp file found: {}",
                name
            );
        }
    }
}
