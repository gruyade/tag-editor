# Requirements Document

## Introduction

TagEditor は、既存の3つのツール（NotCoolTagEditor: 画像閲覧・タグ編集、ImageRenameDotNet8: 画像/フォルダ仕訳、wd14-tagger: WD14タグ自動生成）の機能仕様を統合した、新規の軽量デスクトップアプリである。実装は Rust + Tauri で行い、UIはOSのWebViewを用いたWeb技術、コアロジック（ファイル操作・タグ処理・ONNX推論）はRustで実装する。既存アプリのソースコードは参照せず、本ドキュメントに記載する機能仕様のみを引き継ぐ完全な新規アプリである。

タグ生成は `ort` クレート（ONNX Runtime）による WD14系ONNXモデルのRust内推論で行い、外部のStable Diffusion WebUI拡張には依存せず独立実行できる。Windows を第一級サポート対象として全機能を提供し、macOS および Linux はベストエフォートで中核機能をサポートする。シンボリックリンク作成やWindows/Linuxパス変換などWindows固有の機能は、Windows以外の環境ではUIに表示しない。非機能要件として、Electron等の重量フレームワークを避けメモリ使用量を抑えることを重視する。対応する推論モデルは ONNX形式のもの（WD14系の各バリアント、ML-Danbooru系、およびユーザーが追加するローカルONNXモデル）に限定し、TensorFlow専用モデル（DeepDanbooru等）は対象外とする。これによりTensorFlow依存を排し軽量性を維持する。元のwd14-tagger拡張がTensorFlowで行っていた大量バッチ推論の高速化は、Rust側の並列処理とONNX Runtimeのバッチ入力で代替する。

## Glossary

- **TagEditor**: 本アプリケーション全体。Tauriベースのデスクトップアプリ。
- **Core**: Rustで実装されるコアロジック層（ファイル操作、タグ処理、推論を担う）。
- **UI**: WebView上で動作するフロントエンド層。ユーザー操作を受け付け、Coreを呼び出す。
- **Image_File**: 対応拡張子（jpg / jpeg / png / gif / mp4）を持つメディアファイル。
- **Tag_File**: Image_File と同名の `.txt` ファイル。カンマ区切りのタグ列を格納する。
- **Tag**: Tag_File 内の1つのタグ文字列。信頼度を伴う場合がある（例: `(tag:0.9)`）。
- **Confidence**: タグに付与される 0.0〜1.0 の信頼度スコア。
- **Booru_Format**: Danbooru系タグ表記。アンダースコア区切りの単語と、エスケープされた括弧を含む。
- **Inference_Model**: WD14系のONNX形式画像タグ付けモデル。
- **Model_Store**: Inference_Model の格納先。ローカルディレクトリまたはHuggingFace Hub等のリモートリポジトリ。
- **Confidence_Threshold**: タグを採用するかを判定する信頼度の下限値（0.0〜1.0）。
- **Sorting_Operation**: フォルダ間ファイル仕訳の種別（まとめ / 配布 / 移動 / コピー）。
- **Orphan_Caption**: 対応する Image_File が存在しない Tag_File。
- **Thumbnail**: 一覧表示用に生成される Image_File の縮小表示。
- **Model_Variant**: 選択可能な個別の Inference_Model。WD14系（ViT / ConvNeXT / ConvNeXTV2 / SwinV2 / MoaT）、ML-Danbooru系、またはユーザー追加のローカルONNXモデル。
- **Local_Model_Dir**: ユーザーがローカルONNXモデルを配置するディレクトリ。1つの `.onnx` ファイルとタグ定義ファイル（`.csv` または `.json`）の対で構成される。
- **Batch_Size**: 1回の推論呼び出しにまとめて入力する Image_File の件数。
- **Windows_Only_Feature**: Windows環境に固有で他OSでは意味を持たない機能。シンボリックリンク作成、および Windows/Linux(WSL)パス変換が該当する。

## Requirements

### Requirement 1: フォルダ選択と画像一覧表示

**User Story:** As a ユーザー, I want フォルダを指定して中の画像をサムネイル一覧で見たい, so that 対象画像を素早く把握して編集対象を選べる

#### Acceptance Criteria

