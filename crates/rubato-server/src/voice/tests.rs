//! A plan room's voice channel (ADR 0073, plan 146 "How we'll know it works"): the token's
//! claims, the flag, checks before any transcription, an utterance becoming a spoken room message
//! and a turn asked aloud, a repeat posting once, the provider gate, speech only for replies to
//! spoken turns with one synthesis per message, the clip cache's bounds, no audio stored, failed
//! joins counted, and no process started by this slice.

use super::gate::Clips;
use super::token::{base64url, base64url_decode, sign};
use super::{join_failed, speech, utterance, voice_token};
use crate::auth::CurrentUser;
use crate::kernel::config::{Config, LiveKitConfig, Secret, SpeechConfig, VoiceConfig};
use crate::kernel::db::{connect, ts};
use crate::kernel::github::FakeGitHub;
use crate::kernel::speech::{Speech, SpeechClip, SpeechError};
use crate::kernel::state::{App, AppState, LoadedMap};
use crate::kernel::telemetry::metric_reader;
use crate::{plan_drafts, rooms, session_messages};
use async_trait::async_trait;
use axum::Json;
use axum::body::Body;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::IntoResponse;
use chrono::Utc;
use rubato_protocol::otel::{attr, metric};
use rubato_protocol::voice::{limits, reason, routes};
use rubato_protocol::{
    CallInRequest, DraftBundle, DraftFile, IssueRef, LaunchCommand, RoomAuthorKind, RoomKey,
    RoomMessage, RoomReply, SystemMap, UtteranceQuery, VoiceJoinFailed, VoiceTokenRequest,
    bundle_hash, capability,
};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use uuid::Uuid;

const REPO: &str = "m0x6A/rubato";
const N: u64 = 146;
const BRANCH: &str = "issue/146-voice";
const OWNER: &str = "m0x6A";
const HANDLE: &str = "m0x6a-agent";
const SECRET: &str = "0123456789abcdef0123456789abcdef";

/// A fake provider that counts its calls, answers a fixed transcript, and can be held.
#[derive(Default)]
struct Counting {
    stt: AtomicUsize,
    tts: AtomicUsize,
    transcript: String,
    hold: Option<Arc<tokio::sync::Notify>>,
}

#[async_trait]
impl Speech for Counting {
    fn name(&self) -> &'static str {
        "fake"
    }

    async fn transcribe(
        &self,
        _audio: Vec<u8>,
        _mime: &str,
        _language: Option<&str>,
    ) -> Result<String, SpeechError> {
        self.stt.fetch_add(1, Ordering::SeqCst);
        if let Some(h) = &self.hold {
            h.notified().await;
        }
        Ok(self.transcript.clone())
    }

    async fn synthesize(&self, text: &str) -> Result<SpeechClip, SpeechError> {
        self.tts.fetch_add(1, Ordering::SeqCst);
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        Ok(SpeechClip {
            bytes: format!("clip:{text}").into_bytes(),
            mime: "audio/mpeg".into(),
        })
    }
}

struct H {
    app: App,
    reader: metric_reader::Reader,
    fake: Arc<Counting>,
    tmp: tempfile::TempDir,
}

fn key() -> RoomKey {
    RoomKey {
        repo: REPO.into(),
        number: N,
        branch: BRANCH.into(),
    }
}

fn voice_config() -> VoiceConfig {
    VoiceConfig {
        livekit: Some(LiveKitConfig {
            url: "wss://livekit.example".into(),
            api_key: "APIkey".into(),
            api_secret: Secret::new(SECRET),
        }),
        speech: Some(SpeechConfig::Fake),
    }
}

