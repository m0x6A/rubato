//! Speech behind one port (ADR 0073, plan 146 design §5): `transcribe` and `synthesize`. The
//! `openai_compatible` adapter speaks the `/audio/transcriptions` and `/audio/speech` shape that
//! local servers (Speaches, faster-whisper-server, Kokoro-FastAPI, whisper.cpp's server) and
//! hosted APIs share; the provider is configuration. The `fake` adapter serves tests and the dev
//! server. An adapter to someone else's tool, so it lives in the kernel and names no slice. It
//! never starts or connects to LiveKit, and keeps no audio.

use crate::kernel::config::{OpenAiSpeech, SpeechConfig};
use async_trait::async_trait;
use rubato_protocol::voice::limits;
use std::sync::Arc;
use std::time::Duration;

/// One synthesized clip: its bytes and their media type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpeechClip {
    pub bytes: Vec<u8>,
    pub mime: String,
}

/// Why a speech call failed; each is a `rubato.outcome` and a 502 to the browser.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SpeechError {
    #[error("the speech provider did not answer within {0} s")]
    Timeout(u64),
    #[error("the speech provider answered {0}")]
    Status(u16),
    #[error("the speech provider could not be reached: {0}")]
    Transport(String),
    #[error("the speech provider's answer did not read: {0}")]
    BadBody(String),
}

impl SpeechError {
    /// `rubato.outcome` on `rubato.voice.speech`.
    pub fn outcome(&self) -> &'static str {
        match self {
            Self::Timeout(_) => "timeout",
            Self::Status(_) => "status",
            Self::Transport(_) => "transport",
            Self::BadBody(_) => "bad_body",
        }
    }
}

#[async_trait]
pub trait Speech: Send + Sync {
    /// `openai_compatible` or `fake`: `rubato.voice.provider`.
    fn name(&self) -> &'static str;
    /// Turn one utterance into text. The bytes are dropped when the call returns.
    async fn transcribe(
        &self,
        audio: Vec<u8>,
        mime: &str,
        language: Option<&str>,
    ) -> Result<String, SpeechError>;
    /// Speak `text`.
    async fn synthesize(&self, text: &str) -> Result<SpeechClip, SpeechError>;
}

/// The adapter a configuration names.
pub fn from_config(c: &SpeechConfig) -> Arc<dyn Speech> {
    match c {
        SpeechConfig::Fake => Arc::new(FakeSpeech),
        SpeechConfig::OpenAiCompatible(o) => Arc::new(OpenAiCompatible::new(o.clone())),
    }
}

/// Tests and the dev server only (`RUBATO_SPEECH=fake` needs `RUBATO_DEV_USER`): the "audio" is
/// UTF-8 text, and a clip is the text's bytes.
pub struct FakeSpeech;

#[async_trait]
impl Speech for FakeSpeech {
    fn name(&self) -> &'static str {
        "fake"
    }

    async fn transcribe(
        &self,
        audio: Vec<u8>,
        _mime: &str,
        _language: Option<&str>,
    ) -> Result<String, SpeechError> {
        String::from_utf8(audio)
            .map(|s| s.trim().to_owned())
            .map_err(|e| SpeechError::BadBody(e.to_string()))
    }

    async fn synthesize(&self, text: &str) -> Result<SpeechClip, SpeechError> {
        Ok(SpeechClip {
            bytes: text.as_bytes().to_vec(),
            mime: "text/plain".into(),
        })
    }
}

/// `POST {stt}/audio/transcriptions` and `POST {tts}/audio/speech`.
pub struct OpenAiCompatible {
    c: OpenAiSpeech,
    http: reqwest::Client,
    deadline: Duration,
}

impl OpenAiCompatible {
    pub fn new(c: OpenAiSpeech) -> Self {
        Self::with_deadline(c, Duration::from_secs(limits::PROVIDER_DEADLINE_SECS))
    }

    pub fn with_deadline(c: OpenAiSpeech, deadline: Duration) -> Self {
        Self {
            c,
            http: reqwest::Client::new(),
            deadline,
        }
    }

