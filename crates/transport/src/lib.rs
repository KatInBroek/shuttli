//! Platform-neutral live transfer mechanics. Native adapters decide where a
//! verified reception is committed; only that completion permits APPLIED.
use shuttli_model::sync::{DeviceId, EventId, Format, Metadata};
use shuttli_protocol::FrameV2;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

pub mod history;

pub async fn write_hello(
    stream: &mut (impl AsyncWrite + Unpin),
    hello: &shuttli_protocol::Hello,
) -> Result<()> {
    let bytes = shuttli_protocol::encode_hello(hello).map_err(str::to_owned)?;
    stream
        .write_u32(bytes.len() as u32)
        .await
        .map_err(|e| e.to_string())?;
    stream.write_all(&bytes).await.map_err(|e| e.to_string())?;
    stream.flush().await.map_err(|e| e.to_string())
}

pub async fn read_hello(stream: &mut (impl AsyncRead + Unpin)) -> Result<shuttli_protocol::Hello> {
    let len = stream.read_u32().await.map_err(|e| e.to_string())? as usize;
    if len == 0 || len > shuttli_protocol::MAX_CONTROL_FRAME_BYTES {
        return Err("invalid hello size".into());
    }
    let mut bytes = vec![0; len];
    stream
        .read_exact(&mut bytes)
        .await
        .map_err(|e| e.to_string())?;
    shuttli_protocol::decode_hello(&bytes).map_err(str::to_owned)
}

pub type Result<T> = std::result::Result<T, String>;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum WireVersion {
    V1,
    V2,
}

pub async fn write_live_frame(
    stream: &mut (impl AsyncWrite + Unpin),
    frame: &FrameV2,
    version: WireVersion,
) -> Result<()> {
    if version == WireVersion::V2 {
        return write_frame(stream, frame).await;
    }
    use shuttli_protocol::FrameV1;
    let old = match frame {
        FrameV2::Offer { event, meta } => FrameV1::Offer {
            event: *event,
            meta: meta.clone(),
        },
        FrameV2::Ready => FrameV1::Ready,
        FrameV2::Applied => FrameV1::Applied,
        FrameV2::Poll => FrameV1::Poll,
        FrameV2::Idle => FrameV1::Idle,
        FrameV2::Select { initiator } => FrameV1::Select {
            initiator: *initiator,
        },
        FrameV2::Status { event } => FrameV1::Status { event: *event },
        FrameV2::Receipt { event, state } => FrameV1::Receipt {
            event: *event,
            state: *state,
        },
        FrameV2::Error { code } => FrameV1::Error {
            message: code.clone(),
        },
        _ => return Err("operation requires wire v2".into()),
    };
    let bytes = serde_json::to_vec(&old).map_err(|e| e.to_string())?;
    if bytes.len() > shuttli_protocol::MAX_CONTROL_FRAME_BYTES {
        return Err("oversized v1 frame".into());
    }
    stream
        .write_u32(bytes.len() as u32)
        .await
        .map_err(|e| e.to_string())?;
    stream.write_all(&bytes).await.map_err(|e| e.to_string())?;
    stream.flush().await.map_err(|e| e.to_string())
}

pub async fn read_live_frame(
    stream: &mut (impl AsyncRead + Unpin),
    version: WireVersion,
) -> Result<FrameV2> {
    if version == WireVersion::V2 {
        return read_frame(stream).await;
    }
    let len = stream.read_u32().await.map_err(|e| e.to_string())? as usize;
    if len == 0 || len > shuttli_protocol::MAX_CONTROL_FRAME_BYTES {
        return Err("invalid v1 frame size".into());
    }
    let mut bytes = vec![0; len];
    stream
        .read_exact(&mut bytes)
        .await
        .map_err(|e| e.to_string())?;
    use shuttli_protocol::FrameV1;
    let old: FrameV1 = serde_json::from_slice(&bytes).map_err(|_| "invalid v1 frame")?;
    let frame = match old {
        FrameV1::Offer { event, meta } => FrameV2::Offer { event, meta },
        FrameV1::Ready => FrameV2::Ready,
        FrameV1::Applied => FrameV2::Applied,
        FrameV1::Poll => FrameV2::Poll,
        FrameV1::Idle => FrameV2::Idle,
        FrameV1::Select { initiator } => FrameV2::Select { initiator },
        FrameV1::Status { event } => FrameV2::Status { event },
        FrameV1::Receipt { event, state } => FrameV2::Receipt { event, state },
        FrameV1::Error { .. } => FrameV2::Error {
            code: "offer_rejected".into(),
        },
    };
    frame.encode().map_err(str::to_owned)?;
    Ok(frame)
}

