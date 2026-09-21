// Feature: tag-editor, Property 20: パス変換のラウンドトリップ
//
// Property 20: 任意の妥当な Windows ドライブ表記とパス要素列について、
// Windows形式→Linux形式→Windows形式の変換は元のパスと一致し、変換後の
// 文字列に変換前OSの区切り文字が混在しない。対象OSのドライブ表記規則に
// 適合しない入力は変換不能として扱われる。
//
// Validates: Requirements 13.7, 13.8, 13.9

use proptest::prelude::*;
use tag_editor_core::logic::path_convert::{
    convert_path, linux_to_windows, windows_to_linux, ConvertDirection,
};

/// ドライブ文字生成器（`A`〜`Z` の大文字 ASCII 英字）。
///
/// ラウンドトリップの正規形は「大文字ドライブ」。`windows_to_linux` は
/// ドライブを小文字化し、`linux_to_windows` は大文字化するため、元の
/// Windows パスのドライブが大文字であれば win→linux→win で一致する。
/// そこで生成側を大文字に限定し、正規形の入力のみを対象にする。
fn drive_strategy() -> impl Strategy<Value = char> {
    (0u8..26).prop_map(|n| (b'A' + n) as char)
}

/// パス要素（コンポーネント）生成器。
///
/// 制約（Property 20 の「妥当な入力」定義に対応）:
/// - 非空であること。
/// - 区切り文字 `\` `/` を含まないこと（要素の境界と混同しないため）。
/// - `:`（コロン）を含まないこと（ドライブ表記との混同を避けるため）。
///
/// 使用文字プールには空白や非 ASCII（多バイト文字）を含め、区切り以外の
/// 文字がそのまま保存されることを検証できるようにする。
fn component_strategy() -> impl Strategy<Value = String> {
    let chars = prop::sample::select(vec![
        'a', 'Z', 'M', 'x', // 英字（大小混在）
        '0', '7', '9', // 数字
        '_', '-', '.', ' ', // 記号・空白（区切り/コロン以外）
        '猫', '髪', 'あ', '한', '🎨', // 非 ASCII / マルチバイト
    ]);
    prop::collection::vec(chars, 1..12).prop_map(|v| v.into_iter().collect())
}

/// 妥当な canonical Windows パス生成器。
///
/// `<大文字drive>:\` に続けて、要素列を `\` で連結する。要素が 0 個の
/// 場合はドライブルート `<drive>:\` そのものになる。ドライブ直後に必ず
/// 区切り `\` を置くため（ドライブ相対 `C:foo` を作らない）、
/// win→linux→win のラウンドトリップが成立する形のみを生成する。
fn canonical_windows_path_strategy() -> impl Strategy<Value = String> {
    (
        drive_strategy(),
        prop::collection::vec(component_strategy(), 0..6),
    )
        .prop_map(|(drive, components)| {
            let mut s = String::new();
            s.push(drive);
            s.push(':');
            s.push('\\');
            s.push_str(&components.join("\\"));
            s
        })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// Property 20（本体）: win→linux→win は元のパスと一致する。
    ///
    /// canonical な Windows パス（大文字ドライブ + `\` 区切り）について、
    /// Windows→Linux→Windows の往復変換が元の文字列を完全に復元する。
    #[test]
    fn roundtrip_win_linux_win_equals_original(
        path in canonical_windows_path_strategy(),
    ) {
        let linux = windows_to_linux(&path)
            .expect("canonical Windows パスは変換可能");
        let back = linux_to_windows(&linux)
            .expect("生成された Linux パスは変換可能");

        prop_assert_eq!(
            &back,
            &path,
            "ラウンドトリップ不一致: {:?} -> {:?} -> {:?}",
            path,
            linux,
            back
        );
    }

    /// Property 20（区切り混在なし・Win→Linux）: 変換後の Linux 文字列に
    /// 変換前OS（Windows）の区切り文字 `\` が混在しない。
    #[test]
    fn win_to_linux_output_has_no_backslash(
        path in canonical_windows_path_strategy(),
    ) {
        let linux = windows_to_linux(&path).expect("変換可能");
        prop_assert!(
            !linux.contains('\\'),
            "Linux 変換結果にバックスラッシュが混在: input={:?} output={:?}",
            path,
            linux
        );
    }

    /// Property 20（区切り混在なし・Linux→Win）: 変換後の Windows 文字列に
    /// 変換前OS（Linux）の区切り文字 `/` が混在しない。
    #[test]
    fn linux_to_win_output_has_no_forward_slash(
        path in canonical_windows_path_strategy(),
    ) {
        // canonical Windows パスから一度 Linux 形式を得て、それを Windows へ戻す。
        let linux = windows_to_linux(&path).expect("変換可能");
        let back = linux_to_windows(&linux).expect("変換可能");
        prop_assert!(
            !back.contains('/'),
            "Windows 変換結果にスラッシュが混在: linux={:?} output={:?}",
            linux,
            back
        );
    }

    /// Property 20（dispatch 経路の一致）: `convert_path` 経由の往復でも
    /// 元のパスと一致する（方向指定 API と直接 API の整合）。
    #[test]
    fn convert_path_roundtrip_matches_direct(
        path in canonical_windows_path_strategy(),
    ) {
        let linux = convert_path(&path, ConvertDirection::WindowsToLinux)
            .expect("変換可能");
        let back = convert_path(&linux, ConvertDirection::LinuxToWindows)
            .expect("変換可能");
        prop_assert_eq!(&back, &path, "convert_path 往復不一致: {:?}", path);
    }

    /// Property 20（ドライブ表記規則違反は変換不能・相対パス）: ドライブ
    /// 表記を持たない相対パスは Win→Linux 変換不能（Err）。
    #[test]
    fn relative_windows_path_is_not_convertible(
        components in prop::collection::vec(component_strategy(), 1..6),
    ) {
        // 先頭にドライブ表記を持たない `foo\bar\...` 形式。
        let rel = components.join("\\");
        // 先頭要素がたまたま `X:` のようなドライブ表記にならないことは
        // component_strategy がコロンを含まないため保証される。
        prop_assert!(
            windows_to_linux(&rel).is_err(),
            "相対パスが変換されてしまった: {:?}",
            rel
        );
    }
}

/// ドライブ表記規則違反（ルート絶対・ドライブ相対・非 mnt）の代表例。
///
/// proptest では表現しづらい固定パターンを明示的に確認する。いずれも
/// 変換不能（Err）でなければならない（要件 13.9）。
#[test]
fn drive_rule_violations_are_not_convertible() {
    // Windows: ドライブなしのルート絶対パス。
    assert!(windows_to_linux("\\foo\\bar").is_err());
    // Windows: ドライブ相対（ドライブ直後に区切りがない）。
    assert!(windows_to_linux("C:foo").is_err());
    // Windows: 先頭が英字でない。
    assert!(windows_to_linux("1:\\foo").is_err());
    // Linux: `/mnt/` 接頭辞なし。
    assert!(linux_to_windows("/home/user").is_err());
    // Linux: 複数文字ドライブ。
    assert!(linux_to_windows("/mnt/cc/foo").is_err());
    // Linux: ドライブ文字が空。
    assert!(linux_to_windows("/mnt/").is_err());
}