1. WHEN ユーザーがフォルダを選択する, THE TagEditor SHALL 選択フォルダ直下の Image_File（拡張子 jpg / jpeg / png / gif / mp4、大文字小文字を区別しない）を列挙する
2. WHEN Image_File の列挙件数が 10,000 件を超える, THE TagEditor SHALL 先頭 10,000 件を一覧対象とし、上限を超えた旨を表示する
3. WHEN Image_File の列挙が完了する, THE TagEditor SHALL 各 Image_File の Thumbnail を一覧に表示する
4. WHEN ユーザーが Thumbnail サイズの変更を指示する, THE TagEditor SHALL 64〜512 ピクセルの範囲内で指定された表示サイズに Thumbnail を再描画する
5. WHEN ユーザーが一覧内の Image_File を選択する, THE TagEditor SHALL 選択された Image_File の拡大プレビューを表示する
6. IF 一覧内の個別 Image_File が破損等で読み込めない, THEN THE TagEditor SHALL 当該項目を代替表示にし残りの一覧表示を継続する
7. IF 選択フォルダに対応拡張子の Image_File が存在しない, THEN THE TagEditor SHALL 画像が存在しない旨のメッセージを一覧領域に表示する
8. IF 選択されたパスがフォルダとして読み取れない, THEN THE TagEditor SHALL エラーメッセージを表示し一覧を空のまま維持する

### Requirement 2: タグファイルの読み込み・編集・保存

**User Story:** As a ユーザー, I want 画像に対応するタグファイルを表示して編集・保存したい, so that 学習データのタグを整備できる

#### Acceptance Criteria

1. WHEN ユーザーが Image_File を選択する, THE TagEditor SHALL 同名の Tag_File が存在する場合その内容を UTF-8 として解釈し編集領域に表示する
2. IF 選択された Image_File に対応する Tag_File が存在しない, THEN THE TagEditor SHALL 編集領域を空文字列にし新規作成可能な状態にする
3. WHEN ユーザーが保存を指示する, THE TagEditor SHALL 編集領域の内容を対応する Tag_File に UTF-8 で書き込む
4. WHEN Tag_File が存在しない Image_File に対してユーザーが保存を指示する, THE TagEditor SHALL 同名の Tag_File を新規作成して内容を書き込む
5. WHEN Tag_File への書き込みが成功する, THE TagEditor SHALL 保存が成功した旨を表示する
6. IF Tag_File の書き込みに失敗する, THEN THE TagEditor SHALL 失敗した旨のエラーメッセージを表示し元のファイル内容を変更しないまま維持する

### Requirement 3: タグの一括操作（追加・削除・重複除去）

**User Story:** As a ユーザー, I want 複数画像のタグをまとめて追加・削除・整理したい, so that 大量画像のタグ編集を効率化できる

#### Acceptance Criteria

1. WHEN ユーザーが対象 Image_File 群と追加 Tag を指定して一括追加を指示する, THE TagEditor SHALL 指定された各 Image_File の Tag_File に当該 Tag を追加する
2. WHEN 追加対象 Tag が既に Tag_File に存在する（前後空白をトリムした完全一致・大文字小文字を区別しない）, THE TagEditor SHALL 当該 Tag を重複追加せず既存内容を維持する
3. WHEN ユーザーが対象 Image_File 群と削除 Tag を指定して一括削除を指示する, THE TagEditor SHALL 各 Tag_File から当該 Tag（前後空白をトリムした完全一致・大文字小文字を区別しない）に一致するすべての Tag を除去する
4. WHEN ユーザーが重複除去を指示する, THE TagEditor SHALL 対象 Tag_File 内で同一の Tag（前後空白をトリムした完全一致・大文字小文字を区別しない）が複数存在する場合、最初の出現を残し以降の重複を除去する
5. WHEN ユーザーが複数の Image_File を選択して一括タグ付けを指示する, THE TagEditor SHALL 選択された各 Image_File に指定された Tag 群を付与する
6. WHERE 一括操作の対象に Tag_File を持たない Image_File が含まれる, THE TagEditor SHALL 当該 Image_File の Tag_File を新規作成したうえで操作を適用する
7. IF 一括操作の途中で個別 Tag_File の書き込みに失敗する, THEN THE TagEditor SHALL 当該ファイルをスキップして残りの対象への処理を継続し、失敗した件数を結果に記録する
8. IF 一括操作の対象 Image_File が1件も指定されていない, THEN THE TagEditor SHALL 操作を実行せず対象未選択である旨を表示する

