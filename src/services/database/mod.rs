mod connection;
#[cfg(test)]
mod compatibility_tests;
mod favorite_repository;
mod history_repository;
mod migrations;
mod models;
mod note_repository;
pub mod query;
mod repository;
mod tag_repository;

pub use connection::{Database, DatabaseError};
#[cfg(test)]
pub(crate) use connection::DatabaseConnection;
pub use favorite_repository::{FavoriteDeleteError, FavoriteReorderError, FavoriteRepository};
pub use history_repository::{
    HistoryCleanupError, HistoryCleanupReport, HistoryRepository, HistoryReorderError,
    HistoryRetentionCounts,
};
pub use models::DatabaseInfo;
pub use note_repository::NoteRepository;
pub(crate) use models::LIST_ORDER_GAP;
pub use tag_repository::{
    TagCreateError, TagDeleteError, TagItemDeleteError, TagItemWriteError, TagQueryError,
    TagReorderError, TagRepository, TagUpdateError,
};
