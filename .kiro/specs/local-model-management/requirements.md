# Requirements Document

## Introduction

本機能は、既存 TagEditor（Tauri + Rust、`ort` による ONNX 推論タガー）の「モデル選択・取得・読み込み」の仕組みを再設計し、あわせて推論結果に対するタグフィルタ（残す/残さないタグの制御）とバッチ後のタグ一覧表示を追加する。

再設計の柱は次の 6 点である。

1. **リモート実行の廃止**: 推論は常に、モデル配置フォルダ内に保存済みのローカルモデルに対してのみ実行する。リモートリポジトリ上のモデルを直接指して推論することは行わない。
2. **固定モデル配置フォルダ**: モデルの保存先を、アプリのインストールディレクトリ配下の固定フォルダに統一する（任意ディレクトリ指定は行わない）。
3. **推論時の遅延ダウンロード**: 推論実行時、選択モデルが配置フォルダに未取得なら、まず配置フォルダへダウンロードしてから推論する。
4. **モデル管理タブでの明示ダウンロード**: モデル管理タブに各モデルのダウンロードボタンを設け、推論とは独立に事前取得できる。
5. **タグフィルタ機能**: 参照実装 wd14-tagger の使用感をベースに、推論結果に対して「残すタグ・残さないタグ」を制御するフィルタ（keep / exclude / search→replace / 追加タグ / 信頼度閾値 / バッチ内出現割合閾値）を提供する。
6. **バッチ後のタグ一覧表示**: バッチ推論後に、採用されたタグと採用されなかった（除外・閾値落ち）タグの一覧を確信度付きで提示し、一覧から keep/exclude のフィルタ設定へ送れるようにする。

選択可能なモデルの種類は、参照実装 stable-diffusion-webui-wd14-tagger（`tagger/utils.py`）に定義されたモデル（WD14 系各バリアント、ML-Danbooru 系）のうち、現行 TagEditor の仕組み（ONNX 形式 + `selected_tags.csv`/タグ定義対応、WD14 前処理規約）で利用できるものを取り込んだ **Model_Catalog** として提供する。

タグフィルタとタグ一覧の使用感は、参照実装 stable-diffusion-webui-wd14-tagger（`tagger/uiset.py` の `QData` フィルタ、`tagger/ui.py` の採用/除外タグ 2 タブ表示・タグ検索・「表示中タグを keep/exclude へ移動」操作）をベースとする。

本ドキュメントは、既存 tag-editor spec の要件 15 系（推論モデルの選択・取得・読み込み）を置き換え、あわせて推論結果のタグフィルタ・一覧提示を規定する位置づけである。

なお以下は本要件では確定させず、設計フェーズで詰める前提とする。

- インストールディレクトリの具体的な解決方法（Tauri のリソース/実行ファイルパス基準か、OS 別の扱いか）
- Model_Catalog に最終的に載せる具体的モデル集合
- v3 系タガー（EVA02-Large v3 等）の取り込み可否

要件レベルでは「アプリのインストールディレクトリ配下の固定モデル配置フォルダ」「wd14-tagger 由来のモデルを取り込んだカタログ」という水準で記述する。

## Glossary