### Requirement 4: タグファイル名の正規化

**User Story:** As a ユーザー, I want 拡張子付きのtxtファイル名を統一形式に直したい, so that タグファイルの命名を一貫させられる

#### Acceptance Criteria

1. WHEN ユーザーがファイル名正規化を指示する, THE TagEditor SHALL 対象フォルダ内の `.<ext>.txt` 形式（例: `image.png.txt`）のファイルを `<basename>.txt` 形式（例: `image.txt`）へ改名する
2. IF 正規化後の名前を持つファイルが既に存在する, THEN THE TagEditor SHALL 当該ファイルを改名せず、衝突した旨を操作結果に記録する
3. WHEN 正規化処理が完了する, THE TagEditor SHALL 改名した件数と衝突により改名しなかった件数を結果として表示する

### Requirement 5: Booru形式・信頼度付きタグの変換

**User Story:** As a ユーザー, I want booru形式や信頼度付きのタグを扱いたい, so that 異なる形式のタグデータを統一的に編集できる

#### Acceptance Criteria

1. WHEN ユーザーが Booru_Format 変換を指示する, THE TagEditor SHALL 各 Tag 内のアンダースコアをスペースに置換する
2. WHEN ユーザーが Booru_Format 変換を指示する, THE TagEditor SHALL Tag 内の括弧文字 `(` および `)` をバックスラッシュでエスケープする
3. WHEN Confidence を伴う `(tag:0.9)` 形式の Tag を読み込む, THE TagEditor SHALL Tag 本体と 0.0〜1.0 の範囲の Confidence 値を分離して保持する
4. IF `(tag:値)` 形式の Confidence 部分が 0.0〜1.0 の数値として解釈できない, THEN THE TagEditor SHALL 当該項目を Confidence を持たない Tag 本体として扱う
5. WHERE ユーザーが Confidence 表示を有効にする, THE TagEditor SHALL Confidence を持つ各 Tag を小数第2位までの `(tag:<confidence>)` 形式で編集領域に表示する
6. WHERE ユーザーが Confidence 表示を無効にする, THE TagEditor SHALL 各 Tag を Confidence を除いた本体のみで編集領域に表示する

### Requirement 6: タグ集計とタグによる画像フィルタ

**User Story:** As a ユーザー, I want 全画像のタグ出現数を集計し特定タグで画像を絞り込みたい, so that データセット全体のタグ傾向を把握し対象画像を選別できる

#### Acceptance Criteria

1. WHEN ユーザーがタグ集計を指示する, THE TagEditor SHALL 対象フォルダ内の全 Tag_File を走査し各 Tag の出現回数を算出する
2. IF タグ集計中に Tag_File の読み込みに失敗する, THEN THE TagEditor SHALL 当該 Tag_File を集計対象から除外して残りの走査を継続し、読み込めなかったファイル数を示すエラー表示を提示する
3. WHEN タグ集計が完了する, THE TagEditor SHALL 集計結果を出現回数の降順で一覧表示し、出現回数が同一の Tag は Tag 名の昇順で並べる
4. IF 対象フォルダ内に有効な Tag が1件も存在しない, THEN THE TagEditor SHALL 空の集計結果一覧を表示し、集計対象タグが存在しない旨の表示を提示する
5. WHEN ユーザーが1つ以上の包含 Tag を指定してフィルタを適用する, THE TagEditor SHALL 指定 Tag をすべて含む Image_File のみを一覧に表示する
6. WHEN ユーザーが1つ以上の除外 Tag を指定してフィルタを適用する, THE TagEditor SHALL 指定 Tag のいずれかを含む Image_File を一覧から除外する
7. WHEN ユーザーが包含 Tag と除外 Tag を同時に指定してフィルタを適用する, THE TagEditor SHALL 指定包含 Tag をすべて含みかつ指定除外 Tag のいずれも含まない Image_File のみを一覧に表示する
8. WHEN ユーザーがフィルタを解除する, THE TagEditor SHALL 対象フォルダの全 Image_File を一覧に再表示する

### Requirement 7: フォルダ間のファイル仕訳

**User Story:** As a ユーザー, I want フォルダ間でファイルをまとめ・配布・移動・コピーしたい, so that 画像とタグファイルを目的別に整理できる

