//! mm-core：编排层（扫描 / 去重 / 查询），不依赖任何 UI 框架。

pub mod duplicate;
pub mod query;
pub mod scan;
pub mod service;

use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum CoreError {
    #[error("数据库错误: {0}")]
    Db(#[from] mm_db::DbError),
    #[error("SQLite 错误: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("MIDI 解析错误: {0}")]
    Parse(#[from] midi_scan::ParseError),
    #[error("IO 错误: {0}")]
    Io(#[from] std::io::Error),
    #[error("目录不存在: {0}")]
    DirNotFound(PathBuf),
    #[error("{0}")]
    Other(String),
}
