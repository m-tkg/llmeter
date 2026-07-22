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

キャッシュは `~/.cache/llmeter/` に保存され、ソースファイルの path + mtime + size をキーに無効化されます。

## コスト計算

- モデル単価テーブルをバイナリに内蔵（Anthropic はキャッシュ書込 1.25x / キャッシュ読取 0.1x を反映）
- `~/.config/llmeter/pricing.toml` で単価の上書き・新モデルの追加が可能
- 未知モデルのコストは「不明」として合計から分離表示

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

構成: `src/sources/`（各ツールのパーサ）、`src/aggregate.rs`（集計）、`src/pricing.rs`（コスト計算）、`src/insights.rs`（気づき生成）、`src/render/`（HTML / Markdown 出力）、`src/cache.rs`（増分キャッシュ）。

## ライセンス

MIT