async fn harness_with(voice: Option<VoiceConfig>, transcript: &str) -> H {
    let tmp = tempfile::tempdir().unwrap();
    let url = format!("sqlite://{}/t.db?mode=rwc", tmp.path().display());
    let pool = connect(&url).await.unwrap();
    let (reader, metrics) = metric_reader::start();
    let mut config = Config::for_tests(url, OWNER);
    config.voice = voice;
    let mut app = AppState::new(pool, config, Arc::new(FakeGitHub::new()));
    Arc::get_mut(&mut app).unwrap().metrics = metrics;
    let text = include_str!("../../../../docs/design/rubato-map.yml");
    let map: SystemMap = serde_yaml_ng::from_str(text).unwrap();
    app.set_map(
        &crate::kernel::tenancy::WsKey::of_default(rubato_protocol::DEFAULT_WORKSPACE),
        LoadedMap {
            map,
            version: "v".into(),
            loaded_at: Utc::now(),
            yaml: text.into(),
        },
    )
    .await;
    app.clock.set(Utc::now());
    let fake = Arc::new(Counting {
        transcript: transcript.into(),
        ..Default::default()
    });
    if app.speech().is_some() {
        app.set_speech(Some(fake.clone()));
    }
    H {
        app,
        reader,
        fake,
        tmp,
    }
}

async fn harness() -> H {
    harness_with(Some(voice_config()), "why this seam?").await
}

/// A draft on the branch, so the room exists.
async fn draft(h: &H) {
    let path = format!("docs/plans/{N}.md");
    let text = "---\nissue: m0x6A/rubato#146\n---\n# Plan\n".to_owned();
    plan_drafts::on_draft(
        &h.app,
        &DraftBundle {
            launch_id: Uuid::from_u128(1),
            session_id: None,
            issue: IssueRef::new(REPO, N),
            branch: BRANCH.into(),
            bundle_hash: bundle_hash(&path, &[(path.clone(), text.clone())]),
            files: vec![DraftFile { path, text }],
            missing: vec![],
            refused: None,
            at: Utc::now(),
        },
        OWNER,
    )
    .await;
}

/// The owner's launch on the branch, called into the room. Returns (launch, companion).
async fn agent_in(h: &H) -> (Uuid, Uuid) {
    let companion = Uuid::new_v4();
    let id = Uuid::new_v4();
    let cmd = LaunchCommand {
        id,
        user: OWNER.into(),
        issue: IssueRef::new(REPO, N),
        repo_url: "https://github.com/m0x6A/rubato.git".into(),
        branch: BRANCH.into(),
        base_ref: "origin/main".into(),
        node_ids: vec![],
        spend_cap_usd: 10.0,
        attempt_limit: 3,
        attempt: 1,
        session_id: Uuid::nil(),
        prompt: "p".into(),
        otel_resource: Default::default(),
        map_version: None,
        plan_path: Some(format!("docs/plans/{N}.md")),
        draft: None,
        planning: Default::default(),
        role: Default::default(),
        lead_launch_id: None,
    };
    sqlx::query("INSERT INTO launches (tenant_id, workspace_id, id, companion_id, user, repo, number, command, created_at) VALUES ('default', 'default', ?, ?, ?, ?, ?, ?, ?)")
        .bind(id.to_string())
        .bind(companion.to_string())
        .bind(OWNER)
        .bind(REPO)
        .bind(N as i64)
        .bind(serde_json::to_string(&cmd).unwrap())
        .bind(ts(Utc::now()))
        .execute(&h.app.db)
        .await
        .unwrap();
    h.app.set_capabilities(
        companion,
        [
            capability::SESSION_INPUT,
            capability::HOOK_ALLOWLIST,
            capability::ROOM_TURNS,
        ]
        .into_iter()
        .map(str::to_owned)
        .collect(),
    );
    let _joined = rooms::call_in(
        State(h.app.clone()),
        CurrentUser(OWNER.into()),
        Json(CallInRequest {
            repo: REPO.into(),
            number: N,
            branch: BRANCH.into(),
            launch_id: id,
        }),
    )
    .await
    .unwrap();
    (id, companion)
}

type Said = Result<(StatusCode, RoomMessage), (StatusCode, String)>;