#### Acceptance Criteria

1. WHEN ユーザーが Sorting_Operation（まとめ / 配布 / 移動 / コピー のいずれか1種別）と対象フォルダおよび宛先フォルダを指定して実行する, THE TagEditor SHALL 指定された Sorting_Operation に従って対象範囲内の各 Image_File を処理する
2. WHERE Sorting_Operation が移動またはコピーである, THE TagEditor SHALL 各 Image_File に対応する Tag_File が同一フォルダに存在する場合は当該 Tag_File も同じ宛先へ同一操作で処理し、対応する Tag_File が存在しない場合は当該 Image_File のみを処理する
3. WHERE Sorting_Operation がまとめである, THE TagEditor SHALL 対象フォルダの直下サブフォルダ内の各ファイルについて、当該サブフォルダ名を接頭辞として付与したうえで単一の宛先フォルダへ集約する
4. WHERE Sorting_Operation が配布である, THE TagEditor SHALL 対象フォルダ内の各ファイル名を接頭辞部分と元ファイル名部分に分解し、接頭辞部分と一致する名前のサブフォルダを宛先に作成または再利用して当該ファイルを元ファイル名で配置する
5. IF 宛先に同名ファイルが既に存在する, THEN THE TagEditor SHALL 当該ファイルを上書きせず処理対象から除外し、衝突した旨を操作結果に記録する
6. IF 指定された対象フォルダまたは宛先フォルダが存在しないかアクセスできない, THEN THE TagEditor SHALL 仕訳処理を開始せず、いかなるファイルも変更・移動せず、対象または宛先が利用不可である旨を示すエラーを表示する
7. IF 配布処理の対象ファイル名が接頭辞と元ファイル名に分解できない, THEN THE TagEditor SHALL 当該ファイルを処理対象から除外し、分解できなかった旨を操作結果に記録する
8. WHEN 仕訳処理が完了する, THE TagEditor SHALL 処理成功件数・衝突件数・スキップ件数を含む結果を表示する

### Requirement 8: ファイル名の一括置換と連番付与

**User Story:** As a ユーザー, I want ファイル名を正規表現で一括置換し連番を付けたい, so that 命名規則を統一できる

#### Acceptance Criteria

1. WHEN ユーザーが正規表現パターンと置換文字列を指定して一括置換を指示する, THE TagEditor SHALL 対象フォルダ内の各ファイル名に対しパターンにマッチした部分を置換文字列へ変換する
2. WHEN ユーザーが連番付与を指示する, THE TagEditor SHALL 対象ファイルをファイル名昇順に並べ、指定された開始番号から1ずつ増加する連番を、置換文字列中のプレースホルダ（NUM）を指定桁数でゼロ埋めした番号に展開して改名する
3. WHERE Image_File が改名される, THE TagEditor SHALL 対応する Tag_File が存在する場合、同じ連番・同じ規則で Tag_File も改名し Image_File と対を維持する
4. IF 指定された正規表現パターンが不正である, THEN THE TagEditor SHALL エラーメッセージを表示し改名を実行しない
5. IF 改名後の名前がOSで使用できない禁止文字を含む, THEN THE TagEditor SHALL 当該ファイルを改名せず、無効な名前である旨を操作結果に記録する
6. IF 改名後の名前が既存ファイルと衝突する, THEN THE TagEditor SHALL 当該ファイルを改名せず、衝突した旨を操作結果に記録する

### Requirement 9: 画像サイズによる自動仕訳

**User Story:** As a ユーザー, I want 画像の縦横サイズや閾値でフォルダ振り分けしたい, so that 解像度別にデータセットを整理できる

#### Acceptance Criteria