    async fn send(&self, req: reqwest::RequestBuilder) -> Result<Vec<u8>, SpeechError> {
        let secs = self.deadline.as_secs().max(1);
        let call = async {
            let res = req
                .send()
                .await
                .map_err(|e| SpeechError::Transport(e.without_url().to_string()))?;
            if !res.status().is_success() {
                return Err(SpeechError::Status(res.status().as_u16()));
            }
            read_capped(res, limits::PROVIDER_BODY_MAX_BYTES).await
        };
        tokio::time::timeout(self.deadline, call)
            .await
            .map_err(|_| SpeechError::Timeout(secs))?
    }
}

/// A response body, refused past `max` bytes so a provider cannot fill the server's memory.
async fn read_capped(mut res: reqwest::Response, max: usize) -> Result<Vec<u8>, SpeechError> {
    let mut out = Vec::new();
    while let Some(chunk) = res
        .chunk()
        .await
        .map_err(|e| SpeechError::Transport(e.without_url().to_string()))?
    {
        if out.len() + chunk.len() > max {
            return Err(SpeechError::BadBody(format!("over {max} bytes")));
        }
        out.extend_from_slice(&chunk);
    }
    Ok(out)
}

fn bearer(
    req: reqwest::RequestBuilder,
    key: Option<&crate::kernel::config::Secret>,
) -> reqwest::RequestBuilder {
    match key {
        Some(k) => req.bearer_auth(k.expose()),
        None => req,
    }
}

/// `{base}{path}`, keeping a query the base carries after the path: Azure's deployment URLs need
/// `?api-version=…` (`…/openai/deployments/<name>?api-version=…` becomes
/// `…/openai/deployments/<name>/audio/transcriptions?api-version=…`).
fn endpoint(base: &str, path: &str) -> String {
    match base.split_once('?') {
        Some((head, query)) => format!("{}{path}?{query}", head.trim_end_matches('/')),
        None => format!("{}{path}", base.trim_end_matches('/')),
    }
}

/// The file name a provider expects for a media type: some read the format from it.
fn file_name(mime: &str) -> &'static str {
    let m = mime.to_ascii_lowercase();
    if m.contains("ogg") {
        "utterance.ogg"
    } else if m.contains("mp4") || m.contains("m4a") || m.contains("aac") {
        "utterance.m4a"
    } else if m.contains("wav") {
        "utterance.wav"
    } else {
        "utterance.webm"
    }
}