async fn say(h: &H, user: &str, id: Uuid, to: Option<&str>, audio: Vec<u8>) -> Said {
    let mut headers = HeaderMap::new();
    headers.insert(
        header::CONTENT_TYPE,
        "audio/webm;codecs=opus".parse().unwrap(),
    );
    utterance(
        State(h.app.clone()),
        crate::kernel::tenancy::default_scope(),
        CurrentUser(user.into()),
        Query(UtteranceQuery {
            repo: REPO.into(),
            number: N,
            branch: BRANCH.into(),
            id,
            to: to.map(str::to_owned),
        }),
        headers,
        Body::from(audio),
    )
    .await
    .map(|(s, j)| (s, j.0))
    .map_err(|(s, j)| (s, j.0["reason"].as_str().unwrap_or("").to_owned()))
}

async fn hear(h: &H, id: Uuid) -> Result<(StatusCode, Vec<u8>), (StatusCode, String)> {
    match speech(
        State(h.app.clone()),
        crate::kernel::tenancy::default_scope(),
        CurrentUser("carol".into()),
        Path(id.into()),
    )
    .await
    {
        Ok(res) => {
            let res = res.into_response();
            let status = res.status();
            let bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
                .await
                .unwrap()
                .to_vec();
            Ok((status, bytes))
        }
        Err((s, j)) => Err((s, j.0["reason"].as_str().unwrap_or("").to_owned())),
    }
}

fn rejected(h: &H, route: &str, why: &str) -> u64 {
    h.reader
        .points(metric::WEB_REJECTED)
        .iter()
        .filter(|p| {
            p.attrs.get(attr::DROP_REASON).map(String::as_str) == Some(why)
                && p.attrs.get(attr::ROUTE).map(String::as_str) == Some(route)
        })
        .map(|p| p.value)
        .sum()
}

/// The room turns queued for `launch`: (session message id, turn text).
async fn turns(h: &H, launch: Uuid) -> Vec<(Uuid, String)> {
    session_messages::list(
        State(h.app.clone()),
        crate::kernel::tenancy::default_scope(),
        CurrentUser(OWNER.into()),
        Path(launch.into()),
    )
    .await
    .unwrap()
    .0
    .messages
    .into_iter()
    .filter(|v| v.kind == "room_turn")
    .map(|v| (v.id, v.text))
    .collect()
}

/// The agent answers the turn carried by session message `msg`; returns its room message id.
async fn agent_answers(h: &H, companion: Uuid, msg: Uuid, text: &str) -> Uuid {
    let t = session_messages::room_turn(&h.app.db, msg)
        .await
        .unwrap()
        .unwrap();
    assert!(
        rooms::on_room_reply(
            &h.app,
            companion,
            &RoomReply {
                message_id: t.message_id,
                turn_id: t.turn_id,
                text: text.into(),
                fork_session_id: None,
                cost_usd: None,
                error: None,
                at: Utc::now(),
            },
        )
        .await
    );
    let view = rooms::room(
        State(h.app.clone()),
        CurrentUser("carol".into()),
        Query(key()),
    )
    .await
    .unwrap()
    .0;
    view.messages
        .iter()
        .rev()
        .find(|m| m.author_kind == RoomAuthorKind::Agent && m.turn_id == Some(t.turn_id))
        .unwrap()
        .id
}

fn claims(token: &str) -> serde_json::Value {
    let parts: Vec<&str> = token.split('.').collect();
    assert_eq!(parts.len(), 3);
    serde_json::from_slice(&base64url_decode(parts[1]).unwrap()).unwrap()
}

fn verifies(token: &str, secret: &str) -> bool {
    let (signing, sig) = token.rsplit_once('.').unwrap();
    base64url(&sign(secret.as_bytes(), signing.as_bytes())) == sig
}

