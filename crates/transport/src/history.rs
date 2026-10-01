//! One history response path for all endpoints. Adapters supply authorized
//! snapshots and a revocable permission token; framing and body validation
//! cannot vary by platform or reception target.
use crate::{Result, valid_metadata, write_frame};
use shuttli_model::{
    mobile::HistoryListResponse,
    sync::{EventId, Metadata},
};
use shuttli_protocol::FrameV2;
use std::sync::Arc;
use tokio::io::{AsyncWrite, AsyncWriteExt};

pub enum Export {
    List(HistoryListResponse),
    Body {
        event: EventId,
        metadata: Metadata,
        bytes: Arc<[u8]>,
    },
}

pub fn error_code(error: &str) -> &'static str {
    match error {
        "Disabled" | "history_denied" => "history_denied",
        "stale history cursor" | "history_cursor_stale" => "history_cursor_stale",
        "history_invalid_request" => "history_invalid_request",
        _ => "history_unavailable",
    }
}

/// The token is captured when the adapter authorizes the snapshot. Check it
/// before replying and throughout a body; revocation closes a partial stream.
pub async fn respond<S: AsyncWrite + Unpin>(
    stream: &mut S,
    result: Result<(u64, Export)>,
    permitted: impl Fn(u64) -> bool,
) -> Result<()> {
    let (token, export) = match result {
        Ok((token, export)) if permitted(token) => (token, export),
        result => {
            return write_frame(
                stream,
                &FrameV2::Error {
                    code: result
                        .err()
                        .map_or("history_denied", |e| error_code(&e))
                        .into(),
                },
            )
            .await;
        }
    };
    match export {
        Export::List(page) => write_frame(stream, &page.into()).await,
        Export::Body {
            event,
            metadata,
            bytes,
        } => {
            if !valid_metadata(&metadata)
                || metadata.size != bytes.len() as u64
                || shuttli_content::canonical_digest(metadata.format, &bytes).ok()
                    != Some(metadata.digest)
            {
                return write_frame(
                    stream,
                    &FrameV2::Error {
                        code: "history_unavailable".into(),
                    },
                )
                .await;
            }
            write_frame(stream, &FrameV2::HistoryBody { event, metadata }).await?;
            for chunk in bytes.chunks(65_536) {
                if !permitted(token) {
                    return Err("history export permission changed".into());
                }
                stream.write_all(chunk).await.map_err(|e| e.to_string())?;
            }
            stream.flush().await.map_err(|e| e.to_string())
        }
    }
}
