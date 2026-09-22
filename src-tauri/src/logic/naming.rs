//! ファイル名正規化・連番・gather/distribute 命名（タスク 5）。
//!
//! ファイル I/O に依存しない決定的な命名規則の純粋関数群。
//! 設計 `RenameService`（要件 8）および `SortService`（要件 7）の
//! 命名ロジックを担う。実ファイルの読み書き・改名は後続の I/O 層
//! （タスク 13 / 14）が本モジュールの関数を呼び出して行う。

/// gather/distribute で用いる接頭辞の区切り文字列。
///
/// gather はサブフォルダ名を `<subfolder><DELIM><filename>` の形で接頭辞
/// 付与し（要件 7.3）、distribute は最初の `<DELIM>` で接頭辞部分と元ファイル名
/// 部分へ分解する（要件 7.4）。両者は逆変換の関係にあり、Property 14
/// （gather → distribute のラウンドトリップ）が成り立つ。
///
/// 単一の `_` はタグ・ファイル名に頻出するため区切りとして曖昧になる。
/// 二重アンダースコア `__` を採用し、通常のファイル名との衝突可能性を下げる。
pub const PREFIX_DELIMITER: &str = "__";

// ---------------------------------------------------------------------------
// タスク 5.1: ファイル名正規化（要件 4.1）
// ---------------------------------------------------------------------------

/// `.<ext>.txt` 形式のファイル名を `<basename>.txt` 形式へ正規化する（要件 4.1）。
///
/// 対応する Image_File の拡張子が二重に付いた Tag_File 名
/// （例: `image.png.txt`）を、拡張子を1つ剥がした形（例: `image.txt`）へ
/// 変換した名前を返す。
///
/// # 正規化の対象
///
/// - 末尾が `.txt`（大文字小文字を区別しない）であること。
/// - `.txt` を除いた残りが、さらに `.<ext>` を持つこと
///   （すなわち basename と拡張子の間にドットがあること）。
/// - `<ext>` 部分が空でないこと。
///
/// これらを満たす場合のみ `Some(<basename>.txt)` を返す。既に
/// `<basename>.txt` 形式であるなど、二重拡張子でない名前は正規化不要と
/// みなし `None` を返す（呼び出し側は改名をスキップする）。
///
/// 出力の拡張子は常に小文字の `.txt` へ揃える。
///
/// # 例
///
/// ```
/// use tag_editor_core::logic::naming::normalize_caption_name;
/// assert_eq!(normalize_caption_name("image.png.txt").as_deref(), Some("image.txt"));
/// assert_eq!(normalize_caption_name("photo.jpeg.txt").as_deref(), Some("photo.txt"));
/// // 既に正規化済み（単一拡張子）は対象外。
/// assert_eq!(normalize_caption_name("image.txt"), None);
/// // .txt 以外は対象外。
/// assert_eq!(normalize_caption_name("image.png"), None);
/// ```
pub fn normalize_caption_name(file_name: &str) -> Option<String> {
    // 末尾 `.txt`（大小無視）を剥がす。
    let stem = strip_txt_suffix(file_name)?;

    // 残りに `.<ext>` があり、かつ ext が空でないことを要求する。
    // 例: "image.png" -> basename="image", ext="png"
    let dot = stem.rfind('.')?;
    let basename = &stem[..dot];
    let ext = &stem[dot + 1..];

    // basename と ext のいずれかが空なら二重拡張子とみなさない。
    // （例: ".txt" -> stem="" は上の rfind で弾かれる。"image." -> ext 空）
    if basename.is_empty() || ext.is_empty() {
        return None;
    }

    Some(format!("{basename}.txt"))
}

/// 末尾の `.txt`（大文字小文字を区別しない）を剥がした残りを返す。
///
/// 末尾が `.txt` でない場合は `None`。
fn strip_txt_suffix(file_name: &str) -> Option<&str> {
    let len = file_name.len();
    if len < 4 {
        return None;
    }
    let (head, tail) = file_name.split_at(len - 4);
    if tail.eq_ignore_ascii_case(".txt") {
        Some(head)
    } else {
        None
    }
}

// ---------------------------------------------------------------------------
// タスク 5.3: 連番付与と NUM プレースホルダ展開（要件 8.1, 8.2）
// ---------------------------------------------------------------------------

