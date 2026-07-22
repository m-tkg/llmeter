# llmeter

AI コーディングツール（Claude Code / Codex / Cursor）の利用状況をローカルログから集計・可視化する CLI ツールです。

自分の使い方を統計的に把握し、プロンプト改善・不要なスキルの整理・コスト管理に役立てることを目的としています。外部サーバーへの送信は一切なく、すべてローカルで完結します。

## 機能

- **Overview** — 期間コスト / 総トークン / セッション数 / アクティブ時間のサマリーカード、ルールベースの「今週の気づき」、日別コスト積み上げグラフ（ツール別）、モデル別・リポジトリ別・日別の内訳
- **セッション一覧** — 初回プロンプト抜粋・ターン数・ツールエラー率・所要時間・コストのテーブル（コスト降順）
- **セッション詳細** — トランスクリプト表示とイベントタイムライン
- **出力形式** — 自己完結型のダークテーマ HTML ダッシュボード（インライン CSS + SVG チャート、単一ファイルでブラウザ表示可）と Markdown
- **増分キャッシュ** — 約 1GB のログでも 2 回目以降は変更ファイルのみ再パース

## 対応データソース

| ツール | 読み取り元 | 備考 |
|---|---|---|
| Claude Code | `~/.claude/projects/**/*.jsonl` | トークン・コストとも正確に集計 |
| Codex | `~/.codex/sessions/YYYY/MM/DD/rollout-*.jsonl` | `token_count` イベントから集計 |
| Cursor | `~/.cursor/chats/<workspace>/<session>/` | ログにトークン情報がないため文字数からの概算（estimated 表示） |

ログの読み取りのみを行い、元ファイルは変更しません。

## インストール

```bash
cargo install --git https://github.com/m-tkg/llmeter
```

またはソースから:

```bash
git clone https://github.com/m-tkg/llmeter
cd llmeter
cargo install --path .
```

### アップデート

```bash
llmeter update           # リモートと版比較し、新しければ再ビルドしてインストール
llmeter update --force   # 同バージョンでも強制再インストール
```

内部で `cargo install --git ... --force` を実行するため cargo が必要です（ビルドに1〜2分）。

## 使い方

### レポート生成（HTML）

```bash
llmeter report                         # 直近30日を ./llmeter-report/ に出力
llmeter report --days 7                # 直近7日
llmeter report --out ~/reports/ai      # 出力先を指定
llmeter report --tools claude,codex    # 対象ツールを絞る（claude / codex / cursor）
```

ブラウザで `./llmeter-report/index.html` を開くとダッシュボードが表示されます。

### レポート生成（Markdown）

`--format md` を付けるだけで、同じ内容を Markdown で出力します。

```bash
llmeter report --format md                     # ./llmeter-report/report.md に出力
llmeter report --format md --out ./docs/usage  # 出力先を指定
```

### レポートの出力構成

| ファイル | 内容 |
|---|---|
| `index.html` / `report.md` | Overview（コスト・トークン・気づき・グラフ）+ セッション一覧 |
| `sessions/<ID>.html` / `sessions/<ID>.md` | 各セッションの詳細トランスクリプト（一覧からリンク） |

`report` の全オプション:

| オプション | デフォルト | 説明 |
|---|---|---|
| `--days <N>` | `30` | 集計対象期間（日数） |
| `--format <html\|md>` | `html` | 出力形式 |
| `--out <DIR>` | `./llmeter-report/` | 出力先ディレクトリ（なければ作成） |
| `--tools <LIST>` | 全ツール | `claude,codex,cursor` のカンマ区切りで限定 |
| `--offline` | オフ | ネットワークアクセスなしで実行（LiteLLM 料金データはキャッシュ+埋め込みのみ使用） |
| `--analyze <AGENT>` | 省略時は分析なし | レポートを AI エージェント CLI に読ませ、コスト削減提案をマージする（`claude` / `codex` / `cursor`） |
| `--analyze-timeout <SECS>` | `300` | `--analyze` 実行時のタイムアウト（秒） |

### AI 分析（--analyze）

`--analyze <agent>` を付けると、生成したレポート（Markdown）を指定した AI エージェント CLI に読ませ、コスト削減提案を生成してレポート本体にマージします。

```bash
llmeter report --analyze claude                          # claude -p でレポートを分析
llmeter report --format md --analyze codex                # codex exec で分析、Markdown 出力
llmeter report --analyze cursor --analyze-timeout 600     # cursor-agent、タイムアウトを延長
```

- 対応エージェント: `claude`（`claude -p`）、`codex`（`codex exec`）、`cursor`（`cursor-agent -p`）。各 CLI がインストール済み・認証済みであることが前提です
- レポート Markdown を stdin で渡し、エージェントの出力（Markdown、`### 分析サマリー` / `### コスト削減提案` / `### 利用パターンの気づき` の3節）を「今週の気づき」の直後にマージします（Markdown 出力では `## AI 分析（<agent>）` セクション、HTML 出力では同トーンのカード）
- エージェント未インストール・実行失敗・タイムアウト・出力が空のいずれかの場合は stderr に警告を出し、分析なしで通常のレポートを出力します（レポート生成自体は失敗しません）

### セッション一覧（ターミナル表示）

```bash
llmeter sessions                          # 直近30日、コスト降順
llmeter sessions --repo llmeter           # リポジトリで絞り込み
llmeter sessions --sort turns             # cost | turns | errors
```

### セッション詳細

セッション ID は `llmeter sessions` の表示や、HTML レポートのリンク先ファイル名で確認できます。