- **TagEditor**: 本アプリケーション全体。Tauri ベースのデスクトップアプリ。
- **Core**: Rust で実装されるコアロジック層（ファイル操作、タグ処理、推論、モデル取得を担う）。
- **UI**: WebView 上で動作するフロントエンド層。ユーザー操作を受け付け Core を呼び出す。
- **Inference_Model**: ONNX 形式の画像タグ付けモデル。
- **Model_Variant**: Model_Catalog に登録された、選択可能な個別の Inference_Model の定義。識別子・表示名・系統・取得元・モデル配置フォルダ内での格納レイアウトを持つ。
- **Model_Catalog**: 選択可能な Model_Variant の集合。wd14-tagger 由来のモデル（WD14 系各バリアント・ML-Danbooru 系）のうち、TagEditor の仕組みで利用できるものを取り込む。
- **Model_Family**: Model_Variant の系統。WD14 系または ML-Danbooru 系。
- **Model_Dir**: アプリのインストールディレクトリ配下に置かれる、固定のモデル配置フォルダ。取得済みモデルはこのフォルダ配下に保存される。
- **Model_Assets**: 1 つの Model_Variant を推論に用いるために必要なファイル一式。少なくとも 1 つの `.onnx` ファイルと、対応するタグ定義ファイル（`.csv` または `.json`）で構成される。
- **Tag_Definition_File**: モデルの出力インデックスとタグ名（およびカテゴリ）を対応付けるファイル（`selected_tags.csv` 等の `.csv`、または `.json`）。
- **Model_Present**: ある Model_Variant の Model_Assets が Model_Dir 配下に揃っており、ローカルから読み込み可能な状態。
- **Not_Present**: ある Model_Variant が Model_Present でない状態。Model_Assets が Model_Dir 配下に揃っておらずローカルから読み込めない。
- **Model_Source**: Model_Variant の取得元（HuggingFace Hub 等のリモートリポジトリと、リポジトリ内のファイル指定）。取得（ダウンロード）にのみ用い、推論では用いない。
- **Download_Operation**: 特定の Model_Variant の Model_Assets を Model_Source から Model_Dir へ取得する処理。遅延ダウンロードと明示ダウンロードの双方を含む。
- **Model_Management_Tab**: UI 上でモデルの一覧・取得状態・ダウンロード操作を提供する画面。
- **Confidence_Threshold**: タグを採用するかを判定する信頼度の下限値（0.0〜1.0）。この値未満の確信度のタグは、Keep_Tags に含まれない限り採用しない。
- **Predicted_Tag**: 推論によりある画像に対して得られた、タグ名と確信度（0.0〜1.0）の組。
- **Tag_Filter**: Predicted_Tag の集合から最終的に採用する Adopted_Tags を決めるための一連の設定。Keep_Tags・Exclude_Rules・Replace_Rules・Additional_Tags・Confidence_Threshold・Fraction_Threshold から成る。
- **Keep_Tags**: 常に採用するタグ名の集合。Confidence_Threshold 未満であっても、また Fraction_Threshold 未満であっても採用する（Exclude_Rules の除外より優先する）。
- **Exclude_Rules**: 除外対象のタグを指定する規則の集合。各規則はタグ名の完全一致パターン（正規表現）で表現し、マッチした Predicted_Tag を採用対象から外す。
- **Replace_Rules**: タグ名を書き換える規則の集合。各規則は検索パターン（正規表現）と置換文字列の対で表現し、採用判定前にタグ名へ適用する。
- **Additional_Tags**: 推論結果に関わらず、採用結果へ無条件に加えるタグ名の集合。
- **Fraction_Threshold**: バッチ推論において、あるタグが「対象画像数のうち何割で出現したか」の下限値（0.0〜1.0）。この割合未満でしか出現しなかったタグは、Keep_Tags/Additional_Tags でない限り採用しない。
- **Adopted_Tags**: Tag_Filter を適用した後に、ある画像の Tag_File へ書き込まれる最終的なタグ集合。
- **Discarded_Tags**: 推論で得られたが Tag_Filter により採用されなかったタグの集合（Exclude_Rules に該当・Confidence_Threshold 未満・Fraction_Threshold 未満などの理由による）。
- **Tag_Overview**: バッチ推論後に提示する、Adopted_Tags と Discarded_Tags の一覧。各タグは名称と代表確信度を持ち、UI 上で一覧・検索・フィルタ設定への送出の対象となる。

## Requirements

### Requirement 1: モデルカタログの提供

**User Story:** As a ユーザー, I want 利用可能なモデルの一覧を見たい, so that 用途に応じた推論モデルを選べる

#### Acceptance Criteria