/// The token's claims (design §2), checked one by one, and a wrong secret fails verification.
#[tokio::test]
async fn the_token_names_the_person_the_room_and_a_ten_minute_join() {
    let h = harness().await;
    draft(&h).await;
    let now = h.app.clock.now();
    let t = voice_token(
        State(h.app.clone()),
        CurrentUser(OWNER.into()),
        Json(VoiceTokenRequest {
            repo: REPO.into(),
            number: N,
            branch: BRANCH.into(),
        }),
    )
    .await
    .unwrap()
    .0;
    assert_eq!(t.url, "wss://livekit.example");
    assert!(verifies(&t.token, SECRET), "signed with the LiveKit secret");
    assert!(
        !verifies(&t.token, "another-secret-of-thirty-two-chars!"),
        "a wrong secret must fail verification"
    );
    let header: serde_json::Value =
        serde_json::from_slice(&base64url_decode(t.token.split('.').next().unwrap()).unwrap())
            .unwrap();
    assert_eq!(header["alg"], "HS256");
    let c = claims(&t.token);
    assert_eq!(c["iss"], "APIkey");
    assert_eq!(c["name"], OWNER);
    let sub = c["sub"].as_str().unwrap();
    assert!(
        sub.starts_with("m0x6A#") && sub.len() == "m0x6A#".len() + 8,
        "{sub}"
    );
    assert_eq!(sub, t.identity);
    assert_eq!(c["nbf"], now.timestamp());
    assert_eq!(c["exp"], now.timestamp() + limits::TOKEN_SECS);
    assert_eq!(
        t.expires_at.timestamp(),
        now.timestamp() + limits::TOKEN_SECS
    );
    let v = &c["video"];
    assert_eq!(v["room"], super::room_name(&key()));
    assert_eq!(t.room, super::room_name(&key()));
    assert_eq!(v["roomJoin"], true);
    assert_eq!(v["canSubscribe"], true);
    assert_eq!(v["canPublish"], true);
    assert_eq!(v["canPublishData"], false);
    assert_eq!(v["canPublishSources"], serde_json::json!(["microphone"]));
    // Two tabs of one person get two identities.
    let again = voice_token(
        State(h.app.clone()),
        CurrentUser(OWNER.into()),
        Json(VoiceTokenRequest {
            repo: REPO.into(),
            number: N,
            branch: BRANCH.into(),
        }),
    )
    .await
    .unwrap()
    .0;
    assert_ne!(again.identity, t.identity);
    // No plan or draft on a branch: no room, no token.
    let e = voice_token(
        State(h.app.clone()),
        CurrentUser(OWNER.into()),
        Json(VoiceTokenRequest {
            repo: REPO.into(),
            number: N,
            branch: "issue/146-none".into(),
        }),
    )
    .await
    .unwrap_err();
    assert_eq!(e.0, StatusCode::NOT_FOUND);
    assert_eq!(rejected(&h, routes::VOICE_TOKEN, "no_plan"), 1);
}

