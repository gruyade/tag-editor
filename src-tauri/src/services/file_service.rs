//! FileService の列挙・サムネイル・プレビュー・Tag_File 読み書き。
//!
//! ファイル I/O を伴うサービス層。純粋ロジック（拡張子判定など）を土台に、
//! 実ファイルシステム上の操作を提供する。
//!
//! 本タスク（10.1）では画像列挙を実装する（要件 1.1, 1.2, 1.8）。
//! サムネイル・プレビュー・Tag_File 読み書きは後続タスクで追加する。

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::error::{AppError, AppResult};
use crate::models::ImageEntry;

/// 一覧の件数上限（要件 1.2）。
///
/// 直下の Image_File がこの件数を超える場合、先頭 `MAX_LISTING` 件のみを
/// 対象とし `truncated=true` を立てる。
pub const MAX_LISTING: usize = 10_000;

/// 対応する画像拡張子（小文字表記、要件 1.1）。
///
/// 拡張子の一致判定は大文字小文字を区別しない。mp4 は動画だが Image_File の
/// 対応拡張子として列挙対象に含める（推論時のみ後段で除外、要件 14.8）。
pub const SUPPORTED_EXTENSIONS: [&str; 5] = ["jpg", "jpeg", "png", "gif", "mp4"];

/// 画像列挙の結果（設計 `list_images` の戻り値形状）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImageListing {
    /// 列挙された Image_File（ファイル名昇順、上限適用後）。
    pub items: Vec<ImageEntry>,
    /// 上限（[`MAX_LISTING`]）を超えて切り詰めたか（要件 1.2）。
    pub truncated: bool,
    /// 上限適用前の対応 Image_File の総数。
    pub total: usize,
}

/// 拡張子が対応拡張子のいずれかか判定する（大文字小文字を区別しない、要件 1.1）。
fn has_supported_extension(file_name: &str) -> bool {
    match file_name.rsplit_once('.') {
        Some((_, ext)) => SUPPORTED_EXTENSIONS
            .iter()
            .any(|s| ext.eq_ignore_ascii_case(s)),
        None => false,
    }
}

