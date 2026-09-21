// Feature: tag-editor, Property 14: gather/distribute 命名のラウンドトリップ
//
// Validates: Requirements 7.3, 7.4
//
// 対象: `tag_editor_core::logic::naming::{gather_name, split_gathered_name}`。
// 区切り文字は `__`（PREFIX_DELIMITER）。
//
// プロパティ本文:
//   任意のサブフォルダ名とファイル名について、gather による接頭辞付与命名を
//   distribute で分解すると、元のサブフォルダ名とファイル名が復元される。
//     split_gathered_name(gather_name(sub, file)) == Some((sub, file))
//
// ジェネレータ制約（本文が前提とする入力空間へ最小限に絞り、それぞれ正当化）:
//   - subfolder（サブフォルダ名）は非空で、gather 後に最初に現れる `__` が
//     ちょうど subfolder と file_name の境界に一致する文字列を生成する。
//     具体的には `format!("{subfolder}__").find("__") == Some(subfolder.len())`
//     を満たすものに絞る。
//       split_gathered_name は「最初に現れる `__`」で分割する。subfolder が
//       `__` を部分文字列に含む、あるいは subfolder が `_` で終わる場合、
//       付与した区切り `__` と連結して `__` の並びが境界より前へ前倒しになり
//       （例: subfolder="_" + "__" = "___" は先頭で分割される）、元の
//       subfolder を復元できない。よって「最初の `__` が境界に一致する」ことが
//       ラウンドトリップ成立の前提であり、その入力空間に絞る（要件 7.7 の
//       分解不能ケースは本プロパティの対象外）。
//   - file_name（ファイル名）は非空の文字列を生成する。file_name は最初の
//     区切りより後ろにあるため `__` を含んでよい（分割は最初の区切りのみ使用）。
//       空の subfolder / file_name は split_gathered_name が None を返す
//       分解不能ケースであり、ラウンドトリップ対象外のため除外する。
//   - 文字集合はマルチバイト（日本語）・空白・記号・ASCII を含め、実運用の
//     フォルダ名/ファイル名の多様性を反映する。

use proptest::prelude::*;
use tag_editor_core::logic::naming::{gather_name, split_gathered_name};

/// ラウンドトリップに用いる文字集合。
/// ASCII 英数・空白・記号・マルチバイト（日本語）を混在させ、実運用の
/// フォルダ名/ファイル名の多様性をカバーする。
/// 注意: 単一の `_` は許可するが、後段のフィルタで `__` を含む候補を除外する。
fn name_chars() -> impl Strategy<Value = char> {
    prop_oneof![
        // ASCII 英数字。
        prop::char::range('a', 'z'),
        prop::char::range('A', 'Z'),
        prop::char::range('0', '9'),
        // 空白と一般的なファイル名記号（`_` `.` `-` ` `）。
        Just(' '),
        Just('_'),
        Just('.'),
        Just('-'),
        // マルチバイト（日本語ひらがな・漢字の代表例）。
        Just('あ'),
        Just('ん'),
        Just('猫'),
        Just('画'),
    ]
}

/// subfolder のジェネレータ。
/// 非空、かつ gather 後の最初の `__` が境界に一致する文字列に絞る。
/// これは「`__` を部分文字列に含まず、末尾が `_` でない」ことと同値であり、
/// この条件のみがラウンドトリップを保証する（末尾 `_` は付与区切りと連結して
/// 分割位置が前倒しになるため除外）。
fn subfolder_strategy() -> impl Strategy<Value = String> {
    prop::collection::vec(name_chars(), 1..=20)
        .prop_map(|chars| chars.into_iter().collect::<String>())
        .prop_filter(
            "subfolder は空でなく、gather 後の最初の `__` が境界に一致すること",
            |s| !s.is_empty() && format!("{s}__").find("__") == Some(s.len()),
        )
}

/// file_name のジェネレータ。
/// 非空であればよい（`__` を含んでもラウンドトリップは成立する）。
fn file_name_strategy() -> impl Strategy<Value = String> {
    prop::collection::vec(name_chars(), 1..=20)
        .prop_map(|chars| chars.into_iter().collect::<String>())
        .prop_filter("file_name は空でないこと", |s| !s.is_empty())
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(300))]

    /// gather → distribute のラウンドトリップで元の (subfolder, file_name) を復元する。
    #[test]
    fn gather_then_split_roundtrips(
        subfolder in subfolder_strategy(),
        file_name in file_name_strategy(),
    ) {
        let gathered = gather_name(&subfolder, &file_name);

        // gather は `<subfolder>__<file_name>` を生成する。
        prop_assert_eq!(
            &gathered,
            &format!("{}__{}", subfolder, file_name),
            "gather_name の生成結果が期待形式と不一致"
        );

        // distribute（split）で元の 2 要素へ分解でき、ラウンドトリップが成立する。
        let split = split_gathered_name(&gathered);
        prop_assert_eq!(
            split,
            Some((subfolder.clone(), file_name.clone())),
            "gather → split のラウンドトリップが成立しない: subfolder={:?}, file_name={:?}",
            subfolder,
            file_name
        );
    }

    /// 明示的にマルチバイト・空白を含む file_name（`__` を含むケース含む）でも
    /// 復元されることを確認する（分割は最初の区切りのみを使用）。
    #[test]
    fn roundtrip_with_delimiter_inside_file_name(
        subfolder in subfolder_strategy(),
        file_name in file_name_strategy(),
    ) {
        // file_name に区切り `__` を明示的に含める（先頭以外へ挿入）。
        let file_with_delim = format!("{}__tail", file_name);
        let gathered = gather_name(&subfolder, &file_with_delim);

        let split = split_gathered_name(&gathered);
        prop_assert_eq!(
            split,
            Some((subfolder.clone(), file_with_delim.clone())),
            "file_name 内の `__` があっても最初の区切りで分割されラウンドトリップが成立すべき"
        );
    }
}
