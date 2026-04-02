// ── Session module ───────────────────────────────────────────────────────────
//
// 管理会话的创建、存储、恢复和清理。
//
// 模块结构：
//   - store.rs: 会话文件的读写、列表、清理
//   - restore.rs: 恢复会话状态、解析会话内容

mod restore;
mod store;

pub use store::SessionStore;