1. WHEN ユーザーが Model_Management_Tab を開く, THE TagEditor SHALL Model_Catalog に登録された全 Model_Variant の一覧を、各項目の表示名を含めて提示する
2. IF ユーザーが Model_Management_Tab を開いた時点で Model_Catalog に登録された Model_Variant が 0 件である, THEN THE TagEditor SHALL 空の一覧と登録済みモデルが存在しない旨を示す表示を提示する
3. THE TagEditor SHALL Model_Catalog の各 Model_Variant に対し、識別子・表示名・Model_Family の 3 つのフィールドをいずれも空でない値として保持する
4. IF ある Model_Variant について識別子・表示名・Model_Family のいずれかが欠落または空である, THEN THE TagEditor SHALL 当該 Model_Variant を Model_Catalog に登録せず、除外した旨を示す情報を保持する
5. THE Model_Catalog SHALL wd14-tagger 由来のモデル（WD14 系の各バリアントおよび ML-Danbooru 系）のうち、ONNX 形式と Tag_Definition_File を伴い WD14 前処理規約で読み込めるものを含む
6. WHERE 1 つのリモートリポジトリが複数の `.onnx` ファイルを含む, THE Model_Catalog SHALL 各 `.onnx` ファイルを個別の Model_Variant として登録する
7. THE TagEditor SHALL Model_Catalog の全 Model_Variant にわたり識別子を一意に割り当てる
8. IF 2 つ以上の Model_Variant に対して同一の識別子が生成される, THEN THE TagEditor SHALL 一意になるよう識別子を区別し、Model_Catalog 内に重複する識別子が残らないようにする

### Requirement 2: 固定モデル配置フォルダ

**User Story:** As a ユーザー, I want 取得したモデルが決まった場所に保存されてほしい, so that 保存先を意識せずにモデルを再利用できる

#### Acceptance Criteria

1. THE TagEditor SHALL Model_Dir をアプリのインストールディレクトリ配下の固定相対パスとして解決し、解決結果を絶対パスとして返す
2. WHEN Download_Operation の開始時に Model_Dir が存在しない, THE TagEditor SHALL 推論またはダウンロードのデータ書き込みに先立って Model_Dir を作成する
3. THE TagEditor SHALL 取得済みの各 Model_Variant の Model_Assets を、Model_Variant ごとに一意な配置となるよう Model_Dir 配下に保存する
4. IF Model_Dir の作成に失敗する, THEN THE TagEditor SHALL 作成に失敗した旨を示すエラーメッセージを表示し、当該 Download_Operation を中止し、Model_Dir 配下に当該 Download_Operation が作成した部分的データを残さない
5. IF Model_Dir またはその配下への書き込み権限が無い, THEN THE TagEditor SHALL 書き込み不可である旨を示すエラーメッセージを表示し、当該 Download_Operation を中止する
6. IF Download_Operation が完了前に中止される, THEN THE TagEditor SHALL 当該 Model_Variant について Model_Dir 配下に保存した部分的な Model_Assets を削除し、当該 Model_Variant を未取得の状態に戻す

### Requirement 3: モデルのローカル存在判定

**User Story:** As a ユーザー, I want どのモデルが取得済みかを知りたい, so that 事前ダウンロードやダウンロードの要否を判断できる

#### Acceptance Criteria

1. WHEN ユーザーが Model_Management_Tab を開く, THE TagEditor SHALL 定義済みの各 Model_Variant について Model_Dir 配下を走査し、Model_Present または Not_Present のいずれか一意の状態を各 Model_Variant に対して表示する
2. THE TagEditor SHALL ある Model_Variant の `.onnx` ファイルと、拡張子が `.csv` または `.json` のいずれかである対応 Tag_Definition_File との対（Model_Assets 一式）が Model_Dir 配下に揃っている場合に限り、当該 Model_Variant を Model_Present と判定する
3. IF ある Model_Variant の `.onnx` は存在するが対応する Tag_Definition_File が Model_Dir 配下に欠落している, THEN THE TagEditor SHALL 当該 Model_Variant を Model_Present と判定しない
4. IF ある Model_Variant の Tag_Definition_File は存在するが `.onnx` が Model_Dir 配下に欠落している, THEN THE TagEditor SHALL 当該 Model_Variant を Model_Present と判定しない
5. IF Model_Dir が存在しない, THEN THE TagEditor SHALL すべての Model_Variant を Not_Present として表示し、Model_Dir が未作成である旨を示す表示を提示する
6. IF Model_Dir は存在するが Model_Assets を一切含まない, THEN THE TagEditor SHALL すべての Model_Variant を Not_Present として表示する