/// A single request turn prevents simultaneous OFFERs and history requests
/// from consuming each other's replies or raw payloads. Identity order is
/// symmetric and independent of OS, form factor and connection initiator.
pub fn leads_session(own: DeviceId, peer: DeviceId) -> bool {
    own < peer
}

/// Unsolicited notices do not finish a request turn or acknowledge a transfer.
pub fn validate_notice(frame: &FrameV2, own: DeviceId, peer: DeviceId) -> Result<()> {
    match frame {
        FrameV2::PeerList { revision, peers }
            if shuttli_protocol::valid_peer_list(
                &shuttli_model::mobile::PeerList {
                    revision: *revision,
                    peers: peers.clone(),
                },
                peer,
                own,
            ) =>
        {
            Ok(())
        }
        FrameV2::HistoryChanged { revision } if *revision > 0 => Ok(()),
        _ => Err("invalid session notice".into()),
    }
}

pub async fn write_frame(stream: &mut (impl AsyncWrite + Unpin), frame: &FrameV2) -> Result<()> {
    let bytes = frame.encode().map_err(str::to_owned)?;
    stream
        .write_u32(bytes.len() as u32)
        .await
        .map_err(|e| e.to_string())?;
    stream.write_all(&bytes).await.map_err(|e| e.to_string())?;
    stream.flush().await.map_err(|e| e.to_string())
}

pub async fn read_frame(stream: &mut (impl AsyncRead + Unpin)) -> Result<FrameV2> {
    FrameReader::default().read(stream).await
}

/// Partial control bytes remain owned across cancellation by select! timers.
#[derive(Default)]
pub struct FrameReader {
    header: [u8; 4],
    header_len: usize,
    bytes: Vec<u8>,
    body_len: usize,
}
impl FrameReader {
    pub fn is_idle(&self) -> bool {
        self.header_len == 0
    }
    pub async fn read(&mut self, stream: &mut (impl AsyncRead + Unpin)) -> Result<FrameV2> {
        while self.header_len < 4 {
            let n = stream
                .read(&mut self.header[self.header_len..])
                .await
                .map_err(|e| e.to_string())?;
            if n == 0 {
                return Err("session closed".into());
            }
            self.header_len += n;
        }
        if self.bytes.is_empty() {
            let len = u32::from_be_bytes(self.header) as usize;
            if len == 0 || len > shuttli_protocol::MAX_CONTROL_FRAME_BYTES {
                return Err("invalid control frame size".into());
            }
            self.bytes.resize(len, 0);
        }
        while self.body_len < self.bytes.len() {
            let n = stream
                .read(&mut self.bytes[self.body_len..])
                .await
                .map_err(|e| e.to_string())?;
            if n == 0 {
                return Err("session closed".into());
            }
            self.body_len += n;
        }
        let bytes = std::mem::take(&mut self.bytes);
        self.header_len = 0;
        self.body_len = 0;
        FrameV2::decode(&bytes).map_err(str::to_owned)
    }
}

pub fn valid_metadata(meta: &Metadata) -> bool {
    meta.size > 0
        && meta.size <= 8 * 1024 * 1024
        && (meta.format != Format::Text || meta.size <= 1024 * 1024)
}

#[derive(Debug, PartialEq, Eq)]
pub enum SendOutcome {
    /// Local validation failed before an OFFER was written. The turn is still owned.
    NotOffered,
    Applied,
    Rejected,
}

