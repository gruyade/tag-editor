//! TagService の一括操作（追加・削除・重複除去）のファイル I/O 結線（タスク 12.1）。
//!
//! 純粋ロジック（[`crate::logic::tag_ops`]）の [`add_tags`]/[`remove_tags`]/
//! [`dedup_tags`] を、Tag_File の読み書き（[`crate::services::tag_file`]）と
//! 接続する副作用ありのサービス層。
//!
//! # 処理の流れ（各対象 Image_File につき）
//!
//! 1. 対応する Tag_File を読み込む（[`tag_file::read_tag_file`]）。無ければ
//!    既存内容を空として扱う（要件 3.6）。
//! 2. 読み込んだ生内容を [`tag_ops::split_tokens`] でトークン列へ分割する。
//! 3. コアロジック（add/remove/dedup）を適用する。
//! 4. 結果を `", "` 区切りの 1 行へ描画し、[`tag_file::write_tag_file`] で
//!    書き戻す。Tag_File が無ければ新規作成される（要件 3.6）。
//!
//! # 部分失敗の扱い（要件 3.7）
//!
//! 個別 Image_File の読み込み／書き込みに失敗した場合、その対象をスキップして
//! 残りの処理を継続し、失敗件数を [`OperationReport::skipped`] に加算しつつ
//! メッセージへ記録する。全体を中断しない。
//!
//! # 対象 0 件の扱い（要件 3.8）
//!
//! 対象 Image_File が 1 件も指定されていない場合、いかなるファイルも変更せず、
//! 「対象未選択」である旨のメッセージのみを持つ [`OperationReport`] を返す
//! （成功・スキップ・衝突すべて 0）。呼び出し側（UI）はこのメッセージを提示する。

use std::path::Path;

use crate::logic::tag_ops;
use crate::models::OperationReport;
use crate::services::tag_file;

/// 対象 0 件のときにレポートへ載せる通知メッセージ（要件 3.8）。
const NO_TARGETS_MESSAGE: &str = "対象の画像が1件も選択されていないため、操作を実行しませんでした";

/// トークン列を Tag_File の内容（`", "` 区切りの 1 行）へ描画する。
///
/// 空列は空文字列となり、書き込むと空の Tag_File が作成される。
fn render_content(tokens: &[String]) -> String {
    tokens.join(", ")
}

/// 各対象 Image_File に対して 1 つの Tag_File 変換処理を適用する共通ドライバ。
///
/// `transform` は「Tag_File の既存トークン列」を受け取り「書き戻すトークン列」を
/// 返すクロージャ。読み込み・描画・書き込み・部分失敗の記録・対象 0 件の扱いを
/// この関数へ集約する（要件 3.6, 3.7, 3.8）。
fn run_bulk<P, F>(targets: &[P], mut transform: F) -> OperationReport
where
    P: AsRef<Path>,
    F: FnMut(Vec<String>) -> Vec<String>,
{
    let mut report = OperationReport::default();

    // 対象 0 件は未実行＋通知（要件 3.8）。
    if targets.is_empty() {
        report.messages.push(NO_TARGETS_MESSAGE.to_string());
        return report;
    }

    for target in targets {
        let path = target.as_ref();

        // 1. Tag_File を読み込む。無ければ空内容として扱う（要件 3.6）。
        //    読み込み自体が I/O エラー（権限不足等）で失敗した場合は
        //    当該対象をスキップし、件数とメッセージへ記録する（要件 3.7）。
        let existing = match tag_file::read_tag_file(path) {
            Ok(content) => content.content,
            Err(e) => {
                report.skipped += 1;
                report.messages.push(format!(
                    "{} の読み込みに失敗したためスキップしました: {}",
                    path.display(),
                    e
                ));
                continue;
            }
        };

        // 2. トークン化 → 3. コアロジック適用。
        let tokens = tag_ops::split_tokens(&existing);
        let next = transform(tokens);

        // 4. 描画して書き戻す。無ければ新規作成される（要件 3.6）。
        let content = render_content(&next);
        match tag_file::write_tag_file(path, &content) {
            Ok(()) => report.succeeded += 1,
            Err(e) => {
                // 個別書き込み失敗はスキップし件数記録（要件 3.7）。
                report.skipped += 1;
                report.messages.push(format!(
                    "{} の書き込みに失敗したためスキップしました: {}",
                    path.display(),
                    e
                ));
            }
        }
    }

    report
}