/// With `RUBATO_VOICE` off every route answers 404 `voice_off`, counted under its own route, and
/// with voice on but no LiveKit the token route answers `no_livekit`.
#[tokio::test]
async fn every_voice_route_is_off_by_default() {
    let h = harness_with(None, "x").await;
    draft(&h).await;
    assert!(h.app.speech().is_none());
    let tok = voice_token(
        State(h.app.clone()),
        CurrentUser(OWNER.into()),
        Json(VoiceTokenRequest {
            repo: REPO.into(),
            number: N,
            branch: BRANCH.into(),
        }),
    )
    .await
    .unwrap_err();
    assert_eq!(tok.0, StatusCode::NOT_FOUND);
    assert_eq!(
        say(&h, OWNER, Uuid::new_v4(), None, b"x".to_vec())
            .await
            .unwrap_err(),
        (StatusCode::NOT_FOUND, reason::VOICE_OFF.into())
    );
    assert_eq!(
        hear(&h, Uuid::new_v4()).await.unwrap_err(),
        (StatusCode::NOT_FOUND, reason::VOICE_OFF.into())
    );
    let jf = join_failed(
        State(h.app.clone()),
        CurrentUser(OWNER.into()),
        Body::from("{}"),
    )
    .await
    .unwrap_err();
    assert_eq!(jf.0, StatusCode::NOT_FOUND);
    let routes_checked = [
        routes::VOICE_TOKEN,
        routes::VOICE_UTTERANCE,
        routes::VOICE_SPEECH,
        routes::VOICE_JOIN_FAILED,
    ];
    for r in routes_checked {
        assert_eq!(rejected(&h, r, reason::VOICE_OFF), 1, "{r}");
    }
    println!("checked {} voice routes off", routes_checked.len());

    let h = harness_with(
        Some(VoiceConfig {
            livekit: None,
            speech: None,
        }),
        "x",
    )
    .await;
    draft(&h).await;
    let e = voice_token(
        State(h.app.clone()),
        CurrentUser(OWNER.into()),
        Json(VoiceTokenRequest {
            repo: REPO.into(),
            number: N,
            branch: BRANCH.into(),
        }),
    )
    .await
    .unwrap_err();
    assert_eq!(e.0, StatusCode::CONFLICT);
    assert_eq!(e.1.0["reason"], reason::NO_LIVEKIT);
    assert_eq!(
        say(&h, OWNER, Uuid::new_v4(), None, b"x".to_vec())
            .await
            .unwrap_err(),
        (StatusCode::CONFLICT, reason::NO_SPEECH.into())
    );
}

/// An utterance to an agent becomes `@handle transcript`, posted as the speaker and marked
/// spoken, with a turn whose text says it was asked aloud. The same id again answers the message
/// and makes no second turn and no second transcription.
#[tokio::test]
async fn an_utterance_becomes_a_spoken_message_and_a_turn_asked_aloud() {
    let h = harness().await;
    draft(&h).await;
    let (launch, _) = agent_in(&h).await;
    let id = Uuid::new_v4();
    let (status, m) = say(&h, "carol", id, Some(HANDLE), b"opus".to_vec())
        .await
        .unwrap();
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(m.id, id);
    assert_eq!(m.text, format!("@{HANDLE} why this seam?"));
    assert_eq!(m.author, "carol");
    assert!(m.spoken);
    assert!(m.turn_id.is_some());
    let t = turns(&h, launch).await;
    assert_eq!(t.len(), 1);
    assert!(t[0].1.contains(rooms::ASKED_ALOUD), "{}", t[0].1);
    let (status, again) = say(&h, "carol", id, Some(HANDLE), b"opus".to_vec())
        .await
        .unwrap();
    assert_eq!(status, StatusCode::OK);
    assert_eq!(again, m);
    assert_eq!(
        turns(&h, launch).await.len(),
        1,
        "a repeat makes no second turn"
    );
    assert_eq!(
        h.fake.stt.load(Ordering::SeqCst),
        1,
        "nor a second transcription"
    );
    // The same id from someone else is refused.
    assert_eq!(
        say(&h, "dave", id, None, b"opus".to_vec())
            .await
            .unwrap_err(),
        (StatusCode::UNPROCESSABLE_ENTITY, reason::ID_REUSED.into())
    );
    // Spoken to the room, not to the agent: no turn.
    let (_, m) = say(&h, "carol", Uuid::new_v4(), None, b"opus".to_vec())
        .await
        .unwrap();
    assert!(m.spoken && m.turn_id.is_none());
    assert_eq!(turns(&h, launch).await.len(), 1);
}

