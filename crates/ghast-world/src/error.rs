use thiserror::Error;

use crate::ChunkPos;

/// Malformed world data from the server. The affected column has been unloaded.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum WorldError {
    /// Chunk data is shorter than its section mask requires.
    #[error("chunk {pos:?}: data too short ({len} bytes, need {need})")]
    Truncated {
        /// Column.
        pos: ChunkPos,
        /// Bytes received.
        len: usize,
        /// Bytes required.
        need: usize,
    },
    /// A 1.8 map_chunk_bulk whose data does not cover all listed columns.
    #[error("map_chunk_bulk: data too short ({len} bytes, need {need})")]
    BulkTruncated {
        /// Bytes received.
        len: usize,
        /// Bytes required.
        need: usize,
    },
    /// A 1.16 section is malformed.
    #[error("chunk {pos:?} section {section}: {reason}")]
    Section {
        /// Column.
        pos: ChunkPos,
        /// Section index (`y >> 4`).
        section: u8,
        /// What is wrong.
        reason: &'static str,
    },
}