/// 複数の Image_File へタグ群を一括追加する（要件 3.1, 3.5, 3.6, 3.7, 3.8, 12）。
///
/// 各対象の Tag_File を読み込み、既存タグへ `tags` を [`tag_ops::add_tags`] で
/// 追加して書き戻す。追加は包含的・非重複（正規化キー一致は増やさない）・冪等。
/// Tag_File が無い対象は空から開始し、新規作成する（要件 3.6）。
///
/// # 引数
/// - `targets`: 対象 Image_File のパス列。空なら未実行＋通知（要件 3.8）。
/// - `tags`: 追加するタグ本体の列。
pub fn bulk_add_tags<P: AsRef<Path>>(targets: &[P], tags: &[String]) -> OperationReport {
    run_bulk(targets, |existing| tag_ops::add_tags(&existing, tags))
}

/// 複数の Image_File から指定タグ群を一括削除する（要件 3.3, 3.6, 3.7, 3.8）。
///
/// 各対象の Tag_File を読み込み、正規化キーが `tags` のいずれかと一致するタグを
/// [`tag_ops::remove_tags`] で全除去して書き戻す。Tag_File が無い対象は空から
/// 開始する（削除対象なしで実質空のまま新規作成される、要件 3.6）。
///
/// # 引数
/// - `targets`: 対象 Image_File のパス列。空なら未実行＋通知（要件 3.8）。
/// - `tags`: 削除するタグ本体の列。
pub fn bulk_remove_tags<P: AsRef<Path>>(targets: &[P], tags: &[String]) -> OperationReport {
    run_bulk(targets, |existing| tag_ops::remove_tags(&existing, tags))
}