/// 連番プレースホルダの文字列。置換文字列中のこの並びが連番へ展開される。
pub const NUM_PLACEHOLDER: &str = "NUM";

/// インデックス `i` に対する連番値を算出する（要件 8.2）。
///
/// ファイル名昇順で並べた対象の `i` 番目（0 起点）に割り当てる番号は
/// `start + i` となる。連続性・一意性・昇順性は呼び出し側が `i` を
/// 0,1,2,… と与えることで保証される（Property 15）。
///
/// # 例
///
/// ```
/// use tag_editor_core::logic::naming::sequence_number;
/// assert_eq!(sequence_number(5, 0), 5);
/// assert_eq!(sequence_number(5, 3), 8);
/// ```
pub fn sequence_number(start: u64, index: u64) -> u64 {
    start + index
}

/// 番号を指定桁数でゼロ埋めした文字列へ整形する（要件 8.2）。
///
/// `width` 桁に満たない場合は先頭を `0` で埋める。番号が `width` 桁を
/// 超える場合はそのままの桁数で出力する（切り詰めない）。
///
/// # 例
///
/// ```
/// use tag_editor_core::logic::naming::format_number;
/// assert_eq!(format_number(7, 3), "007");
/// assert_eq!(format_number(123, 3), "123");
/// assert_eq!(format_number(1234, 3), "1234");
/// assert_eq!(format_number(0, 4), "0000");
/// ```
pub fn format_number(number: u64, width: usize) -> String {
    format!("{number:0width$}")
}

/// 置換文字列中の `NUM` プレースホルダを、ゼロ埋めした連番へ展開する
/// （要件 8.2）。
///
/// `template` 中に現れるすべての `NUM`（[`NUM_PLACEHOLDER`]）を、
/// `format_number(number, width)` の結果へ置換した文字列を返す。
/// プレースホルダが存在しない場合は `template` をそのまま返す。
///
/// # 例
///
/// ```
/// use tag_editor_core::logic::naming::expand_num_placeholder;
/// assert_eq!(expand_num_placeholder("img_NUM", 7, 3), "img_007");
/// assert_eq!(expand_num_placeholder("NUM_NUM", 7, 2), "07_07");
/// assert_eq!(expand_num_placeholder("no_placeholder", 7, 3), "no_placeholder");
/// ```
pub fn expand_num_placeholder(template: &str, number: u64, width: usize) -> String {
    template.replace(NUM_PLACEHOLDER, &format_number(number, width))
}

/// インデックス `i` に対する連番を算出し、置換文字列へ展開する便宜関数。
///
/// [`sequence_number`] と [`expand_num_placeholder`] を合成したもの。
/// ファイル名昇順で `index` 番目（0 起点）の対象に対する最終的な
/// 置換結果（`NUM` 展開済み）を返す。
///
/// # 例
///
/// ```
/// use tag_editor_core::logic::naming::apply_numbering;
/// // start=1, width=3 のとき、0 番目は 001、1 番目は 002。
/// assert_eq!(apply_numbering("img_NUM", 1, 0, 3), "img_001");
/// assert_eq!(apply_numbering("img_NUM", 1, 1, 3), "img_002");
/// ```
pub fn apply_numbering(template: &str, start: u64, index: u64, width: usize) -> String {
    let number = sequence_number(start, index);
    expand_num_placeholder(template, number, width)
}

// ---------------------------------------------------------------------------
// タスク 5.5: gather/distribute の接頭辞命名（要件 7.3, 7.4, 7.7）
// ---------------------------------------------------------------------------

/// gather: サブフォルダ名を接頭辞としてファイル名へ付与する（要件 7.3）。
///
/// `<subfolder><PREFIX_DELIMITER><file_name>` 形式の名前を返す。
/// 生成した名前は [`split_gathered_name`] で元のサブフォルダ名と
/// ファイル名へ分解でき、両者はラウンドトリップの関係にある（Property 14）。
///
/// # 例
///
/// ```
/// use tag_editor_core::logic::naming::gather_name;
/// assert_eq!(gather_name("catA", "001.png"), "catA__001.png");
/// ```
pub fn gather_name(subfolder: &str, file_name: &str) -> String {
    format!("{subfolder}{PREFIX_DELIMITER}{file_name}")
}