### Requirement 4: 推論時の遅延ダウンロード

**User Story:** As a ユーザー, I want 選んだモデルが未取得でも推論を実行したい, so that 事前準備なしにタグ生成を始められる

#### Acceptance Criteria

1. WHEN ユーザーが Model_Present でない Model_Variant を指定して推論を実行する, THE TagEditor SHALL 推論に先立って当該 Model_Variant の Download_Operation を実行し、当該 Download_Operation の進行中である旨を表示する
2. WHEN 遅延ダウンロードが完了して当該 Model_Variant が Model_Present になる, THE TagEditor SHALL Model_Dir 配下の Model_Assets を読み込んで推論を実行する
3. WHEN ユーザーが既に Model_Present である Model_Variant を指定して推論を実行する, THE TagEditor SHALL Download_Operation を行わず Model_Dir 配下の Model_Assets を読み込んで推論を実行する
4. IF 遅延ダウンロードが失敗する, THEN THE TagEditor SHALL 失敗した旨のエラーメッセージを表示し、推論を実行せず、当該 Model_Variant を Model_Present にしないまま Model_Dir の状態を変更しない
5. IF 遅延ダウンロード完了後に Model_Dir 配下の Model_Assets の読み込みに失敗する, THEN THE TagEditor SHALL 読み込みに失敗した旨のエラーメッセージを表示し、推論を実行しない

### Requirement 5: モデル管理タブでの明示ダウンロード

**User Story:** As a ユーザー, I want モデルを事前にダウンロードしたい, so that 推論時の待ち時間を避けオフラインでも使える状態にできる

#### Acceptance Criteria

1. THE Model_Management_Tab SHALL 各 Model_Variant にダウンロード操作の要素を提示する
2. WHEN ユーザーがある Model_Variant のダウンロードを指示する, THE TagEditor SHALL 当該 Model_Variant の Download_Operation を推論処理とは独立に（推論を阻害せず）実行する
3. WHILE Download_Operation が進行中である, THE TagEditor SHALL 当該 Model_Variant のダウンロード進捗を 0〜100% の完了割合として、1 秒以内の間隔で更新し UI に表示する
4. WHEN ユーザーが進行中の Download_Operation のキャンセルを指示する, THE TagEditor SHALL 当該 Download_Operation を中止し、Model_Dir 配下に当該 Download_Operation が生成した未完了の Model_Assets を残さない
5. WHEN ある Model_Variant の Download_Operation が正常に完了する, THE TagEditor SHALL 当該 Model_Variant を Model_Present として表示に反映する
6. IF Download_Operation がネットワーク到達不能・接続中断・保存先ディスク容量不足のいずれかにより失敗する, THEN THE TagEditor SHALL 当該 Download_Operation を中止し、Model_Dir 配下に当該操作が生成した未完了の Model_Assets を残さず、失敗を示すエラー表示を UI に提示する
7. WHERE 既に Model_Present である Model_Variant に対してダウンロードが指示される, THE TagEditor SHALL 当該 Model_Assets を再取得して上書きする
8. IF Model_Present である Model_Variant の上書き用 Download_Operation が失敗する, THEN THE TagEditor SHALL 上書き前の既存 Model_Assets を保持し、当該 Model_Variant の Model_Present 状態を維持したうえで、失敗を示すエラー表示を UI に提示する

### Requirement 6: ダウンロードの堅牢性（タイムアウト・再試行・原子的保存）

**User Story:** As a ユーザー, I want ダウンロードが失敗しても壊れたモデルが残らないでほしい, so that 再試行で確実にモデルを揃えられる

#### Acceptance Criteria