/// 選択フォルダ直下の Image_File を列挙する（要件 1.1, 1.2, 1.8）。
///
/// - 直下（非再帰）の対応拡張子（jpg / jpeg / png / gif / mp4、大小無視）を持つ
///   ファイルのみを列挙する（要件 1.1）。
/// - ファイル名昇順で安定に整列する。
/// - 件数が [`MAX_LISTING`] を超える場合は先頭 [`MAX_LISTING`] 件を対象とし
///   `truncated=true` を返す（要件 1.2）。`total` には上限適用前の総数を入れる。
/// - 各項目の `has_tag_file` は同名 `.txt`（`<basename>.txt`）の存在で判定する。
///   `thumbnail_available` は既定で true（実サムネイル生成は後続タスク）。
/// - フォルダとして読み取れない（存在しない / ファイルである / 権限不足）場合は
///   `Err(AppError)` を返す（要件 1.8）。空フォルダは空の `items` を返す。
///
/// # 引数
///
/// - `folder`: 走査対象フォルダのパス。
pub fn list_images(folder: impl AsRef<Path>) -> AppResult<ImageListing> {
    let folder = folder.as_ref();

    // フォルダとして読み取れない場合はエラー（要件 1.8）。
    // read_dir はファイル・不存在・権限不足を io::Error として返し、
    // From<io::Error> により適切な AppError 種別へ変換される。
    let read_dir = std::fs::read_dir(folder).map_err(|e| {
        AppError::from(e).with_path(folder.to_string_lossy().into_owned())
    })?;

    // 直下のファイルのみ対象。サブディレクトリ・読み取り不能なエントリは除外。
    let mut file_names: Vec<String> = Vec::new();
    for entry in read_dir {
        let entry = match entry {
            Ok(e) => e,
            // 個別エントリの読み取り失敗はスキップして継続。
            Err(_) => continue,
        };
        // ディレクトリは列挙対象外。file_type 取得失敗もスキップ。
        match entry.file_type() {
            Ok(ft) if ft.is_dir() => continue,
            Ok(_) => {}
            Err(_) => continue,
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        if has_supported_extension(&name) {
            file_names.push(name);
        }
    }

    // 決定的な順序のためファイル名昇順で整列。
    file_names.sort();

    let total = file_names.len();
    let truncated = total > MAX_LISTING;
    if truncated {
        file_names.truncate(MAX_LISTING);
    }

    let items = file_names
        .into_iter()
        .map(|file_name| {
            let path = folder.join(&file_name);
            let has_tag_file = tag_file_path(&path).exists();
            ImageEntry {
                path: path.to_string_lossy().into_owned(),
                file_name,
                has_tag_file,
                thumbnail_available: true,
            }
        })
        .collect();

    Ok(ImageListing {
        items,
        truncated,
        total,
    })
}

/// Image_File のパスから同名 Tag_File（`<basename>.txt`）のパスを導出する。
fn tag_file_path(image_path: &Path) -> std::path::PathBuf {
    image_path.with_extension("txt")
}

#[cfg(test)]
mod list_images_tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    /// 空ファイルを作成するヘルパー。
    fn touch(dir: &Path, name: &str) {
        fs::write(dir.join(name), b"").unwrap();
    }

    #[test]
    fn lists_only_supported_extensions() {
        let dir = tempdir().unwrap();
        touch(dir.path(), "a.jpg");
        touch(dir.path(), "b.jpeg");
        touch(dir.path(), "c.png");
        touch(dir.path(), "d.gif");
        touch(dir.path(), "e.mp4");
        // 対応外拡張子は除外される。
        touch(dir.path(), "f.txt");
        touch(dir.path(), "g.bmp");
        touch(dir.path(), "h.webp");
        touch(dir.path(), "noext");

        let listing = list_images(dir.path()).unwrap();
        let names: Vec<&str> = listing.items.iter().map(|e| e.file_name.as_str()).collect();
        assert_eq!(names, vec!["a.jpg", "b.jpeg", "c.png", "d.gif", "e.mp4"]);
        assert!(!listing.truncated);
        assert_eq!(listing.total, 5);
    }

    #[test]
    fn extension_match_is_case_insensitive() {
        let dir = tempdir().unwrap();
        touch(dir.path(), "A.JPG");
        touch(dir.path(), "B.Png");
        touch(dir.path(), "C.GiF");
        touch(dir.path(), "D.MP4");

        let listing = list_images(dir.path()).unwrap();
        assert_eq!(listing.items.len(), 4);
        assert_eq!(listing.total, 4);
    }

    #[test]
    fn empty_folder_yields_empty_listing() {
        let dir = tempdir().unwrap();
        let listing = list_images(dir.path()).unwrap();
        assert!(listing.items.is_empty());
        assert!(!listing.truncated);
        assert_eq!(listing.total, 0);
    }

    #[test]
    fn subdirectories_are_not_listed() {
        let dir = tempdir().unwrap();
        touch(dir.path(), "a.png");
        // 直下のサブフォルダおよびその中身は列挙されない（非再帰）。
        let sub = dir.path().join("sub");
        fs::create_dir(&sub).unwrap();
        touch(&sub, "nested.png");
        // 対応拡張子に見えるサブフォルダ名も除外される。
        fs::create_dir(dir.path().join("folder.png")).unwrap();

        let listing = list_images(dir.path()).unwrap();
        let names: Vec<&str> = listing.items.iter().map(|e| e.file_name.as_str()).collect();
        assert_eq!(names, vec!["a.png"]);
    }

    #[test]
    fn results_are_sorted_by_file_name() {
        let dir = tempdir().unwrap();
        touch(dir.path(), "c.png");
        touch(dir.path(), "a.png");
        touch(dir.path(), "b.png");

        let listing = list_images(dir.path()).unwrap();
        let names: Vec<&str> = listing.items.iter().map(|e| e.file_name.as_str()).collect();
        assert_eq!(names, vec!["a.png", "b.png", "c.png"]);
    }

    #[test]
    fn has_tag_file_reflects_sibling_txt() {
        let dir = tempdir().unwrap();
        touch(dir.path(), "with.png");
        touch(dir.path(), "with.txt");
        touch(dir.path(), "without.png");

        let listing = list_images(dir.path()).unwrap();
        let with = listing
            .items
            .iter()
            .find(|e| e.file_name == "with.png")
            .unwrap();
        let without = listing
            .items
            .iter()
            .find(|e| e.file_name == "without.png")
            .unwrap();
        assert!(with.has_tag_file);
        assert!(!without.has_tag_file);
    }

    #[test]
    fn unreadable_folder_is_error() {
        let dir = tempdir().unwrap();
        // 存在しないパス。
        let missing = dir.path().join("does-not-exist");
        assert!(list_images(&missing).is_err());

        // ファイルをフォルダとして開こうとするとエラー。
        let file = dir.path().join("file.png");
        fs::write(&file, b"").unwrap();
        assert!(list_images(&file).is_err());
    }
}