```bash
llmeter session <セッションID>                              # Markdown で標準出力
llmeter session <セッションID> --format html > session.html # HTML をファイルに保存
```

### キャッシュ操作

```bash
llmeter cache status   # キャッシュ状態の確認
llmeter cache clear    # キャッシュ削除（次回はフルパース）
```

キャッシュは OS 標準のキャッシュディレクトリ（macOS: `~/Library/Caches/llmeter/`、Linux: `~/.cache/llmeter/`）に保存され、ソースファイルの path + mtime + size をキーに無効化されます。

## コスト計算

料金解決は ccusage と同様、3層構造で行います。

1. **pricing.toml**（ユーザー上書き、モデル名との部分一致、最優先）
2. **LiteLLM 料金データベース**（[BerriAI/litellm](https://github.com/BerriAI/litellm) が公開する `model_prices_and_context_window.json` をキャッシュして利用。完全一致 → プロバイダ接頭辞除去後の完全一致 → 最長プレフィックス一致の順で照合）
3. **埋め込みデフォルト**（`src/pricing.rs` の `embedded_defaults()`。ネットワーク不可・LiteLLM 未収録時の fallback）

未知モデルのコストは「不明」として合計から分離表示します。

### LiteLLM 料金データベース

- キャッシュ先: `<OS標準キャッシュディレクトリ>/llmeter/litellm_prices.json`（macOS: `~/Library/Caches/llmeter/`）
- TTL: 7日。`report` / `sessions` / `session` 実行時、キャッシュが TTL 切れなら自動で再取得（タイムアウト10秒）。取得に失敗した場合は古いキャッシュ、それも無ければ埋め込みデフォルトにフォールバックします（stderr に警告）
- `--offline` フラグ（`report` / `sessions` / `session` 共通）を付けるとネットワークアクセスを一切行わず、キャッシュ + 埋め込みデフォルトのみで動作します
- `llmeter pricing refresh` — TTL を無視して強制的に再取得
- `llmeter pricing show <model>` — 指定モデルがどの層で解決され、単価がいくらかを表示（デバッグ用）

```bash
llmeter pricing refresh
llmeter pricing show claude-sonnet-5-20260115
llmeter report --days 7 --offline
```

### 内蔵デフォルト単価（$ / 100万トークン）

| パターン | input | output | cache_write | cache_read |
|---|---|---|---|---|
| `claude-opus` | 15.0 | 75.0 | input×1.25 | input×0.1 |
| `claude-sonnet` | 3.0 | 15.0 | input×1.25 | input×0.1 |
| `claude-haiku` | 0.8 | 4.0 | input×1.25 | input×0.1 |
| `claude-fable` | 3.0 | 15.0 | input×1.25 | input×0.1 |
| `gpt-5` | 5.0 | 15.0 | 5.0 | 0.5 |
| `gpt-4.1` | 2.0 | 8.0 | 2.0 | 0.5 |
| `gpt-4o` | 2.5 | 10.0 | 2.5 | 1.25 |
| `o3` | 2.0 | 8.0 | input×1.25 | input×0.1 |
| `o4-mini` | 1.1 | 4.4 | input×1.25 | input×0.1 |

パターンはモデル名との**部分一致**（小文字比較）。例えば `claude-sonnet` は `claude-sonnet-5-20260115` にマッチします。単価は公式価格と異なる場合があるため、正確なコストが必要な場合は `pricing.toml` で上書きしてください。

### pricing.toml の書式

配置場所は `~/.config/llmeter/pricing.toml`（全 OS 共通で優先）。無い場合は OS 標準の設定ディレクトリ（macOS: `~/Library/Application Support/llmeter/pricing.toml`、Linux: `~/.config/llmeter/pricing.toml`）を読みます。

```toml
# キー = モデル名の部分一致パターン。ユーザー定義は内蔵デフォルトより優先。
# 単価はすべて $ / 100万トークン。

# 既存パターンの単価を上書き
[models."claude-sonnet"]
input = 3.0
output = 15.0
# cache_write / cache_read は省略可
# 省略時: cache_write = input × 1.25、cache_read = input × 0.1

# 新モデルの追加
[models."gpt-5.6"]
input = 5.0
output = 15.0
cache_write = 5.0
cache_read = 0.5
```

- 必須キー: `input` / `output`
- 省略可: `cache_write` / `cache_read`（省略時は input からの倍率で自動計算）
- 複数の層でマッチする場合、pricing.toml → LiteLLM → 内蔵デフォルトの順で最初のマッチを採用

## 既知の制限

- アクティブ時間はセッションの開始〜終了の単純合計のため、日をまたぐ resume セッションでは過大になる（アイドル時間の除外は未実装）
- Cursor はトークン・コストとも概算のみ
- Codex で 1 セッション中にモデルを切り替えた場合、モデル別内訳は最後のモデルに寄る（合計コストは正確）
- サブエージェントの会話が別セッションファイルとして記録されるため、セッション数はやや多めに出る

## 開発

```bash
cargo test     # 単体テスト
cargo clippy   # lint
```

構成: `src/sources/`（各ツールのパーサ）、`src/aggregate.rs`（集計）、`src/pricing.rs`（コスト計算・3層解決）、`src/litellm.rs`（LiteLLM 料金データの取得・キャッシュ）、`src/analyze.rs`（AI 分析エージェント実行・md→html変換）、`src/insights.rs`（気づき生成）、`src/render/`（HTML / Markdown 出力）、`src/cache.rs`（増分キャッシュ）。

## ライセンス

MIT