1. IF Download_Operation 中の個々のファイル取得が 30 秒以内に完了しない, THEN THE TagEditor SHALL 当該取得を中断し、初回を含めて最大 3 回まで取得を試行する
2. IF 個々のファイル取得が 3 回すべて 30 秒以内に完了せず失敗する, THEN THE TagEditor SHALL 当該 Download_Operation を失敗として終了する
3. IF Download_Operation が失敗として終了する, THEN THE TagEditor SHALL 失敗した旨と失敗したファイル名を含むエラーメッセージを表示する
4. WHEN Download_Operation が対象全ファイルの取得に成功する, THE TagEditor SHALL 取得内容を Model_Dir 配下へ原子的に確定させ、確定後に全対象ファイルが Model_Dir 配下に存在する状態にする
5. IF Download_Operation が失敗またはキャンセルされる, THEN THE TagEditor SHALL 当該 Download_Operation で Model_Dir 配下に作成した部分ファイルをすべて除去し、Model_Dir 配下を Download_Operation 開始前の状態に戻す
6. IF ネットワークに接続できず Download_Operation を開始できない, THEN THE TagEditor SHALL ネットワーク不通である旨のエラーメッセージを表示し、当該 Download_Operation を実行せずに中止する

### Requirement 7: リモート実行の廃止

**User Story:** As a ユーザー, I want 推論が常にローカルのモデルで行われてほしい, so that 推論のたびにネットワークへ依存しないで済む

#### Acceptance Criteria

1. THE TagEditor SHALL 全ての推論を Model_Dir 配下に保存された Model_Assets に対してのみ実行し、Model_Dir 配下以外の場所にあるモデルデータを推論に用いない
2. THE TagEditor SHALL Model_Source をモデルの取得（Download_Operation）にのみ用い、推論には用いない
3. IF ユーザーが推論を実行し、対象 Model_Variant がリモートリポジトリ上にのみ存在して Model_Dir 配下に Model_Assets が揃っていない, THEN THE TagEditor SHALL リモートリポジトリ上のモデルを直接参照した推論を行わず、推論を実行しない
4. WHEN ユーザーが Model_Present である Model_Variant を指定して推論を実行し、その時点でネットワークに接続できない, THE TagEditor SHALL Model_Source へのアクセスを行わず Model_Dir 配下の Model_Assets のみで推論を実行して完了する
5. IF 推論の実行時に対象 Model_Variant の Model_Assets が Model_Dir 配下に存在しない, THEN THE TagEditor SHALL 対象モデルがローカルに存在しない旨のエラーメッセージを表示し、推論を実行しない

### Requirement 8: モデル読み込みと異常系

**User Story:** As a ユーザー, I want モデルの不備が明確に通知されてほしい, so that 原因を把握して対処できる

#### Acceptance Criteria

1. WHEN 対象 Model_Variant が Model_Present であり読み込みが要求される, THE TagEditor SHALL Model_Dir 配下の `.onnx` と対応する Tag_Definition_File を読み込み、読み込み完了後に当該 Model_Variant を推論に用いる
2. IF Model_Dir 配下の `.onnx` が ONNX 形式として読み込めない, THEN THE TagEditor SHALL ONNX の読み込みに失敗した原因を識別できる情報を含むエラーメッセージを表示し、推論を開始せず既存状態を保持する
3. IF 対象 Model_Variant の Tag_Definition_File が読み込めない、または解析できない, THEN THE TagEditor SHALL タグ定義の読み込みまたは解析に失敗した原因を識別できる情報を含むエラーメッセージを表示し、推論を開始せず既存状態を保持する
4. IF 取得または読み込みの過程で Model_Dir 配下への書き込みに失敗する, THEN THE TagEditor SHALL 書き込みに失敗した原因を識別できる情報を含むエラーメッセージを表示し、推論を開始せず既存状態を保持する

### Requirement 9: タグフィルタ設定

**User Story:** As a ユーザー, I want どのタグを残しどのタグを残さないかを制御したい, so that 用途に合ったタグだけを Tag_File に出力できる

#### Acceptance Criteria

