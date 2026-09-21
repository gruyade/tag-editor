// Feature: tag-editor, Property 1: 対応拡張子の列挙
//
// Property 1: 任意のファイル名集合について、フォルダ列挙結果は対応拡張子
// （jpg / jpeg / png / gif / mp4、大文字小文字を区別しない）を持つ項目のみを
// 含み、対応外拡張子（txt / bmp / webp / 拡張子なし 等）の項目を1つも含まない。
//
// Validates: Requirements 1.1
//
// 各ケースで一時ディレクトリを作成し、対応拡張子（大小混在）と対応外拡張子を
// 混ぜた実ファイルを touch してから `list_images` を呼ぶ。実ファイル生成を
// 伴うためケース数は控えめ（128 件）に設定する。

use std::collections::{BTreeSet, HashSet};
use std::fs;

use proptest::prelude::*;
use tag_editor_core::services::file_service::{list_images, SUPPORTED_EXTENSIONS};

/// 拡張子の一致判定（大文字小文字を区別しない）。
///
/// 検証側の期待値算出に用いる。実装 `has_supported_extension` と同じく
/// 最後の `.` 以降を拡張子とみなし、対応拡張子集合と ASCII 大小無視で比較する。
fn is_supported(file_name: &str) -> bool {
    match file_name.rsplit_once('.') {
        Some((_, ext)) => SUPPORTED_EXTENSIONS
            .iter()
            .any(|s| ext.eq_ignore_ascii_case(s)),
        None => false,
    }
}

/// 対応拡張子（大小混在バリエーション）の生成器。
///
/// jpg / jpeg / png / gif / mp4 を、小文字・大文字・混在の各表記で生成する。
/// 大文字小文字を区別しない一致判定（要件 1.1）を網羅的に踏むため。
fn supported_ext_strategy() -> impl Strategy<Value = String> {
    prop::sample::select(vec![
        "jpg", "JPG", "Jpg", "jpeg", "JPEG", "Jpeg", "jPeG", "png", "PNG", "Png", "gif", "GIF",
        "Gif", "mp4", "MP4", "Mp4",
    ])
    .prop_map(|s| s.to_string())
}

/// 対応外拡張子（および拡張子なし）の生成器。
///
/// txt / bmp / webp などの非対応拡張子に加え、拡張子を持たない名前
/// （空文字を表現）も含める。空文字の場合は呼び出し側でドットを付けない。
fn unsupported_ext_strategy() -> impl Strategy<Value = String> {
    prop::sample::select(vec![
        "txt", "TXT", "bmp", "BMP", "webp", "WEBP", "jpgg", "pngx", "mp", "gi", "json", "csv", "",
    ])
    .prop_map(|s| s.to_string())
}

/// ファイル名の stem（拡張子を除いた本体）の生成器。
///
/// 衝突しても set 化されるため重複は問題ない。OS のファイル名として安全な
/// ASCII 英数字・アンダースコア・ハイフンに限定する（ドットは含めない。
/// 含めると最後のドット以降が拡張子判定に使われ、期待値が複雑化するため）。
fn stem_strategy() -> impl Strategy<Value = String> {
    prop::collection::vec(
        prop::sample::select(vec![
            'a', 'b', 'c', 'M', 'Z', 'x', '0', '1', '7', '9', '_', '-',
        ]),
        1..12,
    )
    .prop_map(|v| v.into_iter().collect())
}

/// 1 ファイル分の (stem, ext) を対応/非対応いずれかから生成する。
fn file_spec_strategy() -> impl Strategy<Value = (String, String)> {
    let supported = (stem_strategy(), supported_ext_strategy());
    let unsupported = (stem_strategy(), unsupported_ext_strategy());
    prop_oneof![supported, unsupported]
}

/// (stem, ext) からファイル名を構築する。ext が空なら拡張子なし。
fn build_name(stem: &str, ext: &str) -> String {
    if ext.is_empty() {
        stem.to_string()
    } else {
        format!("{stem}.{ext}")
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(128))]

    /// Property 1: 列挙結果は対応拡張子項目のみを含み、対応外を含まない。
    ///
    /// - 生成したファイル名集合を一時ディレクトリに空ファイルとして作成する。
    /// - `list_images` の結果 items について:
    ///   1. すべての項目が対応拡張子（大小無視）を持つ。
    ///   2. 対応外拡張子の項目を1つも含まない。
    ///   3. 作成した対応拡張子ファイルはすべて結果に含まれる（漏れなし）。
    #[test]
    fn listing_contains_only_supported_extensions(
        specs in prop::collection::vec(file_spec_strategy(), 0..24)
    ) {
        let dir = tempfile::tempdir().unwrap();

        // ファイル名を作成する。
        //
        // Windows などケース非依存のファイルシステムでは `b.JPG` と `b.jpg`
        // は同一パスに解決され、後から書いた方が前を上書きしてしまう。その結果
        // ディスク上の実ファイル数が名前（大小区別）の集合より少なくなる。
        // これを避けるため、ASCII 大小を無視したキーで一意化し、最初に現れた
        // 表記だけをディスクに書き込む。`created` にはディスクに実在する
        // ファイル名のみを保持する。
        let mut created: BTreeSet<String> = BTreeSet::new();
        let mut seen_ci: HashSet<String> = HashSet::new();
        for (stem, ext) in &specs {
            let name = build_name(stem, ext);
            // 空名は無視。
            if name.is_empty() {
                continue;
            }
            // 大小無視で既出なら同一ファイルとみなしスキップ。
            if !seen_ci.insert(name.to_ascii_lowercase()) {
                continue;
            }
            // ファイル生成失敗時はスキップ。
            if fs::write(dir.path().join(&name), b"").is_ok() {
                created.insert(name);
            }
        }

        let listing = list_images(dir.path()).unwrap();

        // 期待される対応拡張子ファイル名集合。
        let expected_supported: BTreeSet<&String> =
            created.iter().filter(|n| is_supported(n)).collect();

        let returned: BTreeSet<String> =
            listing.items.iter().map(|e| e.file_name.clone()).collect();

        // 1 & 2. 返却された全項目は対応拡張子を持つ（対応外は含まれない）。
        for name in &returned {
            prop_assert!(
                is_supported(name),
                "対応外拡張子が列挙された: {:?}",
                name
            );
        }

        // 3. 作成した対応拡張子ファイルはすべて含まれる。
        for name in &expected_supported {
            prop_assert!(
                returned.contains(*name),
                "対応拡張子ファイルが列挙漏れ: {:?}",
                name
            );
        }

        // 返却集合は期待集合と完全一致（多過ぎ・少な過ぎがない）。
        let expected_owned: BTreeSet<String> =
            expected_supported.into_iter().cloned().collect();
        prop_assert_eq!(
            returned,
            expected_owned,
            "列挙結果が期待と不一致"
        );

        // total は返却件数と一致（上限未満のため truncated=false）。
        prop_assert!(!listing.truncated);
        prop_assert_eq!(listing.total, listing.items.len());
    }
}