1. WHEN ユーザーが長辺の閾値（1 以上 100000 以下の整数ピクセル）と振り分け先ルートフォルダを指定して実行する, THE TagEditor SHALL 各 Image_File の幅と高さのピクセル寸法を取得する
2. IF 指定された閾値が 1 未満、100000 超、または整数として解釈できない, THEN THE TagEditor SHALL 処理を開始せず、閾値が無効である旨を操作結果に記録する
3. WHEN Image_File の寸法取得が成功する, THE TagEditor SHALL 幅と高さの大小関係（幅>高さ を横長、幅<高さ を縦長、幅==高さ を横長として扱う）と長辺が閾値以上か未満かの組み合わせで振り分け先フォルダを一意に決定する
4. WHEN 振り分け先フォルダが決定する, THE TagEditor SHALL 当該 Image_File を決定された振り分け先フォルダへ移動する
5. WHERE Image_File が移動される, THE TagEditor SHALL 対応する Tag_File が存在する場合、同一の振り分け先フォルダへ同じ移動操作を行う
6. IF 振り分け先フォルダに同名ファイルが既に存在する, THEN THE TagEditor SHALL 当該 Image_File および対応する Tag_File を移動せず元の場所に保持し、名前衝突により移動できなかった旨を操作結果に記録する
7. IF Image_File の寸法を取得できない, THEN THE TagEditor SHALL 当該ファイルを振り分けから除外して元の場所に保持し、処理できなかった旨を操作結果に記録する

### Requirement 10: 特定タグによる画像仕訳

**User Story:** As a ユーザー, I want 特定タグの有無で画像をフォルダに振り分けたい, so that タグ内容に基づいてデータセットを分類できる

#### Acceptance Criteria

1. WHEN ユーザーが1つ以上の判定 Tag と振り分け先を指定して実行する, THE TagEditor SHALL 各 Image_File の Tag_File に判定 Tag（前後空白をトリムした完全一致）のいずれかが含まれるかを判定する
2. WHEN Tag_File に判定 Tag のいずれかが含まれる, THE TagEditor SHALL 当該 Image_File と対応する Tag_File を「含む」振り分け先へ移動する
3. WHEN Tag_File に判定 Tag のいずれも含まれない, THE TagEditor SHALL 当該 Image_File と対応する Tag_File を「含まない」振り分け先へ移動する
4. IF Image_File に対応する Tag_File が存在しない, THEN THE TagEditor SHALL 当該 Image_File を「含まない」として扱う
5. IF 振り分け先に同名ファイルが既に存在する, THEN THE TagEditor SHALL 当該ファイルを移動せず元の場所に保持し、衝突した旨を操作結果に記録する
6. IF 指定された振り分け先が存在せず作成もできない, THEN THE TagEditor SHALL 処理を開始せず、振り分け先が利用不可である旨を表示する

### Requirement 11: 孤立キャプションファイルの削除

**User Story:** As a ユーザー, I want 対応画像がないタグファイルを削除したい, so that 不要なキャプションファイルを掃除できる

#### Acceptance Criteria

1. WHEN ユーザーが孤立キャプション削除を指示する, THE TagEditor SHALL 対象フォルダ内で対応する Image_File を持たない Tag_File を Orphan_Caption として特定する
2. WHEN Orphan_Caption の特定が完了する, THE TagEditor SHALL 削除実行前に対象一覧をユーザーに提示する
3. WHEN ユーザーが削除を承認する, THE TagEditor SHALL 提示された Orphan_Caption を削除する
4. WHEN 削除が完了する, THE TagEditor SHALL 削除した件数を結果として表示する

### Requirement 12: タグ付与（作者名など）

**User Story:** As a ユーザー, I want 作者名などのタグを対象画像へ付与したい, so that 出所や属性の情報をタグとして残せる

#### Acceptance Criteria

1. WHEN ユーザーが付与 Tag と対象 Image_File 群を指定する, THE TagEditor SHALL 各対象 Image_File の Tag_File に当該 Tag を追加する
2. WHERE 対象 Image_File が Tag_File を持たない, THE TagEditor SHALL 当該 Tag_File を新規作成して Tag を追加する
3. WHEN 指定 Tag が既に Tag_File に存在する, THE TagEditor SHALL 当該 Tag_File を重複追加せず既存内容を維持する

### Requirement 13: シンボリックリンク作成とパス変換（Windows限定）

**User Story:** As a Windowsユーザー, I want シンボリックリンク作成とWindows/Linuxパス変換をしたい, so that 実体を複製せずファイルを参照し、WSL等の異なるOS間でパスを扱える

#### Acceptance Criteria

