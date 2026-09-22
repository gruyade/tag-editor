//! Windows/Linux(WSL) パス変換（タスク 7.1）。
//!
//! 純粋ロジックとして、Windows 形式パスと Linux(WSL) 形式パスを相互変換する。
//! ファイル I/O には依存せず、入力文字列のみから決定的に結果を算出する。
//!
//! 変換規則（要件 13.7 / 13.8 / 13.9、design.md `PlatformService::convert_path`）:
//!
//! - Win→Linux: 区切り文字 `\` を `/` へ、ドライブ表記 `C:\` を `/mnt/c/` へ。
//!   ドライブ文字は小文字化する。
//! - Linux→Win: 区切り文字 `/` を `\` へ、ドライブ表記 `/mnt/c/` を `C:\` へ。
//!   ドライブ文字は大文字化する。
//! - 対象 OS 形式のドライブ表記規則に適合しない入力は変換不能
//!   （[`AppErrorKind::InvalidInput`]）とする。
//!
//! 妥当な入力について Win→Linux→Win のラウンドトリップが成立し、
//! 変換結果に変換前 OS の区切り文字が混在しないことを保証する（Property 20）。

use crate::error::{AppError, AppResult};

/// パス変換の方向。
///
/// Tauri コマンド境界の引数として受け取るため serde 対応する。フロントエンドは
/// バリアント名（"WindowsToLinux" / "LinuxToWindows"）の文字列で指定する。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ConvertDirection {
    /// Windows 形式 → Linux(WSL) 形式。
    WindowsToLinux,
    /// Linux(WSL) 形式 → Windows 形式。
    LinuxToWindows,
}

/// 指定方向に従ってパスを変換する（要件 13.7 / 13.8 / 13.9）。
///
/// # エラー
///
/// 入力が対象 OS 形式のドライブ表記規則に適合しない場合、
/// [`AppErrorKind::InvalidInput`](crate::error::AppErrorKind::InvalidInput)
/// のエラーを返す（変換不能）。
pub fn convert_path(path: &str, direction: ConvertDirection) -> AppResult<String> {
    match direction {
        ConvertDirection::WindowsToLinux => windows_to_linux(path),
        ConvertDirection::LinuxToWindows => linux_to_windows(path),
    }
}

/// Windows 形式パスを Linux(WSL) 形式へ変換する（要件 13.7）。
///
/// 入力は `<drive>:\...` または `<drive>:` 形式のドライブ表記を持つ必要がある。
/// `\` は `/` へ、`<drive>:` は `/mnt/<小文字drive>` へ変換する。
/// ドライブ表記を持たない入力は変換不能（要件 13.9）。
pub fn windows_to_linux(path: &str) -> AppResult<String> {
    // ドライブ表記 `<letter>:` を先頭に要求する。
    let (drive, rest) = split_windows_drive(path)
        .ok_or_else(|| invalid("Windows ドライブ表記（例: C:\\foo）ではありません", path))?;

    // ドライブ直後は区切り（`\` または `/`）か終端でなければならない。
    // 例: `C:foo`（ドライブ相対パス）は WSL パスへ一意変換できないため不適合とする。
    if let Some(first) = rest.chars().next() {
        if first != '\\' && first != '/' {
            return Err(invalid(
                "ドライブ直後に区切り文字がありません（ドライブ相対パスは非対応）",
                path,
            ));
        }
    }

    // ルート `/mnt/<小文字drive>` を構築し、残りの区切りを `/` へ正規化する。
    let mut out = String::with_capacity(rest.len() + 8);
    out.push_str("/mnt/");
    out.push(drive.to_ascii_lowercase());
    for ch in rest.chars() {
        out.push(if ch == '\\' { '/' } else { ch });
    }
    Ok(out)
}

/// Linux(WSL) 形式パスを Windows 形式へ変換する（要件 13.8）。
///
/// 入力は `/mnt/<letter>` または `/mnt/<letter>/...` 形式である必要がある。
/// `/mnt/<letter>` を `<大文字letter>:` へ、`/` を `\` へ変換する。
/// この形式に適合しない入力は変換不能（要件 13.9）。
pub fn linux_to_windows(path: &str) -> AppResult<String> {
    let (drive, rest) = split_mnt_drive(path)
        .ok_or_else(|| invalid("WSL ドライブ表記（例: /mnt/c/foo）ではありません", path))?;

    // ルート `<大文字drive>:\` を構築し、残りの区切りを `\` へ正規化する。
    let mut out = String::with_capacity(rest.len() + 3);
    out.push(drive.to_ascii_uppercase());
    out.push(':');
    out.push('\\');
    // rest は先頭の `/` を含まない残余（split_mnt_drive が除去済み）。
    for ch in rest.chars() {
        out.push(if ch == '/' { '\\' } else { ch });
    }
    Ok(out)
}

