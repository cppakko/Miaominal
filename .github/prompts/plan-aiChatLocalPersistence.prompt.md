# Plan: AI Chat 本地持久化（SQLite + AES-GCM 内容加密）

## TL;DR
在 `miaominal-storage` 新增 `chat_store` 模块，用标准 rusqlite（bundled）存储聊天历史，对 content/tool_summary 列复用现有 AES-256-GCM 加密（`credential_backend.rs`），密钥存 keyring；新增 `ChatService` 服务层；UI 中 AI Chat 面板升为一级入口，内部有会话列表（二级）→ 对话视图导航。不做导出按钮。

---

## 数据库 Schema

```sql
CREATE TABLE chat_sessions (
    id TEXT PRIMARY KEY,
    title TEXT NOT NULL DEFAULT '',
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE TABLE chat_messages (
    id TEXT PRIMARY KEY,
    session_id TEXT NOT NULL REFERENCES chat_sessions(id) ON DELETE CASCADE,
    role TEXT NOT NULL,       -- 'user' | 'assistant' | 'tool_call' | 'thinking'
    content BLOB NOT NULL,    -- AES-GCM encrypted
    tool_name TEXT,
    tool_summary BLOB,        -- AES-GCM encrypted, nullable
    sort_order INTEGER NOT NULL,
    created_at INTEGER NOT NULL
);
CREATE INDEX idx_messages_session ON chat_messages(session_id, sort_order);
```

DB 文件: `paths::config_file("chat_history.db")`（明文结构，内容加密）
密钥: keyring service=`dev.akko.miaominal`, account=`chat-db-key`，首次启动生成 32 字节随机 key，之后复用。
加密: 复用 `credential_backend.rs` 中 `encrypt_with_aad` / `decrypt_with_aad`（AES-256-GCM），AAD = `b"miaominal.chat.v1"`。
解密失败时该条消息 content = `[无法解密]`，不阻断加载。

---

## Phase 1 — 存储层（miaominal-storage）

1. 根 `Cargo.toml` workspace.dependencies 新增：
   `rusqlite = { version = "0.32", features = ["bundled"] }`

2. `crates/miaominal-storage/Cargo.toml` 引入：
   `rusqlite.workspace = true`（keyring 已是 workspace dep，按需添加）

3. 新建 `crates/miaominal-storage/src/chat_store.rs`：
   - `ChatMessageRole` enum：User/Assistant/ToolCall/Thinking → TEXT 序列化
   - `ChatSessionRecord` { id: String, title: String, created_at: i64, updated_at: i64 }
   - `ChatMessageRecord` { id, session_id, role, content, tool_name, tool_summary, sort_order, created_at }
   - `ChatStore(Connection)`
   - `ChatStore::open(path: &Path) -> Result<Self>`：open + run_migrations
   - `fn encrypt_text(key: &[u8;32], plain: &str) -> Result<Vec<u8>>`：调用 credential_backend encrypt_with_aad，AAD=`b"miaominal.chat.v1"`
   - `fn decrypt_text(key: &[u8;32], cipher: &[u8]) -> Option<String>`：失败返回 None
   - `create_session(id, now) -> Result<()>`
   - `update_session_title(id, title) -> Result<()>`（title 不加密，用于列表展示）
   - `list_sessions() -> Result<Vec<ChatSessionRecord>>`
   - `insert_message(record: &ChatMessageRecord, key: &[u8;32]) -> Result<()>`
   - `load_session_messages(session_id, key: &[u8;32]) -> Result<Vec<ChatMessageRecord>>`
   - `delete_session(id) -> Result<()>`

4. `crates/miaominal-storage/src/lib.rs` 新增 `pub mod chat_store;`

**依赖**：`credential_backend.rs` 中 `encrypt_with_aad`/`decrypt_with_aad` 需从 `miaominal-secrets` pub(crate) 升为 pub 或在 storage 内复制实现（storage 已依赖 secrets，优先 re-export）。

---

## Phase 2 — 服务层（miaominal-services）