// ---------------------------------------------------------------------------
// サムネイル生成・拡大プレビュー（タスク 10.4、要件 1.3, 1.4, 1.5, 1.6）
// ---------------------------------------------------------------------------

use image::ImageEncoder;

/// サムネイル表示サイズの下限（ピクセル、要件 1.4）。
pub const MIN_THUMBNAIL_SIZE: u32 = 64;

/// サムネイル表示サイズの上限（ピクセル、要件 1.4）。
pub const MAX_THUMBNAIL_SIZE: u32 = 512;

/// 拡大プレビューの最大一辺（ピクセル）。
///
/// 元画像がこれより大きい場合はアスペクト比を保って内接するよう縮小する。
/// 元画像がこれ以下の場合は拡大せず元寸法のまま返す。
pub const MAX_PREVIEW_SIZE: u32 = 2048;

/// サムネイルデータ（要件 1.3, 1.4, 1.6）。
///
/// 画像は PNG エンコード済みバイト列（`rgba`→PNG）として保持し、UI 側は
/// そのまま `<img>` の data URL 等で表示できる。読込に失敗した場合は
/// `placeholder=true`（＝ `thumbnail_available=false`）とし、`png` は空になる。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThumbnailData {
    /// 生成されたサムネイルの幅（ピクセル）。placeholder 時は 0。
    pub width: u32,
    /// 生成されたサムネイルの高さ（ピクセル）。placeholder 時は 0。
    pub height: u32,
    /// PNG エンコード済みバイト列。placeholder 時は空。
    pub png: Vec<u8>,
    /// 代替表示フラグ。読込・生成に失敗した場合 true（要件 1.6）。
    pub placeholder: bool,
}

impl ThumbnailData {
    /// 代替表示（プレースホルダ）用の空データを生成する（要件 1.6）。
    fn placeholder() -> Self {
        Self {
            width: 0,
            height: 0,
            png: Vec::new(),
            placeholder: true,
        }
    }
}

/// 拡大プレビューデータ（要件 1.5, 1.6）。
///
/// サムネイルと同様に PNG エンコード済みバイト列で保持する。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PreviewData {
    /// プレビュー画像の幅（ピクセル）。placeholder 時は 0。
    pub width: u32,
    /// プレビュー画像の高さ（ピクセル）。placeholder 時は 0。
    pub height: u32,
    /// PNG エンコード済みバイト列。placeholder 時は空。
    pub png: Vec<u8>,
    /// 代替表示フラグ。読込・生成に失敗した場合 true（要件 1.6）。
    pub placeholder: bool,
}

impl PreviewData {
    /// 代替表示（プレースホルダ）用の空データを生成する（要件 1.6）。
    fn placeholder() -> Self {
        Self {
            width: 0,
            height: 0,
            png: Vec::new(),
            placeholder: true,
        }
    }
}

/// 指定サイズを許容範囲 [`MIN_THUMBNAIL_SIZE`, `MAX_THUMBNAIL_SIZE`] にクランプする
/// （要件 1.4）。
///
/// - 下限未満は [`MIN_THUMBNAIL_SIZE`] に、
/// - 上限超は [`MAX_THUMBNAIL_SIZE`] に、
/// - 範囲内はそのまま返す。
pub fn clamp_thumbnail_size(size: u32) -> u32 {
    size.clamp(MIN_THUMBNAIL_SIZE, MAX_THUMBNAIL_SIZE)
}

/// RGBA 画像を PNG バイト列へエンコードする内部ヘルパー。
fn encode_png(img: &image::RgbaImage) -> Result<Vec<u8>, image::ImageError> {
    let mut buf: Vec<u8> = Vec::new();
    let encoder = image::codecs::png::PngEncoder::new(&mut buf);
    encoder.write_image(
        img.as_raw(),
        img.width(),
        img.height(),
        image::ExtendedColorType::Rgba8,
    )?;
    Ok(buf)
}

