// Feature: tag-editor, Property 8: ファイル名正規化
//
// Property 8: 任意の basename と拡張子について、`.<ext>.txt` 形式の
// ファイル名は正規化により `<basename>.txt` 形式へ変換される。
//
// Validates: Requirements 4.1

use proptest::prelude::*;
use tag_editor_core::logic::naming::normalize_caption_name;

/// basename 生成器。
///
/// 実装 `normalize_caption_name` は `.txt` を剥がした残り（stem）に対し
/// **最後の** `.` で basename と ext を分割する。したがって Property 8 で
/// 期待する `<basename>.txt` を一意に成立させるため、basename には以下の
/// 制約を課す（実装のロジックと整合させる）:
///
/// - 空でないこと（basename が空だと実装は `None` を返す）。
/// - `.`（ドット）を含まないこと。basename にドットがあると、stem 上の
///   最後のドットが basename 内側になり得るため、期待値が
///   「`<basename>` の最後のドット以降を剥がした形」となり単純化できない。
///   ここでは Property 8 の中核（1 段の二重拡張子剥がし）を検証するため
///   ドットなしに限定する。
/// - `.txt`（大小無視）末尾に化けないよう、末尾判定を乱す文字は使わない。
///   使用文字は ASCII 英数字・アンダースコア・ハイフン・空白・非 ASCII の
///   多様な文字プールから選ぶ（ドットとヌルを除く）。
fn basename_strategy() -> impl Strategy<Value = String> {
    let chars = prop::sample::select(vec![
        'a', 'Z', 'M', 'x', // 英字（大小混在）
        '0', '7', '9', // 数字
        '_', '-', ' ', // 区切り・空白（ドット以外の記号）
        '猫', '髪', 'あ', '한', '🎨', // 非 ASCII / マルチバイト
    ]);
    // 1 文字以上（空 basename を避ける）。
    prop::collection::vec(chars, 1..24).prop_map(|v| v.into_iter().collect())
}

/// 拡張子生成器。
///
/// 実装は ext が空でなければ正規化対象とみなす（ext の中身は問わない）。
/// 代表的な Image_File 拡張子（png/jpg/jpeg/gif/mp4）に加え、任意の
/// 拡張子も網羅するため以下の制約で生成する:
///
/// - 空でないこと（ext が空だと実装は `None` を返す）。
/// - `.`（ドット）を含まないこと。ext にドットがあると stem 上の最後の
///   ドットが ext 内側へ移り、期待する分割位置がずれるため除外する。
fn ext_strategy() -> impl Strategy<Value = String> {
    prop_oneof![
        // 代表的な Image_File 拡張子。
        Just("png".to_string()),
        Just("jpg".to_string()),
        Just("jpeg".to_string()),
        Just("gif".to_string()),
        Just("mp4".to_string()),
        // 任意の拡張子（ドットを含まない 1〜8 文字）。
        prop::collection::vec(
            prop::sample::select(vec!['a', 'B', 'z', '0', '5', '_', '猫']),
            1..8,
        )
        .prop_map(|v| v.into_iter().collect::<String>()),
    ]
}

/// `.txt` 末尾サフィックスの大小バリエーション生成器。
///
/// 実装 `strip_txt_suffix` は末尾 4 文字を `.eq_ignore_ascii_case(".txt")`
/// で判定するため、`.txt` / `.TXT` / `.Txt` などいずれでも剥がされ、
/// 出力は常に小文字 `.txt` へ揃う。
fn txt_suffix_strategy() -> impl Strategy<Value = String> {
    prop::sample::select(vec![
        ".txt".to_string(),
        ".TXT".to_string(),
        ".Txt".to_string(),
        ".tXt".to_string(),
        ".TxT".to_string(),
    ])
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// Property 8: `<basename>.<ext>.txt` は `<basename>.txt` へ正規化される。
    ///
    /// basename（ドットなし・非空）と ext（ドットなし・非空）から
    /// `<basename>.<ext>.txt` を組み立て、正規化結果が
    /// `Some("<basename>.txt")` となることを検証する。出力拡張子は常に
    /// 小文字 `.txt`。
    #[test]
    fn double_extension_normalizes_to_basename_txt(
        basename in basename_strategy(),
        ext in ext_strategy(),
        txt in txt_suffix_strategy(),
    ) {
        let input = format!("{basename}.{ext}{txt}");
        let expected = format!("{basename}.txt");

        let result = normalize_caption_name(&input);

        prop_assert_eq!(
            result.as_deref(),
            Some(expected.as_str()),
            "正規化失敗: input={:?} basename={:?} ext={:?} txt={:?}",
            input,
            basename,
            ext,
            txt
        );
    }

    /// 明示的な境界: 大文字小文字を問わない `.txt` サフィックスでも、
    /// 出力は必ず小文字 `.txt` で終わる。
    #[test]
    fn output_always_ends_with_lowercase_txt(
        basename in basename_strategy(),
        ext in ext_strategy(),
        txt in txt_suffix_strategy(),
    ) {
        let input = format!("{basename}.{ext}{txt}");
        let result = normalize_caption_name(&input).expect("二重拡張子は正規化される");
        prop_assert!(
            result.ends_with(".txt") && !result.ends_with(".TXT"),
            "出力が小文字 .txt で終わらない: input={:?} result={:?}",
            input,
            result
        );
    }
}