1. THE TagEditor SHALL 推論の実行に用いる Tag_Filter として、Keep_Tags・Exclude_Rules・Replace_Rules・Additional_Tags・Confidence_Threshold・Fraction_Threshold を設定できるようにする
2. WHEN ユーザーが Confidence_Threshold を設定する, THE TagEditor SHALL 確信度が Confidence_Threshold 以上の Predicted_Tag のみを採用候補とする
3. WHERE ある Predicted_Tag のタグ名が Keep_Tags に含まれる, THE TagEditor SHALL 当該タグを、Confidence_Threshold 未満・Fraction_Threshold 未満・Exclude_Rules に該当のいずれであっても Adopted_Tags に採用する
4. WHERE ある Predicted_Tag のタグ名が Exclude_Rules のいずれかにマッチし、かつ Keep_Tags に含まれない, THE TagEditor SHALL 当該タグを Adopted_Tags に採用せず Discarded_Tags に含める
5. WHEN Replace_Rules が設定されている, THE TagEditor SHALL 採用判定に先立って、各 Predicted_Tag のタグ名に検索パターンがマッチする場合は対応する置換文字列へ書き換える
6. THE TagEditor SHALL Additional_Tags に含まれる各タグを、推論結果に含まれるか否かに関わらず Adopted_Tags に加える
7. IF Exclude_Rules または Replace_Rules の検索パターンが正規表現として無効である, THEN THE TagEditor SHALL 当該パターンが無効である旨のエラーメッセージを表示し、当該パターンを推論の採用判定に適用しない
8. WHERE 同一のタグ名が Keep_Tags と Exclude_Rules の双方に該当する, THE TagEditor SHALL 当該タグを採用する（Keep_Tags を Exclude_Rules より優先する）
9. THE TagEditor SHALL Adopted_Tags を、対応する各画像の Tag_File へ書き込む対象とする

### Requirement 10: バッチ内出現割合による絞り込み

**User Story:** As a ユーザー, I want バッチ全体でまれにしか付かないタグを落としたい, so that ノイズの少ない共通タグに整理できる

#### Acceptance Criteria

1. WHEN ユーザーがバッチ推論を Fraction_Threshold を指定して実行する, THE TagEditor SHALL 各タグについて、そのタグを採用候補とした画像数をバッチの対象画像数で割った出現割合を算出する
2. WHERE あるタグの出現割合が Fraction_Threshold 未満であり、かつ当該タグが Keep_Tags にも Additional_Tags にも含まれない, THE TagEditor SHALL 当該タグを全画像の Adopted_Tags から除き Discarded_Tags に含める
3. WHERE Fraction_Threshold が 0 に設定されている, THE TagEditor SHALL 出現割合による除外を行わない
4. THE TagEditor SHALL Fraction_Threshold による絞り込みを、バッチ推論に限って適用し、単一画像推論では適用しない

### Requirement 11: バッチ後のタグ一覧表示

**User Story:** As a ユーザー, I want バッチ実行後にどんなタグが付いたかを一覧で確認したい, so that フィルタの調整や結果の把握ができる

#### Acceptance Criteria

1. WHEN バッチ推論が完了する, THE TagEditor SHALL 当該バッチの Adopted_Tags と Discarded_Tags を Tag_Overview として提示する
2. THE Tag_Overview SHALL 各タグについてタグ名と代表確信度を表示し、Adopted_Tags と Discarded_Tags を区別して提示する
3. WHEN ユーザーが Tag_Overview に対して検索文字列を入力する, THE TagEditor SHALL 検索文字列にマッチするタグのみに Tag_Overview の表示を絞り込む
4. WHEN ユーザーが Tag_Overview に表示中のタグを Keep_Tags へ送る操作を行う, THE TagEditor SHALL 当該の表示中タグを Keep_Tags に追加する
5. WHEN ユーザーが Tag_Overview に表示中のタグを Exclude_Rules へ送る操作を行う, THE TagEditor SHALL 当該の表示中タグを Exclude_Rules に追加する
6. WHEN ユーザーが Tag_Overview から Keep_Tags または Exclude_Rules を更新した後に同一バッチに対して推論を再実行する, THE TagEditor SHALL 更新後の Tag_Filter を適用した Adopted_Tags と Discarded_Tags を Tag_Overview に反映する