/// 複数の Image_File のタグから重複を一括除去する（要件 3.4, 3.6, 3.7, 3.8）。
///
/// 各対象の Tag_File を読み込み、[`tag_ops::dedup_tags`] で初出を残し以降の
/// 重複（正規化キー一致）を除去して書き戻す。初出順序を保存し冪等。
///
/// # 引数
/// - `targets`: 対象 Image_File のパス列。空なら未実行＋通知（要件 3.8）。
pub fn dedup_tags<P: AsRef<Path>>(targets: &[P]) -> OperationReport {
    run_bulk(targets, |existing| tag_ops::dedup_tags(&existing))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    /// 指定 Image_File パスに対応する Tag_File（`<basename>.txt`）を読み出す。
    fn read_raw(image: &Path) -> String {
        fs::read_to_string(image.with_extension("txt")).unwrap()
    }

    #[test]
    fn add_creates_tag_file_when_absent() {
        let dir = tempdir().unwrap();
        let image = dir.path().join("a.png");
        // Tag_File は作成しない。
        assert!(!image.with_extension("txt").exists());

        let report = bulk_add_tags(&[&image], &["1girl".to_string(), "solo".to_string()]);

        assert_eq!(report.succeeded, 1);
        assert_eq!(report.skipped, 0);
        // 新規作成された Tag_File に追加タグが入っている（要件 3.6）。
        assert!(image.with_extension("txt").exists());
        assert_eq!(read_raw(&image), "1girl, solo");
    }

    #[test]
    fn add_is_non_duplicating() {
        let dir = tempdir().unwrap();
        let image = dir.path().join("b.png");
        fs::write(image.with_extension("txt"), "1girl, Solo").unwrap();

        // 既存と正規化キーが一致する " solo " は増やさず、新規 "long hair" のみ追加。
        let report = bulk_add_tags(&[&image], &[" solo ".to_string(), "long hair".to_string()]);

        assert_eq!(report.succeeded, 1);
        assert_eq!(read_raw(&image), "1girl, Solo, long hair");
    }

    #[test]
    fn remove_removes_matching() {
        let dir = tempdir().unwrap();
        let image = dir.path().join("c.png");
        fs::write(image.with_extension("txt"), "1girl, Solo, solo").unwrap();

        // 正規化キー一致する全出現を除去（要件 3.3）。
        let report = bulk_remove_tags(&[&image], &["SOLO".to_string()]);

        assert_eq!(report.succeeded, 1);
        assert_eq!(read_raw(&image), "1girl");
    }

    #[test]
    fn dedup_dedups() {
        let dir = tempdir().unwrap();
        let image = dir.path().join("d.png");
        fs::write(image.with_extension("txt"), "Solo, 1girl, solo").unwrap();

        // 初出を残し以降の重複を除去（要件 3.4）。
        let report = dedup_tags(&[&image]);

        assert_eq!(report.succeeded, 1);
        assert_eq!(read_raw(&image), "Solo, 1girl");
    }

    #[test]
    fn empty_targets_is_noop_with_notice() {
        // 対象 0 件は未実行＋通知（要件 3.8）。ファイルは一切作らない。
        let empty: [&Path; 0] = [];
        let report = bulk_add_tags(&empty, &["solo".to_string()]);

        assert_eq!(report.succeeded, 0);
        assert_eq!(report.skipped, 0);
        assert_eq!(report.conflicted, 0);
        assert_eq!(report.messages.len(), 1);
        assert!(report.messages[0].contains("対象"));
    }

    #[test]
    fn partial_failure_is_skipped_and_counted() {
        // 1件は正常な対象、もう1件は書き込めないパス（存在しないディレクトリ配下）。
        let dir = tempdir().unwrap();
        let ok_image = dir.path().join("ok.png");
        let bad_image = dir.path().join("no_such_dir").join("bad.png");

        let report = bulk_add_tags(&[&ok_image, &bad_image], &["solo".to_string()]);

        // 正常分は成功、書き込み不可分はスキップして継続（要件 3.7）。
        assert_eq!(report.succeeded, 1);
        assert_eq!(report.skipped, 1);
        assert_eq!(read_raw(&ok_image), "solo");
        assert!(report.messages.iter().any(|m| m.contains("bad.png")));
    }

    // -------------------------------------------------------------------
    // 要件 3.7 の網羅補強: 部分失敗が発生しても残り対象は処理継続し、
    // succeeded は成功対象数・skipped は失敗対象数と一致すること。
    // 失敗は「存在しないディレクトリ配下」を書込先にすることで誘発する。
    // add / remove / dedup すべてで検証する。
    // -------------------------------------------------------------------

    /// 追加: 失敗対象を挟んでも前後の正常対象が両方成功し、件数が一致する（要件 3.7）。
    #[test]
    fn add_continues_after_failure_and_counts_match() {
        let dir = tempdir().unwrap();
        let ok1 = dir.path().join("ok1.png");
        let bad = dir.path().join("missing").join("bad.png");
        let ok2 = dir.path().join("ok2.png");
        // ok2 は既存内容ありから開始（追加が既存へ効くことも同時に確認）。
        fs::write(ok2.with_extension("txt"), "1girl").unwrap();

        // 失敗対象を先頭でも末尾でもなく「間」に挟む。
        let report = bulk_add_tags(&[&ok1, &bad, &ok2], &["solo".to_string()]);

        // 正常 2 件は成功、失敗 1 件はスキップ。件数は対象数と一致。
        assert_eq!(report.succeeded, 2);
        assert_eq!(report.skipped, 1);
        assert_eq!(report.conflicted, 0);
        // 失敗を挟んだ後の ok2 も確かに処理されている。
        assert_eq!(read_raw(&ok1), "solo");
        assert_eq!(read_raw(&ok2), "1girl, solo");
        // スキップ理由メッセージに失敗対象が含まれる。
        assert!(report.messages.iter().any(|m| m.contains("bad.png")));
        // 失敗件数とメッセージ件数が対応（この呼び出しでは失敗のみメッセージ化）。
        assert_eq!(report.messages.len(), report.skipped);
    }

    /// 削除: 複数の失敗対象があってもそれぞれスキップし、成功対象は処理継続（要件 3.7）。
    #[test]
    fn remove_continues_after_multiple_failures_and_counts_match() {
        let dir = tempdir().unwrap();
        let ok = dir.path().join("ok.png");
        fs::write(ok.with_extension("txt"), "1girl, solo, dog").unwrap();
        // 2 件とも書込不能（別々の存在しないディレクトリ配下）。
        let bad1 = dir.path().join("nodir1").join("bad1.png");
        let bad2 = dir.path().join("nodir2").join("bad2.png");

        let report = bulk_remove_tags(&[&bad1, &ok, &bad2], &["solo".to_string()]);

        // 成功 1・スキップ 2。スキップ数は失敗対象数と一致。
        assert_eq!(report.succeeded, 1);
        assert_eq!(report.skipped, 2);
        assert_eq!(report.conflicted, 0);
        // 成功対象は削除が適用されている。
        assert_eq!(read_raw(&ok), "1girl, dog");
        // 失敗した 2 対象それぞれのメッセージが記録される。
        assert!(report.messages.iter().any(|m| m.contains("bad1.png")));
        assert!(report.messages.iter().any(|m| m.contains("bad2.png")));
        assert_eq!(report.messages.len(), report.skipped);
    }

    /// 重複除去: 失敗対象を挟んでも正常対象の dedup は継続適用される（要件 3.7）。
    #[test]
    fn dedup_continues_after_failure_and_counts_match() {
        let dir = tempdir().unwrap();
        let ok = dir.path().join("ok.png");
        fs::write(ok.with_extension("txt"), "Solo, 1girl, solo").unwrap();
        let bad = dir.path().join("missing").join("bad.png");

        let report = dedup_tags(&[&ok, &bad]);

        assert_eq!(report.succeeded, 1);
        assert_eq!(report.skipped, 1);
        assert_eq!(report.conflicted, 0);
        // 初出を残し以降の重複を除去（要件 3.4）が成功対象に適用される。
        assert_eq!(read_raw(&ok), "Solo, 1girl");
        assert!(report.messages.iter().any(|m| m.contains("bad.png")));
        assert_eq!(report.messages.len(), report.skipped);
    }

    // -------------------------------------------------------------------
    // 要件 3.8 の網羅補強: 対象 0 件では一切のファイルを作成/変更せず、
    // 通知メッセージのみを返す。remove / dedup でも同様であること。
    // -------------------------------------------------------------------

    /// 削除: 対象 0 件は未実行＋通知、ファイルは作られない（要件 3.8）。
    #[test]
    fn remove_empty_targets_is_noop_with_notice() {
        let dir = tempdir().unwrap();
        let empty: [&Path; 0] = [];

        let report = bulk_remove_tags(&empty, &["solo".to_string()]);

        assert_eq!(report.succeeded, 0);
        assert_eq!(report.skipped, 0);
        assert_eq!(report.conflicted, 0);
        assert_eq!(report.messages.len(), 1);
        assert!(report.messages[0].contains("対象"));
        // ディレクトリ配下に .txt が 1 つも作られていない。
        assert!(!has_any_txt(dir.path()));
    }

    /// 重複除去: 対象 0 件は未実行＋通知、ファイルは作られない（要件 3.8）。
    #[test]
    fn dedup_empty_targets_is_noop_with_notice() {
        let dir = tempdir().unwrap();
        let empty: [&Path; 0] = [];

        let report = dedup_tags(&empty);

        assert_eq!(report.succeeded, 0);
        assert_eq!(report.skipped, 0);
        assert_eq!(report.conflicted, 0);
        assert_eq!(report.messages.len(), 1);
        assert!(report.messages[0].contains("対象"));
        assert!(!has_any_txt(dir.path()));
    }

    /// 追加: 対象 0 件では既存 Tag_File も変更されない（要件 3.8）。
    #[test]
    fn empty_targets_does_not_modify_existing_files() {
        let dir = tempdir().unwrap();
        // 事前に存在する Tag_File。対象未指定なので触れられてはならない。
        let existing = dir.path().join("keep.png");
        fs::write(existing.with_extension("txt"), "1girl, solo").unwrap();
        let empty: [&Path; 0] = [];

        let report = bulk_add_tags(&empty, &["new".to_string()]);

        assert_eq!(report.succeeded, 0);
        assert_eq!(report.skipped, 0);
        // 既存内容は不変。
        assert_eq!(read_raw(&existing), "1girl, solo");
        assert_eq!(report.messages.len(), 1);
        assert!(report.messages[0].contains("対象"));
    }

    /// 指定ディレクトリ直下に `.txt` ファイルが 1 つでも存在するか。
    fn has_any_txt(dir: &Path) -> bool {
        fs::read_dir(dir).unwrap().any(|e| {
            e.unwrap()
                .path()
                .extension()
                .map(|ext| ext == "txt")
                .unwrap_or(false)
        })
    }
}

