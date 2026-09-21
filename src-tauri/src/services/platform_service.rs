//! PlatformService（Windows 限定機能）。
//!
//! Windows_Only_Feature（シンボリックリンク作成・Windows/Linux パス変換）を
//! 提供するサービス層。本モジュールでは起動時の機能可否通知（[`capabilities`]）と
//! シンボリックリンク作成（[`create_symlink`]）を扱う（要件 13.1〜13.6, 16.6）。
//!
//! # プラットフォーム分岐
//!
//! Windows_Only_Feature は Windows 環境でのみ意味を持つ（用語定義 Windows_Only_Feature）。
//! 非 Windows 環境では [`create_symlink`] は [`AppErrorKind::Unsupported`] を返し、
//! [`capabilities`] は `windows_only == false` を返す。UI 層はこの値に基づき
//! Windows_Only_Feature のUIを表示・除外する（要件 13.2, 16.6）。
//!
//! [`AppErrorKind::Unsupported`]: crate::error::AppErrorKind::Unsupported

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::error::{AppError, AppResult};

/// 起動時にUI層へ通知するプラットフォーム機能可否（要件 13.1, 13.2, 16.6）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Capabilities {
    /// Windows_Only_Feature（シンボリックリンク作成・パス変換）を提供できるか。
    ///
    /// 実行環境が Windows なら `true`、それ以外なら `false`。UI はこの値が
    /// `false` のとき Windows_Only_Feature のUIを表示しない（要件 13.2, 16.6）。
    pub windows_only: bool,
}

/// 実行環境の機能可否を返す（要件 13.1, 13.2, 16.6）。
///
/// `windows_only` はコンパイル対象が Windows か否か（[`cfg!(windows)`]）で決まる。
pub fn capabilities() -> Capabilities {
    Capabilities {
        windows_only: cfg!(windows),
    }
}

/// シンボリックリンクを作成する（要件 13.3〜13.6）。
///
/// `link_path` に `link_target` を指すシンボリックリンクを作成する。作成前に
/// 以下を検査し、該当する場合はリンクを作成せず個別のエラーを返す。
///
/// - `link_target` が存在しない => [`AppErrorKind::NotFound`]（要件 13.4）。
/// - `link_path` に既存のファイル/ディレクトリが存在する =>
///   [`AppErrorKind::AlreadyExists`]（宛先使用中、要件 13.5）。
/// - 作成に必要な権限が不足 => [`AppErrorKind::AccessDenied`]（要件 13.6）。
///
/// 非 Windows 環境では Windows_Only_Feature を提供しないため、何もせず
/// [`AppErrorKind::Unsupported`] を返す（要件 13.2, 16.6）。
///
/// # 引数
/// - `link_target`: リンクが指す実体（リンク元）のパス。
/// - `link_path`: 作成するシンボリックリンク（作成先）のパス。
///
/// [`AppErrorKind::NotFound`]: crate::error::AppErrorKind::NotFound
/// [`AppErrorKind::AlreadyExists`]: crate::error::AppErrorKind::AlreadyExists
/// [`AppErrorKind::AccessDenied`]: crate::error::AppErrorKind::AccessDenied
/// [`AppErrorKind::Unsupported`]: crate::error::AppErrorKind::Unsupported
pub fn create_symlink(
    link_target: impl AsRef<Path>,
    link_path: impl AsRef<Path>,
) -> AppResult<()> {
    let target = link_target.as_ref();
    let link = link_path.as_ref();

    // プラットフォーム非依存の事前検査（要件 13.4, 13.5）。
    // 非 Windows でも同順で検査するが、実際の作成段階で Unsupported を返す。

    // リンク元不在（要件 13.4）。symlink_metadata で対象自体の存在を確認する
    // （target が壊れたリンクの場合でも「存在する」とみなす）。
    if std::fs::symlink_metadata(target).is_err() {
        return Err(AppError::not_found(format!(
            "リンク元が存在しません: {}",
            target.display()
        ))
        .with_path(target.display().to_string()));
    }

    // 宛先使用中（要件 13.5）。既存のファイル/ディレクトリ/リンクがあれば拒否する。
    if std::fs::symlink_metadata(link).is_ok() {
        return Err(AppError::already_exists(format!(
            "作成先が既に使用中です: {}",
            link.display()
        ))
        .with_path(link.display().to_string()));
    }

    create_symlink_platform(target, link)
}