/// Refusals that cost no transcription: no plan, an agent not in the room, too long. An empty
/// transcript posts nothing.
#[tokio::test]
async fn cheap_checks_come_before_any_transcription() {
    let h = harness().await;
    // No draft yet: no room.
    assert_eq!(
        say(&h, OWNER, Uuid::new_v4(), None, b"x".to_vec())
            .await
            .unwrap_err(),
        (StatusCode::NOT_FOUND, "no_plan".into())
    );
    draft(&h).await;
    assert_eq!(
        say(&h, OWNER, Uuid::new_v4(), Some("bob-agent"), b"x".to_vec())
            .await
            .unwrap_err(),
        (
            StatusCode::UNPROCESSABLE_ENTITY,
            reason::UNKNOWN_AGENT.into()
        )
    );
    let big = vec![0u8; limits::UTTERANCE_MAX_BYTES + 1];
    assert_eq!(
        say(&h, OWNER, Uuid::new_v4(), None, big).await.unwrap_err(),
        (StatusCode::PAYLOAD_TOO_LARGE, "too_long".into())
    );
    assert_eq!(rejected(&h, routes::VOICE_UTTERANCE, "too_long"), 1);
    assert_eq!(rejected(&h, routes::VOICE_UTTERANCE, "no_plan"), 1);
    assert_eq!(
        rejected(&h, routes::VOICE_UTTERANCE, reason::UNKNOWN_AGENT),
        1
    );
    assert_eq!(
        h.fake.stt.load(Ordering::SeqCst),
        0,
        "no transcription for any refusal"
    );
    // Exactly the limit is taken.
    let fits = vec![b'a'; limits::UTTERANCE_MAX_BYTES];
    assert!(say(&h, OWNER, Uuid::new_v4(), None, fits).await.is_ok());

    let h = harness_with(Some(voice_config()), "   ").await;
    draft(&h).await;
    assert_eq!(
        say(&h, OWNER, Uuid::new_v4(), None, b"x".to_vec())
            .await
            .unwrap_err(),
        (StatusCode::UNPROCESSABLE_ENTITY, "empty".into())
    );
}

/// At most two provider calls at once: a third waits two seconds, then answers 503 `busy`.
#[tokio::test]
async fn a_third_concurrent_call_is_busy() {
    let mut h = harness().await;
    draft(&h).await;
    let hold = Arc::new(tokio::sync::Notify::new());
    let held = Arc::new(Counting {
        transcript: "held".into(),
        hold: Some(hold.clone()),
        ..Default::default()
    });
    h.fake = held.clone();
    h.app.set_speech(Some(held.clone()));
    let h = Arc::new(h);
    let mut tasks = Vec::new();
    for _ in 0..limits::PROVIDER_CONCURRENCY {
        let h = h.clone();
        tasks.push(tokio::spawn(async move {
            say(&h, OWNER, Uuid::new_v4(), None, b"x".to_vec()).await
        }));
    }
    while held.stt.load(Ordering::SeqCst) < limits::PROVIDER_CONCURRENCY {
        tokio::task::yield_now().await;
    }
    let third = say(&h, OWNER, Uuid::new_v4(), None, b"x".to_vec()).await;
    assert_eq!(
        third.unwrap_err(),
        (StatusCode::SERVICE_UNAVAILABLE, reason::BUSY.into())
    );
    hold.notify_waiters();
    for t in tasks {
        assert!(t.await.unwrap().is_ok());
    }
    let busy: u64 = h
        .reader
        .points(metric::VOICE_SPEECH)
        .iter()
        .filter(|p| p.attrs.get(attr::OUTCOME).map(String::as_str) == Some("busy"))
        .map(|p| p.value)
        .sum();
    assert_eq!(busy, 1);
}