/// distribute: 接頭辞付きファイル名を接頭辞部分と元ファイル名部分へ分解する
/// （要件 7.4, 7.7）。
///
/// 最初に現れる [`PREFIX_DELIMITER`] で分割し、`Some((prefix, original))` を
/// 返す。接頭辞部分は宛先サブフォルダ名、元ファイル名部分は配置時の名前に
/// 用いる。
///
/// 以下のいずれかに該当する名前は接頭辞と元ファイル名へ分解できないため
/// `None` を返す（要件 7.7。呼び出し側は当該ファイルを処理対象から除外し
/// 記録する）:
///
/// - 区切り文字 `__` を含まない。
/// - 区切りの左側（接頭辞）が空。
/// - 区切りの右側（元ファイル名）が空。
///
/// # 例
///
/// ```
/// use tag_editor_core::logic::naming::split_gathered_name;
/// assert_eq!(
///     split_gathered_name("catA__001.png"),
///     Some(("catA".to_string(), "001.png".to_string()))
/// );
/// // 区切りなしは分解不能。
/// assert_eq!(split_gathered_name("001.png"), None);
/// ```
pub fn split_gathered_name(file_name: &str) -> Option<(String, String)> {
    let sep = file_name.find(PREFIX_DELIMITER)?;
    let prefix = &file_name[..sep];
    let original = &file_name[sep + PREFIX_DELIMITER.len()..];

    if prefix.is_empty() || original.is_empty() {
        return None;
    }

    Some((prefix.to_string(), original.to_string()))
}

#[cfg(test)]
mod normalize_caption_name_tests {
    use super::*;

    #[test]
    fn double_extension_is_normalized() {
        assert_eq!(
            normalize_caption_name("image.png.txt").as_deref(),
            Some("image.txt")
        );
        assert_eq!(
            normalize_caption_name("photo.jpeg.txt").as_deref(),
            Some("photo.txt")
        );
        assert_eq!(
            normalize_caption_name("clip.mp4.txt").as_deref(),
            Some("clip.txt")
        );
    }

    #[test]
    fn single_extension_is_not_normalized() {
        // 既に <basename>.txt 形式なら対象外。
        assert_eq!(normalize_caption_name("image.txt"), None);
        assert_eq!(normalize_caption_name("notes.txt"), None);
    }

    #[test]
    fn non_txt_is_not_normalized() {
        assert_eq!(normalize_caption_name("image.png"), None);
        assert_eq!(normalize_caption_name("image.png.dat"), None);
        assert_eq!(normalize_caption_name("image"), None);
    }

    #[test]
    fn txt_suffix_is_case_insensitive() {
        // 末尾 .TXT / .Txt も .txt として扱い、小文字 .txt へ揃える。
        assert_eq!(
            normalize_caption_name("image.png.TXT").as_deref(),
            Some("image.txt")
        );
        assert_eq!(
            normalize_caption_name("image.PNG.Txt").as_deref(),
            Some("image.txt")
        );
    }

    #[test]
    fn basename_with_dots_keeps_leading_part() {
        // basename にドットを含む場合、末尾の一段のみを剥がす。
        assert_eq!(
            normalize_caption_name("my.image.png.txt").as_deref(),
            Some("my.image.txt")
        );
    }

    #[test]
    fn empty_basename_or_ext_is_not_normalized() {
        // ".txt" は stem が空 → 対象外。
        assert_eq!(normalize_caption_name(".txt"), None);
        // "image..txt" は ext が空 → 対象外。
        assert_eq!(normalize_caption_name("image..txt"), None);
        // "png.txt" は basename が空でなく ext が "png"… ではなく
        // stem="png" に '.' がない → 対象外（単一拡張子扱い）。
        assert_eq!(normalize_caption_name("png.txt"), None);
    }

    #[test]
    fn empty_input_is_not_normalized() {
        assert_eq!(normalize_caption_name(""), None);
        assert_eq!(normalize_caption_name(".t"), None);
    }

    #[test]
    fn non_ascii_basename_preserved() {
        assert_eq!(
            normalize_caption_name("猫.png.txt").as_deref(),
            Some("猫.txt")
        );
    }
}

#[cfg(test)]
mod numbering_tests {
    use super::*;