/// Windows 環境でのシンボリックリンク作成本体（要件 13.3, 13.6）。
///
/// リンク元がディレクトリなら [`std::os::windows::fs::symlink_dir`]、それ以外は
/// [`std::os::windows::fs::symlink_file`] を用いる。I/O エラーは種別へ写像し、
/// 権限不足は [`AppErrorKind::AccessDenied`] とする（要件 13.6）。
///
/// [`AppErrorKind::AccessDenied`]: crate::error::AppErrorKind::AccessDenied
#[cfg(windows)]
fn create_symlink_platform(target: &Path, link: &Path) -> AppResult<()> {
    use std::os::windows::fs::{symlink_dir, symlink_file};

    // リンク元の種別に応じて dir/file を選択する。事前検査で存在は確認済み。
    let is_dir = std::fs::metadata(target).map(|m| m.is_dir()).unwrap_or(false);

    let result = if is_dir {
        symlink_dir(target, link)
    } else {
        symlink_file(target, link)
    };

    result.map_err(|e| map_symlink_io_error(e, target, link))
}

/// 非 Windows 環境ではシンボリックリンク作成を提供しない（要件 13.2, 16.6）。
#[cfg(not(windows))]
fn create_symlink_platform(_target: &Path, _link: &Path) -> AppResult<()> {
    Err(AppError::unsupported(
        "シンボリックリンク作成は Windows 環境でのみ利用できます",
    ))
}

