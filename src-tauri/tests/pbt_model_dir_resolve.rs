// Feature: local-model-management, Property 4: Model_Dir 解決の決定性と配下性
//
// 任意のインストール基準ディレクトリ base_dir について、resolve_model_dir(base_dir)
// は同一入力に対し常に同一の絶対パスを返し（決定性）、その結果は base_dir を接頭辞に
// 持つ固定相対パスの結合である。
//
// Validates: Requirements 2.1

use std::path::{Path, PathBuf};

use proptest::prelude::*;
use tag_editor_core::services::model_service::resolve_model_dir;

/// base_dir 生成器。
///
/// Testing Strategy の base_dir エッジ（空・末尾区切りあり／なし・非 ASCII）を
/// 必ずカバーするため、以下を組み合わせて `PathBuf` を構築する:
/// - ルート種別: 空（相対ルート）／絶対（`/` 始まり）
/// - 中間コンポーネント列（0〜4 個、非 ASCII を含む文字プール）
/// - 末尾区切りの有無
///
/// 決定性・配下性は実ファイルシステムへ問い合わせない純粋関数の性質であり、
/// 実在しないパスでも成立する。よって存在チェックは行わない。
fn base_dir_strategy() -> impl Strategy<Value = PathBuf> {
    // コンポーネント文字プール: 英字・数字・記号に加え非 ASCII を含める。
    // 区切り文字（`/` `\`）は含めない（コンポーネント境界と混同しないため）。
    let component = prop::collection::vec(
        prop::sample::select(vec![
            'a', 'Z', 'M', '0', '7', '_', '-', '.', // ASCII
            '猫', '髪', 'あ', '한', '🎨', // 非 ASCII / マルチバイト
        ]),
        1..10,
    )
    .prop_map(|v| v.into_iter().collect::<String>());

    (
        any::<bool>(),                               // 絶対パスにするか
        prop::collection::vec(component, 0..5),      // 中間コンポーネント
        any::<bool>(),                               // 末尾区切りを付けるか
    )
        .prop_map(|(absolute, components, trailing_sep)| {
            let mut s = String::new();
            if absolute {
                s.push('/');
            }
            s.push_str(&components.join("/"));
            if trailing_sep {
                // 末尾区切りあり（空文字列でも `/` を付けてルート＋末尾区切りを表現）。
                s.push('/');
            }
            PathBuf::from(s)
        })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// Property 4（決定性）: 同一 base_dir に対し 2 回呼び出すと一致する。
    #[test]
    fn resolve_is_deterministic(base in base_dir_strategy()) {
        let a = resolve_model_dir(&base);
        let b = resolve_model_dir(&base);
        prop_assert_eq!(a, b, "同一入力で異なるパスを返した: base={:?}", base);
    }

    /// Property 4（配下性）: 結果は base_dir を接頭辞に持つ。
    ///
    /// PathBuf の components 比較で堅牢に検証する（`starts_with` は文字列一致
    /// ではなくコンポーネント単位で判定するため、末尾区切りの有無に依存しない）。
    #[test]
    fn resolve_is_prefixed_by_base(base in base_dir_strategy()) {
        let dir = resolve_model_dir(&base);
        prop_assert!(
            dir.starts_with(&base),
            "base_dir を接頭辞に持たない: base={:?} dir={:?}",
            base,
            dir
        );
    }

    /// Property 4（固定相対パスの結合）: base_dir の全コンポーネントに続けて
    /// 固定のモデル配置サブディレクトリ 1 段が付与される。
    ///
    /// 定数 `MODEL_DIR_RELATIVE` は非公開のため直接参照せず、結果の
    /// コンポーネント列が base のコンポーネント列に「ちょうど 1 個」追加した
    /// 形（末尾が単一の固定サブディレクトリ）であることを検証する。
    #[test]
    fn resolve_appends_single_fixed_component(base in base_dir_strategy()) {
        let dir = resolve_model_dir(&base);
        let base_components: Vec<_> = base.components().collect();
        let dir_components: Vec<_> = dir.components().collect();

        // base の全コンポーネントが dir の先頭に一致する。
        prop_assert!(
            dir_components.starts_with(&base_components),
            "base のコンポーネント列が接頭辞になっていない: base={:?} dir={:?}",
            base,
            dir
        );
        // dir は base より 1 段だけ深い（固定相対パス "models" の結合）。
        prop_assert_eq!(
            dir_components.len(),
            base_components.len() + 1,
            "追加コンポーネントが 1 段でない: base={:?} dir={:?}",
            base,
            dir
        );

        // 追加された末尾コンポーネントは base に依らず常に同一（決定的な固定値）。
        // Component は借用元パスの寿命に縛られるため、OsStr 化して比較する。
        let tail_from_base = dir_components.last().unwrap().as_os_str().to_owned();
        let reference_tail = {
            let ref_dir = resolve_model_dir(Path::new("/ref"));
            ref_dir
                .components()
                .last()
                .unwrap()
                .as_os_str()
                .to_owned()
        };
        prop_assert_eq!(
            tail_from_base,
            reference_tail,
            "末尾の固定サブディレクトリが入力により変化した: base={:?} dir={:?}",
            base,
            dir
        );
    }
}

/// 末尾区切りの有無で結果が一致すること（決定性の代表例）。
///
/// proptest では両者が独立に生成されるため、同一の論理パスに対する
/// 末尾区切り有無の等価性を固定例で明示的に確認する。
#[test]
fn trailing_separator_does_not_change_result() {
    let without = resolve_model_dir(Path::new("/opt/tageditor"));
    let with = resolve_model_dir(Path::new("/opt/tageditor/"));
    assert_eq!(without, with);
}

/// 空 base_dir（相対ルート）でも決定的に固定サブディレクトリを返す。
#[test]
fn empty_base_dir_resolves_to_fixed_component() {
    let a = resolve_model_dir(Path::new(""));
    let b = resolve_model_dir(Path::new(""));
    assert_eq!(a, b);
    // 空 base に対しては固定サブディレクトリ 1 段のみになる。
    assert_eq!(a.components().count(), 1);
}
