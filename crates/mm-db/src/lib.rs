//! SQLite 存储层：建表/迁移、文件/乐器/关联表读写、扫描批次与去重队列。

pub mod migrations;
pub mod repository;

pub use repository::{DbError, FileRecord, Repository, ScannedFileInput, ScannedInstrumentInput};