// ---------------------------------------------------------------------------
// タグ集計・フィルタのファイル走査結線（タスク 12.3、要件 6.1〜6.8）
// ---------------------------------------------------------------------------
//
// 純粋ロジック（[`crate::logic::tag_stats`]）の集計・フィルタ述語を、フォルダ
// 走査（[`crate::services::file_service::list_images`]）と Tag_File 読み込み
// （[`crate::services::tag_file`]）へ接続する副作用ありのサービス層。
//
// # Tag_File の走査方針（要件 6.1）
//
// 本プロジェクトでは Tag_File は Image_File と同名の `<basename>.txt` として
// 定義される（[`crate::services::tag_file`]）。そのため「全 Tag_File を走査する」
// ことは「フォルダ直下の全 Image_File を列挙し、それぞれに対応する Tag_File を
// 読む」ことと等価である。孤立 Tag_File（対応 Image を持たない `.txt`）は集計・
// フィルタの対象外とする（それらはタスク 13.5 の孤立キャプション処理の関心事）。
// この方針により、列挙規則が一覧表示（要件 1.1）と一貫し、対象画像の集合が
// 集計・フィルタ間でも一致する。

use crate::error::AppResult;
use crate::logic::tag_stats;
use crate::models::{ImageEntry, TagCount};
use crate::services::file_service;

