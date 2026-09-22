// Feature: tag-editor, Property 4: Tag_File 書込→読込のラウンドトリップと上書き
//
// Property 4: 任意の UTF-8 タグ内容について、Tag_File へ書き込んだ後に
// 読み込むと同一の内容が得られ、既存内容の有無に関わらず読み込み結果は
// 最後に書き込んだ内容（既存内容を含まない）と一致する。
//
// 対象: tag_editor_core::services::tag_file::{write_tag_file, read_tag_file}
// 書き込みは一時ファイル → リネームでの原子的置換であり、`&str`（妥当な
// UTF-8）を書いて読み戻すためロスあり変換（U+FFFD 置換）は発生しない。
// よって書込内容と読込内容は完全一致する。
//
// Validates: Requirements 2.1, 2.3, 2.4, 14.5, 14.6

use proptest::prelude::*;
use tempfile::tempdir;

use tag_editor_core::services::tag_file::{read_tag_file, write_tag_file};

/// 任意の UTF-8 タグ内容の生成器。
///
/// Property 4 の「任意の UTF-8 内容」を表現するため、以下を確実に含める:
/// - 空文字列（要素数 0 を許可）。
/// - マルチバイト文字（日本語・韓国語・絵文字）。
/// - 改行 `\n` / キャリッジリターン `\r`。
/// - カンマ `,`（Tag_File のタグ区切り）。
/// - 通常の ASCII 英数字・空白・記号。
///
/// `proptest::char::any()` に加えて上記の代表文字を選択プールへ混ぜ、
/// 幅広い Unicode スカラ値と重要文字の双方を網羅する。
fn content_strategy() -> impl Strategy<Value = String> {
    let special = prop::sample::select(vec![
        'a',
        'Z',
        '0',
        '9',
        ' ',
        '_',
        '-',
        '.', // ASCII
        ',',
        '\n',
        '\r',
        '\t', // 区切り・制御
        '猫',
        '髪',
        'あ',
        '한',
        '🎨',
        '🐈',
        '\u{1F600}', // マルチバイト
    ]);
    // 特殊文字プールと任意の Unicode スカラ値を混ぜる。
    let mixed = prop_oneof![special, prop::char::any()];
    // 要素数 0 を許可し、空文字列を必ず生成対象に含める。
    prop::collection::vec(mixed, 0..64).prop_map(|v| v.into_iter().collect())
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// Property 4（ラウンドトリップ）: 書き込んだ内容を読み戻すと一致する。
    ///
    /// 新規（Tag_File が存在しない状態）に書き込み、読み込むと `exists=true`
    /// かつ `content` が書込内容と完全一致する（要件 2.1, 2.3, 2.4）。
    #[test]
    fn write_then_read_roundtrip_equals_original(content in content_strategy()) {
        let dir = tempdir().expect("一時ディレクトリ作成");
        let image = dir.path().join("image.png");

        // 事前状態: Tag_File は存在しない。
        let before = read_tag_file(&image).expect("読み込み成功（未存在）");
        prop_assert!(!before.exists, "書込前は Tag_File が存在しないはず");

        write_tag_file(&image, &content).expect("書き込み成功");

        let after = read_tag_file(&image).expect("読み込み成功");
        prop_assert!(after.exists, "書込後は Tag_File が存在するはず");
        prop_assert_eq!(
            &after.content,
            &content,
            "ラウンドトリップ不一致: 書込={:?} 読込={:?}",
            content,
            after.content
        );
    }

    /// Property 4（上書き）: 既存内容の有無に関わらず、読込結果は最後に
    /// 書き込んだ内容と一致し、以前の内容を含まない（要件 2.3, 2.4）。
    ///
    /// 2 つの任意 UTF-8 内容 `first` / `second` について、`first` を書いた
    /// 後に `second` を書き、読み戻すと `second` と完全一致する。さらに
    /// `first` が非空かつ `second` が `first` を部分文字列として含まない
    /// 場合、読込結果に `first` が残っていないことを確認する。
    #[test]
    fn overwrite_read_equals_last_written(
        first in content_strategy(),
        second in content_strategy(),
    ) {
        let dir = tempdir().expect("一時ディレクトリ作成");
        let image = dir.path().join("image.png");

        // 1 回目の書き込み（既存内容を作る）。
        write_tag_file(&image, &first).expect("1回目書き込み成功");
        let after_first = read_tag_file(&image).expect("1回目読み込み成功");
        prop_assert_eq!(&after_first.content, &first, "1回目ラウンドトリップ不一致");

        // 2 回目の書き込み（上書き）。
        write_tag_file(&image, &second).expect("2回目書き込み成功");
        let after_second = read_tag_file(&image).expect("2回目読み込み成功");

        prop_assert!(after_second.exists, "上書き後も Tag_File は存在する");
        prop_assert_eq!(
            &after_second.content,
            &second,
            "上書き後の内容が最後の書込と不一致: 期待={:?} 実際={:?}",
            second,
            after_second.content
        );

        // 旧内容が残っていないことの確認（部分列としての混入がない場合のみ）。
        // second が first を部分文字列として含むケースは正当なので除外する。
        if !first.is_empty() && !second.contains(&first) {
            prop_assert!(
                !after_second.content.contains(&first),
                "上書き後に旧内容が残存: 旧={:?} 新={:?} 結果={:?}",
                first,
                second,
                after_second.content
            );
        }
    }

    /// Property 4（事前既存ファイルの上書き）: read/write API 外で用意した
    /// 既存 Tag_File を write_tag_file で上書きしても、読込結果は最後に
    /// 書き込んだ内容のみと一致する（要件 2.3, 2.4）。
    #[test]
    fn overwrite_preexisting_file_equals_written(
        preexisting in content_strategy(),
        new_content in content_strategy(),
    ) {
        let dir = tempdir().expect("一時ディレクトリ作成");
        let image = dir.path().join("image.png");
        let tag_path = dir.path().join("image.txt");

        // API を介さずに既存 Tag_File を直接用意する。
        std::fs::write(&tag_path, preexisting.as_bytes()).expect("既存ファイル作成");

        write_tag_file(&image, &new_content).expect("上書き書き込み成功");

        let after = read_tag_file(&image).expect("読み込み成功");
        prop_assert!(after.exists);
        prop_assert_eq!(
            &after.content,
            &new_content,
            "既存上書き後の内容不一致: 期待={:?} 実際={:?}",
            new_content,
            after.content
        );
    }
}
