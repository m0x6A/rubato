//! Calls to the speech provider, bounded and observed (plan 146 design §3–§5): at most
//! `PROVIDER_CONCURRENCY` at once, STT and TTS together, a permit waited for at most
//! `PROVIDER_QUEUE_SECS`; each call spanned and counted with its outcome and duration. The clip
//! cache keeps synthesized replies in memory only, at most `CLIP_CACHE_ENTRIES` for
//! `CLIP_CACHE_SECS`, one synthesis per message whatever the number of listeners.

use crate::kernel::speech::{Speech, SpeechClip, SpeechError};
use crate::kernel::state::App;
use chrono::{DateTime, Duration, Utc};
use opentelemetry::KeyValue;
use rubato_protocol::otel::{attr, span};
use rubato_protocol::voice::limits;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tokio::sync::{OnceCell, Semaphore};
use tracing::Instrument;
use uuid::Uuid;

/// The provider permits, one per server (kept in `Slots`).
pub struct Gate(pub Semaphore);

impl Default for Gate {
    fn default() -> Self {
        Self(Semaphore::new(limits::PROVIDER_CONCURRENCY))
    }
}

/// Why a provider call did not give an answer.
#[derive(Debug)]
pub enum CallError {
    /// No permit within `PROVIDER_QUEUE_SECS`.
    Busy,
    Provider(SpeechError),
}

/// One provider call under the gate: `op` is `stt` or `tts`.
async fn gated<T>(
    app: &App,
    speech: &Arc<dyn Speech>,
    op: &'static str,
    room: &str,
    bytes: usize,
    call: impl std::future::Future<Output = Result<T, SpeechError>>,
) -> Result<T, CallError> {
    let gate = app.slots.get::<Gate>();
    let provider = speech.name();
    let record = |outcome: &'static str, secs: Option<f64>| {
        let kv = [
            KeyValue::new(attr::VOICE_OP, op),
            KeyValue::new(attr::OUTCOME, outcome),
            KeyValue::new(attr::VOICE_PROVIDER, provider),
        ];
        app.metrics.voice_speech.add(1, &kv);
        if let Some(s) = secs {
            app.metrics
                .voice_duration
                .record(s, &[KeyValue::new(attr::VOICE_OP, op)]);
        }
    };
    let permit = match tokio::time::timeout(
        std::time::Duration::from_secs(limits::PROVIDER_QUEUE_SECS),
        gate.0.acquire(),
    )
    .await
    {
        Ok(Ok(p)) => p,
        _ => {
            record("busy", None);
            return Err(CallError::Busy);
        }
    };
    let s = if op == "stt" {
        tracing::info_span!(
            span::VOICE_STT,
            rubato.voice.provider = provider,
            rubato.room = room,
            rubato.voice.bytes = bytes,
            rubato.outcome = tracing::field::Empty
        )
    } else {
        tracing::info_span!(
            span::VOICE_TTS,
            rubato.voice.provider = provider,
            rubato.room = room,
            rubato.voice.bytes = bytes,
            rubato.outcome = tracing::field::Empty
        )
    };
    let started = std::time::Instant::now();
    let out = call.instrument(s.clone()).await;
    drop(permit);
    let secs = started.elapsed().as_secs_f64();
    match &out {
        Ok(_) => {
            s.record("rubato.outcome", "ok");
            record("ok", Some(secs));
        }
        Err(e) => {
            s.record("rubato.outcome", e.outcome());
            tracing::warn!(rubato.room = room, rubato.voice.op = op, error = %e, "speech call failed");
            record(e.outcome(), Some(secs));
        }
    }
    out.map_err(CallError::Provider)
}

/// Transcribe one utterance under the gate.
pub async fn transcribe(
    app: &App,
    speech: &Arc<dyn Speech>,
    room: &str,
    audio: Vec<u8>,
    mime: &str,
) -> Result<String, CallError> {
    let n = audio.len();
    let s = speech.clone();
    gated(app, speech, "stt", room, n, async move {
        s.transcribe(audio, mime, None).await
    })
    .await
}

/// Count a transcript that came back empty.
pub fn record_empty(app: &App, speech: &Arc<dyn Speech>) {
    app.metrics.voice_speech.add(
        1,
        &[
            KeyValue::new(attr::VOICE_OP, "stt"),
            KeyValue::new(attr::OUTCOME, "empty"),
            KeyValue::new(attr::VOICE_PROVIDER, speech.name()),
        ],
    );
}

type Cell = Arc<OnceCell<SpeechClip>>;

/// Synthesized clips by message id, in memory only.
#[derive(Default)]
pub struct Clips(Mutex<HashMap<Uuid, (DateTime<Utc>, Cell)>>);

impl Clips {
    /// The cell for `id`, made if absent, after evicting expired entries and the oldest past the
    /// size limit.
    fn cell(&self, id: Uuid, now: DateTime<Utc>) -> Cell {
        let mut m = self.0.lock().unwrap();
        let ttl = Duration::seconds(limits::CLIP_CACHE_SECS);
        m.retain(|_, (at, _)| now - *at < ttl);
        if let Some((_, c)) = m.get(&id) {
            return c.clone();
        }
        while m.len() >= limits::CLIP_CACHE_ENTRIES {
            let oldest = m.iter().min_by_key(|(_, (at, _))| *at).map(|(k, _)| *k);
            match oldest {
                Some(k) => m.remove(&k),
                None => break,
            };
        }
        let c: Cell = Arc::default();
        m.insert(id, (now, c.clone()));
        c
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.0.lock().unwrap().len()
    }

    #[cfg(test)]
    pub fn contains(&self, id: Uuid) -> bool {
        self.0.lock().unwrap().contains_key(&id)
    }
}

/// The clip of message `id`: from the cache, or one synthesis of `cut` that every concurrent
/// listener waits on. A failed synthesis is not kept.
pub async fn clip(
    app: &App,
    speech: &Arc<dyn Speech>,
    room: &str,
    id: Uuid,
    cut: &str,
) -> Result<SpeechClip, CallError> {
    let clips = app.slots.get::<Clips>();
    let cell = clips.cell(id, app.clock.now());
    cell.get_or_try_init(|| async {
        let s = speech.clone();
        let text = cut.to_owned();
        gated(app, speech, "tts", room, cut.len(), async move {
            s.synthesize(&text).await
        })
        .await
    })
    .await
    .cloned()
}