/// タグ集計の結果（タスク 12.3、要件 6.1, 6.2）。
///
/// Tauri コマンド境界（`aggregate_tags`）の戻り値として JSON DTO で返すため
/// serde 対応する。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct TagAggregation {
    /// 集計済みタグ（出現回数降順・同数はタグ名昇順、要件 6.1, 6.3）。
    pub tags: Vec<TagCount>,
    /// 読み込みに失敗し集計から除外した Tag_File の件数（要件 6.2）。
    pub unreadable: usize,
}

/// フォルダ直下の全 Tag_File を走査してタグ出現回数を集計する（要件 6.1〜6.4）。
///
/// フォルダ直下の Image_File を列挙し（[`file_service::list_images`]）、各対応
/// Tag_File を読み込み、[`tag_ops::split_tokens`] でトークン化して
/// [`tag_stats::aggregate_tags`] へ投入する。
///
/// - 読み込みに失敗した Tag_File はその件数を数え、集計対象から除外する
///   （要件 6.2）。返り値の [`TagAggregation::unreadable`] に反映する。
/// - 対応 Tag_File が存在しない Image_File は「タグ無し（空）」として扱い、
///   読込失敗にはカウントしない（存在しないことは失敗ではない）。
/// - 結果は出現回数降順・同数はタグ名昇順で整列する（要件 6.3、
///   [`tag_stats::aggregate_tags`] に準拠）。
/// - 有効なタグが 1 件も無い場合は空リストを返す（要件 6.4）。
///
/// フォルダ自体が読み取れない場合（不存在・権限不足等）は
/// [`file_service::list_images`] のエラーをそのまま伝播する（要件 1.8）。
///
/// # 引数
/// - `folder`: 走査対象フォルダのパス。
pub fn aggregate_tags(folder: impl AsRef<Path>) -> AppResult<TagAggregation> {
    let listing = file_service::list_images(folder)?;

    let mut unreadable = 0usize;
    // 各 Tag_File のトークン列を集めてから集計へ渡す。
    let mut per_file_tokens: Vec<Vec<String>> = Vec::with_capacity(listing.items.len());

    for item in &listing.items {
        let path = Path::new(&item.path);
        match tag_file::read_tag_file(path) {
            // 存在有無に関わらず、読めた内容をトークン化して集計対象へ。
            // 未存在は空内容（exists=false, content=""）でありタグ 0 件として扱う。
            Ok(content) => {
                per_file_tokens.push(tag_ops::split_tokens(&content.content));
            }
            // 読み込み失敗（権限不足等）は除外し件数を記録（要件 6.2）。
            Err(_) => {
                unreadable += 1;
            }
        }
    }

    // 純粋ロジックの集計（module 修飾で本関数と区別）。
    let tags = tag_stats::aggregate_tags(per_file_tokens);

    Ok(TagAggregation { tags, unreadable })
}