    #[test]
    fn sequence_number_is_start_plus_index() {
        assert_eq!(sequence_number(0, 0), 0);
        assert_eq!(sequence_number(5, 0), 5);
        assert_eq!(sequence_number(5, 3), 8);
        assert_eq!(sequence_number(100, 10), 110);
    }

    #[test]
    fn format_number_zero_pads_to_width() {
        assert_eq!(format_number(7, 3), "007");
        assert_eq!(format_number(0, 4), "0000");
        assert_eq!(format_number(42, 5), "00042");
    }

    #[test]
    fn format_number_does_not_truncate_overflow() {
        // width を超える桁はそのまま出力（切り詰めない）。
        assert_eq!(format_number(123, 3), "123");
        assert_eq!(format_number(1234, 3), "1234");
    }

    #[test]
    fn format_number_width_zero() {
        assert_eq!(format_number(0, 0), "0");
        assert_eq!(format_number(9, 0), "9");
    }

    #[test]
    fn expand_num_placeholder_replaces_all() {
        assert_eq!(expand_num_placeholder("img_NUM", 7, 3), "img_007");
        assert_eq!(expand_num_placeholder("NUM_NUM", 7, 2), "07_07");
    }

    #[test]
    fn expand_num_placeholder_without_placeholder() {
        assert_eq!(
            expand_num_placeholder("no_placeholder", 7, 3),
            "no_placeholder"
        );
        assert_eq!(expand_num_placeholder("", 7, 3), "");
    }

    #[test]
    fn apply_numbering_composes_sequence_and_expand() {
        assert_eq!(apply_numbering("img_NUM", 1, 0, 3), "img_001");
        assert_eq!(apply_numbering("img_NUM", 1, 1, 3), "img_002");
        assert_eq!(apply_numbering("frame_NUM", 100, 5, 4), "frame_0105");
    }

    #[test]
    fn apply_numbering_consecutive_and_ascending() {
        // ファイル名昇順で 0..n の index に対し番号が連続・昇順・一意になる。
        let start = 10;
        let width = 3;
        let numbers: Vec<String> = (0..5)
            .map(|i| apply_numbering("NUM", start, i, width))
            .collect();
        assert_eq!(numbers, vec!["010", "011", "012", "013", "014"]);
    }
}

#[cfg(test)]
mod gather_distribute_tests {
    use super::*;

    #[test]
    fn gather_name_prepends_prefix_with_delimiter() {
        assert_eq!(gather_name("catA", "001.png"), "catA__001.png");
        assert_eq!(gather_name("sub", "a.txt"), "sub__a.txt");
    }

    #[test]
    fn split_gathered_name_recovers_parts() {
        assert_eq!(
            split_gathered_name("catA__001.png"),
            Some(("catA".to_string(), "001.png".to_string()))
        );
    }

    #[test]
    fn split_uses_first_delimiter_only() {
        // 元ファイル名に区切りが含まれても、最初の区切りで分解する。
        assert_eq!(
            split_gathered_name("catA__weird__name.png"),
            Some(("catA".to_string(), "weird__name.png".to_string()))
        );
    }

    #[test]
    fn split_returns_none_without_delimiter() {
        // 要件 7.7: 分解不能ケース。
        assert_eq!(split_gathered_name("001.png"), None);
        assert_eq!(split_gathered_name("no_delimiter_here.png"), None);
    }

    #[test]
    fn split_returns_none_for_empty_prefix_or_original() {
        // 接頭辞が空、または元ファイル名が空は分解不能。
        assert_eq!(split_gathered_name("__001.png"), None);
        assert_eq!(split_gathered_name("catA__"), None);
        assert_eq!(split_gathered_name("__"), None);
    }

    #[test]
    fn gather_then_split_roundtrip() {
        // Property 14 の代表例: gather → split で元へ戻る。
        // 元ファイル名側に区切りを含まない前提（サブフォルダ直下の実ファイル名）。
        let cases = [
            ("catA", "001.png"),
            ("sub_folder", "image.jpg"),
            ("猫", "写真.png"),
            ("a", "b"),
        ];
        for (subfolder, file_name) in cases {
            let gathered = gather_name(subfolder, file_name);
            let (prefix, original) =
                split_gathered_name(&gathered).expect("gather した名前は分解できる");
            assert_eq!(prefix, subfolder);
            assert_eq!(original, file_name);
        }
    }
}