/// 指定 Image_File のサムネイルを生成する（要件 1.3, 1.4, 1.6）。
///
/// - `size` は [`clamp_thumbnail_size`] で 64〜512 にクランプされる（要件 1.4）。
/// - 画像を読み込み、アスペクト比を保ったまま一辺 `size` に内接するよう縮小する
///   （要件 1.3）。拡大はしない（元がクランプ後サイズ以下ならそのまま）。
/// - 読み込み・デコードに失敗した場合はエラーにせず、`placeholder=true` の
///   [`ThumbnailData`] を返して一覧表示の継続を可能にする（要件 1.6）。
///
/// # 引数
///
/// - `path`: 対象 Image_File のパス。
/// - `size`: 希望する表示サイズ（クランプ対象）。
pub fn get_thumbnail(path: impl AsRef<Path>, size: u32) -> ThumbnailData {
    let size = clamp_thumbnail_size(size);
    let path = path.as_ref();

    let img = match image::open(path) {
        Ok(img) => img,
        // 破損等で読み込めない場合は代替表示（要件 1.6）。
        Err(_) => return ThumbnailData::placeholder(),
    };

    // アスペクト比を保って size×size に内接させる（拡大はしない）。
    let (w, h) = (img.width(), img.height());
    let thumb = if w <= size && h <= size {
        img.to_rgba8()
    } else {
        img.thumbnail(size, size).to_rgba8()
    };

    match encode_png(&thumb) {
        Ok(png) => ThumbnailData {
            width: thumb.width(),
            height: thumb.height(),
            png,
            placeholder: false,
        },
        // エンコードに失敗した場合も代替表示にフォールバック（要件 1.6）。
        Err(_) => ThumbnailData::placeholder(),
    }
}

/// 指定 Image_File の拡大プレビューを生成する（要件 1.5, 1.6）。
///
/// - 画像を読み込み、一辺が [`MAX_PREVIEW_SIZE`] を超える場合のみアスペクト比を
///   保って縮小する。それ以下なら元寸法のまま返す。
/// - 読み込み・デコードに失敗した場合はエラーにせず、`placeholder=true` の
///   [`PreviewData`] を返す（要件 1.6）。
///
/// # 引数
///
/// - `path`: 対象 Image_File のパス。
pub fn get_preview(path: impl AsRef<Path>) -> PreviewData {
    let path = path.as_ref();

    let img = match image::open(path) {
        Ok(img) => img,
        Err(_) => return PreviewData::placeholder(),
    };

    let (w, h) = (img.width(), img.height());
    let preview = if w <= MAX_PREVIEW_SIZE && h <= MAX_PREVIEW_SIZE {
        img.to_rgba8()
    } else {
        img.thumbnail(MAX_PREVIEW_SIZE, MAX_PREVIEW_SIZE).to_rgba8()
    };

    match encode_png(&preview) {
        Ok(png) => PreviewData {
            width: preview.width(),
            height: preview.height(),
            png,
            placeholder: false,
        },
        Err(_) => PreviewData::placeholder(),
    }
}

#[cfg(test)]
mod thumbnail_tests {
    use super::*;
    use std::path::PathBuf;
    use tempfile::tempdir;

    /// 指定寸法の PNG 画像をテンポラリに書き出しパスを返すヘルパー。
    fn write_png(dir: &Path, name: &str, w: u32, h: u32) -> PathBuf {
        let img = image::RgbaImage::from_pixel(w, h, image::Rgba([120, 60, 200, 255]));
        let path = dir.join(name);
        img.save(&path).unwrap();
        path
    }

    #[test]
    fn clamp_below_min_yields_min() {
        assert_eq!(clamp_thumbnail_size(0), MIN_THUMBNAIL_SIZE);
        assert_eq!(clamp_thumbnail_size(63), MIN_THUMBNAIL_SIZE);
    }

    #[test]
    fn clamp_above_max_yields_max() {
        assert_eq!(clamp_thumbnail_size(513), MAX_THUMBNAIL_SIZE);
        assert_eq!(clamp_thumbnail_size(u32::MAX), MAX_THUMBNAIL_SIZE);
    }