/// フォルダ直下の Image_File を include/exclude 条件で絞り込む（要件 6.5〜6.8）。
///
/// フォルダ直下の Image_File を列挙し（[`file_service::list_images`]）、各対応
/// Tag_File を読み込み、[`tag_ops::split_tokens`] でトークン化して
/// [`tag_stats::matches_filter`] を適用する。条件に一致した [`ImageEntry`] のみを
/// 元の列挙順（ファイル名昇順）で返す。
///
/// - `include` をすべて含み、かつ `exclude` をいずれも含まない画像のみ残す
///   （要件 6.5, 6.6, 6.7）。判定は正規化キー（前後トリム＋大文字小文字無視）。
/// - `include`・`exclude` がともに空なら全件を返す（要件 6.8）。
/// - Tag_File が存在しない画像はタグ無し（空）として述語に掛ける。したがって
///   `include` 指定時は除外され、`exclude` のみ指定時は残る。
/// - Tag_File の読み込みに失敗した画像は結果から除外する（フィルタ結果として
///   確定できないため保守的に落とす）。
///
/// フォルダ自体が読み取れない場合は [`file_service::list_images`] のエラーを
/// そのまま伝播する（要件 1.8）。
///
/// # 引数
/// - `folder`: 走査対象フォルダのパス。
/// - `include`: 包含タグ列（空可）。
/// - `exclude`: 除外タグ列（空可）。
pub fn filter_images(
    folder: impl AsRef<Path>,
    include: &[String],
    exclude: &[String],
) -> AppResult<Vec<ImageEntry>> {
    let listing = file_service::list_images(folder)?;

    let mut matched: Vec<ImageEntry> = Vec::new();
    for item in listing.items {
        let path = Path::new(&item.path);
        let tokens = match tag_file::read_tag_file(path) {
            Ok(content) => tag_ops::split_tokens(&content.content),
            // 読込失敗は判定不能のため除外。
            Err(_) => continue,
        };
        if tag_stats::matches_filter(&tokens, include, exclude) {
            matched.push(item);
        }
    }

    Ok(matched)
}