1. WHERE 実行環境が Windows である, THE TagEditor SHALL Windows_Only_Feature（シンボリックリンク作成・Windows/Linuxパス変換）のUI操作を提供する
2. WHERE 実行環境が Windows 以外である, THE TagEditor SHALL Windows_Only_Feature のUIを表示しない
3. WHEN ユーザーがリンク元と作成先を指定してシンボリックリンク作成を指示する, THE TagEditor SHALL 指定された作成先にリンク元を指すシンボリックリンクを作成する
4. IF 指定されたリンク元が存在しない, THEN THE TagEditor SHALL リンク元が存在しない旨のエラーメッセージを表示しリンクを作成しない
5. IF 指定された作成先のパスに既存のファイルまたはディレクトリが存在する, THEN THE TagEditor SHALL 作成先が既に使用中である旨のエラーメッセージを表示し既存の対象を変更せずリンクを作成しない
6. IF シンボリックリンクの作成に必要な権限が不足している, THEN THE TagEditor SHALL 権限不足の旨のエラーメッセージを表示しリンクを作成しない
7. WHEN ユーザーが Windows形式パスの Linux形式への変換を指示する, THE TagEditor SHALL パス区切り文字（バックスラッシュ）をスラッシュへ変換し、ドライブ表記（例: C:\foo\bar）を /mnt/<小文字ドライブ文字>/foo/bar 形式へ変換した文字列を出力する
8. WHEN ユーザーが Linux形式パスの Windows形式への変換を指示する, THE TagEditor SHALL パス区切り文字（スラッシュ）をバックスラッシュへ変換し、/mnt/<ドライブ文字>/ 形式のドライブ表記を <大文字ドライブ文字>:\ 形式へ変換した文字列を出力する
9. IF 変換対象として指定されたパスが対象OS形式のドライブ表記規則に適合しない, THEN THE TagEditor SHALL 変換不能である旨のエラーメッセージを表示し変換結果を出力しない

### Requirement 14: WD14 ONNXモデルによるタグ自動生成

**User Story:** As a ユーザー, I want WD14モデルで画像からタグを自動生成したい, so that 手作業なしにタグ付けを開始できる

#### Acceptance Criteria

1. WHEN ユーザーが単一 Image_File を指定して推論を実行する, THE TagEditor SHALL Inference_Model を用いて当該 Image_File の Tag と各 Tag の 0.0〜1.0 の範囲の Confidence を算出する
2. WHEN ユーザーが複数 Image_File を指定してバッチ推論を実行する, THE TagEditor SHALL 各 Image_File に対して順に推論を実行する
3. IF バッチ推論中に個別 Image_File の推論が失敗する, THEN THE TagEditor SHALL 当該ファイルをスキップして残りの Image_File への推論を継続し、失敗した件数を結果に記録する
4. WHEN ユーザーが 0.0〜1.0 の範囲の Confidence_Threshold を指定する, THE TagEditor SHALL Confidence が Confidence_Threshold 以上の Tag のみを採用する
5. WHEN 推論結果の出力を指示する, THE TagEditor SHALL 採用された Tag 群を対象 Image_File と同名の Tag_File へ書き込む
6. WHERE 対象 Image_File に既存の Tag_File が存在する, THE TagEditor SHALL 出力時に当該 Tag_File を推論結果で上書きする
7. IF 指定された Image_File を画像として読み込めない, THEN THE TagEditor SHALL 当該ファイルの推論をスキップし、処理できなかった旨を結果に記録する
8. WHERE 対象 Image_File が動画（mp4）である, THE TagEditor SHALL 当該ファイルを推論対象から除外する

### Requirement 15: 推論モデルの選択・取得と読み込み

**User Story:** As a ユーザー, I want 複数のONNXモデルから選んでローカル読み込みまたはダウンロードしたい, so that 用途に応じた推論モデルを準備して使い分けられる

#### Acceptance Criteria