/// `<letter>:` 形式のドライブ接頭辞を分離する。
///
/// 成功時、ドライブ文字（ASCII 英字）とドライブ表記以降の残余を返す。
/// 残余には `:` は含まれない（`C:\foo` なら `\foo`、`C:` なら空文字列）。
fn split_windows_drive(path: &str) -> Option<(char, &str)> {
    let mut chars = path.chars();
    let drive = chars.next()?;
    if !drive.is_ascii_alphabetic() {
        return None;
    }
    if chars.next()? != ':' {
        return None;
    }
    // `<letter>:` の 2 バイト（ASCII）を除いた残余。
    Some((drive, &path[2..]))
}

/// `/mnt/<letter>` 形式のドライブ接頭辞を分離する。
///
/// 成功時、ドライブ文字（ASCII 英字）とドライブ表記以降の残余を返す。
/// 残余は先頭の区切り `/` を含まない（`/mnt/c/foo` なら `foo`、
/// `/mnt/c` や `/mnt/c/` なら空文字列）。
fn split_mnt_drive(path: &str) -> Option<(char, &str)> {
    // 接頭辞 `/mnt/` を要求する。
    let after_prefix = path.strip_prefix("/mnt/")?;

    let mut chars = after_prefix.chars();
    let drive = chars.next()?;
    if !drive.is_ascii_alphabetic() {
        return None;
    }

    // ドライブ文字直後は終端か区切り `/` でなければならない
    // （`/mnt/cc/...` のような複数文字ドライブは不適合）。
    let rest_with_sep = &after_prefix[1..];
    match rest_with_sep.chars().next() {
        None => Some((drive, "")),
        Some('/') => Some((drive, &rest_with_sep[1..])),
        Some(_) => None,
    }
}