/// Only a reply to a spoken turn is spoken, and two listeners make one synthesis. A reply to a
/// typed turn, and a person's message, never reach the provider.
#[tokio::test]
async fn only_replies_to_spoken_turns_are_spoken_once() {
    let h = harness().await;
    draft(&h).await;
    let (launch, companion) = agent_in(&h).await;
    say(&h, "carol", Uuid::new_v4(), Some(HANDLE), b"opus".to_vec())
        .await
        .unwrap();
    let t = turns(&h, launch).await;
    let reply = agent_answers(
        &h,
        companion,
        t[0].0,
        "Because the seam is owned there.\n\n```\ncode\n```\n\nMore in the room.",
    )
    .await;
    let h = Arc::new(h);
    let (a, b) = tokio::join!(hear(&h, reply), hear(&h, reply));
    let (sa, ba) = a.unwrap();
    let (_, bb) = b.unwrap();
    assert_eq!(sa, StatusCode::OK);
    assert_eq!(ba, bb);
    assert_eq!(
        String::from_utf8(ba).unwrap(),
        "clip:Because the seam is owned there. More in the room."
    );
    assert_eq!(
        h.fake.tts.load(Ordering::SeqCst),
        1,
        "one synthesis for two listeners"
    );

    // A typed question's reply is not spoken. One second later, so its turn sorts after.
    h.app
        .clock
        .set(h.app.clock.now() + chrono::Duration::seconds(1));
    rooms::post_as(
        &h.app,
        "carol",
        &key(),
        &format!("@{HANDLE} and typed?"),
        rooms::PostSource::Typed,
    )
    .await
    .unwrap();
    let t = turns(&h, launch).await;
    assert_eq!(t.len(), 2);
    let typed_reply = agent_answers(&h, companion, t[1].0, "Typed answer.").await;
    assert_eq!(
        hear(&h, typed_reply).await.unwrap_err(),
        (StatusCode::NOT_FOUND, reason::NOT_SPOKEN.into())
    );
    // A person's message, and an unknown id, are not spoken either.
    let (_, mine) = say(&h, "carol", Uuid::new_v4(), None, b"x".to_vec())
        .await
        .unwrap();
    assert_eq!(hear(&h, mine.id).await.unwrap_err().1, reason::NOT_SPOKEN);
    assert_eq!(
        hear(&h, Uuid::new_v4()).await.unwrap_err().1,
        reason::NOT_SPOKEN
    );
    assert_eq!(
        h.fake.tts.load(Ordering::SeqCst),
        1,
        "no provider call for any of them"
    );
}

/// The clip cache keeps at most 32 clips, each for 10 minutes, in memory.
#[tokio::test]
async fn the_clip_cache_is_bounded_in_size_and_age() {
    let h = harness().await;
    let speech = h.app.speech().unwrap();
    let ids: Vec<Uuid> = (0..=limits::CLIP_CACHE_ENTRIES)
        .map(|_| Uuid::new_v4())
        .collect();
    for (i, id) in ids.iter().enumerate() {
        h.app
            .clock
            .set(Utc::now() + chrono::Duration::milliseconds(i as i64));
        super::gate::clip(&h.app, &speech, "r", *id, "hi")
            .await
            .unwrap();
    }
    let clips = h.app.slots.get::<Clips>();
    assert_eq!(clips.len(), limits::CLIP_CACHE_ENTRIES);
    assert!(!clips.contains(ids[0]), "the oldest went first");
    assert!(clips.contains(ids[limits::CLIP_CACHE_ENTRIES]));
    h.app
        .clock
        .set(Utc::now() + chrono::Duration::seconds(limits::CLIP_CACHE_SECS + 1));
    super::gate::clip(&h.app, &speech, "r", Uuid::new_v4(), "hi")
        .await
        .unwrap();
    assert_eq!(clips.len(), 1, "every clip older than 10 minutes is gone");
}