/// シンボリックリンク作成時の I/O エラーを [`AppError`] へ写像する（要件 13.6）。
///
/// 権限不足は [`crate::error::AppErrorKind::AccessDenied`]、既存衝突は
/// [`crate::error::AppErrorKind::AlreadyExists`]、対象不在は
/// [`crate::error::AppErrorKind::NotFound`]、それ以外は
/// [`crate::error::AppErrorKind::Io`] とし、作成先パスを付与する。
#[cfg(windows)]
fn map_symlink_io_error(e: std::io::Error, target: &Path, link: &Path) -> AppError {
    use std::io::ErrorKind as IoKind;

    // Windows は権限不足を PermissionDenied で表さないことがある。symlink 作成には
    // 特権（SeCreateSymbolicLinkPrivilege）が必要で、非開発者モード/非管理者では
    // 生 OS エラー ERROR_PRIVILEGE_NOT_HELD (1314) が返る。ERROR_ACCESS_DENIED (5)
    // も併せて権限不足として扱う（要件 13.6）。
    const ERROR_ACCESS_DENIED: i32 = 5;
    const ERROR_PRIVILEGE_NOT_HELD: i32 = 1314;

    let is_privilege_error = matches!(
        e.raw_os_error(),
        Some(ERROR_ACCESS_DENIED) | Some(ERROR_PRIVILEGE_NOT_HELD)
    );

    let err = if e.kind() == IoKind::PermissionDenied || is_privilege_error {
        AppError::access_denied(format!(
            "シンボリックリンク作成の権限が不足しています: {}",
            e
        ))
    } else {
        match e.kind() {
            IoKind::AlreadyExists => AppError::already_exists(format!(
                "作成先が既に使用中です: {}",
                link.display()
            )),
            IoKind::NotFound => AppError::not_found(format!(
                "リンク元が存在しません: {}",
                target.display()
            )),
            _ => AppError::io(format!("シンボリックリンク作成に失敗しました: {}", e)),
        }
    };
    err.with_path(link.display().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::AppErrorKind;
    use tempfile::tempdir;

    #[test]
    fn capabilities_matches_cfg_windows() {
        // capabilities は cfg!(windows) と一致する（要件 13.1, 13.2, 16.6）。
        assert_eq!(capabilities().windows_only, cfg!(windows));
    }

    #[test]
    fn create_symlink_missing_target_is_not_found() {
        // リンク元が存在しない場合、事前検査で NotFound を返す（要件 13.4）。
        // 事前検査はプラットフォーム非依存であり、現在の環境で常に検証できる。
        let dir = tempdir().unwrap();
        let missing_target = dir.path().join("no_such_target");
        let link = dir.path().join("link");

        let err = create_symlink(&missing_target, &link).unwrap_err();
        assert_eq!(err.kind, AppErrorKind::NotFound);
        // リンクは作成されない。
        assert!(std::fs::symlink_metadata(&link).is_err());
    }

    #[test]
    fn create_symlink_existing_link_path_is_already_exists() {
        // 作成先が既に使用中なら AlreadyExists を返しリンクを作成しない（要件 13.5）。
        // リンク元は存在させ、事前検査の順序（不在検査 → 使用中検査）を通す。
        let dir = tempdir().unwrap();
        let target = dir.path().join("target.txt");
        std::fs::write(&target, "target").unwrap();

        let link = dir.path().join("link.txt");
        std::fs::write(&link, "existing").unwrap();

        let err = create_symlink(&target, &link).unwrap_err();
        assert_eq!(err.kind, AppErrorKind::AlreadyExists);
        // 既存の対象は変更されない（内容は維持）。
        assert_eq!(std::fs::read_to_string(&link).unwrap(), "existing");
    }

    /// 非 Windows では事前検査を通過後に Unsupported を返す（要件 13.2, 16.6）。
    #[cfg(not(windows))]
    #[test]
    fn create_symlink_unsupported_off_windows() {
        let dir = tempdir().unwrap();
        let target = dir.path().join("target.txt");
        std::fs::write(&target, "target").unwrap();
        let link = dir.path().join("link.txt");

        let err = create_symlink(&target, &link).unwrap_err();
        assert_eq!(err.kind, AppErrorKind::Unsupported);
        // リンクは作成されない。
        assert!(std::fs::symlink_metadata(&link).is_err());
    }

    /// Windows では有効なリンク元/作成先で symlink を作成できる（要件 13.3）。
    #[cfg(windows)]
    #[test]
    fn create_symlink_creates_link_on_windows() {
        let dir = tempdir().unwrap();
        let target = dir.path().join("target.txt");
        std::fs::write(&target, "target").unwrap();
        let link = dir.path().join("link.txt");

        // 開発者モード無効/権限不足の環境では AccessDenied となり得るため、
        // 成功か AccessDenied のいずれかを許容する（要件 13.3, 13.6）。
        match create_symlink(&target, &link) {
            Ok(()) => {
                assert!(std::fs::symlink_metadata(&link).is_ok());
                assert_eq!(std::fs::read_to_string(&link).unwrap(), "target");
            }
            Err(e) => assert_eq!(e.kind, AppErrorKind::AccessDenied),
        }
    }

    #[test]
    fn create_symlink_missing_target_with_dir_link_parent_is_not_found() {
        // リンク元不在の検査は link_path の親が実在ディレクトリでも変わらず
        // NotFound を返す（要件 13.4）。事前検査は link_target の存在のみを見る。
        let dir = tempdir().unwrap();
        let parent = dir.path().join("link_parent");
        std::fs::create_dir(&parent).unwrap();
        let missing_target = dir.path().join("no_such_target");
        let link = parent.join("link");

        let err = create_symlink(&missing_target, &link).unwrap_err();
        assert_eq!(err.kind, AppErrorKind::NotFound);
        // リンクは作成されず、親ディレクトリは空のまま。
        assert!(std::fs::symlink_metadata(&link).is_err());
        assert_eq!(std::fs::read_dir(&parent).unwrap().count(), 0);
    }

    /// 壊れた（dangling）シンボリックリンクをリンク元に指定した場合の挙動を検証する
    /// （要件 13.4）。事前検査は `symlink_metadata` を用いるため、実体が無くても
    /// 「リンク自体は存在する」とみなし NotFound では弾かれない。
    #[cfg(not(windows))]
    #[test]
    fn create_symlink_dangling_target_symlink_passes_precheck() {
        use std::os::unix::fs::symlink;

        let dir = tempdir().unwrap();
        // 実体の無い宛先を指す壊れたリンクを target として用意する。
        let dangling = dir.path().join("dangling");
        symlink(dir.path().join("missing_entity"), &dangling).unwrap();
        // symlink_metadata はリンク自体を検出できる（NotFound ではない）。
        assert!(std::fs::symlink_metadata(&dangling).is_ok());

        let link = dir.path().join("link");
        // 事前検査（不在検査）は通過し、作成段階で Unsupported（非 Windows）となる。
        // 少なくとも「リンク元不在(NotFound)」では弾かれないことを確認する。
        let err = create_symlink(&dangling, &link).unwrap_err();
        assert_eq!(err.kind, AppErrorKind::Unsupported);
        assert!(std::fs::symlink_metadata(&link).is_err());
    }

    #[test]
    fn create_symlink_existing_directory_link_path_is_already_exists() {
        // 作成先が既存の「ディレクトリ」でも AlreadyExists を返し、
        // そのディレクトリの中身は変更しない（要件 13.5）。
        let dir = tempdir().unwrap();
        let target = dir.path().join("target.txt");
        std::fs::write(&target, "target").unwrap();

        // link_path として既存ディレクトリ（中に既存ファイルあり）を用意する。
        let link_dir = dir.path().join("existing_dir");
        std::fs::create_dir(&link_dir).unwrap();
        let inner = link_dir.join("keep.txt");
        std::fs::write(&inner, "keep").unwrap();

        let err = create_symlink(&target, &link_dir).unwrap_err();
        assert_eq!(err.kind, AppErrorKind::AlreadyExists);
        // 既存ディレクトリは untouched（ディレクトリのまま・中身も維持）。
        assert!(std::fs::metadata(&link_dir).unwrap().is_dir());
        assert_eq!(std::fs::read_to_string(&inner).unwrap(), "keep");
    }

    /// 権限不足（要件 13.6）の写像を検証する。
    ///
    /// Windows では symlink 作成に特権が必要で、テストから権限不足を決定論的に
    /// 強制するのは難しい（開発者モード/管理者権限の有無に依存）。そこで有効な
    /// リクエストで作成を試み、`Err` になる場合はそれが事前検査/作成段階で規定した
    /// 種別（NotFound/AlreadyExists/AccessDenied/Unsupported）のいずれかであり、
    /// 生の `Io` へは落ちないことを確認する。
    #[cfg(windows)]
    #[test]
    fn create_symlink_permission_denied_maps_to_known_kinds_on_windows() {
        let dir = tempdir().unwrap();
        let target = dir.path().join("target.txt");
        std::fs::write(&target, "target").unwrap();
        let link = dir.path().join("perm_link.txt");

        match create_symlink(&target, &link) {
            Ok(()) => {
                // 権限があれば作成される。
                assert!(std::fs::symlink_metadata(&link).is_ok());
            }
            Err(e) => {
                assert!(
                    matches!(
                        e.kind,
                        AppErrorKind::NotFound
                            | AppErrorKind::AlreadyExists
                            | AppErrorKind::AccessDenied
                            | AppErrorKind::Unsupported
                    ),
                    "予期しないエラー種別: {:?}",
                    e.kind
                );
                // 特に生の Io へは落ちない（要件 13.6 の写像）。
                assert_ne!(e.kind, AppErrorKind::Io);
            }
        }
    }

    /// 非 Windows では、その他は有効なリクエストでも作成段階で Unsupported を返す
    /// （要件 13.2, 13.6, 16.6）。権限不足の写像は Windows 専用経路のため、
    /// 非 Windows では「機能未提供」であることを保証する。
    #[cfg(not(windows))]
    #[test]
    fn create_symlink_valid_request_is_unsupported_off_windows() {
        let dir = tempdir().unwrap();
        let target = dir.path().join("target.txt");
        std::fs::write(&target, "target").unwrap();
        let link = dir.path().join("link.txt");

        let err = create_symlink(&target, &link).unwrap_err();
        assert_eq!(err.kind, AppErrorKind::Unsupported);
        assert!(std::fs::symlink_metadata(&link).is_err());
    }
}

/// パス変換の方向（[`crate::logic::path_convert::ConvertDirection`] の再公開）。
///
/// サービス／コマンド境界の呼び出し側が logic 層を直接参照せずに方向を指定できるよう、
/// 純粋ロジックの型をそのまま公開する。
pub use crate::logic::path_convert::ConvertDirection;

/// Windows/Linux(WSL) パスを変換する（要件 13.7, 13.8, 13.9）。
///
/// タスク 7.1 の純粋ロジック [`crate::logic::path_convert::convert_path`] を
/// サービス／コマンド境界へ公開する薄いアダプタ。変換規則・エラー条件
/// （変換不能な入力に対する [`AppErrorKind::InvalidInput`]）はすべて logic 層に委譲する。
///
/// # プラットフォームについて
///
/// シンボリックリンク作成（[`create_symlink`]）と異なり、パス変換は入力文字列のみに
/// 依存する純粋な文字列変換であり OS 機能に依存しない。したがって Core 関数自体は
/// プラットフォームで分岐せず、非 Windows 環境でも同じ結果を返す。UI 上は
/// Windows_Only_Feature として扱われるが、その表示制御は [`capabilities`] の
/// `windows_only` により UI 層で行う（要件 13.2, 16.6）。
///
/// # エラー
///
/// 入力が対象 OS 形式のドライブ表記規則に適合しない場合、
/// [`AppErrorKind::InvalidInput`] を返す（要件 13.9）。
///
/// [`AppErrorKind::InvalidInput`]: crate::error::AppErrorKind::InvalidInput
pub fn convert_path(path: &str, direction: ConvertDirection) -> AppResult<String> {
    crate::logic::path_convert::convert_path(path, direction)
}

#[cfg(test)]
mod path_convert_tests {
    use super::*;
    use crate::error::AppErrorKind;

    #[test]
    fn convert_path_delegates_win_to_linux() {
        // Win→Linux 変換が logic 層へ委譲され期待どおりの結果を返す（要件 13.7）。
        assert_eq!(
            convert_path("C:\\foo\\bar", ConvertDirection::WindowsToLinux).unwrap(),
            "/mnt/c/foo/bar"
        );
    }

    #[test]
    fn convert_path_delegates_linux_to_win() {
        // Linux→Win 変換が logic 層へ委譲され期待どおりの結果を返す（要件 13.8）。
        assert_eq!(
            convert_path("/mnt/c/foo/bar", ConvertDirection::LinuxToWindows).unwrap(),
            "C:\\foo\\bar"
        );
    }

    #[test]
    fn convert_path_invalid_input_is_invalid() {
        // 変換規則に適合しない入力は InvalidInput（要件 13.9）。
        let err = convert_path("foo\\bar", ConvertDirection::WindowsToLinux).unwrap_err();
        assert_eq!(err.kind, AppErrorKind::InvalidInput);
    }
}