5. 新建 `crates/miaominal-services/src/services/chat_service.rs`：
   - `ChatService { store: ChatStore, key: [u8;32] }`
   - `ChatService::open(secrets: &CredentialStore) -> Result<Self>`：
     从 keyring 取 `chat-db-key`；不存在则生成 32 字节随机 → 存入 keyring；
     调用 `ChatStore::open(paths::config_file("chat_history.db")?)`
   - 公开方法全部委托 store（传入 key）

6. `crates/miaominal-services/src/lib.rs` 导出 `ChatService`

7. `crates/miaominal-services/src/services/app_services.rs`：
   - `AppServices` 新增 `pub chat_service: Option<ChatService>`
   - `AppServices::load()` 中调用 `ChatService::open(&secrets.credentials)`，失败 warn + None

---

## Phase 3 — 状态绑定（miaominal-ui）

8. `state.rs` `SessionAgentState` 新增 `pub session_id: Option<String>`
   `AppView` 新增：
   - `chat_sessions: Vec<ChatSessionRecord>`
   - `chat_panel_view: ChatPanelView` enum { SessionList, Conversation }

9. `bootstrap.rs` 启动时 `list_sessions()` → `chat_sessions`

10. `finish_session_agent_stream`（session_agent.rs）：
    - `session_id` 为 None：`uuid::Uuid::new_v4()` 生成 id，`create_session(id, now)` → 存入 `session_agent.session_id`
    - 批量 `insert_message` 所有 messages（User/Assistant/ToolCall/Thinking），sort_order 按索引
    - 刷新 `chat_sessions` 列表

11. 标题更新：bootstrap.rs 中订阅标题 input 的地方（L529）→ 同步 `update_session_title()`

12. `reset_session_agent_chat` → `session_agent.session_id = None`

13. 加载历史：点击列表项 → `load_session_messages()` → 还原 `Vec<SessionAgentMessage>` → 切换 Conversation 视图

14. 删除会话：`delete_session(id)` → 刷新列表

---

## Phase 4 — UI（miaominal-ui）

15. AI Chat 面板顶部：
    - `Conversation` 视图：左上角"＜ 会话列表"返回按钮 + 标题
    - `SessionList` 视图：标题栏"新建对话"按钮 + 会话列表（每项：title / 相对时间）+ 删除按钮

16. 参考现有 Sessions / Keychain 面板 list+detail 模式（Bootstrap 初始化 + 面板切换）

---

## 关键文件

- 根 `Cargo.toml` — 新增 rusqlite workspace dep
- `crates/miaominal-storage/Cargo.toml` — 引入 rusqlite
- `crates/miaominal-storage/src/chat_store.rs` — 新建
- `crates/miaominal-storage/src/lib.rs` — 导出
- `crates/miaominal-secrets/src/credential_backend.rs` — encrypt/decrypt 函数改为 pub
- `crates/miaominal-services/src/services/chat_service.rs` — 新建
- `crates/miaominal-services/src/lib.rs` — 导出
- `crates/miaominal-services/src/services/app_services.rs` — 新增字段
- `crates/miaominal-ui/src/ui/shell/state.rs` — 新增字段
- `crates/miaominal-ui/src/ui/shell/actions/session_agent.rs` — 持久化触发
- `crates/miaominal-ui/src/ui/shell/bootstrap.rs` — 初始化 + 标题同步

---

## 验证

1. `cargo build` 编译通过
2. 新建对话 → 发消息 → 重启 → 会话列表显示
3. 点击历史 → 消息完整还原（含 Thinking）
4. 标题生成后 DB 更新
5. 删除会话 → 消息级联删除
6. sync payload 不含 chat 数据（无需额外修改）
7. 手动篡改 DB content 列 → 加载时显示 `[无法解密]` 而非 crash

---

## 不做的事

- 导出按钮（聊天记录含潜在敏感输出，暂不优先）
- DB 结构加密（无需 SQLCipher）
- 自动保留上限（用户手动删除）