/// 変換不能エラーを生成する（要件 13.9）。
fn invalid(message: &str, path: &str) -> AppError {
    AppError::invalid_input(message).with_path(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::AppErrorKind;

    // --- Windows → Linux ---

    #[test]
    fn win_to_linux_basic() {
        assert_eq!(windows_to_linux("C:\\foo\\bar").unwrap(), "/mnt/c/foo/bar");
    }

    #[test]
    fn win_to_linux_lowercases_drive() {
        assert_eq!(windows_to_linux("D:\\Work").unwrap(), "/mnt/d/Work");
        assert_eq!(windows_to_linux("Z:\\a\\b").unwrap(), "/mnt/z/a/b");
    }

    #[test]
    fn win_to_linux_drive_root_only() {
        assert_eq!(windows_to_linux("C:\\").unwrap(), "/mnt/c/");
    }

    #[test]
    fn win_to_linux_drive_no_trailing_separator() {
        // `C:` はドライブ直後が終端。/mnt/c を返す。
        assert_eq!(windows_to_linux("C:").unwrap(), "/mnt/c");
    }

    #[test]
    fn win_to_linux_preserves_non_separator_chars() {
        assert_eq!(
            windows_to_linux("C:\\Users\\名前\\a b").unwrap(),
            "/mnt/c/Users/名前/a b"
        );
    }

    #[test]
    fn win_to_linux_accepts_forward_slashes_in_body() {
        // 既にスラッシュ混在でも区切りとして扱い正規化する。
        assert_eq!(windows_to_linux("C:/foo/bar").unwrap(), "/mnt/c/foo/bar");
    }

    #[test]
    fn win_to_linux_no_drive_is_invalid() {
        let err = windows_to_linux("\\foo\\bar").unwrap_err();
        assert_eq!(err.kind, AppErrorKind::InvalidInput);
    }

    #[test]
    fn win_to_linux_relative_is_invalid() {
        let err = windows_to_linux("foo\\bar").unwrap_err();
        assert_eq!(err.kind, AppErrorKind::InvalidInput);
    }

    #[test]
    fn win_to_linux_drive_relative_is_invalid() {
        // `C:foo`（ドライブ相対）は一意変換不能として拒否する。
        let err = windows_to_linux("C:foo").unwrap_err();
        assert_eq!(err.kind, AppErrorKind::InvalidInput);
    }

    #[test]
    fn win_to_linux_output_has_no_backslash() {
        let out = windows_to_linux("C:\\a\\b\\c").unwrap();
        assert!(!out.contains('\\'), "出力にバックスラッシュが混在: {out:?}");
    }

    // --- Linux → Windows ---

    #[test]
    fn linux_to_win_basic() {
        assert_eq!(linux_to_windows("/mnt/c/foo/bar").unwrap(), "C:\\foo\\bar");
    }

    #[test]
    fn linux_to_win_uppercases_drive() {
        assert_eq!(linux_to_windows("/mnt/d/work").unwrap(), "D:\\work");
        assert_eq!(linux_to_windows("/mnt/z/a/b").unwrap(), "Z:\\a\\b");
    }

    #[test]
    fn linux_to_win_drive_root_only() {
        assert_eq!(linux_to_windows("/mnt/c/").unwrap(), "C:\\");
        assert_eq!(linux_to_windows("/mnt/c").unwrap(), "C:\\");
    }

    #[test]
    fn linux_to_win_preserves_non_separator_chars() {
        assert_eq!(
            linux_to_windows("/mnt/c/Users/名前/a b").unwrap(),
            "C:\\Users\\名前\\a b"
        );
    }

    #[test]
    fn linux_to_win_uppercase_drive_letter_input() {
        // 入力ドライブ文字が大文字でも受理し大文字化して出力する。
        assert_eq!(linux_to_windows("/mnt/C/foo").unwrap(), "C:\\foo");
    }

    #[test]
    fn linux_to_win_without_prefix_is_invalid() {
        let err = linux_to_windows("/home/user").unwrap_err();
        assert_eq!(err.kind, AppErrorKind::InvalidInput);
    }

    #[test]
    fn linux_to_win_relative_is_invalid() {
        let err = linux_to_windows("mnt/c/foo").unwrap_err();
        assert_eq!(err.kind, AppErrorKind::InvalidInput);
    }

    #[test]
    fn linux_to_win_multichar_drive_is_invalid() {
        // `/mnt/cc/...` は単一ドライブ文字規則に不適合。
        let err = linux_to_windows("/mnt/cc/foo").unwrap_err();
        assert_eq!(err.kind, AppErrorKind::InvalidInput);
    }

    #[test]
    fn linux_to_win_non_alpha_drive_is_invalid() {
        let err = linux_to_windows("/mnt/1/foo").unwrap_err();
        assert_eq!(err.kind, AppErrorKind::InvalidInput);
    }

    #[test]
    fn linux_to_win_empty_drive_is_invalid() {
        let err = linux_to_windows("/mnt/").unwrap_err();
        assert_eq!(err.kind, AppErrorKind::InvalidInput);
    }

    #[test]
    fn linux_to_win_output_has_no_forward_slash() {
        let out = linux_to_windows("/mnt/c/a/b/c").unwrap();
        assert!(!out.contains('/'), "出力にスラッシュが混在: {out:?}");
    }

    // --- convert_path dispatch ---

    #[test]
    fn convert_path_dispatches_win_to_linux() {
        assert_eq!(
            convert_path("C:\\foo", ConvertDirection::WindowsToLinux).unwrap(),
            "/mnt/c/foo"
        );
    }

    #[test]
    fn convert_path_dispatches_linux_to_win() {
        assert_eq!(
            convert_path("/mnt/c/foo", ConvertDirection::LinuxToWindows).unwrap(),
            "C:\\foo"
        );
    }

    // --- Roundtrip (Property 20 の代表例) ---

    #[test]
    fn roundtrip_win_linux_win_holds() {
        let inputs = [
            "C:\\foo\\bar",
            "D:\\Users\\名前\\a b",
            "Z:\\",
            "E:\\deeply\\nested\\path\\to\\file.txt",
        ];
        for input in inputs {
            let linux = windows_to_linux(input).unwrap();
            let back = linux_to_windows(&linux).unwrap();
            assert_eq!(
                back, input,
                "ラウンドトリップ不一致: {input:?} -> {linux:?} -> {back:?}"
            );
        }
    }
}