#[cfg(test)]
mod aggregate_and_filter_tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    /// 画像（空ファイル）とその Tag_File を作成するヘルパー。
    /// `tags` が `Some` の場合のみ Tag_File を書き出す。
    fn make_image(dir: &Path, stem: &str, tags: Option<&str>) {
        fs::write(dir.join(format!("{stem}.png")), b"").unwrap();
        if let Some(t) = tags {
            fs::write(dir.join(format!("{stem}.txt")), t).unwrap();
        }
    }

    fn v(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn aggregates_counts_across_multiple_tag_files() {
        let dir = tempdir().unwrap();
        make_image(dir.path(), "a", Some("cat, dog, cat"));
        make_image(dir.path(), "b", Some("dog, bird"));
        make_image(dir.path(), "c", Some("cat"));

        let result = aggregate_tags(dir.path()).unwrap();
        assert_eq!(result.unreadable, 0);
        assert_eq!(
            result.tags,
            vec![
                TagCount {
                    tag: "cat".into(),
                    count: 3
                },
                TagCount {
                    tag: "dog".into(),
                    count: 2
                },
                TagCount {
                    tag: "bird".into(),
                    count: 1
                },
            ]
        );
    }

    #[test]
    fn image_without_tag_file_contributes_no_tags_and_is_not_unreadable() {
        let dir = tempdir().unwrap();
        make_image(dir.path(), "with", Some("cat, dog"));
        // Tag_File 無しの画像は空タグ扱い（読込失敗ではない）。
        make_image(dir.path(), "without", None);

        let result = aggregate_tags(dir.path()).unwrap();
        assert_eq!(result.unreadable, 0);
        assert_eq!(
            result.tags,
            vec![
                TagCount {
                    tag: "cat".into(),
                    count: 1
                },
                TagCount {
                    tag: "dog".into(),
                    count: 1
                },
            ]
        );
    }

    #[test]
    fn unreadable_tag_file_is_excluded_and_counted() {
        // 読込失敗を再現するため、対応 Tag_File パスをディレクトリにする。
        // read_tag_file はディレクトリの read で NotFound 以外の I/O エラーを返し、
        // 集計から除外され unreadable に計上される（要件 6.2）。
        let dir = tempdir().unwrap();
        make_image(dir.path(), "good", Some("cat, dog"));
        // "bad.png" に対応する "bad.txt" をディレクトリとして作成。
        fs::write(dir.path().join("bad.png"), b"").unwrap();
        fs::create_dir(dir.path().join("bad.txt")).unwrap();

        let result = aggregate_tags(dir.path()).unwrap();
        assert_eq!(result.unreadable, 1);
        assert_eq!(
            result.tags,
            vec![
                TagCount {
                    tag: "cat".into(),
                    count: 1
                },
                TagCount {
                    tag: "dog".into(),
                    count: 1
                },
            ]
        );
    }

    #[test]
    fn empty_folder_yields_empty_aggregation() {
        let dir = tempdir().unwrap();
        let result = aggregate_tags(dir.path()).unwrap();
        assert!(result.tags.is_empty());
        assert_eq!(result.unreadable, 0);
    }

    #[test]
    fn folder_with_only_empty_tag_files_yields_empty_aggregation() {
        // 有効タグ 0 件（すべて空 Tag_File）なら空結果（要件 6.4）。
        let dir = tempdir().unwrap();
        make_image(dir.path(), "a", Some(""));
        make_image(dir.path(), "b", Some("   ,  "));

        let result = aggregate_tags(dir.path()).unwrap();
        assert!(result.tags.is_empty());
        assert_eq!(result.unreadable, 0);
    }

    #[test]
    fn filter_include_keeps_only_matching_images() {
        let dir = tempdir().unwrap();
        make_image(dir.path(), "a", Some("cat, dog, sky"));
        make_image(dir.path(), "b", Some("cat, sky"));
        make_image(dir.path(), "c", Some("cat, dog"));

        // include=[cat, dog] を両方含むのは a, c。
        let result = filter_images(dir.path(), &v(&["cat", "dog"]), &[]).unwrap();
        let names: Vec<&str> = result.iter().map(|e| e.file_name.as_str()).collect();
        assert_eq!(names, vec!["a.png", "c.png"]);
    }

    #[test]
    fn filter_exclude_drops_matching_images() {
        let dir = tempdir().unwrap();
        make_image(dir.path(), "a", Some("cat, night"));
        make_image(dir.path(), "b", Some("cat, day"));

        // exclude=[night] を含む a を落とす。
        let result = filter_images(dir.path(), &[], &v(&["night"])).unwrap();
        let names: Vec<&str> = result.iter().map(|e| e.file_name.as_str()).collect();
        assert_eq!(names, vec!["b.png"]);
    }

    #[test]
    fn filter_include_and_exclude_combined() {
        let dir = tempdir().unwrap();
        make_image(dir.path(), "a", Some("cat, dog, sky"));
        make_image(dir.path(), "b", Some("cat, dog, night"));
        make_image(dir.path(), "c", Some("cat, sky"));

        // include=[cat, dog] かつ exclude=[night] → a のみ。
        let result = filter_images(dir.path(), &v(&["cat", "dog"]), &v(&["night"])).unwrap();
        let names: Vec<&str> = result.iter().map(|e| e.file_name.as_str()).collect();
        assert_eq!(names, vec!["a.png"]);
    }

    #[test]
    fn filter_empty_conditions_returns_all_images() {
        // include/exclude 空なら全件（要件 6.8）。Tag_File 無し画像も含む。
        let dir = tempdir().unwrap();
        make_image(dir.path(), "a", Some("cat"));
        make_image(dir.path(), "b", None);

        let result = filter_images(dir.path(), &[], &[]).unwrap();
        let names: Vec<&str> = result.iter().map(|e| e.file_name.as_str()).collect();
        assert_eq!(names, vec!["a.png", "b.png"]);
    }

    #[test]
    fn filter_include_excludes_image_without_tag_file() {
        // Tag_File 無し画像はタグ無し扱い → include 指定時は除外される。
        let dir = tempdir().unwrap();
        make_image(dir.path(), "a", Some("cat"));
        make_image(dir.path(), "b", None);

        let result = filter_images(dir.path(), &v(&["cat"]), &[]).unwrap();
        let names: Vec<&str> = result.iter().map(|e| e.file_name.as_str()).collect();
        assert_eq!(names, vec!["a.png"]);
    }

    // -------------------------------------------------------------------
    // 要件 6.2 の網羅補強: 複数の読込失敗 Tag_File があっても、
    // それぞれ除外して unreadable 件数が失敗ファイル数と一致し、
    // 読める Tag_File は正しく集計される。読込失敗は「対応 Tag_File パスを
    // ディレクトリにする」ことで誘発する（read_tag_file が I/O エラー）。
    // -------------------------------------------------------------------

    /// 対応 Image_File を作りつつ、その Tag_File パスをディレクトリにして
    /// 読込失敗を誘発するヘルパー（`<stem>.txt` がディレクトリ）。
    fn make_image_with_unreadable_tag_file(dir: &Path, stem: &str) {
        fs::write(dir.join(format!("{stem}.png")), b"").unwrap();
        fs::create_dir(dir.join(format!("{stem}.txt"))).unwrap();
    }

    #[test]
    fn multiple_unreadable_tag_files_are_excluded_and_count_matches() {
        // 読込失敗 3 件・読める 2 件を混在させる（要件 6.2）。
        let dir = tempdir().unwrap();
        make_image(dir.path(), "good1", Some("cat, dog"));
        make_image_with_unreadable_tag_file(dir.path(), "bad1");
        make_image(dir.path(), "good2", Some("cat, bird"));
        make_image_with_unreadable_tag_file(dir.path(), "bad2");
        make_image_with_unreadable_tag_file(dir.path(), "bad3");

        let result = aggregate_tags(dir.path()).unwrap();

        // 失敗 3 件が unreadable に計上され、失敗ファイル数と一致する。
        assert_eq!(result.unreadable, 3);
        // 読める 2 件のみが集計される（cat×2, dog×1, bird×1、降順→同数はタグ名昇順）。
        assert_eq!(
            result.tags,
            vec![
                TagCount {
                    tag: "cat".into(),
                    count: 2
                },
                TagCount {
                    tag: "bird".into(),
                    count: 1
                },
                TagCount {
                    tag: "dog".into(),
                    count: 1
                },
            ]
        );
    }

    #[test]
    fn readable_files_aggregate_correctly_despite_interleaved_unreadable() {
        // 読込失敗が読める対象の「間」に挟まっても集計結果に影響しない（要件 6.2）。
        // 列挙順（ファイル名昇順）: a(読める), b(失敗), c(読める)。
        let dir = tempdir().unwrap();
        make_image(dir.path(), "a", Some("sky, sky, cloud"));
        make_image_with_unreadable_tag_file(dir.path(), "b");
        make_image(dir.path(), "c", Some("sky, cloud"));

        let result = aggregate_tags(dir.path()).unwrap();

        assert_eq!(result.unreadable, 1);
        assert_eq!(
            result.tags,
            vec![
                TagCount {
                    tag: "sky".into(),
                    count: 3
                },
                TagCount {
                    tag: "cloud".into(),
                    count: 2
                },
            ]
        );
    }

    // -------------------------------------------------------------------
    // 要件 6.4 の網羅補強: 有効タグが 0 件なら空リストを返しつつ、
    // 読込失敗が存在する場合はその件数を報告する（UI が「除外あり」通知を
    // 出せるようにする）。空タグ Tag_File と読込失敗の組み合わせでも同様。
    // -------------------------------------------------------------------

    #[test]
    fn no_valid_tags_yields_empty_but_reports_unreadable_count() {
        // 読める Tag_File はすべて空（有効タグ 0 件）＋読込失敗 2 件（要件 6.2, 6.4）。
        let dir = tempdir().unwrap();
        make_image(dir.path(), "empty1", Some(""));
        make_image(dir.path(), "empty2", Some("   ,  ,"));
        make_image_with_unreadable_tag_file(dir.path(), "bad1");
        make_image_with_unreadable_tag_file(dir.path(), "bad2");

        let result = aggregate_tags(dir.path()).unwrap();

        // 有効タグ 0 件 → 空リスト（要件 6.4）。
        assert!(result.tags.is_empty());
        // 読込失敗件数は報告され、UI が除外通知を出せる（要件 6.2）。
        assert_eq!(result.unreadable, 2);
    }

    #[test]
    fn all_tag_files_unreadable_yields_empty_with_full_unreadable_count() {
        // すべての Tag_File が読込失敗 → 集計は空、unreadable は全件（要件 6.2, 6.4）。
        let dir = tempdir().unwrap();
        make_image_with_unreadable_tag_file(dir.path(), "a");
        make_image_with_unreadable_tag_file(dir.path(), "b");

        let result = aggregate_tags(dir.path()).unwrap();

        assert!(result.tags.is_empty());
        assert_eq!(result.unreadable, 2);
    }
}