    #[test]
    fn clamp_in_range_is_unchanged() {
        assert_eq!(clamp_thumbnail_size(64), 64);
        assert_eq!(clamp_thumbnail_size(200), 200);
        assert_eq!(clamp_thumbnail_size(512), 512);
    }

    #[test]
    fn thumbnail_fits_within_clamped_size_preserving_aspect() {
        let dir = tempdir().unwrap();
        // 横長画像 800x400 を size=200 に縮小 → 200x100（アスペクト比保持）。
        let path = write_png(dir.path(), "wide.png", 800, 400);

        let thumb = get_thumbnail(&path, 200);
        assert!(!thumb.placeholder);
        assert!(!thumb.png.is_empty());
        assert!(thumb.width <= 200 && thumb.height <= 200);
        assert_eq!(thumb.width, 200);
        assert_eq!(thumb.height, 100);
    }

    #[test]
    fn thumbnail_uses_clamped_size_for_generation() {
        let dir = tempdir().unwrap();
        // 2000x2000 の画像に size=10（<64）を指定 → 64 にクランプされ 64x64。
        let path = write_png(dir.path(), "big.png", 2000, 2000);

        let thumb = get_thumbnail(&path, 10);
        assert!(!thumb.placeholder);
        assert_eq!(thumb.width, MIN_THUMBNAIL_SIZE);
        assert_eq!(thumb.height, MIN_THUMBNAIL_SIZE);
    }

    #[test]
    fn thumbnail_does_not_upscale_small_image() {
        let dir = tempdir().unwrap();
        // 32x16 の小画像に size=256 → 拡大せず 32x16 のまま。
        let path = write_png(dir.path(), "small.png", 32, 16);

        let thumb = get_thumbnail(&path, 256);
        assert!(!thumb.placeholder);
        assert_eq!(thumb.width, 32);
        assert_eq!(thumb.height, 16);
    }

    #[test]
    fn corrupt_file_yields_placeholder() {
        let dir = tempdir().unwrap();
        // 画像として解釈できない内容の .png。
        let path = dir.path().join("broken.png");
        std::fs::write(&path, b"this is not an image").unwrap();

        let thumb = get_thumbnail(&path, 128);
        assert!(thumb.placeholder);
        assert!(thumb.png.is_empty());
        assert_eq!(thumb.width, 0);
        assert_eq!(thumb.height, 0);
    }

    #[test]
    fn missing_file_yields_placeholder() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("does-not-exist.png");
        let thumb = get_thumbnail(&path, 128);
        assert!(thumb.placeholder);
    }

    #[test]
    fn preview_returns_data_for_valid_image() {
        let dir = tempdir().unwrap();
        let path = write_png(dir.path(), "p.png", 640, 480);

        let preview = get_preview(&path);
        assert!(!preview.placeholder);
        assert!(!preview.png.is_empty());
        // MAX_PREVIEW_SIZE 以下なので元寸法のまま。
        assert_eq!(preview.width, 640);
        assert_eq!(preview.height, 480);
    }

    #[test]
    fn preview_downscales_oversized_image() {
        let dir = tempdir().unwrap();
        // 一辺が MAX_PREVIEW_SIZE 超 → アスペクト比保持で内接縮小。
        let path = write_png(dir.path(), "huge.png", 4096, 2048);

        let preview = get_preview(&path);
        assert!(!preview.placeholder);
        assert!(preview.width <= MAX_PREVIEW_SIZE && preview.height <= MAX_PREVIEW_SIZE);
        assert_eq!(preview.width, MAX_PREVIEW_SIZE);
        assert_eq!(preview.height, MAX_PREVIEW_SIZE / 2);
    }

    #[test]
    fn preview_corrupt_file_yields_placeholder() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("broken.png");
        std::fs::write(&path, b"nope").unwrap();

        let preview = get_preview(&path);
        assert!(preview.placeholder);
        assert!(preview.png.is_empty());
    }
}

#[cfg(test)]
mod corrupt_image_resilience_tests {
    //! 破損画像の代替表示・列挙継続の単体テスト（タスク 10.6、要件 1.6）。
    //!
    //! 個別 Image_File の読込失敗が他項目へ波及しないこと（代替表示にしつつ
    //! 残りの表示を継続すること）を、サムネイル生成と一覧列挙の両面から検証する。