#[async_trait]
impl Speech for OpenAiCompatible {
    fn name(&self) -> &'static str {
        "openai_compatible"
    }

    async fn transcribe(
        &self,
        audio: Vec<u8>,
        mime: &str,
        language: Option<&str>,
    ) -> Result<String, SpeechError> {
        let part = reqwest::multipart::Part::bytes(audio)
            .file_name(file_name(mime))
            .mime_str(mime)
            .map_err(|e| SpeechError::BadBody(e.to_string()))?;
        let mut form = reqwest::multipart::Form::new()
            .part("file", part)
            .text("model", self.c.stt_model.clone())
            .text("response_format", "json");
        if let Some(l) = language.or(self.c.language.as_deref()) {
            form = form.text("language", l.to_owned());
        }
        let url = endpoint(&self.c.stt_url, "/audio/transcriptions");
        let body = self
            .send(bearer(self.http.post(url), self.c.stt_key.as_ref()).multipart(form))
            .await?;
        #[derive(serde::Deserialize)]
        struct Out {
            text: String,
        }
        serde_json::from_slice::<Out>(&body)
            .map(|o| o.text.trim().to_owned())
            .map_err(|e| SpeechError::BadBody(e.to_string()))
    }

    async fn synthesize(&self, text: &str) -> Result<SpeechClip, SpeechError> {
        let url = endpoint(&self.c.tts_url, "/audio/speech");
        let body = serde_json::json!({
            "model": self.c.tts_model,
            "input": text,
            "voice": self.c.tts_voice,
            "response_format": "mp3",
        });
        let bytes = self
            .send(bearer(self.http.post(url), self.c.tts_key.as_ref()).json(&body))
            .await?;
        if bytes.is_empty() {
            return Err(SpeechError::BadBody("an empty clip".into()));
        }
        Ok(SpeechClip {
            bytes,
            mime: "audio/mpeg".into(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::Router;
    use axum::extract::{Multipart, RawQuery, State};
    use axum::http::{HeaderMap, StatusCode};
    use axum::routing::post;
    use std::sync::Mutex;

    #[derive(Default)]
    struct Seen {
        fields: Vec<(String, String)>,
        auth: Option<String>,
        query: Option<String>,
        speech: Option<serde_json::Value>,
    }

    type Shared = Arc<Mutex<Seen>>;

    /// A local stand-in for an OpenAI-compatible server; `mode` picks how it answers.
    async fn stub(mode: &'static str) -> (String, Shared) {
        let seen: Shared = Arc::default();
        async fn stt(
            State((seen, mode)): State<(Shared, &'static str)>,
            RawQuery(query): RawQuery,
            headers: HeaderMap,
            mut form: Multipart,
        ) -> (StatusCode, String) {
            let mut s = seen.lock().unwrap().fields.clone();
            while let Ok(Some(f)) = form.next_field().await {
                let name = f.name().unwrap_or("").to_owned();
                let ct = f.content_type().unwrap_or("").to_owned();
                let file = f.file_name().map(str::to_owned);
                let value = f.text().await.unwrap_or_default();
                s.push((name.clone(), value));
                if let Some(file) = file {
                    s.push((format!("{name}.file_name"), file));
                    s.push((format!("{name}.content_type"), ct));
                }
            }
            {
                let mut g = seen.lock().unwrap();
                g.fields = s;
                g.query = query;
                g.auth = headers
                    .get("authorization")
                    .map(|v| v.to_str().unwrap().to_owned());
            }
            match mode {
                "ok" => (StatusCode::OK, r#"{"text":"  why this seam?  "}"#.into()),
                "500" => (StatusCode::INTERNAL_SERVER_ERROR, "no".into()),
                "bad" => (StatusCode::OK, "not json".into()),
                _ => {
                    tokio::time::sleep(Duration::from_secs(5)).await;
                    (StatusCode::OK, "{}".into())
                }
            }
        }
        async fn tts(
            State((seen, mode)): State<(Shared, &'static str)>,
            axum::Json(v): axum::Json<serde_json::Value>,
        ) -> (StatusCode, Vec<u8>) {
            seen.lock().unwrap().speech = Some(v);
            match mode {
                "big" => (
                    StatusCode::OK,
                    vec![0u8; limits::PROVIDER_BODY_MAX_BYTES + 1],
                ),
                _ => (StatusCode::OK, b"ID3clip".to_vec()),
            }
        }
        let app = Router::new()
            .route("/v1/audio/transcriptions", post(stt))
            .route("/v1/audio/speech", post(tts))
            .route("/v1/deployments/stt/audio/transcriptions", post(stt))
            .with_state((seen.clone(), mode));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        (format!("http://{addr}/v1"), seen)
    }

    fn adapter(base: &str, deadline: Duration) -> OpenAiCompatible {
        OpenAiCompatible::with_deadline(
            OpenAiSpeech {
                stt_url: base.into(),
                tts_url: base.into(),
                stt_key: Some(crate::kernel::config::Secret::new("sk-test")),
                tts_key: None,
                stt_model: "whisper-1".into(),
                tts_model: "kokoro".into(),
                tts_voice: "af_heart".into(),
                language: Some("sv".into()),
            },
            deadline,
        )
    }

    fn field<'a>(s: &'a Seen, name: &str) -> Option<&'a str> {
        s.fields
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    }

    #[tokio::test]
    async fn transcription_sends_the_file_model_and_language_and_reads_text() {
        let (base, seen) = stub("ok").await;
        let a = adapter(&base, Duration::from_secs(5));
        let text = a
            .transcribe(b"OggS...".to_vec(), "audio/ogg;codecs=opus", None)
            .await
            .unwrap();
        assert_eq!(text, "why this seam?");
        let s = seen.lock().unwrap();
        assert_eq!(field(&s, "model"), Some("whisper-1"));
        assert_eq!(field(&s, "language"), Some("sv"));
        assert_eq!(field(&s, "response_format"), Some("json"));
        assert_eq!(field(&s, "file.file_name"), Some("utterance.ogg"));
        assert_eq!(
            field(&s, "file.content_type"),
            Some("audio/ogg;codecs=opus")
        );
        assert_eq!(s.auth.as_deref(), Some("Bearer sk-test"));
    }

    /// An Azure deployment URL keeps its `?api-version=` after the path Rubato adds; a base
    /// without a query, or with a trailing slash, reads as before.
    #[tokio::test]
    async fn a_query_on_the_base_url_stays_after_the_path() {
        assert_eq!(
            endpoint("https://x.openai.azure.com/openai/v1", "/audio/speech"),
            "https://x.openai.azure.com/openai/v1/audio/speech"
        );
        assert_eq!(
            endpoint(
                "https://x/openai/deployments/stt/?api-version=2025-03-01-preview",
                "/audio/transcriptions"
            ),
            "https://x/openai/deployments/stt/audio/transcriptions?api-version=2025-03-01-preview"
        );
        let (base, seen) = stub("ok").await;
        let a = adapter(
            &format!("{base}/deployments/stt?api-version=2025-03-01-preview"),
            Duration::from_secs(5),
        );
        let text = a
            .transcribe(b"x".to_vec(), "audio/webm", None)
            .await
            .unwrap();
        assert_eq!(text, "why this seam?");
        assert_eq!(
            seen.lock().unwrap().query.as_deref(),
            Some("api-version=2025-03-01-preview"),
            "the stub answers only on the deployment path, and saw the query"
        );
        println!("checked 2 URLs and 1 call with a query on the base");
    }

    #[tokio::test]
    async fn synthesis_sends_model_input_voice_and_format_and_returns_the_bytes() {
        let (base, seen) = stub("ok").await;
        let a = adapter(&base, Duration::from_secs(5));
        let clip = a
            .synthesize("Because the seam is owned there.")
            .await
            .unwrap();
        assert_eq!(clip.bytes, b"ID3clip");
        assert_eq!(clip.mime, "audio/mpeg");
        let v = seen.lock().unwrap().speech.clone().unwrap();
        assert_eq!(
            v,
            serde_json::json!({"model": "kokoro", "input": "Because the seam is owned there.", "voice": "af_heart", "response_format": "mp3"})
        );
    }

    #[tokio::test]
    async fn each_failure_maps_to_its_error() {
        let (base, _) = stub("500").await;
        let e = adapter(&base, Duration::from_secs(5))
            .transcribe(b"x".to_vec(), "audio/webm", None)
            .await
            .unwrap_err();
        assert_eq!(e, SpeechError::Status(500));
        let (base, _) = stub("bad").await;
        let e = adapter(&base, Duration::from_secs(5))
            .transcribe(b"x".to_vec(), "audio/webm", None)
            .await
            .unwrap_err();
        assert_eq!(e.outcome(), "bad_body");
        let (base, _) = stub("slow").await;
        let e = adapter(&base, Duration::from_millis(200))
            .transcribe(b"x".to_vec(), "audio/webm", None)
            .await
            .unwrap_err();
        assert_eq!(e.outcome(), "timeout");
        let (base, _) = stub("big").await;
        let e = adapter(&base, Duration::from_secs(5))
            .synthesize("hi")
            .await
            .unwrap_err();
        assert_eq!(e.outcome(), "bad_body", "a body over 4 MiB is refused: {e}");
        let e = adapter("http://127.0.0.1:9/v1", Duration::from_secs(5))
            .synthesize("hi")
            .await
            .unwrap_err();
        assert_eq!(e.outcome(), "transport");
        println!("checked 5 provider failures against their errors");
    }

    #[tokio::test]
    async fn the_fake_echoes_text_both_ways() {
        let f = FakeSpeech;
        assert_eq!(
            f.transcribe(b" @m0x6a-agent hi ".to_vec(), "audio/webm", None)
                .await
                .unwrap(),
            "@m0x6a-agent hi"
        );
        assert_eq!(f.synthesize("hi").await.unwrap().bytes, b"hi");
        assert_eq!(
            f.transcribe(vec![0xff, 0xfe], "audio/webm", None)
                .await
                .unwrap_err()
                .outcome(),
            "bad_body"
        );
    }
}
