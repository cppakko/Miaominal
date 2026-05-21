# Miaominal Crates 能力细拆分阶段计划

## Summary
目标是把当前单 crate `miaominal` 拆成一个私有 workspace，按“模型/设置/存储/能力/服务/UI/启动器”分层，并进一步把 terminal、ssh、sftp、sync 拆成独立能力 crate。每一阶段都必须保持可编译、可回滚，最终根 package 只保留二进制入口、打包元数据和启动胶水。

最终 crate 图：
`core/settings/paths -> secrets/storage/terminal/ssh/sync/sftp -> services -> ui -> miaominal bin`

## Public APIs
- `miaominal-core`: `SessionProfile`、`PortForwardRule`、`SnippetRecord`、`ManagedKeyRecord`、`KnownHostEntry`、`SftpEntry`、`TransferId` 等纯领域类型。
- `miaominal-settings`: `AppSettings`、`SyncedSettings`、`Theme`、`ThemeId`、主题生成、字体默认值、全局 settings 状态；GPUI 组件主题同步留在 UI。
- `miaominal-secrets`: `SecretKind`、`SecretStore`、`CredentialStore`、`VaultCredentialBackend`。
- `miaominal-storage`: `SessionStore`、`SnippetStore`、`SettingsStore`、`KnownHostsStore`、`ManagedKeyStore`。
- `miaominal-terminal`: `TerminalState`、`TerminalSnapshot`、`TerminalCell`、输入编码、粘贴清洗、鼠标报告、终端常量。
- `miaominal-ssh`: `start_session`、`start_port_forward_session`、`execute_profile_command`、`SessionConnection`、`SessionCommandSender`、`SessionEvent`，并公开 SFTP 需要的认证/ProxyJump 辅助接口。
- `miaominal-sftp`: `start_session`、`SftpConnection`、`SftpCommandSender`、`SftpEvent`。
- `miaominal-sync`: `SyncEngine`、`SyncConfigStore`、`SyncConfig`、`SyncProvider`、`SyncStatus`、同步 payload/encryption/provider 能力。
- `miaominal-services`: 现有 profile/settings/keychain/terminal/sftp/sync service 和 `AppServices`。
- `miaominal-ui`: `AppView`、assets、i18n、components、shell。
- 持久化文件格式不改：`sessions.toml`、`snippets.toml`、`settings.toml`、`managed_keys.toml`、`known_hosts`、vault/sync payload 都保持兼容。

## Phased Plan
1. 基线冻结  
   记录当前 `cargo check --workspace --all-targets`、`cargo test --workspace` 状态；确认 `Cargo.toml` 仍是单 package、根 `src/main.rs` 是唯一 bin 入口；建立拆分分支。之后每阶段单独提交，失败时只回滚当前阶段。

2. Workspace 骨架  
   在根 `Cargo.toml` 增加 `[workspace]`、`[workspace.dependencies]`，保留根 package 名称 `miaominal` 和现有 `build.rs`/bundle metadata。创建 `crates/*` 目录和空 `lib.rs`，所有新 crate `publish = false`、edition 2024，先不搬代码，只让 workspace 编译行为稳定。

3. 先解循环依赖  
   抽出 `miaominal-paths` 承接当前 `app::paths`，让 infra/storage 不再依赖 app。抽出 `miaominal-settings`，把当前 settings model 和纯 theme 逻辑放进去；把 `sync_component_theme`、`scaled_font_size -> Pixels` 这类 GPUI 辅助留给 UI 包装。完成后禁止 `domain/settings` 反向依赖 `ui::theme`，禁止 infra 依赖 `app::paths`。

4. Core 领域模型  
   把 profile、keychain、known_host 数据结构、forwarding、snippet、sftp 领域类型迁入 `miaominal-core`。`core` 只保留 serde/std 级依赖；`known_host` 里的 russh `PublicKey` 指纹/算法 helper 移到 storage/ssh 侧。UI 和 services 先改成从 `miaominal_core::*` 导入。

5. Secrets 与 Storage  
   把 credential backend、vault backend、`SecretStore`、`SecretKind` 迁入 `miaominal-secrets`。把 config store、settings store、known hosts store、managed key store 迁入 `miaominal-storage`。构造函数继续保留默认路径，同时保留测试用 `with_path`/fallback 能力。该阶段结束时 storage 不得依赖 UI、services、ssh、sftp、sync。

6. Terminal 能力包  
   把当前 terminal emulator 和 terminal-domain 输入逻辑迁入 `miaominal-terminal`。它可以依赖 `alacritty_terminal`、`gpui`、`miaominal-settings`，但不依赖 UI shell。保持 `TerminalState::default()`、snapshot、search、mouse report、paste/input encoding 行为不变；SSH 需要的最小列数常量由 terminal crate 暴露或下沉为纯常量。

7. SSH 与 SFTP 能力包  
   把 `infra/ssh` 迁入 `miaominal-ssh`，把 session、auth、forwarding、monitor 暴露为 crate API。随后把 `infra/sftp` 迁入 `miaominal-sftp`，依赖 `miaominal-ssh` 的认证、ProxyJump、client config 公共接口。保持事件枚举和 command sender 的外部行为不变。

8. Sync 能力包  
   把 sync model 和 `infra/sync` 迁入 `miaominal-sync`。`SyncEngine` 继续接收 storage/secrets/settings 的具体 store 类型，provider、payload、encryption 保持内部模块。同步拉取后 settings 替换必须继续更新全局 settings 状态，避免 UI 主题/字体不同步。

9. Services 与 UI  
   把 `services/*` 迁入 `miaominal-services`，只依赖 core/settings/storage/secrets/terminal/ssh/sftp/sync。最后搬 `src/ui` 到 `miaominal-ui`，提供内部 `settings` facade 以减少大规模 UI import 震荡；根 bin 只负责日志、Tokio runtime、GPUI application、窗口打开和菜单。

10. 清理兼容层  
   删除旧的 `src/domain.rs`、`src/infra.rs`、`src/services.rs`、`src/settings.rs`、`src/secrets.rs`、`src/terminal.rs` 兼容 re-export。用 `rg "crate::(domain|infra|services|settings|secrets|terminal)::"` 确认旧路径清零。收紧可见性：只有 crate 根需要的 API 设为 `pub`，内部仍用 `pub(crate)`。

## Test Plan
- 每阶段：`cargo check --workspace --all-targets`。
- 每个能力包落地后：`cargo test -p miaominal-core/settings/storage/secrets/terminal/ssh/sftp/sync/services`。
- 全量结束：`cargo fmt --check`、`cargo clippy --workspace --all-targets -- -D warnings`、`cargo test --workspace`。
- 手工烟测：启动应用、加载旧配置、修改设置主题/字体、创建 SSH 会话、HostKey 提示、SFTP 列目录/传输、sync push/pull、vault lock/unlock、managed key import/generate。
- 打包回归：Windows MSI 脚本和 macOS DMG 脚本路径不变；根 package metadata 继续服务 bundle。

## Assumptions
- 选择“能力细拆”，接受比稳妥分层更多 crate 和更多 import 调整。
- 所有 crate 都是 workspace 私有 crate，暂不面向 crates.io 发布。
- 不改变用户数据格式、不改 UI 行为、不重写业务逻辑；本次目标是边界拆分和依赖方向整理。