    use super::*;
    use std::fs;
    use std::path::PathBuf;
    use tempfile::tempdir;

    /// 指定寸法の有効な PNG 画像を書き出しパスを返すヘルパー。
    fn write_valid_png(dir: &Path, name: &str, w: u32, h: u32) -> PathBuf {
        let img = image::RgbaImage::from_pixel(w, h, image::Rgba([30, 200, 90, 255]));
        let path = dir.join(name);
        img.save(&path).unwrap();
        path
    }

    /// 拡張子は対応拡張子だが中身が画像として不正なファイルを書き出す。
    fn write_corrupt(dir: &Path, name: &str) -> PathBuf {
        let path = dir.join(name);
        fs::write(&path, b"this is definitely not a decodable image").unwrap();
        path
    }

    #[test]
    fn broken_item_is_placeholder_while_sibling_valid_image_still_thumbnails() {
        // 同一フォルダに破損画像と有効画像を混在させ、
        // 破損側は代替表示（placeholder=true）、有効側は実サムネイルになることを確認。
        let dir = tempdir().unwrap();
        let broken = write_corrupt(dir.path(), "broken.png");
        let valid = write_valid_png(dir.path(), "valid.png", 300, 150);

        // 破損項目 → 代替表示。他項目の処理に依存せず単独で degrade する。
        let broken_thumb = get_thumbnail(&broken, 128);
        assert!(broken_thumb.placeholder);
        assert!(broken_thumb.png.is_empty());
        assert_eq!(broken_thumb.width, 0);
        assert_eq!(broken_thumb.height, 0);

        // 同じフォルダの有効画像 → 実サムネイル（破損項目の存在に影響されない）。
        let valid_thumb = get_thumbnail(&valid, 128);
        assert!(!valid_thumb.placeholder);
        assert!(!valid_thumb.png.is_empty());
        assert!(valid_thumb.width <= 128 && valid_thumb.height <= 128);
        // 300x150 を 128 内接 → 128x64（アスペクト比保持）。
        assert_eq!(valid_thumb.width, 128);
        assert_eq!(valid_thumb.height, 64);
    }

    #[test]
    fn iterating_a_mixed_folder_degrades_only_broken_items() {
        // 一覧走査を模し、複数の破損／有効画像を順に処理しても
        // 破損項目のみが placeholder になり、列挙処理自体は継続する。
        let dir = tempdir().unwrap();
        write_valid_png(dir.path(), "a.png", 100, 100);
        write_corrupt(dir.path(), "b.png");
        write_valid_png(dir.path(), "c.png", 120, 60);
        write_corrupt(dir.path(), "d.png");

        let listing = list_images(dir.path()).unwrap();
        assert_eq!(listing.items.len(), 4);

        let mut placeholders = 0usize;
        let mut real = 0usize;
        for item in &listing.items {
            let thumb = get_thumbnail(&item.path, 128);
            if thumb.placeholder {
                placeholders += 1;
            } else {
                real += 1;
                assert!(!thumb.png.is_empty());
            }
        }
        // 破損 2 件が代替表示、有効 2 件が実サムネイル。
        assert_eq!(placeholders, 2);
        assert_eq!(real, 2);
    }

    #[test]
    fn list_images_enumerates_corrupt_files_by_extension() {
        // 一覧列挙は拡張子で判定するため、破損ファイルも列挙対象に含まれる
        // （サムネイル生成のみが degrade する）。有効・破損の混在でも全件列挙。
        let dir = tempdir().unwrap();
        write_valid_png(dir.path(), "good.png", 64, 64);
        write_corrupt(dir.path(), "bad.jpg");
        write_corrupt(dir.path(), "empty.gif");
        write_valid_png(dir.path(), "ok.png", 64, 64);

        let listing = list_images(dir.path()).unwrap();
        let names: Vec<&str> = listing.items.iter().map(|e| e.file_name.as_str()).collect();
        // ファイル名昇順で全 4 件（破損含む）が列挙される。
        assert_eq!(names, vec!["bad.jpg", "empty.gif", "good.png", "ok.png"]);
        assert_eq!(listing.total, 4);
        assert!(!listing.truncated);
    }
}