pub struct LiveOffer<'a> {
    pub event: EventId,
    pub metadata: &'a Metadata,
    pub body: &'a [u8],
}

async fn reply<S, N>(stream: &mut S, notice: &mut N, version: WireVersion) -> Result<FrameV2>
where
    S: AsyncRead + AsyncWrite + Unpin,
    N: FnMut(FrameV2) -> Result<()>,
{
    loop {
        match read_live_frame(stream, version).await? {
            frame @ (FrameV2::PeerList { .. } | FrameV2::HistoryChanged { .. }) => notice(frame)?,
            frame @ (FrameV2::Ready | FrameV2::Applied | FrameV2::Error { .. }) => {
                return Ok(frame);
            }
            _ => return Err("unexpected transfer reply".into()),
        }
    }
}

/// OFFER/READY/body/APPLIED is shared by every v2 sender. Notices may be
/// consumed while awaiting replies, but never replace a delivery receipt.
pub async fn send_live<S, P, N, G>(
    stream: &mut S,
    event: EventId,
    meta: &Metadata,
    body: &[u8],
    permitted: P,
    mut notice: N,
    sending: G,
) -> Result<SendOutcome>
where
    S: AsyncRead + AsyncWrite + Unpin,
    P: Fn() -> bool,
    N: FnMut(FrameV2) -> Result<()>,
    G: FnOnce(),
{
    send_live_version(
        stream,
        LiveOffer {
            event,
            metadata: meta,
            body,
        },
        permitted,
        &mut notice,
        sending,
        WireVersion::V2,
    )
    .await
}

pub async fn send_live_version<S, P, N, G>(
    stream: &mut S,
    offer: LiveOffer<'_>,
    permitted: P,
    mut notice: N,
    sending: G,
    version: WireVersion,
) -> Result<SendOutcome>
where
    S: AsyncRead + AsyncWrite + Unpin,
    P: Fn() -> bool,
    N: FnMut(FrameV2) -> Result<()>,
    G: FnOnce(),
{
    let LiveOffer {
        event,
        metadata: meta,
        body,
    } = offer;
    if !permitted()
        || !valid_metadata(meta)
        || meta.size != body.len() as u64
        || shuttli_content::canonical_digest(meta.format, body).ok() != Some(meta.digest)
    {
        return Ok(SendOutcome::NotOffered);
    }
    write_live_frame(
        stream,
        &FrameV2::Offer {
            event,
            meta: meta.clone(),
        },
        version,
    )
    .await?;
    match reply(stream, &mut notice, version).await? {
        FrameV2::Ready => (),
        FrameV2::Error { .. } => return Ok(SendOutcome::Rejected),
        _ => return Err("missing READY".into()),
    }
    sending();
    for chunk in body.chunks(65_536) {
        if !permitted() {
            return Err("send permission changed".into());
        }
        stream.write_all(chunk).await.map_err(|e| e.to_string())?;
    }
    stream.flush().await.map_err(|e| e.to_string())?;
    match reply(stream, &mut notice, version).await? {
        FrameV2::Applied => Ok(SendOutcome::Applied),
        FrameV2::Error { .. } => Ok(SendOutcome::Rejected),
        _ => Err("missing APPLIED".into()),
    }
}

/// Called only after the adapter grants READY. Never expose partial or
/// unverified bytes. Revocation during a payload closes the session.
pub async fn receive_body<S: AsyncRead + Unpin>(
    stream: &mut S,
    meta: &Metadata,
    permitted: impl Fn() -> bool,
) -> Result<Vec<u8>> {
    if !valid_metadata(meta) || !permitted() {
        return Err("receive denied".into());
    }
    let mut body = vec![0; meta.size as usize];
    for chunk in body.chunks_mut(65_536) {
        stream.read_exact(chunk).await.map_err(|e| e.to_string())?;
        if !permitted() {
            return Err("receive permission changed".into());
        }
    }
    if shuttli_content::canonical_digest(meta.format, &body).ok() != Some(meta.digest) {
        return Err("content integrity mismatch".into());
    }
    Ok(body)
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