1. WHEN ユーザーがモデル選択を開く, THE TagEditor SHALL 利用可能な Model_Variant（WD14系の各バリアント、ML-Danbooru系、および Local_Model_Dir 内で検出されたローカルモデル）の一覧を提示する
2. WHEN ユーザーが Local_Model_Dir 内のローカルモデルを選択する, THE TagEditor SHALL 当該ディレクトリの `.onnx` ファイルとタグ定義ファイル（`.csv` または `.json`）の対を読み込む
3. WHEN ユーザーがリモートの Model_Variant 取得を指示する, THE TagEditor SHALL 当該モデルの `model.onnx` とタグ定義ファイルを Model_Store からローカルへダウンロードする
4. WHEN Model_Variant のダウンロードが完了する, THE TagEditor SHALL ダウンロードしたモデルをローカルに保存し次回以降ローカルから読み込み可能にする
5. IF 選択された Model_Variant が TensorFlow専用形式（`.onnx` を持たない DeepDanbooruプロジェクト等）である, THEN THE TagEditor SHALL 当該モデルを選択対象に含めず、対象外である旨を提示する
6. IF 指定された `model.onnx` が ONNX形式として読み込めない、または対応するタグ定義ファイルが欠落している, THEN THE TagEditor SHALL エラーメッセージを表示し推論機能を無効のまま維持する
7. IF Model_Variant のダウンロードが 30 秒以内に完了しない, THEN THE TagEditor SHALL 最大3回まで再試行し、それでも失敗する場合は失敗した旨のエラーメッセージを表示する
8. IF ダウンロードした Model_Variant のローカル保存に失敗する, THEN THE TagEditor SHALL 失敗した旨のエラーメッセージを表示し推論機能を無効のまま維持する

### Requirement 16: 非機能要件（メモリ・アーキテクチャ・対応プラットフォーム）

**User Story:** As a ユーザー, I want 軽量なネイティブアプリとして主にWindowsで動作させたい, so that リソースの限られた環境でも快適に使える

#### Acceptance Criteria

1. THE TagEditor SHALL コアロジック（ファイル操作・タグ処理・推論）を Rust で実装する
2. THE TagEditor SHALL UI を Tauri を通じてOSの WebView 上でレンダリングする
3. THE TagEditor SHALL Windows を第一級サポート対象とし、全機能を Windows 上で提供する
4. WHEN ユーザーが Windows 上でアプリケーションを起動する, THE TagEditor SHALL ネイティブデスクトップアプリケーションとして起動し、10秒以内に操作可能なメインウィンドウを表示する
5. WHERE 対象OSが提供する WebView と Rust ツールチェインが利用可能である, THE TagEditor SHALL macOS および Linux でもベストエフォートでビルドおよび起動でき、中核機能（画像閲覧・タグ編集・ファイル仕訳・タグ自動生成）を提供する
6. WHERE 実行環境が Windows 以外である, THE TagEditor SHALL Windows_Only_Feature を機能一覧およびUIから除外する
7. WHILE バッチ推論または大量ファイル仕訳などの長時間処理を実行している, THE TagEditor SHALL UI操作への応答を維持し、処理の進捗状態を表示する
8. IF 長時間処理の実行中にメモリ確保またはファイルアクセスが失敗する, THEN THE TagEditor SHALL 処理を中断し、失敗内容を示すエラー表示を行い、処理前の状態を保持する

### Requirement 17: バッチ推論の高速化

**User Story:** As a ユーザー, I want 大量画像のタグ自動生成を高速に実行したい, so that データセット全体のタグ付けを短時間で終えられる

#### Acceptance Criteria

1. WHEN ユーザーが複数 Image_File を指定してバッチ推論を実行する, THE TagEditor SHALL 画像の読み込みと前処理を複数のスレッドで並列に実行する
2. WHEN バッチ推論を実行する, THE TagEditor SHALL 前処理済み画像を Batch_Size 単位でまとめて1回の推論呼び出しとして Inference_Model へ入力する
3. WHERE ユーザーが Batch_Size を指定する, THE TagEditor SHALL 1 以上 64 以下の範囲で指定された Batch_Size を用いる
4. IF ユーザーが Batch_Size を指定しない, THEN THE TagEditor SHALL 既定の Batch_Size として 8 を用いる
5. IF 指定された Batch_Size が 1 未満または 64 超である, THEN THE TagEditor SHALL 範囲内に収まる値へ丸めるか、無効である旨を提示して実行しない
6. WHILE バッチ推論を実行している, THE TagEditor SHALL 処理済み件数と総件数からなる進捗を UI に表示する
7. WHILE バッチ推論を実行している, THE TagEditor SHALL ユーザーによる処理のキャンセル操作を受け付け、キャンセル時は未処理の Image_File への推論を中止する
8. IF バッチ内の個別 Image_File の前処理または推論が失敗する, THEN THE TagEditor SHALL 当該ファイルを当該バッチから除外して残りの処理を継続し、失敗件数を結果に記録する