/// No audio is stored: the utterance's bytes are in no table and no file Rubato wrote.
#[tokio::test]
async fn no_audio_is_stored() {
    let h = harness().await;
    draft(&h).await;
    let (launch, companion) = agent_in(&h).await;
    let marker = b"AUDIO-MARKER-7f3a".to_vec();
    say(&h, "carol", Uuid::new_v4(), Some(HANDLE), marker.clone())
        .await
        .unwrap();
    let t = turns(&h, launch).await;
    let reply = agent_answers(&h, companion, t[0].0, "Spoken back.").await;
    hear(&h, reply).await.unwrap();
    // Every file under the data dir, the database and its journal included.
    let mut files = 0;
    for entry in std::fs::read_dir(h.tmp.path()).unwrap() {
        let p = entry.unwrap().path();
        let bytes = std::fs::read(&p).unwrap();
        files += 1;
        assert!(
            !bytes.windows(marker.len()).any(|w| w == marker.as_slice()),
            "audio found in {}",
            p.display()
        );
        assert!(
            !bytes.windows(10).any(|w| w == b"clip:Spoke"),
            "a clip found in {}",
            p.display()
        );
    }
    println!("checked {files} files for audio");
    assert!(files >= 1);
}

/// A failed join is counted by its reason (unknown ones as `other`), at most once a minute per
/// person and room, and a body over 1 KiB is refused.
#[tokio::test]
async fn failed_joins_are_counted_by_reason() {
    let h = harness().await;
    let report = |reason: &str| {
        serde_json::to_vec(&VoiceJoinFailed {
            repo: REPO.into(),
            number: N,
            branch: BRANCH.into(),
            reason: reason.into(),
        })
        .unwrap()
    };
    let send = |user: &'static str, body: Vec<u8>| {
        join_failed(
            State(h.app.clone()),
            CurrentUser(user.into()),
            Body::from(body),
        )
    };
    assert_eq!(
        send("carol", report("ice")).await.unwrap(),
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        send("carol", report("ice")).await.unwrap(),
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        send("dave", report("udp blocked")).await.unwrap(),
        StatusCode::NO_CONTENT
    );
    let big = vec![b' '; limits::JOIN_FAILED_MAX_BYTES + 1];
    assert_eq!(
        send("erin", big).await.unwrap_err().0,
        StatusCode::PAYLOAD_TOO_LARGE
    );
    let by = |why: &str| -> u64 {
        h.reader
            .points(metric::VOICE_JOIN_FAILED)
            .iter()
            .filter(|p| p.attrs.get(attr::DROP_REASON).map(String::as_str) == Some(why))
            .map(|p| p.value)
            .sum()
    };
    assert_eq!(
        by("ice"),
        1,
        "the second report within a minute is not counted"
    );
    assert_eq!(by("other"), 1);
}

/// Words that would start a process (ADR 0073: Rubato never starts LiveKit).
const BANNED: [&str; 3] = ["std::process", "tokio::process", "Command"];

/// The files that may not start a process, and the banned words each names.
fn spawners(files: &[(&str, &str)]) -> Vec<String> {
    let mut out = Vec::new();
    for (name, text) in files {
        // Only code: drop this test module's own list.
        let code: String = text
            .lines()
            .filter(|l| !l.trim_start().starts_with("//") && !l.contains("const BANNED"))
            .collect::<Vec<_>>()
            .join("\n");
        for b in BANNED {
            if code.contains(b) {
                out.push(format!("{name}: {b}"));
            }
        }
    }
    out
}

#[test]
fn the_voice_slice_and_the_speech_port_start_no_process() {
    let files: Vec<(&str, &str)> = vec![
        ("voice/mod.rs", include_str!("mod.rs")),
        ("voice/cut.rs", include_str!("cut.rs")),
        ("voice/gate.rs", include_str!("gate.rs")),
        ("voice/handlers.rs", include_str!("handlers.rs")),
        ("voice/token.rs", include_str!("token.rs")),
        ("kernel/speech.rs", include_str!("../kernel/speech.rs")),
    ];
    let found = spawners(&files);
    println!("read {} files for process starts", files.len());
    assert!(found.is_empty(), "a voice file starts a process: {found:?}");
    // Must fail: a file that spawns.
    let bad = [(
        "fixture.rs",
        "let c = tokio::process::Command::new(\"livekit-server\");",
    )];
    assert_eq!(
        spawners(&bad).len(),
        2,
        "the check must catch a spawning file"
    );
}
